//! Kendi bildirimlerimiz: uygulama bildirimleri dock tarzı kartlarla.
//!
//! - Okuma: `UserNotificationListener` (resmi API). Belgeye göre paketli
//!   uygulama + `userNotificationListener` yeteneği ister; bu makinede paketsiz
//!   süreçte de erişim "Allowed" döndü. Olay (`NotificationChanged`) paketsizde
//!   çalışmayabilir: kayıt başarısızsa kısa aralıkla yoklanır.
//! - Windows'un kutucuğu: uygulama başına `ShowBanner = 0`
//!   (HKCU\…\Notifications\Settings\<AUMID>). Bildirim merkezi ve ses kalır.
//!   Önceki değerler diske yazılır; ayar kapanınca ve dock kapanırken geri
//!   yüklenir (dock çalışmıyorken Windows'un kutucukları görünsün).
//! - Kart yalnızca kutucuğunu bizim kapattığımız uygulamalar için çıkar: ilk kez
//!   görülen uygulamanın ilk bildirimini Windows gösterir, sonrakileri biz.
//! - Windows'un kendi sistem bildirimleri (`Windows.SystemToast.*`: düğmeli
//!   onaylar, pil uyarısı) dokunulmadan Windows'ta kalır.
//! - Ses: kendi bildirim seslerimizden biri seçiliyse (`notify_sound`), kartını
//!   gösterdiğimiz uygulamaların Windows'taki sesi kapatılır ve kart çıkınca
//!   seçili ses çalınır. Gerekçe: WhatsApp gibi uygulamalar bildirimde kendi ses
//!   dosyasını ister (`<audio src="ms-appx:///...">`), Windows'un bildirim sesi
//!   ayarı onlara uygulanmaz. Uygulama başına ses `SoundFile` (REG_SZ): boş
//!   metin = kapalı, değer yok = açık (Ayarlar'dan açıp kapatarak bu makinede
//!   doğrulandı, 2026-11-17). Önceki değerler diske yazılır, kutucuklar gibi
//!   geri yüklenir. Aramalar / alarmlar (zil sesi) Windows'ta kalır.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{
    KnownNotificationBindings, NotificationKinds, ToastNotificationManager, UserNotification,
};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegNotifyChangeKeyValue,
    RegOpenKeyExW,
};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::Win32::UI::Shell::{QUNS_ACCEPTS_NOTIFICATIONS, SHQueryUserNotificationState};
use windows::core::{HSTRING, PCWSTR};
use windows_registry::CURRENT_USER;

pub const NOTIFY_SHOW: &str = "notify://show";
pub const NOTIFY_GONE: &str = "notify://gone";

const SETTINGS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Notifications\Settings";
const SHOW_BANNER: &str = "ShowBanner";
const SAVED_FILE: &str = "notification-banners.json";
const SOUND_FILE: &str = "SoundFile";
const SAVED_SOUNDS_FILE: &str = "notification-sounds.json";
/// Art arda gelen bildirimlerde (ör. birkaç mesaj birden) tek ses.
const SOUND_GAP: Duration = Duration::from_millis(1200);
/// Bir okuma (`GetNotificationsAsync`) arka planda ~150-200 ms işlemci harcıyor:
/// sık yoklama boşta tek çekirdeğin %20-30'u demek. Bu yüzden yalnızca bir şey
/// değişince okunur (`watch_registry`). Yedek yoklama seyrek.
const SAFETY_POLL: Duration = Duration::from_secs(30);
/// Ne olay ne kayıt izleme varsa.
const FALLBACK_POLL: Duration = Duration::from_secs(3);
/// Değişiklikten sonra okuma; değer bildirim okunabilir olmadan önce
/// yazılabilir diye bir kez daha (`FOLLOWUP` sonra).
const SETTLE: Duration = Duration::from_millis(150);
const FOLLOWUP: Duration = Duration::from_secs(1);

#[derive(Serialize, Clone)]
pub struct Notice {
    pub id: u32,
    pub app: String,
    pub icon: Option<String>,
    pub title: String,
    pub body: String,
}

