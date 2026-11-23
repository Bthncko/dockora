//! Kendi bildirim alanımız (tepsi): uygulamaların tepsi simgelerini biz alırız,
//! dock tarzı panelde gösteririz.
//!
//! Windows bunun için API vermiyor; kabuk değiştiricilerin (Cairo/RetroBar'ın
//! ManagedShell'i) yolu:
//! - `Shell_NotifyIcon`, `FindWindow("Shell_TrayWnd")` ile bulduğu pencereye
//!   `WM_COPYDATA` (dwData 1: SHELLTRAYDATA) gönderir. Aynı sınıf adıyla, en
//!   üstte (topmost) duran görünmez bir pencere açınca ilk bulunan biz oluruz.
//! - "TaskbarCreated" yayını uygulamalara simgelerini yeniden ekletir.
//! - Her şey Explorer'ın çubuğuna da iletilir: Windows'un durumu (bildirim
//!   balonları, Win+B taşması, dock kapanınca simgeler) aynen sürer.
//! - Tıklama, uygulamanın geri çağırma iletisiyle gönderilir (sürüm 4 ise
//!   wParam = imleç, lParam = ileti | uID << 16; değilse wParam = uID).
//!
//! Yapılar belgelenmemiş (64 bit Windows'ta da tutamaçlar 32 bit); düzen
//! ManagedShell'deki tanımlarla aynı.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, FindWindowExW, FindWindowW, GetCursorPos, GetMessageW,
    GetWindowThreadProcessId, HICON, HWND_TOPMOST, IsWindow, KillTimer, MSG, PostMessageW,
    PostQuitMessage, RegisterClassW, RegisterWindowMessageW, SMTO_ABORTIFHUNG, SMTO_NORMAL,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageTimeoutW, SendNotifyMessageW,
    SetTimer, SetWindowPos, TranslateMessage, WINDOW_EX_STYLE, WM_ACTIVATEAPP, WM_CLOSE,
    WM_COMMAND, WM_CONTEXTMENU, WM_COPYDATA, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER, WM_USER, WNDCLASSW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{GUID, w};

use super::{is_own_window, placement};

pub const TRAY_CHANGED: &str = "tray://changed";

const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const NIF_STATE: u32 = 0x8;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x1;
const NIN_SELECT: u32 = WM_USER;
/// Explorer bunu gönderilen değil postalanan ileti olarak bekliyor (ManagedShell).
const FORWARD_POST: u32 = WM_USER + 372;

const TIMER_WATCH: usize = 1;
const TIMER_REBROADCAST: usize = 2;
const WATCH_MS: u32 = 250;
const STARTUP_RESYNC_MS: u32 = 3000;
/// Explorer öne geçtiğinde (ama yeniden başlamadan) simgeleri yeniden isteme
/// sıklığı (en fazla). "TaskbarCreated" yayını başka uygulamalarca (FluentFlyout,
/// Wallpaper Engine) Explorer yeniden başladı diye okunur: seyrek olmalı.
const RESYNC_EVERY: Duration = Duration::from_secs(600);
/// Kendi yayınımızdan sonra gelen "TaskbarCreated" bizimki sayılır.
const OWN_BROADCAST: Duration = Duration::from_secs(5);
/// Explorer'a iletmede en fazla bekleme.
const FORWARD_TIMEOUT_MS: u32 = 2000;

/// NOTIFYICONDATAW, tutamaçlar 32 bit (kabuğa giden biçim).
#[repr(C)]
#[derive(Clone, Copy)]
struct NotifyIconData {
    cb_size: u32,
    hwnd: u32,
    uid: u32,
    flags: u32,
    callback: u32,
    hicon: u32,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: GUID,
    balloon_icon: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ShellTrayData {
    signature: u32,
    message: u32,
    nid: NotifyIconData,
}

/// `Shell_NotifyIconGetRect` (dwData 3).
#[repr(C)]
#[derive(Clone, Copy)]
struct IconIdentifier {
    magic: u32,
    message: u32,
    cb_size: u32,
    padding: u32,
    hwnd: u32,
    uid: u32,
    guid: GUID,
}

struct Entry {
    hwnd: u32,
    uid: u32,
    guid: GUID,
    callback: u32,
    version: u32,
    tip: String,
    icon: Option<String>,
    hidden: bool,
    /// Son tıklamadaki imleç çevresi: uygulama penceresini buraya göre açar.
    rect: Option<RECT>,
}

impl Entry {
    fn id(&self) -> String {
        format!("{}-{}", self.hwnd, self.uid)
    }

