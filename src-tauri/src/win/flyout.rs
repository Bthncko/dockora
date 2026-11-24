//! Ekran üstü kartlar penceresi ("flyout"): medya, sıradaki şarkı, kilit
//! tuşları, ses, parlaklık. Odak almaz.
//!
//! Sayfa içeriğinin boyunu bildirir (`layout`); pencere tam o boyutta (gölge
//! payı sayfanın kendi boyutuna dahil) ayarlardaki yere (`flyouts.placement`,
//! bildirim kartlarıyla aynı köşe/uzaklık mantığı) konur ve gösterilir.
//! Otomatik gizleme sayfada; Rust yalnızca gizler.
//!
//! Ekran: `flyouts.monitor` ("primary" | "cursor" | "focused"). Alt kenarda ve
//! ana ekranda alan dock'un üstünde biter (`notify::area_in`).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, HWND_TOPMOST, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SetWindowPos, ShowWindow,
};

use super::notify::{OFFSET_MAX, Spot, area_in};
use super::placement;
use crate::config::ConfigStore;
use crate::error::{AppError, AppResult};
use crate::services::flylog::log;

pub const LABEL: &str = "flyout";
pub const FLYOUT_SHOW: &str = "flyout:show";

/// İmleç kartın üstünde (sayfa bildirir; şimdilik yalnızca bilgi).
static HELD: AtomicBool = AtomicBool::new(false);

/// `flyout:show` yükü.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Show {
    /// Durum mevcut medya komutlarından / olaylarından okunur.
    Media {
        trigger: &'static str,
    },
    NextUp {
        title: String,
        artist: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        album: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        art: Option<String>,
        app_id: String,
    },
    LockKey {
        key: &'static str,
        on: bool,
    },
    Volume {
        level: u8,
        muted: bool,
        device: String,
    },
    Brightness {
        level: u8,
    },
}

fn window(app: &AppHandle) -> AppResult<WebviewWindow> {
    app.get_webview_window(LABEL)
        .ok_or_else(|| AppError::Win32("kart penceresi bulunamadı".into()))
}

pub fn init(app: &AppHandle) -> AppResult<()> {
    super::window_style::apply_dock_style(&window(app)?)
}

/// Sayfa dinleyicisi hazır olmadan gelen kart: en yeni tek olay, bu süreden
/// eskiyse atılır (geç gelen eski tuş kartı kullanıcıyı şaşırtır).
const PENDING_MAX_AGE: Duration = Duration::from_secs(3);

struct Pending {
    /// Sayfa `flyout_ready` dedi mi (dinleyici kayıtlı).
    ready: bool,
    latest: Option<(Instant, Show)>,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending {
    ready: false,
    latest: None,
});

/// Sayfa dinleyicisini kaydetti. Bekleyen (≤ 3 sn) kart varsa şimdi gönderilir.
pub fn ready(app: &AppHandle) {
    let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    p.ready = true;
    log(&format!(
        "kart sayfası hazır (bekleyen: {})",
        if p.latest.is_some() { "var" } else { "yok" }
    ));
    if let Some((at, payload)) = p.latest.take() {
        if at.elapsed() <= PENDING_MAX_AGE {
            let ok = app.emit_to(LABEL, FLYOUT_SHOW, &payload).is_ok();
            log(&format!(
                "bekleyen kart gönderildi (yaş {} ms): {}",
                at.elapsed().as_millis(),
                if ok { "ok" } else { "emit hatası" }
            ));
        } else {
            log("bekleyen kart çok eski, atıldı");
        }
    }
}

fn kind_name(p: &Show) -> String {
    match p {
        Show::Media { trigger } => format!("media/{trigger}"),
        Show::NextUp { .. } => "nextUp".into(),
        Show::LockKey { key, .. } => format!("lockKey/{key}"),
        Show::Volume { .. } => "volume".into(),
        Show::Brightness { .. } => "brightness".into(),
    }
}

/// Tam ekran oyun / sunum gibi durumlar (kart gösterilmez). Yalnızca
/// `QUNS_BUSY`, `QUNS_RUNNING_D3D_FULL_SCREEN`, `QUNS_PRESENTATION_MODE`
/// bastırır; `QUNS_NOT_PRESENT` (oturum açılışında olur), `QUNS_QUIET_TIME`,
/// `QUNS_ACCEPTS_NOTIFICATIONS`, `QUNS_APP` bastırmaz. Dönüş: (bastır, ham değer).
pub fn busy() -> (bool, Option<i32>) {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => (
            matches!(
                state,
                QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
            ),
            Some(state.0),
        ),
        Err(_) => (false, None),
    }
}