struct State {
    saved_path: PathBuf,
    /// AUMID → kutucuğu kapatmadan önceki `ShowBanner` (`None`: değer yoktu).
    saved: HashMap<String, Option<u32>>,
    sounds_path: PathBuf,
    /// AUMID → sesini kapatmadan önceki `SoundFile` (`None`: değer yoktu).
    sounds: HashMap<String, Option<String>>,
    last_sound: Option<Instant>,
    /// Kartı gösterilen bildirim → gönderen AUMID (tıklayınca uygulamayı aç).
    senders: HashMap<u32, String>,
    icons: HashMap<String, Option<String>>,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Kendi bildirim seslerimizden biri seçili: uygulama seslerini biz çalarız.
static OWN_SOUND: AtomicBool = AtomicBool::new(false);
/// Kendi kartlarımız (zamanlayıcı ...): Windows bildirim kimlikleriyle çakışmasın.
static LOCAL_ID: AtomicU32 = AtomicU32::new(0xF000_0000);
/// Dinleyici çalışıyor (izin var). Değilse kutucuklar asla kapatılmaz: yoksa
/// bildirimler hiçbir yerde görünmezdi.
static WORKING: AtomicBool = AtomicBool::new(false);
static STATE: Mutex<Option<State>> = Mutex::new(None);
static LISTENER: OnceLock<UserNotificationListener> = OnceLock::new();

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    guard.as_mut().map(f)
}

pub fn spawn(app: &AppHandle, config_dir: PathBuf, enabled: bool) {
    let saved_path = config_dir.join(SAVED_FILE);
    let sounds_path = config_dir.join(SAVED_SOUNDS_FILE);
    let sounds = std::fs::read_to_string(&sounds_path)
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, Option<String>>>(&t).ok())
        .unwrap_or_default();
    // Anahtarlar küçük harfle (kayıt defteri ve AUMID'ler büyük/küçük harf duyarsız).
    let saved = std::fs::read_to_string(&saved_path)
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, Option<u32>>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(State {
        saved_path,
        saved,
        sounds_path,
        sounds,
        last_sound: None,
        senders: HashMap::new(),
        icons: HashMap::new(),
    });
    // Kutucuklar dinleyici çalışınca kapanır (`run`); kapalıysa önceki
    // oturumdan (çökme) kalanlar geri yüklenir.
    ENABLED.store(enabled, Ordering::Relaxed);
    OWN_SOUND.store(super::notify_sound::is_ours(), Ordering::Relaxed);
    if !enabled {
        restore_all();
    } else if !OWN_SOUND.load(Ordering::Relaxed) {
        restore_sounds();
    }
    let app = app.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("notifications".into())
        .spawn(move || run(app))
    {
        eprintln!("[notify] iş parçacığı başlatılamadı: {e}");
    }
}

/// Ayar değişti: kutucukları kapat ya da Windows'unkileri geri getir.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    if !on {
        restore_all();
    } else if WORKING.load(Ordering::Relaxed) {
        suppress_all();
    }
}

/// Bildirim sesi seçimi değişti (`notify_sound::set`): kendi sesimizse
/// uygulama seslerini kapat, Windows'unkiyse geri getir.
pub fn set_own_sound(on: bool) {
    OWN_SOUND.store(on, Ordering::Relaxed);
    if !on {
        restore_sounds();
    } else if ENABLED.load(Ordering::Relaxed) && WORKING.load(Ordering::Relaxed) {
        let apps: Vec<String> = with(|s| s.saved.keys().cloned().collect()).unwrap_or_default();
        for aumid in apps {
            mute(&aumid);
        }
    }
}

/// Çıkışta: dock yokken bildirimler yine Windows'ta görünsün.
pub fn shutdown() {
    restore_all();
}

// --- Windows kutucukları ---------------------------------------------------------------

/// Windows'un kendi sistem bildirimleri (düğmeli onaylar vb.) Windows'ta kalır.
fn excluded(aumid: &str) -> bool {
    aumid.is_empty()
        || aumid.starts_with("Windows.SystemToast.")
        || aumid.starts_with("Windows.ActionCenter.")
}

fn suppress_all() {
    let Ok(key) = CURRENT_USER.open(SETTINGS_KEY) else {
        return;
    };
    let Ok(names) = key.keys() else { return };
    let names: Vec<String> = names.collect();
    for name in names {
        suppress(&name);
        mute(&name);
    }
}