    fn matches(&self, hwnd: u32, uid: u32, guid: &GUID) -> bool {
        (self.hwnd == hwnd && self.uid == uid) || (*guid != GUID::zeroed() && self.guid == *guid)
    }
}

#[derive(Serialize, Clone)]
pub struct TrayIcon {
    pub id: String,
    pub tip: String,
    pub icon: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct TrayState {
    /// Simgeleri biz alıyoruz; `false` ise panel Windows'un tepsisine yönlendirir.
    pub active: bool,
    pub icons: Vec<TrayIcon>,
}

#[derive(Default)]
struct Host {
    tray: isize,
    explorer: isize,
    entries: Vec<Entry>,
    dirty: bool,
    broadcast_at: Option<Instant>,
    ticks: u32,
    rect: Option<RECT>,
}

static HOST: Mutex<Option<Host>> = Mutex::new(None);
static APP: OnceLock<AppHandle> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

fn with<R>(f: impl FnOnce(&mut Host) -> R) -> R {
    let mut guard = HOST.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Host::default))
}

fn hwnd(v: isize) -> HWND {
    HWND(v as *mut _)
}

/// Tutamaç 32 bit gelir; işaretle genişletilir (Windows tutamaçları böyle).
fn wide_handle(v: u32) -> isize {
    v as i32 as isize
}

pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
    std::thread::Builder::new()
        .name("tray-host".into())
        .spawn(run)
        .expect("tray-host başlatılamadı");
}

/// Çıkışta: pencere kapanır, uygulamalar yine Explorer'ı bulur (simgeler
/// zaten orada da kayıtlı).
pub fn shutdown() {
    let tray = with(|h| h.tray);
    if tray != 0 {
        unsafe {
            let _ = PostMessageW(Some(hwnd(tray)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn state() -> TrayState {
    with(|h| TrayState {
        // Pencere sırası ölçüt değil: Windows'un çubuğu ara sıra (ölçüm: ~%1)
        // bir an öne geçiyor, `watch` hemen geri alıyor.
        active: h.tray != 0,
        icons: h
            .entries
            .iter()
            .filter(|e| !e.hidden)
            .map(|e| TrayIcon {
                id: e.id(),
                tip: e.tip.clone(),
                icon: e.icon.clone(),
            })
            .collect(),
    })
}

fn run() {
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let instance = module.into();
        let tray_class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: w!("Shell_TrayWnd"),
            ..Default::default()
        };
        let notify_class = WNDCLASSW {
            lpfnWndProc: Some(default_proc),
            hInstance: instance,
            lpszClassName: w!("TrayNotifyWnd"),
            ..Default::default()
        };
        if RegisterClassW(&tray_class) == 0 || RegisterClassW(&notify_class) == 0 {
            eprintln!("[tray] pencere sınıfı kaydedilemedi");
            return;
        }
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));

        // Görünmez; boyutu dock şeridi (görev çubuğunun yerini soran uygulamalar için).
        let r = placement::reserved_rect().unwrap_or_default();
        let Ok(tray) = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("Shell_TrayWnd"),
            w!(""),
            WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            None,
            None,
            Some(instance),
            None,
        ) else {
            eprintln!("[tray] pencere açılamadı");
            return;
        };
        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("TrayNotifyWnd"),
            w!(""),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            r.right - r.left,
            r.bottom - r.top,
            Some(tray),
            None,
            Some(instance),
            None,
        );
        with(|h| {
            h.tray = tray.0 as isize;
            h.rect = Some(r);
            h.explorer = explorer_tray().map_or(0, |e| e.0 as isize);
        });
        make_topmost(tray);
        SetTimer(Some(tray), TIMER_WATCH, WATCH_MS, None);
        broadcast();
        // Açılışta Windows çubuğu gizlenirken sıra karışabilir: bir kez daha iste.
        SetTimer(Some(tray), TIMER_REBROADCAST, STARTUP_RESYNC_MS, None);
        eprintln!("[tray] simgeler alınıyor");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with(|h| {
            h.tray = 0;
            h.entries.clear();
        });
    }
}

unsafe extern "system" fn default_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