/// Config'e göre kart gösterilebilir mi; gösterilebilirse `flyout:show` yayınlar.
/// `app_id`: medya kartlarında oturumun uygulaması (uygulama süzgeci için).
pub fn show(app: &AppHandle, payload: Show, app_id: Option<&str>) -> bool {
    let kind = kind_name(&payload);
    let Some(store) = app.try_state::<ConfigStore>() else {
        log(&format!("show {kind}: reddedildi (ayar deposu yok)"));
        return false;
    };
    let cfg = store.get().flyouts;
    if !cfg.enabled {
        log(&format!("show {kind}: reddedildi (flyout kapalı)"));
        return false;
    }
    let kind_on = match &payload {
        Show::Media { .. } => cfg.media.enabled,
        Show::NextUp { .. } => cfg.next_up.enabled,
        Show::LockKey { key, .. } => {
            cfg.lock_keys.enabled
                && match *key {
                    "caps" => cfg.lock_keys.caps,
                    "num" => cfg.lock_keys.num,
                    "scroll" => cfg.lock_keys.scroll,
                    _ => cfg.lock_keys.insert,
                }
        }
        Show::Volume { .. } => cfg.volume.enabled,
        Show::Brightness { .. } => cfg.brightness.enabled,
    };
    if !kind_on {
        log(&format!("show {kind}: reddedildi (tür kapalı)"));
        return false;
    }
    if matches!(payload, Show::Media { .. } | Show::NextUp { .. })
        && let Some(id) = app_id
        && !cfg.app_filter.allows(id)
    {
        log(&format!("show {kind}: reddedildi (uygulama süzgeci, {id})"));
        return false;
    }
    if cfg.hide_in_fullscreen {
        let (suppress, quns) = busy();
        if suppress {
            log(&format!("show {kind}: reddedildi (busy, QUNS={quns:?})"));
            return false;
        }
        log(&format!("show {kind}: busy değil (QUNS={quns:?})"));
    }
    let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if !p.ready {
        p.latest = Some((Instant::now(), payload));
        log(&format!("show {kind}: sayfa hazır değil, beklemeye alındı"));
        return true;
    }
    match app.emit_to(LABEL, FLYOUT_SHOW, &payload) {
        Ok(()) => {
            log(&format!("show {kind}: gönderildi"));
            true
        }
        Err(e) => {
            log(&format!("show {kind}: emit hatası: {e}"));
            false
        }
    }
}

fn monitor_of(mode: &str) -> HMONITOR {
    unsafe {
        let primary = || MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        match mode {
            "cursor" => {
                let mut p = POINT::default();
                if GetCursorPos(&mut p).is_ok() {
                    MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
                } else {
                    primary()
                }
            }
            "focused" => {
                let fg: HWND = GetForegroundWindow();
                if fg.is_invalid() {
                    primary()
                } else {
                    MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
                }
            }
            _ => primary(),
        }
    }
}

/// Ekranın dikdörtgeni (fiziksel piksel) ve DPI ölçeği.
fn monitor_rect(monitor: HMONITOR) -> (RECT, f64) {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let rect = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        info.rcMonitor
    } else {
        placement::primary_monitor_rect()
    };
    let (mut dx, mut dy) = (0u32, 0u32);
    let scale = match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) } {
        Ok(()) if dx > 0 => f64::from(dx) / 96.0,
        _ => 1.0,
    };
    (rect, scale)
}

/// `width`/`height`: sayfanın tam içeriği (CSS piksel). 0 → gizle.
/// Her kart gösterilişinde / boyutu değişince sayfa yeniden çağırır.
pub fn layout(app: &AppHandle, width: f64, height: f64) -> AppResult<()> {
    let hwnd = window(app)?.hwnd()?;
    if width <= 0.0 || height <= 0.0 {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        return Ok(());
    }
    let cfg = app
        .try_state::<ConfigStore>()
        .map(|c| c.get().flyouts)
        .unwrap_or_default();
    let (monitor, dpi) = monitor_rect(monitor_of(&cfg.monitor));
    let px = |v: f64| (v.clamp(0.0, OFFSET_MAX) * dpi).round() as i32;
    let spot = Spot::parse(
        &cfg.placement.anchor,
        px(cfg.placement.x),
        px(cfg.placement.y),
    );
    let w = (width * dpi).round() as i32;
    let h = (height * dpi).round() as i32;
    let (x, y) = spot.origin(area_in(monitor), w, h);
    unsafe {
        // Pencere başka DPI'lı ekrana geçince Windows boyutu yeniden hesaplar:
        // ikinci çağrı hedef boyutu o DPI'da yeniden oturtur.
        let before = placement::scale(hwnd);
        SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE)?;
        if (placement::scale(hwnd) - before).abs() > f64::EPSILON {
            SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE)?;
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

pub fn hide(app: &AppHandle) -> AppResult<()> {
    HELD.store(false, Ordering::SeqCst);
    let hwnd = window(app)?.hwnd()?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    Ok(())
}

pub fn hold(held: bool) {
    HELD.store(held, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_shape() {
        let v = serde_json::to_value(Show::Volume {
            level: 40,
            muted: false,
            device: "Hoparlör".into(),
        })
        .unwrap();
        assert_eq!(v["kind"], "volume");
        assert_eq!(v["level"], 40);
        let v = serde_json::to_value(Show::NextUp {
            title: "a".into(),
            artist: "b".into(),
            album: None,
            art: None,
            app_id: "Spotify.exe".into(),
        })
        .unwrap();
        assert_eq!(v["kind"], "nextUp");
        assert_eq!(v["appId"], "Spotify.exe");
        assert!(v.get("album").is_none());
        let v = serde_json::to_value(Show::Media {
            trigger: "mediaKey",
        })
        .unwrap();
        assert_eq!(v["trigger"], "mediaKey");
    }
}