/// Uygulamanın kutucuğunu kapat. Kullanıcı zaten kapattıysa dokunma (geri
/// yüklerken açmayalım). `true`: artık kartı biz gösteriyoruz.
fn suppress(aumid: &str) -> bool {
    if excluded(aumid) || !ENABLED.load(Ordering::Relaxed) || !WORKING.load(Ordering::Relaxed) {
        return false;
    }
    let lower = aumid.to_lowercase();
    with(|s| {
        if s.saved.contains_key(&lower) {
            return true;
        }
        let Ok(key) = CURRENT_USER.create(format!("{SETTINGS_KEY}\\{aumid}")) else {
            return false;
        };
        let before = key.get_u32(SHOW_BANNER).ok();
        if before == Some(0) {
            return false;
        }
        if key.set_u32(SHOW_BANNER, 0).is_err() {
            return false;
        }
        s.saved.insert(lower, before);
        persist(s);
        true
    })
    .unwrap_or(false)
}

fn restore_all() {
    restore_sounds();
    with(|s| {
        // Geri yüklenemeyen (ör. erişim hatası) kayıtta kalır, sonra yeniden denenir.
        s.saved.retain(|aumid, before| {
            let key = CURRENT_USER
                .options()
                .read()
                .write()
                .open(format!("{SETTINGS_KEY}\\{aumid}"));
            let Ok(key) = key else {
                // Anahtar yok (uygulama kaldırılmış): geri yüklenecek bir şey yok.
                return false;
            };
            let result = match before {
                Some(v) => key.set_u32(SHOW_BANNER, *v),
                None => key.remove_value(SHOW_BANNER),
            };
            if let Err(e) = &result {
                eprintln!("[notify] kutucuk ayarı geri yüklenemedi: {}", e.message());
            }
            result.is_err()
        });
        if s.saved.is_empty() {
            let _ = std::fs::remove_file(&s.saved_path);
        } else {
            persist(s);
        }
    });
}

fn persist(s: &State) {
    if let Ok(text) = serde_json::to_string(&s.saved)
        && let Err(e) = std::fs::write(&s.saved_path, text)
    {
        eprintln!("[notify] kutucuk ayarları kaydedilemedi: {e}");
    }
}

// --- ses ---------------------------------------------------------------------------------

/// Kartını gösterdiğimiz uygulamanın Windows'taki sesini kapat (sesi biz
/// çalacağız). Kullanıcı sesi zaten kapattıysa dokunma (o zaman biz de çalmayız).
fn mute(aumid: &str) {
    if excluded(aumid)
        || !OWN_SOUND.load(Ordering::Relaxed)
        || !ENABLED.load(Ordering::Relaxed)
        || !WORKING.load(Ordering::Relaxed)
    {
        return;
    }
    let lower = aumid.to_lowercase();
    let wanted = with(|s| s.saved.contains_key(&lower) && !s.sounds.contains_key(&lower));
    if wanted != Some(true) || rings(aumid) {
        return;
    }
    with(|s| {
        let Ok(key) = CURRENT_USER.create(format!("{SETTINGS_KEY}\\{aumid}")) else {
            return;
        };
        let before = key.get_string(SOUND_FILE).ok();
        if before.as_deref() == Some("") || key.set_string(SOUND_FILE, "").is_err() {
            return;
        }
        s.sounds.insert(lower, before);
        persist_sounds(s);
    });
}

/// Zil çalan uygulama (arama, alarm): sesi Windows'ta kalır.
fn rings(aumid: &str) -> bool {
    aumid.to_lowercase().contains("calling")
        || history(aumid).iter().any(|xml| {
            xml.contains("scenario=\"incomingCall\"")
                || xml.contains("scenario=\"alarm\"")
                || xml.contains("loop=\"true\"")
        })
}

fn restore_sounds() {
    with(|s| {
        s.sounds.retain(|aumid, before| {
            let key = CURRENT_USER
                .options()
                .read()
                .write()
                .open(format!("{SETTINGS_KEY}\\{aumid}"));
            let Ok(key) = key else {
                return false;
            };
            let result = match before {
                Some(v) => key.set_string(SOUND_FILE, v.as_str()),
                None => key.remove_value(SOUND_FILE),
            };
            if let Err(e) = &result {
                eprintln!("[notify] ses ayarı geri yüklenemedi: {}", e.message());
            }
            result.is_err()
        });
        if s.sounds.is_empty() {
            let _ = std::fs::remove_file(&s.sounds_path);
        } else {
            persist_sounds(s);
        }
    });
}

