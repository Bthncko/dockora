//! Ekran üstü kartların olay kaynakları (`win/flyout.rs` gösterir):
//! küresel klavye kancası (medya / ses / kilit tuşları), şarkı değişimi,
//! parlaklık bildirimi.
//!
//! Kanca (`WH_KEYBOARD_LL`) kendi iş parçacığında, ileti döngüsüyle çalışır ve
//! yalnızca kanala yazar; asıl iş (GSMTC, ses, kart) `worker`da. Tuş yutulduysa
//! ama işlem yapılamadıysa tuş `SendInput` ile (işaretli) yeniden gönderilir:
//! hiçbir tuş kaybolmaz. Ayar kapalıyken kanca hiç kurulmaz.
//!
//! Ses tuşları: varsayılan çıkışın ses düzeyi 2'şer puan (Windows'un adımı,
//! `VolumeStepUp/Down`) değiştirilir; tuş yutulduğu için Windows'un kendi ses
//! göstergesi çıkmaz. Medya çalıyorsa (ve `excludeVolumeKeys` kapalıysa) medya
//! kartı (`trigger: "volumeKey"`), yoksa ses kartı gösterilir.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager as _};
use windows::Win32::Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eMultimedia, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerSettingRegisterNotification,
};
use windows::Win32::System::SystemServices::GUID_DEVICE_POWER_POLICY_VIDEO_BRIGHTNESS;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, RegisterHotKey, SendInput, UnregisterHotKey, VIRTUAL_KEY, VK_CAPITAL,
    VK_INSERT, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP,
    VK_NUMLOCK, VK_SCROLL, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIDEV_REMOVE, RIM_TYPEKEYBOARD, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::HC_ACTION;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DEVICE_NOTIFY_CALLBACK, DispatchMessageW, GetMessageW, HHOOK,
    HWND_MESSAGE, KBDLLHOOKSTRUCT, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_HOTKEY, WM_INPUT, WM_KEYUP, WM_SYSKEYUP, WM_TIMER,
};
use windows::core::w;

use super::audio_devices;
use super::media;
use crate::config::{Config, ConfigStore};
use crate::win::flyout::{self, Show};
use crate::win::osd_guard::{self, Guard, Kind};

/// Kendi yeniden gönderdiğimiz tuşların işareti (`dwExtraInfo`); kanca bunlara dokunmaz.
const REPLAY_MAGIC: usize = 0x444F_434B;
const MSG_SYNC: u32 = WM_APP + 41;
const MSG_REPLAY: u32 = WM_APP + 42;
/// `RAWKEYBOARD.Flags`: tuş bırakıldı (`RI_KEY_BREAK`).
const RI_KEY_BREAK: u32 = 1;
/// Kancanın yeniden kurulma aralığı.
const HOOK_REFRESH_MS: u32 = 30_000;
/// Medya komutunun (GSMTC) en uzun bekleme süresi.
const MEDIA_TIMEOUT: Duration = Duration::from_millis(1500);
/// Kullanıcı bu süre içinde sonraki / önceki dediyse şarkı değişimi "sıradaki" kartı çıkarmaz.
const USER_SKIP_WINDOW: Duration = Duration::from_millis(1500);
/// İki "sıradaki" kartı arası en az süre (üst üste meta veri güncellemeleri).
const NEXT_UP_GAP: Duration = Duration::from_millis(1500);
/// Parlaklığı kendimiz değiştirdikten sonra gelen bildirim yok sayılır.
const OWN_BRIGHTNESS_WINDOW: Duration = Duration::from_millis(1500);

enum Event {
    /// Yutulan medya / ses tuşu (sanal tuş kodu).
    Key(u32),
    Brightness(u8),
}