/// Explorer'ın asıl çubuğu (bizimki de aynı sınıfta).
pub(super) fn explorer_tray() -> Option<HWND> {
    let mut after = None;
    loop {
        let found = unsafe { FindWindowExW(None, after, w!("Shell_TrayWnd"), None) }.ok()?;
        if !is_own_window(found) {
            return Some(found);
        }
        after = Some(found);
    }
}

fn make_topmost(tray: HWND) {
    unsafe {
        let _ = SetWindowPos(
            tray,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Az önce "TaskbarCreated"ı biz yayınladık mı (dock'un AppBar'ı bunu Explorer'ın
/// yeniden başlaması sanmasın).
pub fn own_broadcast() -> bool {
    with(|h| h.broadcast_at.is_some_and(|t| t.elapsed() < OWN_BROADCAST))
}

/// Uygulamalara "görev çubuğu yeniden oluştu, simgelerini ekle" de.
fn broadcast() {
    let Some(&msg) = TASKBAR_CREATED.get() else {
        return;
    };
    with(|h| h.broadcast_at = Some(Instant::now()));
    // Sistem geneli yayın değil: hareketli duvar kâğıdı sıfırlanıyordu (broadcast.rs).
    super::broadcast::send(msg, WPARAM(0), LPARAM(0), super::broadcast::Skip::Wallpaper);
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COPYDATA if lp.0 != 0 => {
            // SAFETY: WM_COPYDATA'da lParam, ileti süresince geçerli COPYDATASTRUCT.
            let cds = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
            match cds.dwData {
                1 => {
                    if let Some(data) = unsafe { read::<ShellTrayData>(cds) } {
                        let accepted = handle(data.message, &data.nid);
                        let forwarded = forward(msg, wp, lp);
                        return if accepted { LRESULT(1) } else { forwarded };
                    }
                }
                3 => {
                    if let Some(id) = unsafe { read::<IconIdentifier>(cds) }
                        && let Some(result) = icon_rect(&id)
                    {
                        return result;
                    }
                }
                _ => {}
            }
            forward(msg, wp, lp)
        }
        WM_TIMER if wp.0 == TIMER_WATCH => {
            watch(h);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_REBROADCAST => {
            unsafe {
                let _ = KillTimer(Some(h), TIMER_REBROADCAST);
            }
            with(|host| host.explorer = explorer_tray().map_or(0, |e| e.0 as isize));
            make_topmost(h);
            broadcast();
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe {
                let _ = DestroyWindow(h);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ if Some(&msg) == TASKBAR_CREATED.get() => {
            // Explorer yeniden başladı: uygulamalar simgelerini ona ekledi.
            // Önce yine en üste geç, sonra herkese yeniden ekletmek için yayınla.
            let own = with(|host| {
                host.broadcast_at
                    .is_some_and(|t| t.elapsed() < OWN_BROADCAST)
            });
            if !own {
                unsafe {
                    SetTimer(Some(h), TIMER_REBROADCAST, 1500, None);
                }
            }
            LRESULT(0)
        }
        // Görev çubuğuna giden diğer iletiler (ör. SHAppBarMessage = WM_COPYDATA 0)
        // Explorer'a; kayıtlı iletiler (0xC000+) hariç.
        WM_ACTIVATEAPP | WM_COMMAND => forward(msg, wp, lp),
        _ if (WM_USER..0x8000).contains(&msg) => forward(msg, wp, lp),
        _ => unsafe { DefWindowProcW(h, msg, wp, lp) },
    }
}

/// `cbData` kısaysa eksik alanlar sıfır kalır.
unsafe fn read<T: Copy>(cds: &COPYDATASTRUCT) -> Option<T> {
    if cds.lpData.is_null() || cds.cbData < 8 {
        return None;
    }
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let len = (cds.cbData as usize).min(size_of::<T>());
    unsafe {
        std::ptr::copy_nonoverlapping(cds.lpData as *const u8, value.as_mut_ptr() as *mut u8, len);
        // SAFETY: T yalnızca tamsayı / dizi alanlı; sıfır geçerli değer.
        Some(value.assume_init())
    }
}

fn forward(msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let explorer = with(|h| {
        if h.explorer == 0 || !unsafe { IsWindow(Some(hwnd(h.explorer))) }.as_bool() {
            h.explorer = explorer_tray().map_or(0, |e| e.0 as isize);
        }
        h.explorer
    });
    if explorer == 0 {
        return LRESULT(0);
    }
    unsafe {
        if msg == FORWARD_POST {
            let _ = PostMessageW(Some(hwnd(explorer)), msg, wp, lp);
            return LRESULT(0);
        }
        let mut result = 0usize;
        SendMessageTimeoutW(
            hwnd(explorer),
            msg,
            wp,
            lp,
            SMTO_NORMAL | SMTO_ABORTIFHUNG,
            FORWARD_TIMEOUT_MS,
            Some(&mut result),
        );
        LRESULT(result as isize)
    }
}

/// NIM_* iletisi. `true`: kabul edildi (uygulamaya başarı döner).
fn handle(message: u32, nid: &NotifyIconData) -> bool {
    let guid = if nid.flags & NIF_GUID != 0 {
        nid.guid
    } else {
        GUID::zeroed()
    };
    if nid.hwnd == 0 && (guid == GUID::zeroed() || message == NIM_ADD) {
        return false;
    }
    with(|h| {
        let index = h
            .entries
            .iter()
            .position(|e| e.matches(nid.hwnd, nid.uid, &guid));
        match message {
            NIM_ADD | NIM_MODIFY => {
                let index = match (index, message) {
                    (Some(i), _) => i,
                    // Bilinmeyen simgeye NIM_MODIFY: simge Explorer'a biz öne geçmeden
                    // eklenmişti (açılış / Explorer öne geçtiği an). Düşürmek yerine
                    // eklenir; yeterli veri yoksa (simge ve ileti yok) Explorer'a kalır.
                    (None, NIM_MODIFY) if nid.flags & (NIF_ICON | NIF_MESSAGE) == 0 => {
                        return false;
                    }
                    (None, _) => {
                        h.entries.push(Entry {
                            hwnd: nid.hwnd,
                            uid: nid.uid,
                            guid,
                            callback: 0,
                            version: 0,
                            tip: String::new(),
                            icon: None,
                            hidden: false,
                            rect: None,
                        });
                        h.entries.len() - 1
                    }
                };
                let e = &mut h.entries[index];
                // GUID'li simge uygulama yeniden başlayınca yeni pencere / uID ile
                // gelir: eski tutamaçta kalırsa kapanmış pencere diye silinirdi.
                if nid.hwnd != 0 {
                    e.hwnd = nid.hwnd;
                    e.uid = nid.uid;
                }
                if nid.flags & NIF_MESSAGE != 0 {
                    e.callback = nid.callback;
                }
                if nid.flags & NIF_ICON != 0 {
                    // Çevrilemeyen simge öncekini silmesin.
                    let new = (nid.hicon != 0)
                        .then(|| {
                            crate::services::icons::hicon_data_url(HICON(
                                wide_handle(nid.hicon) as *mut _
                            ))
                        })
                        .flatten();
                    if new.is_some() || nid.hicon == 0 {
                        e.icon = new;
                    }
                }
                if nid.flags & NIF_TIP != 0 {
                    let end = nid
                        .tip
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(nid.tip.len());
                    e.tip = String::from_utf16_lossy(&nid.tip[..end]);
                }
                if nid.flags & NIF_STATE != 0 && nid.state_mask & NIS_HIDDEN != 0 {
                    e.hidden = nid.state & NIS_HIDDEN != 0;
                }
                if guid != GUID::zeroed() {
                    e.guid = guid;
                }
                h.dirty = true;
                true
            }
            NIM_DELETE => {
                let Some(i) = index else { return false };
                h.entries.remove(i);
                h.dirty = true;
                true
            }
            NIM_SETVERSION => {
                if nid.version > 4 {
                    return false;
                }
                if let Some(i) = index {
                    h.entries[i].version = nid.version;
                }
                true
            }
            // NIM_SETFOCUS: odağı panele geri vermemiz istenir; yapacak bir şey yok.
            _ => true,
        }
    })
}

/// `Shell_NotifyIconGetRect`: dwMessage 1 → sol-üst, 2 → sağ-alt (ManagedShell).
fn icon_rect(id: &IconIdentifier) -> Option<LRESULT> {
    let rect = with(|h| {
        h.entries
            .iter()
            .find(|e| e.matches(id.hwnd, id.uid, &id.guid))
            .and_then(|e| e.rect)
    })?;
    let pack =
        |x: i32, y: i32| LRESULT(((x as u16 as u32) | ((y as u16 as u32) << 16)) as i32 as isize);
    match id.message {
        1 => Some(pack(rect.left, rect.top)),
        2 => Some(pack(rect.right, rect.bottom)),
        _ => None,
    }
}

fn watch(tray: HWND) {
    // Explorer (ör. yeniden başlayınca) öne geçtiyse yine en üste.
    let first = unsafe { FindWindowW(w!("Shell_TrayWnd"), None) };
    if let Ok(front) = first
        && front != tray
    {
        make_topmost(tray);
        // Explorer gerçekten yeniden başladıysa (pencere değişti) yayın şart;
        // yalnızca öne geçtiyse (simge iletileri o arada Explorer'a gitmiş
        // olabilir) en çok `RESYNC_EVERY`'de bir.
        let (restarted, stale) = with(|h| {
            let id = front.0 as isize;
            let restarted = h.explorer != 0 && h.explorer != id;
            h.explorer = id;
            (
                restarted,
                h.broadcast_at.is_none_or(|t| t.elapsed() > RESYNC_EVERY),
            )
        });
        if restarted || stale {
            broadcast();
        }
    }
    let (emit, rect_changed) = with(|h| {
        h.ticks = h.ticks.wrapping_add(1);
        // ~2 sn'de bir: kapanmış uygulamaların simgeleri, dock şeridi.
        let mut rect_changed = None;
        if h.ticks % 8 == 0 {
            let before = h.entries.len();
            h.entries.retain(|e| {
                e.hwnd == 0 || unsafe { IsWindow(Some(hwnd(wide_handle(e.hwnd)))) }.as_bool()
            });
            h.dirty |= h.entries.len() != before;
            let r = placement::reserved_rect();
            if r.is_some() && r != h.rect {
                h.rect = r;
                rect_changed = r;
            }
        }
        (std::mem::take(&mut h.dirty), rect_changed)
    });
    if let Some(r) = rect_changed {
        unsafe {
            let _ = SetWindowPos(
                tray,
                None,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    if emit && let Some(app) = APP.get() {
        let _ = app.emit(TRAY_CHANGED, state());
    }
}

/// Panelden tıklama: "left", "double" ya da "right".
pub fn click(id: &str, button: &str) -> Result<(), String> {
    let mut cursor = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut cursor);
    }
    let target = with(|h| {
        let e = h.entries.iter_mut().find(|e| e.id() == id)?;
        e.rect = Some(RECT {
            left: cursor.x - 16,
            top: cursor.y - 16,
            right: cursor.x + 16,
            bottom: cursor.y + 16,
        });
        Some((e.hwnd, e.uid, e.callback, e.version))
    });
    let Some((owner, uid, callback, version)) = target else {
        return Err("simge artık yok".into());
    };
    let owner = hwnd(wide_handle(owner));
    if !unsafe { IsWindow(Some(owner)) }.as_bool() {
        return Err("uygulama kapanmış".into());
    }
    if callback == 0 {
        return Ok(());
    }
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(owner, Some(&mut pid));
        // Uygulama penceresini / menüsünü öne getirebilsin (biz öndeyiz).
        let _ = AllowSetForegroundWindow(pid);
    }
    let mouse = (cursor.x as u16 as u32) | ((cursor.y as u16 as u32) << 16);
    let send = |message: u32| {
        let (wp, lp) = if version > 3 {
            (mouse as usize, (message | (uid << 16)) as isize)
        } else {
            (uid as usize, message as isize)
        };
        unsafe {
            let _ = SendNotifyMessageW(owner, callback, WPARAM(wp), LPARAM(lp));
        }
    };
    // Explorer NIN_SELECT / WM_CONTEXTMENU'yü sürüm 3'te de gönderiyor (ManagedShell).
    match button {
        "right" => {
            send(WM_RBUTTONDOWN);
            send(WM_RBUTTONUP);
            if version >= 3 {
                send(WM_CONTEXTMENU);
            }
        }
        "double" => {
            send(WM_LBUTTONDBLCLK);
            send(WM_LBUTTONUP);
        }
        _ => {
            send(WM_LBUTTONDOWN);
            send(WM_LBUTTONUP);
            if version >= 3 {
                send(NIN_SELECT);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_shell() {
        // ManagedShell / ReactOS: NOTIFYICONDATAW (32 bit tutamaç) 956 bayt.
        assert_eq!(size_of::<NotifyIconData>(), 956);
        assert_eq!(size_of::<ShellTrayData>(), 964);
        assert_eq!(size_of::<IconIdentifier>(), 40);
    }
}