fn persist_sounds(s: &State) {
    if let Ok(text) = serde_json::to_string(&s.sounds)
        && let Err(e) = std::fs::write(&s.sounds_path, text)
    {
        eprintln!("[notify] ses ayarları kaydedilemedi: {e}");
    }
}

/// Uygulamanın bildirim merkezindeki bildirimleri (XML), eskiden yeniye.
/// İçerik yalnızca ses / senaryo için okunur, hiçbir yere yazılmaz.
fn history(aumid: &str) -> Vec<String> {
    ToastNotificationManager::History()
        .and_then(|h| h.GetHistoryWithId(&HSTRING::from(aumid)))
        .map(|list| {
            list.into_iter()
                .filter_map(|t| t.Content().and_then(|c| c.GetXml()).ok())
                .map(|x| x.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// `<audio ... silent="true">`: uygulama bildirimi bilerek sessiz gönderdi.
fn is_silent(xml: &str) -> bool {
    xml.split("<audio")
        .nth(1)
        .and_then(|a| a.split('>').next())
        .is_some_and(|a| a.contains("silent=\"true\""))
}

/// Kartı çıkan bildirim için seçili sesi çal (sesini kapattığımız uygulamalarda).
fn play_sound(aumid: &str) {
    if !OWN_SOUND.load(Ordering::Relaxed) {
        return;
    }
    let lower = aumid.to_lowercase();
    if with(|s| s.sounds.contains_key(&lower)) != Some(true) || !sound_allowed() {
        return;
    }
    // Sessiz gönderildiyse (ör. sohbet zaten açıkken) çalma; en yeni bildirim sonda.
    if history(aumid).last().is_some_and(|xml| is_silent(xml)) {
        return;
    }
    let now = Instant::now();
    let due = with(|s| {
        let due = s.last_sound.is_none_or(|t| now - t >= SOUND_GAP);
        if due {
            s.last_sound = Some(now);
        }
        due
    });
    if due != Some(true) {
        return;
    }
    // Seçili ses `Notification.Default` olayında (notify_sound).
    unsafe {
        let _ = PlaySoundW(
            windows::core::w!("Notification.Default"),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Windows bildirim sesi çalardı mı: genel ses ve bildirim ayarı açık;
/// rahatsız etmeyin, tam ekran uygulama ya da sunu yok.
fn sound_allowed() -> bool {
    let setting = |name: &str| {
        let value = CURRENT_USER
            .open(SETTINGS_KEY)
            .and_then(|k| k.get_u32(name));
        !matches!(value, Ok(0))
    };
    let quiet = unsafe { SHQueryUserNotificationState() }
        .is_ok_and(|state| state != QUNS_ACCEPTS_NOTIFICATIONS);
    setting("NOC_GLOBAL_SETTING_ALLOW_NOTIFICATION_SOUND")
        && setting("NOC_GLOBAL_SETTING_TOASTS_ENABLED")
        && !quiet
}

// --- okuma -----------------------------------------------------------------------------

fn run(app: AppHandle) {
    crate::win::com_mta();
    let listener = match UserNotificationListener::Current() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[notify] dinleyici yok: {}", e.message());
            restore_all();
            return;
        }
    };
    let allowed = |s| s == UserNotificationListenerAccessStatus::Allowed;
    if !listener.GetAccessStatus().is_ok_and(allowed)
        && !listener
            .RequestAccessAsync()
            .and_then(|op| op.join())
            .is_ok_and(allowed)
    {
        eprintln!("[notify] bildirimlere erişim izni yok");
        restore_all();
        return;
    }
    let (tx, rx) = mpsc::channel::<()>();
    let watching = watch_registry(tx.clone());
    let events = listener
        .NotificationChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(());
            Ok(())
        }))
        .is_ok();
    if cfg!(debug_assertions) {
        eprintln!("[notify] dinleniyor (olay: {events}, kayıt izleme: {watching})");
    }
    let _ = LISTENER.set(listener.clone());

    WORKING.store(true, Ordering::Relaxed);
    if ENABLED.load(Ordering::Relaxed) {
        suppress_all();
    }

    // Açılışta merkezde duran bildirimler kart olmaz. İlk okuma başarısızsa ilk
    // başarılı okuma yalnızca "görüldü" sayılır (hepsi birden kart olmasın).
    let mut primed = false;
    let mut seen: HashSet<u32> = HashSet::new();
    if let Ok(list) = toasts(&listener) {
        seen = list.iter().filter_map(|n| n.Id().ok()).collect();
        primed = true;
    }
    let idle = if events || watching {
        SAFETY_POLL
    } else {
        FALLBACK_POLL
    };
    let mut followup: Option<Instant> = None;
    loop {
        let timeout = followup.map_or(idle, |t| t.saturating_duration_since(Instant::now()));
        match rx.recv_timeout(timeout) {
            Ok(()) => {
                std::thread::sleep(SETTLE);
                while rx.try_recv().is_ok() {}
                followup = Some(Instant::now() + FOLLOWUP);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => followup = None,
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(idle),
        }
        let Ok(list) = toasts(&listener) else {
            continue;
        };
        let mut now = HashSet::with_capacity(list.len());
        for n in &list {
            let Ok(id) = n.Id() else { continue };
            now.insert(id);
            // Özel tam ekran oyunda kart oyunu küçültür: bildirim merkezde kalır.
            if primed
                && !seen.contains(&id)
                && !crate::win::flyout::cards_unsafe()
                && let Some(notice) = notice(n, id)
            {
                crate::win::notify::before_show(&app);
                let _ = app.emit(NOTIFY_SHOW, notice);
                if let Some(aumid) = with(|s| s.senders.get(&id).cloned()).flatten() {
                    play_sound(&aumid);
                }
            }
        }
        // Bildirim merkezinden silinen (ya da orada tıklanan) bildirimin kartı da gitsin.
        for gone in seen.difference(&now) {
            with(|s| s.senders.remove(gone));
            let _ = app.emit(NOTIFY_GONE, *gone);
        }
        seen = now;
        primed = true;
    }
}

/// Bildirim ayarları anahtarı değişince kanala yazar. Windows her yeni
/// bildirimde gönderen uygulamanın `LastNotificationAddedTime` değerini günceller
/// (bu makinede denendi); `RegNotifyChangeKeyValue` bunu olay olarak verir.
/// Kendi `ShowBanner` yazmalarımız da tetikler: yalnızca fazladan bir okuma.
/// `false`: izlenemiyor.
fn watch_registry(tx: mpsc::Sender<()>) -> bool {
    let path = crate::win::wide(SETTINGS_KEY);
    let mut key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            KEY_NOTIFY,
            &mut key,
        )
    };
    if opened.is_err() {
        return false;
    }
    let Ok(event) = (unsafe { CreateEventW(None, false, false, None) }) else {
        unsafe {
            let _ = windows::Win32::System::Registry::RegCloseKey(key);
        }
        return false;
    };
    // Tutamaçlar iş parçacıkları arasında sayı olarak taşınır.
    let (key, event) = (key.0 as isize, event.0 as isize);
    std::thread::Builder::new()
        .name("notify-watch".into())
        .spawn(move || {
            let (key, event) = (HKEY(key as *mut _), HANDLE(event as *mut _));
            // Eşzamansız kayıt, kaydeden iş parçacığı yaşadıkça geçerli: döngü burada.
            loop {
                let armed = unsafe {
                    RegNotifyChangeKeyValue(
                        key,
                        true,
                        REG_NOTIFY_CHANGE_LAST_SET,
                        Some(event),
                        true,
                    )
                };
                if armed.is_err()
                    || unsafe { WaitForSingleObject(event, INFINITE) } != WAIT_OBJECT_0
                    || tx.send(()).is_err()
                {
                    break;
                }
            }
            // Döngü bitti (kayıt düştü / alıcı kapandı): tutamaçları bırak.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(event);
                let _ = windows::Win32::System::Registry::RegCloseKey(key);
            }
        })
        .is_ok()
}

