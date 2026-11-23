//! Ekran üstü kartlarımızın yanında çıkan öteki göstergeleri (OSD) gizler.
//!
//! - Kilit tuşları: Logitech Options+ ajanı (`logioptionsplus_agent.exe`) Caps /
//!   Num Lock'ta kendi Qt penceresini açar. Tuşa basıldıktan hemen sonra o
//!   sürecin yeni bir üst düzey penceresi görününce gizlenir.
//! - Parlaklık / ses: kabuğun (explorer) XAML adası penceresi. Tuş yutulduğu
//!   için ses göstergesi normalde çıkmaz; parlaklık göstergesi Fn tuşuyla
//!   (bizden bağımsız) çıkar. Parlaklık / ses değişimi bildirildiği anda (ya da
//!   hemen önce) görünen böyle bir pencere gizlenir.
//!
//! Sistem ayarı değişmez; yalnızca ilgili ayar açıkken, belirli bir pencere
//! belirli bir zaman aralığında görününce `SW_HIDE` uygulanır. Ayar kapanınca
//! olay kancası kaldırılır. Olay kancası `WINEVENT_OUTOFCONTEXT` olduğundan
//! kuran iş parçacığının ileti döngüsü gerekir (kanca iş parçacığı).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_SHOW, GA_ROOT, GetAncestor, GetClassNameW, GetWindowThreadProcessId,
    IsWindowVisible, OBJID_WINDOW, SW_HIDE, ShowWindow, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};
use windows::core::PWSTR;

/// Göstergenin çıkabileceği süre: işaretten sonra bu kadar boyunca yeni pencere gizlenir.
const ARM_FOR: Duration = Duration::from_millis(1500);
/// Parlaklık / ses bildirimi göstergeden biraz sonra gelebilir: bu kadar önce görünen aday da gizlenir.
const LOOKBACK: Duration = Duration::from_millis(800);

const LOGI_EXE: &str = "logioptionsplus_agent.exe";
/// Kabuğun ses / parlaklık göstergesini barındıran pencere sınıfları.
const SHELL_CLASSES: [&str; 2] = ["XamlExplorerHostIslandWindow", "NativeHWNDHost"];
const SHELL_EXES: [&str; 2] = ["explorer.exe", "shellexperiencehost.exe"];

static WANT_LOCK: AtomicBool = AtomicBool::new(false);
static WANT_BRIGHTNESS: AtomicBool = AtomicBool::new(false);
static WANT_VOLUME: AtomicBool = AtomicBool::new(false);
static EPOCH: OnceLock<Instant> = OnceLock::new();
/// Bitiş zamanları (EPOCH'tan ms + 1; 0 = hiç).
static LOCK_UNTIL: AtomicU64 = AtomicU64::new(0);
static BRIGHTNESS_UNTIL: AtomicU64 = AtomicU64::new(0);
static VOLUME_UNTIL: AtomicU64 = AtomicU64::new(0);
/// Yakında görünen kabuk adayları (gösterge bildirimi sonradan gelirse gizlenir).
static RECENT: Mutex<Vec<(isize, Instant)>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
pub enum Kind {
    Lock,
    Brightness,
    Volume,
}

fn now_ms() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

fn until(kind: Kind) -> &'static AtomicU64 {
    match kind {
        Kind::Lock => &LOCK_UNTIL,
        Kind::Brightness => &BRIGHTNESS_UNTIL,
        Kind::Volume => &VOLUME_UNTIL,
    }
}

/// Hangi göstergelerin bastırılacağı (config değişince).
pub fn set_wanted(lock: bool, brightness: bool, volume: bool) {
    WANT_LOCK.store(lock, Ordering::SeqCst);
    WANT_BRIGHTNESS.store(brightness, Ordering::SeqCst);
    WANT_VOLUME.store(volume, Ordering::SeqCst);
}

fn any_wanted() -> bool {
    WANT_LOCK.load(Ordering::SeqCst)
        || WANT_BRIGHTNESS.load(Ordering::SeqCst)
        || WANT_VOLUME.load(Ordering::SeqCst)
}

/// Kartımız az sonra (ya da az önce) çıkıyor: o türün göstergesi gizlensin.
pub fn arm(kind: Kind) {
    let wanted = match kind {
        Kind::Lock => &WANT_LOCK,
        Kind::Brightness => &WANT_BRIGHTNESS,
        Kind::Volume => &WANT_VOLUME,
    };
    if !wanted.load(Ordering::SeqCst) {
        return;
    }
    until(kind).store(now_ms() + ARM_FOR.as_millis() as u64, Ordering::SeqCst);
    if matches!(kind, Kind::Brightness | Kind::Volume) {
        // Gösterge bildirimden önce çıkmış olabilir.
        let recent: Vec<isize> = {
            let mut list = RECENT.lock().unwrap();
            list.retain(|(_, t)| t.elapsed() < LOOKBACK);
            list.drain(..).map(|(h, _)| h).collect()
        };
        for h in recent {
            hide(HWND(h as *mut _));
        }
    }
}

fn hide(hwnd: HWND) {
    unsafe {
        if IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Pencerenin sahibi sürecin exe adı (küçük harf).
fn exe_of(hwnd: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return None;
    }
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_lowercase)
    }
}

unsafe extern "system" fn on_show(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 || hwnd.is_invalid() {
        return;
    }
    // Yalnızca üst düzey pencereler.
    if unsafe { GetAncestor(hwnd, GA_ROOT) } != hwnd {
        return;
    }
    let now = now_ms();
    if WANT_LOCK.load(Ordering::Relaxed)
        && now < LOCK_UNTIL.load(Ordering::Relaxed)
        && class_of(hwnd).starts_with("Qt")
        && exe_of(hwnd).as_deref() == Some(LOGI_EXE)
    {
        hide(hwnd);
        return;
    }
    let brightness = WANT_BRIGHTNESS.load(Ordering::Relaxed);
    let volume = WANT_VOLUME.load(Ordering::Relaxed);
    if !(brightness || volume) {
        return;
    }
    let class = class_of(hwnd);
    if !SHELL_CLASSES.contains(&class.as_str())
        || !exe_of(hwnd).is_some_and(|e| SHELL_EXES.contains(&e.as_str()))
    {
        return;
    }
    let armed = (brightness && now < BRIGHTNESS_UNTIL.load(Ordering::Relaxed))
        || (volume && now < VOLUME_UNTIL.load(Ordering::Relaxed));
    if armed {
        hide(hwnd);
    } else {
        let mut list = RECENT.lock().unwrap();
        list.retain(|(_, t)| t.elapsed() < LOOKBACK);
        list.push((hwnd.0 as isize, Instant::now()));
    }
}

/// Olay kancasının sahibi: gereken ayar açıkken kurulur, kapanınca kaldırılır.
#[derive(Default)]
pub struct Guard {
    hook: Option<HWINEVENTHOOK>,
}

impl Guard {
    /// Kanca iş parçacığında (ileti döngüsü olan) çağrılır.
    pub fn sync(&mut self) {
        match (any_wanted(), self.hook) {
            (true, None) => {
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_OBJECT_SHOW,
                        EVENT_OBJECT_SHOW,
                        None,
                        Some(on_show),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };
                if hook.is_invalid() {
                    eprintln!("[flyout] gösterge olay kancası kurulamadı");
                } else {
                    self.hook = Some(hook);
                }
            }
            (false, Some(hook)) => {
                unsafe {
                    let _ = UnhookWinEvent(hook);
                }
                self.hook = None;
                RECENT.lock().unwrap().clear();
            }
            _ => {}
        }
    }
}