static TX: OnceLock<Sender<Event>> = OnceLock::new();
/// Kilit tuşları ayrı iş parçacığında: medya komutu takılsa da kartları çıkar.
static LOCK_TX: OnceLock<Sender<u32>> = OnceLock::new();
static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);
static WANT_HOOK: AtomicBool = AtomicBool::new(false);
static SWALLOW_MEDIA: AtomicBool = AtomicBool::new(false);
static SWALLOW_VOLUME: AtomicBool = AtomicBool::new(false);
/// İzlenen kilit tuşları (bit: 0 caps, 1 num, 2 scroll, 3 insert).
static LOCK_MASK: AtomicU8 = AtomicU8::new(0);
/// Insert'in kendi açma/kapama durumu (Windows tutmuyor): 0 kapalı, 1 açık.
static INSERT_ON: AtomicU8 = AtomicU8::new(0);
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Son sonraki/önceki isteği (EPOCH'tan ms + 1; 0 = hiç).
static LAST_SKIP: AtomicU64 = AtomicU64::new(0);
static LAST_NEXT_UP: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_OWN_BRIGHTNESS: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_BRIGHTNESS: AtomicU32 = AtomicU32::new(u32::MAX);

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

/// Kullanıcı sonraki / önceki parçaya geçti (tuş ya da dock düğmesi).
pub fn mark_user_skip() {
    LAST_SKIP.store(epoch().elapsed().as_millis() as u64 + 1, Ordering::SeqCst);
}

fn user_skipped_recently() -> bool {
    match LAST_SKIP.load(Ordering::SeqCst) {
        0 => false,
        t => epoch().elapsed().as_millis() as u64 + 1 - t < USER_SKIP_WINDOW.as_millis() as u64,
    }
}

/// Dock'un kendi ses ayarı parlaklığı değiştirdi: Windows bildirimi kart açmasın.
pub fn mark_own_brightness() {
    *LAST_OWN_BRIGHTNESS.lock().unwrap() = Some(Instant::now());
}

/// Şarkı (kendiliğinden) değişti: "sıradaki" kartı. `media.rs` çağırır.
pub fn track_changed(
    app: &AppHandle,
    title: &str,
    artist: &str,
    album: Option<String>,
    art: Option<String>,
    app_id: &str,
) {
    if user_skipped_recently() {
        return;
    }
    {
        let mut last = LAST_NEXT_UP.lock().unwrap();
        if last.is_some_and(|t| t.elapsed() < NEXT_UP_GAP) {
            return;
        }
        *last = Some(Instant::now());
    }
    flyout::show(
        app,
        Show::NextUp {
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.filter(|a| !a.is_empty()),
            art,
            app_id: app_id.to_string(),
        },
        Some(app_id),
    );
}

/// Config değişince kanca / yutma ayarları güncellenir.
pub fn sync(config: &Config) {
    let f = &config.flyouts;
    SWALLOW_MEDIA.store(f.enabled && f.media.enabled, Ordering::SeqCst);
    SWALLOW_VOLUME.store(f.enabled && f.volume.enabled, Ordering::SeqCst);
    let l = &f.lock_keys;
    let on = f.enabled && l.enabled;
    let mask = u8::from(on && l.caps)
        | (u8::from(on && l.num) << 1)
        | (u8::from(on && l.scroll) << 2)
        | (u8::from(on && l.insert) << 3);
    LOCK_MASK.store(mask, Ordering::SeqCst);
    // Parlaklık / ses: Windows'un kendi göstergesinin bu sürümde ayrı penceresi
    // yok (kabuk doğrudan çiziyor), gizlenemez; sınıf tahminiyle Alt+Tab gibi
    // ilgisiz kabuk pencerelerini gizleme riski olduğu için kapalı. Ses tuşları
    // zaten yutulduğu için Windows'un ses göstergesi çıkmaz.
    osd_guard::set_wanted(mask != 0, false, false);
    WANT_HOOK.store(f.enabled, Ordering::SeqCst);
    let tid = HOOK_THREAD.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, MSG_SYNC, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn spawn(app: &AppHandle) {
    let (tx, rx) = mpsc::channel();
    if TX.set(tx).is_err() {
        return;
    }
    epoch();
    let handle = app.clone();
    if let Err(e) = thread::Builder::new()
        .name("flyout-worker".into())
        .spawn(move || worker(handle, rx))
    {
        eprintln!("[flyout] işçi başlatılamadı: {e}");
        return;
    }
    let (lock_tx, lock_rx) = mpsc::channel::<u32>();
    let _ = LOCK_TX.set(lock_tx);
    let handle = app.clone();
    if let Err(e) = thread::Builder::new()
        .name("flyout-lock".into())
        .spawn(move || {
            while let Ok(vk) = lock_rx.recv() {
                handle_lock(&handle, vk);
            }
        })
    {
        eprintln!("[flyout] kilit iş parçacığı başlatılamadı: {e}");
    }
    if let Err(e) = thread::Builder::new()
        .name("flyout-hook".into())
        .spawn(hook_thread)
    {
        eprintln!("[flyout] kanca başlatılamadı: {e}");
    }
    subscribe_brightness();
}

// --- kanca --------------------------------------------------------------------

fn send(event: Event) -> bool {
    TX.get().is_some_and(|tx| tx.send(event).is_ok())
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: HC_ACTION'da lParam geçerli bir KBDLLHOOKSTRUCT'a işaret eder.
        let kb = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if kb.dwExtraInfo != REPLAY_MAGIC {
            let up = matches!(wparam.0 as u32, WM_KEYUP | WM_SYSKEYUP);
            let vk = kb.vkCode;
            if !up && is_skip_key(vk) {
                mark_user_skip();
            }
            let swallow = (is_media_key(vk) && SWALLOW_MEDIA.load(Ordering::Relaxed))
                || (is_volume_key(vk) && SWALLOW_VOLUME.load(Ordering::Relaxed));
            // İş parçacığı yoksa tuş geçsin.
            if swallow && (up || send(Event::Key(vk))) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn vk_is(vk: u32, k: VIRTUAL_KEY) -> bool {
    vk == u32::from(k.0)
}

fn is_skip_key(vk: u32) -> bool {
    vk_is(vk, VK_MEDIA_NEXT_TRACK) || vk_is(vk, VK_MEDIA_PREV_TRACK)
}

fn is_media_key(vk: u32) -> bool {
    MEDIA_KEYS.iter().any(|&k| vk_is(vk, k))
}

fn is_volume_key(vk: u32) -> bool {
    VOLUME_KEYS.iter().any(|&k| vk_is(vk, k))
}

/// Kilit tuşunun `LOCK_MASK` biti; kilit tuşu değilse 0.
fn lock_bit(vk: u32) -> u8 {
    [VK_CAPITAL, VK_NUMLOCK, VK_SCROLL, VK_INSERT]
        .iter()
        .position(|&k| vk_is(vk, k))
        .map_or(0, |i| 1 << i)
}

const MEDIA_KEYS: [VIRTUAL_KEY; 4] = [
    VK_MEDIA_PLAY_PAUSE,
    VK_MEDIA_NEXT_TRACK,
    VK_MEDIA_PREV_TRACK,
    VK_MEDIA_STOP,
];
const VOLUME_KEYS: [VIRTUAL_KEY; 3] = [VK_VOLUME_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE];

fn apply_hook(hook: &mut Option<HHOOK>) {
    let want = WANT_HOOK.load(Ordering::SeqCst);
    unsafe {
        match (want, hook.is_some()) {
            (true, false) => {
                let module = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).ok();
                match SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module, 0) {
                    Ok(h) => *hook = Some(h),
                    Err(e) => eprintln!("[flyout] klavye kancası kurulamadı: {e}"),
                }
            }
            (false, true) => {
                if let Some(h) = hook.take() {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
            _ => {}
        }
    }
}

/// Kanca dışı yollar. `WH_KEYBOARD_LL`, ön plandaki pencere kendi sürecimizin
/// penceresiyken (ayarlar) bu Windows sürümünde hiç çağrılmıyor; o durumda
/// - kilit tuşları ve sonraki/önceki işareti: ham girdi (`WM_INPUT`, arka planda da gelir),
/// - yutulacak medya / ses tuşları: `RegisterHotKey` (kanca çalışıyorsa tuş
///   ona hiç gelmez, ikisi aynı tuşu iki kez işlemez)
///
/// ile yakalanır.
struct Inputs {
    window: HWND,
    raw: bool,
    hotkeys: Vec<u32>,
}

impl Inputs {
    fn new() -> Self {
        // İleti penceresi ham girdinin hedefi (görünmez, yalnızca bu iş parçacığı).
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }
        .unwrap_or_default();
        Self {
            window,
            raw: false,
            hotkeys: Vec::new(),
        }
    }

    fn sync(&mut self) {
        let want_raw = WANT_HOOK.load(Ordering::SeqCst) && !self.window.is_invalid();
        if want_raw != self.raw {
            let dev = RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: if want_raw {
                    RIDEV_INPUTSINK
                } else {
                    RIDEV_REMOVE
                },
                hwndTarget: if want_raw {
                    self.window
                } else {
                    HWND::default()
                },
            };
            match unsafe { RegisterRawInputDevices(&[dev], size_of::<RAWINPUTDEVICE>() as u32) } {
                Ok(()) => self.raw = want_raw,
                Err(e) => eprintln!("[flyout] ham girdi kaydedilemedi: {e}"),
            }
        }
        let mut want: Vec<u32> = Vec::new();
        if SWALLOW_MEDIA.load(Ordering::SeqCst) {
            want.extend(MEDIA_KEYS.iter().map(|k| u32::from(k.0)));
        }
        if SWALLOW_VOLUME.load(Ordering::SeqCst) {
            want.extend(VOLUME_KEYS.iter().map(|k| u32::from(k.0)));
        }
        for &vk in &self.hotkeys {
            if !want.contains(&vk) {
                unsafe {
                    let _ = UnregisterHotKey(None, vk as i32);
                }
            }
        }
        let have = std::mem::take(&mut self.hotkeys);
        for vk in want {
            // Başka uygulama aynı tuşu almışsa kanca yolu yine çalışır.
            if have.contains(&vk)
                || unsafe { RegisterHotKey(None, vk as i32, HOT_KEY_MODIFIERS(0), vk) }.is_ok()
            {
                self.hotkeys.push(vk);
            }
        }
    }

    /// Tuş Windows'a geri verilir; kendi kısayolumuz tuşu yeniden yakalamasın
    /// diye o sırada kayıt kaldırılır.
    fn replay(&mut self, vk: u32) {
        let had = self.hotkeys.contains(&vk);
        if had {
            unsafe {
                let _ = UnregisterHotKey(None, vk as i32);
            }
        }
        replay(vk);
        if had {
            thread::sleep(Duration::from_millis(60));
            if unsafe { RegisterHotKey(None, vk as i32, HOT_KEY_MODIFIERS(0), vk) }.is_err() {
                self.hotkeys.retain(|&k| k != vk);
            }
        }
    }

    fn on_hotkey(&mut self, vk: u32) {
        let swallowed = (is_media_key(vk) && SWALLOW_MEDIA.load(Ordering::SeqCst))
            || (is_volume_key(vk) && SWALLOW_VOLUME.load(Ordering::SeqCst));
        if !swallowed {
            self.replay(vk);
            return;
        }
        if is_skip_key(vk) {
            mark_user_skip();
        }
        if !send(Event::Key(vk)) {
            self.replay(vk);
        }
    }

    fn on_raw(&self, lparam: LPARAM) {
        let mut buf = [0u64; 32];
        let mut size = size_of_val(&buf) as u32;
        let got = unsafe {
            GetRawInputData(
                HRAWINPUT(lparam.0 as *mut _),
                RID_INPUT,
                Some(buf.as_mut_ptr().cast()),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        if got == u32::MAX || (got as usize) < size_of::<RAWINPUTHEADER>() {
            return;
        }
        // SAFETY: tampon RAWINPUT için yeterince büyük ve 8 bayt hizalı.
        let raw = unsafe { &*(buf.as_ptr().cast::<RAWINPUT>()) };
        if raw.header.dwType != RIM_TYPEKEYBOARD.0 {
            return;
        }
        let kb = unsafe { raw.data.keyboard };
        let vk = u32::from(kb.VKey);
        let up = u32::from(kb.Flags) & RI_KEY_BREAK != 0;
        if !up && is_skip_key(vk) {
            mark_user_skip();
        }
        if lock_bit(vk) & LOCK_MASK.load(Ordering::Relaxed) != 0 {
            // Üçüncü taraf kilit göstergesi tuşla birlikte açılır: önce hazırla.
            osd_guard::arm(Kind::Lock);
            if up && let Some(tx) = LOCK_TX.get() {
                let _ = tx.send(vk);
            }
        }
    }
}

fn hook_thread() {
    unsafe {
        let mut msg = MSG::default();
        // İleti kuyruğunu oluştur, sonra iş parçacığı kimliğini duyur.
        let _ = PeekMessageW(&mut msg, None, WM_APP, WM_APP, PM_NOREMOVE);
        let mut inputs = Inputs::new();
        let mut guard = Guard::default();
        let mut hook = None;
        HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
        // Kimlik duyurulmadan önce gelmiş ayar değişikliği kaçmasın.
        apply_hook(&mut hook);
        inputs.sync();
        guard.sync();
        // Windows, geç yanıt veren düşük seviyeli kancaları haber vermeden
        // kaldırabilir: kancayı düzenli olarak yeniden kur.
        let _ = SetTimer(None, 0, HOOK_REFRESH_MS, None);
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                MSG_SYNC => {
                    apply_hook(&mut hook);
                    inputs.sync();
                    guard.sync();
                }
                MSG_REPLAY => inputs.replay(msg.wParam.0 as u32),
                WM_HOTKEY => inputs.on_hotkey(msg.wParam.0 as u32),
                WM_INPUT => inputs.on_raw(msg.lParam),
                WM_TIMER if msg.hwnd.0.is_null() => {
                    if let Some(h) = hook.take() {
                        let _ = UnhookWindowsHookEx(h);
                    }
                    apply_hook(&mut hook);
                }
                _ => {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
        if let Some(h) = hook {
            let _ = UnhookWindowsHookEx(h);
        }
    }
}

/// Yutulan tuşu Windows'a geri verir; kısayol yolundan gelmiş olabilir, bu
/// yüzden kanca iş parçacığı üstlenir.
fn request_replay(vk: u32) {
    let tid = HOOK_THREAD.load(Ordering::SeqCst);
    let posted = tid != 0
        && unsafe { PostThreadMessageW(tid, MSG_REPLAY, WPARAM(vk as usize), LPARAM(0)) }.is_ok();
    if !posted {
        replay(vk);
    }
}

/// Yutulan tuşu Windows'a geri verir (işaretli: kanca atlar).
fn replay(vk: u32) {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk as u16),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: REPLAY_MAGIC,
            },
        },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

// --- işçi ---------------------------------------------------------------------

fn worker(app: AppHandle, rx: mpsc::Receiver<Event>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    while let Ok(event) = rx.recv() {
        match event {
            Event::Key(vk) => {
                if !handle_key(&app, vk) {
                    request_replay(vk);
                }
            }
            Event::Brightness(level) => handle_brightness(&app, level),
        }
    }
}

fn config(app: &AppHandle) -> crate::config::FlyoutsConfig {
    app.try_state::<ConfigStore>()
        .map(|c| c.get().flyouts)
        .unwrap_or_default()
}

/// `false`: işlem yapılamadı, tuş Windows'a verilsin.
fn handle_key(app: &AppHandle, vk: u32) -> bool {
    let cfg = config(app);
    if !cfg.enabled {
        return false;
    }
    let is = |k: VIRTUAL_KEY| vk == u32::from(k.0);
    if is(VK_VOLUME_UP) || is(VK_VOLUME_DOWN) || is(VK_VOLUME_MUTE) {
        cfg.volume.enabled && volume_key(app, vk, &cfg)
    } else {
        cfg.media.enabled && media_key(app, vk)
    }
}

fn media_key(app: &AppHandle, vk: u32) -> bool {
    let Some((session, app_id)) = media::current_session() else {
        return false;
    };
    let op = match VIRTUAL_KEY(vk as u16) {
        VK_MEDIA_PLAY_PAUSE => session.TryTogglePlayPauseAsync(),
        VK_MEDIA_NEXT_TRACK => session.TrySkipNextAsync(),
        VK_MEDIA_PREV_TRACK => session.TrySkipPreviousAsync(),
        _ => session.TryStopAsync(),
    };
    match join_timeout(op) {
        Some(true) => {}
        Some(false) => return false,
        // Oynatıcı yanıt vermedi: işçiyi kilitleme; tuşu tekrar gönderme
        // (komut sonradan işlenirse iki kez olur).
        None => eprintln!("[flyout] medya komutu zaman aşımına uğradı"),
    }
    flyout::show(
        app,
        Show::Media {
            trigger: "mediaKey",
        },
        Some(&app_id),
    );
    true
}

fn volume_key(app: &AppHandle, vk: u32, cfg: &crate::config::FlyoutsConfig) -> bool {
    let Some((level, muted, device)) = step_volume(vk) else {
        return false;
    };
    osd_guard::arm(Kind::Volume);
    // Medya çalıyorsa medya kartı (ses satırıyla), yoksa ses kartı.
    if cfg.media.enabled
        && !cfg.media.exclude_volume_keys
        && let Some((session, app_id)) = media::current_session()
        && media::is_playing(&session)
        && flyout::show(
            app,
            Show::Media {
                trigger: "volumeKey",
            },
            Some(&app_id),
        )
    {
        return true;
    }
    flyout::show(
        app,
        Show::Volume {
            level,
            muted,
            device,
        },
        None,
    );
    true
}

/// Varsayılan çıkışın sesini bir adım değiştirir: (düzey 0–100, sessiz, aygıt adı).
fn step_volume(vk: u32) -> Option<(u8, bool, String)> {
    let enumerator = audio_devices::enumerator()?;
    unsafe {
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
        let none = std::ptr::null();
        match VIRTUAL_KEY(vk as u16) {
            VK_VOLUME_UP => {
                // Windows gibi: ses açılırken sessizden çık.
                if volume.GetMute().ok()?.as_bool() {
                    volume.SetMute(false, none).ok()?;
                }
                volume.VolumeStepUp(none).ok()?;
            }
            VK_VOLUME_DOWN => volume.VolumeStepDown(none).ok()?,
            _ => {
                let muted = volume.GetMute().ok()?.as_bool();
                volume.SetMute(!muted, none).ok()?;
            }
        }
        let level = (volume.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as u8;
        let muted = volume.GetMute().ok()?.as_bool();
        let name = audio_devices::friendly_name(&device).unwrap_or_default();
        Some((level.min(100), muted, name))
    }
}

/// WinRT işlemini en fazla `MEDIA_TIMEOUT` bekler; `None`: zaman aşımı.
fn join_timeout(op: windows::core::Result<windows_future::IAsyncOperation<bool>>) -> Option<bool> {
    let op = op.ok()?;
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("flyout-media-op".into())
        .spawn(move || {
            let _ = tx.send(op.join().unwrap_or(false));
        })
        .ok()?;
    rx.recv_timeout(MEDIA_TIMEOUT).ok()
}

fn handle_lock(app: &AppHandle, vk: u32) {
    let (key, on) = match VIRTUAL_KEY(vk as u16) {
        VK_CAPITAL => ("caps", toggled(VK_CAPITAL)),
        VK_NUMLOCK => ("num", toggled(VK_NUMLOCK)),
        VK_SCROLL => ("scroll", toggled(VK_SCROLL)),
        _ => {
            // Insert'in sistem açık/kapalı durumu yok: kendimiz sayarız.
            let now = INSERT_ON.fetch_xor(1, Ordering::SeqCst) == 0;
            ("insert", now)
        }
    };
    flyout::show(app, Show::LockKey { key, on }, None);
}

/// Tuşun açma/kapama durumu; durum tuş işlendikten sonra güncellenir.
fn toggled(key: VIRTUAL_KEY) -> bool {
    thread::sleep(Duration::from_millis(20));
    unsafe { GetKeyState(i32::from(key.0)) & 1 != 0 }
}

// --- parlaklık ----------------------------------------------------------------

/// Parlaklık değişimi (Fn tuşları) Windows'ta `GUID_DEVICE_POWER_POLICY_VIDEO_BRIGHTNESS`
/// güç ayarı bildirimiyle gelir; yoklama yok. Yalnızca kanala yazar.
unsafe extern "system" fn on_brightness(
    _context: *const core::ffi::c_void,
    _kind: u32,
    setting: *const core::ffi::c_void,
) -> u32 {
    // POWERBROADCAST_SETTING { GUID (16), DataLength u32, Data }.
    if !setting.is_null() {
        let base = setting as *const u8;
        let len = unsafe { std::ptr::read_unaligned(base.add(16) as *const u32) };
        if len >= 4 {
            let level = unsafe { std::ptr::read_unaligned(base.add(20) as *const u32) };
            let _ = send(Event::Brightness(level.min(100) as u8));
        }
    }
    0
}

fn subscribe_brightness() {
    let params: &'static DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
        Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_brightness),
            Context: std::ptr::null_mut(),
        }));
    let mut handle = std::ptr::null_mut();
    let err = unsafe {
        PowerSettingRegisterNotification(
            &GUID_DEVICE_POWER_POLICY_VIDEO_BRIGHTNESS,
            DEVICE_NOTIFY_CALLBACK,
            HANDLE(params as *const _ as *mut _),
            &mut handle,
        )
    };
    if err.0 != 0 {
        eprintln!("[flyout] parlaklık bildirimi kaydedilemedi: {}", err.0);
    }
}

fn handle_brightness(app: &AppHandle, level: u8) {
    osd_guard::arm(Kind::Brightness);
    if LAST_OWN_BRIGHTNESS
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < OWN_BRIGHTNESS_WINDOW)
    {
        LAST_BRIGHTNESS.store(u32::from(level), Ordering::SeqCst);
        return;
    }
    // Aynı değer tekrar gelirse (ya da ilk bildirim aynıysa) kart çıkmaz.
    if LAST_BRIGHTNESS.swap(u32::from(level), Ordering::SeqCst) == u32::from(level) {
        return;
    }
    flyout::show(app, Show::Brightness { level }, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_window() {
        assert!(!user_skipped_recently() || LAST_SKIP.load(Ordering::SeqCst) != 0);
        mark_user_skip();
        assert!(user_skipped_recently());
    }
}