fn toasts(listener: &UserNotificationListener) -> windows::core::Result<Vec<UserNotification>> {
    let view = listener
        .GetNotificationsAsync(NotificationKinds::Toast)?
        .join()?;
    Ok(view.into_iter().collect())
}

/// Yeni bildirim → kart. Kutucuğunu Windows gösteriyorsa `None`.
fn notice(n: &UserNotification, id: u32) -> Option<Notice> {
    if !ENABLED.load(Ordering::Relaxed) {
        return None;
    }
    let info = n.AppInfo().ok();
    let aumid = info
        .as_ref()
        .and_then(|i| i.AppUserModelId().ok())
        .map(|h| h.to_string())
        .unwrap_or_default();
    let ours = with(|s| s.saved.contains_key(&aumid.to_lowercase())).unwrap_or(false);
    if !ours {
        // İlk kez görülen uygulama: bunu Windows gösterdi, sonrakiler bizim.
        suppress(&aumid);
        mute(&aumid);
        return None;
    }
    let texts: Vec<String> = n
        .Notification()
        .and_then(|t| t.Visual())
        .and_then(|v| v.GetBinding(&KnownNotificationBindings::ToastGeneric()?))
        .and_then(|b| b.GetTextElements())
        .map(|els| {
            els.into_iter()
                .filter_map(|e| e.Text().ok())
                .map(|t| t.to_string())
                .filter(|t| !t.trim().is_empty())
                .collect()
        })
        .unwrap_or_default();
    let app_name = info
        .as_ref()
        .and_then(|i| i.DisplayInfo().ok())
        .and_then(|d| d.DisplayName().ok())
        .map(|h| h.to_string())
        .unwrap_or_default();
    let icon = with(|s| s.icons.get(&aumid).cloned())
        .flatten()
        .unwrap_or_else(|| {
            let icon = super::icons::shell_info(Some(&aumid), None).icon;
            with(|s| s.icons.insert(aumid.clone(), icon.clone()));
            icon
        });
    with(|s| s.senders.insert(id, aumid.clone()));
    let mut texts = texts.into_iter();
    Some(Notice {
        id,
        app: app_name,
        icon,
        title: texts.next().unwrap_or_default(),
        body: texts.collect::<Vec<_>>().join("\n"),
    })
}

/// Dock'un kendi bildirimi (ör. zamanlayıcı bitti): kart + Windows'un
/// bildirim sesi (kullanıcının seçtiği ses).
pub fn show_local(app: &AppHandle, app_name: &str, title: &str, body: &str) {
    show_card(app, app_name, title, body);
    unsafe {
        let _ = PlaySoundW(
            windows::core::w!("Notification.Default"),
            None,
            SND_ALIAS | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Yalnızca kart (ses yok): ayarlardaki "Dene"nin sessiz hâli.
pub fn show_card(app: &AppHandle, app_name: &str, title: &str, body: &str) {
    let id = LOCAL_ID.fetch_add(1, Ordering::Relaxed);
    crate::win::notify::before_show(app);
    let _ = app.emit(
        NOTIFY_SHOW,
        Notice {
            id,
            app: app_name.to_string(),
            icon: None,
            title: title.to_string(),
            body: body.to_string(),
        },
    );
}

/// Karta tıklandı: gönderen uygulamanın AUMID'si; bildirim merkezden de kalkar
/// (Windows'ta tıklanan bildirim de kalkar).
pub fn take_sender(id: u32) -> Option<String> {
    let aumid = with(|s| s.senders.remove(&id)).flatten()?;
    if let Some(listener) = LISTENER.get() {
        let _ = listener.RemoveNotification(id);
    }
    Some(aumid)
}

#[cfg(test)]
mod sound_tests {
    use super::is_silent;

    #[test]
    fn detects_silent_toasts() {
        let loud = r#"<toast><visual/><audio src="ms-appx:///Sounds/a.m4a" /><actions/></toast>"#;
        let quiet = r#"<toast><audio src="ms-appx:///Sounds/a.m4a" silent="true" /></toast>"#;
        assert!(!is_silent(loud));
        assert!(is_silent(quiet));
        assert!(!is_silent("<toast><visual/></toast>"));
        // Başka bir öğedeki silent sayılmaz.
        assert!(!is_silent(
            r#"<toast><audio src="a"/><x silent="true"/></toast>"#
        ));
    }
}
