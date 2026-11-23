//! Bildirim kartları penceresi: ayarlardaki köşe/kenarda (`notify_position`),
//! odak almaz.
//!
//! Sayfa kart yığınının boyunu bildirir (`layout`); pencere ona göre boyutlanır
//! ve gösterilir, kart kalmayınca gizlenir. Pencere kartlardan her yönde
//! `SHADOW_PAD` büyük (gölge). Alttaki yerlerde yığın yukarı doğru büyür.
//!
//! Buzlu cam (`glass.rs`, panellerle aynı): pencere gizliyken ilk kart gelince,
//! kartların duracağı alan (`GLASS_W` × `GLASS_H` mantıksal piksel, yığınla aynı
//! köşeye yaslı) okunur, bulanıklaştırılır ve kart olayından önce sayfaya
//! gönderilir. Pencere görünürken yeniden okunmaz (kendi kartlarımız görünürdü);
//! yığın kapanınca bir sonraki bildirimde yenilenir.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, IsWindowVisible, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetWindowPos,
    ShowWindow,
};

use super::placement::{self, EDGE, Edge};
use crate::config::ConfigStore;
use crate::error::{AppError, AppResult};

pub const LABEL: &str = "notify";
pub const NOTIFY_BACKDROP: &str = "notify://backdrop";
const SHADOW_PAD: f64 = 16.0;
/// Kenar uzaklığının üst sınırı (mantıksal piksel); ekrandan taşma ayrıca kırpılır.
pub(super) const OFFSET_MAX: f64 = 4000.0;
/// Okunan alan (mantıksal piksel, sayfa zoom'u öncesi): kart (360) + pay;
/// üç kartlık yığından uzun.
const GLASS_W: f64 = 400.0;
const GLASS_H: f64 = 560.0;

#[derive(Serialize, Clone)]
pub struct Backdrop {
    /// `data:image/jpeg;base64,...`
    image: String,
    /// Ortalama parlaklık (0–1): açık zeminde örtü koyulaşır.
    luma: f64,
    /// Görüntünün kapladığı alan (sayfa CSS pikseli, zoom öncesi); yığınla
    /// aynı köşeye yaslı (sayfa `notifyPosition.anchor`'a göre hizalar).
    width: f64,
    height: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Horizontal {
    Left,
    Center,
    Right,
}

/// Kartların yeri (fiziksel piksel).
#[derive(Clone, Copy, Debug)]
pub(super) struct Spot {
    h: Horizontal,
    bottom: bool,
    x: i32,
    y: i32,
}

impl Spot {
    pub(super) fn parse(anchor: &str, x: i32, y: i32) -> Self {
        let bottom = anchor.starts_with("bottom");
        let h = if anchor.ends_with("Left") {
            Horizontal::Left
        } else if anchor.ends_with("Center") {
            Horizontal::Center
        } else {
            Horizontal::Right
        };
        Self { h, bottom, x, y }
    }

    /// `w`×`h` boyutlu yığının `area` içindeki sol üst köşesi (alanın içinde kalır).
    pub(super) fn origin(&self, area: RECT, w: i32, h: i32) -> (i32, i32) {
        let left = match self.h {
            Horizontal::Left => area.left + self.x,
            Horizontal::Center => area.left + (area.right - area.left - w) / 2,
            Horizontal::Right => area.right - self.x - w,
        };
        let top = if self.bottom {
            area.bottom - self.y - h
        } else {
            area.top + self.y
        };
        (
            left.clamp(area.left, (area.right - w).max(area.left)),
            top.clamp(area.top, (area.bottom - h).max(area.top)),
        )
    }
}

fn spot(app: &AppHandle, dpi: f64) -> Spot {
    let p = app
        .try_state::<ConfigStore>()
        .map(|c| c.get().notify_position)
        .unwrap_or_default();
    let px = |v: f64| (v.clamp(0.0, OFFSET_MAX) * dpi).round() as i32;
    Spot::parse(&p.anchor, px(p.x), px(p.y))
}

/// Kartların durabileceği alan: ana ekran, alt kenarda dock'un üstüne kadar.
fn area() -> RECT {
    area_in(placement::primary_monitor_rect())
}

/// `monitor` ekranında kartların durabileceği alan; dock yalnızca ana ekranda
/// olduğundan şerit orada düşülür (ekran kartları `flyout.rs` da kullanır).
pub(super) fn area_in(monitor: RECT) -> RECT {
    if monitor != placement::primary_monitor_rect() {
        return monitor;
    }
    match (EDGE, placement::reserved_rect()) {
        (Edge::Bottom, Some(bar)) => RECT {
            bottom: bar.top.min(monitor.bottom),
            ..monitor
        },
        (Edge::Top, Some(bar)) => RECT {
            top: bar.bottom.max(monitor.top),
            ..monitor
        },
        _ => monitor,
    }
}

fn window(app: &AppHandle) -> AppResult<WebviewWindow> {
    app.get_webview_window(LABEL)
        .ok_or_else(|| AppError::Win32("bildirim penceresi bulunamadı".into()))
}

pub fn init(app: &AppHandle) -> AppResult<()> {
    super::window_style::apply_dock_style(&window(app)?)
}

/// Yeni kart gösterilmeden hemen önce (`NOTIFY_SHOW`'dan önce) çağrılır.
pub fn before_show(app: &AppHandle) {
    let Ok(hwnd) = window(app).and_then(|w| Ok(w.hwnd()?)) else {
        return;
    };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return;
    }
    let dpi = placement::scale(hwnd);
    let scale = dpi * placement::ui_scale();
    let area = area();
    let w = ((GLASS_W * scale).round() as i32).min(area.right - area.left);
    let h = ((GLASS_H * scale).round() as i32).min(area.bottom - area.top);
    let (x, y) = spot(app, dpi).origin(area, w, h);
    if let Some(glass) = super::glass::capture(x, y, w, h) {
        let _ = app.emit_to(
            LABEL,
            NOTIFY_BACKDROP,
            Backdrop {
                image: glass.image,
                luma: glass.luma,
                width: f64::from(w) / scale,
                height: f64::from(h) / scale,
            },
        );
    }
}

/// `width`/`height`: kart yığını (CSS piksel × sayfa zoom'u). 0 → gizle.
/// Yer ayarı değişince sayfa yeniden çağırır (görünen kartlar yeni yere geçer).
pub fn layout(app: &AppHandle, width: f64, height: f64) -> AppResult<()> {
    let hwnd = window(app)?.hwnd()?;
    if height <= 0.0 {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        return Ok(());
    }
    let scale = placement::scale(hwnd);
    let pad = (SHADOW_PAD * scale).round() as i32;
    let w = (width * scale).round() as i32;
    let h = (height * scale).round() as i32;
    let (x, y) = spot(app, scale).origin(area(), w, h);
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x - pad,
            y - pad,
            w + 2 * pad,
            h + 2 * pad,
            SWP_NOACTIVATE,
        )?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1000,
    };

    #[test]
    fn corners() {
        assert_eq!(
            Spot::parse("topRight", 12, 12).origin(AREA, 360, 80),
            (1548, 12)
        );
        assert_eq!(
            Spot::parse("topLeft", 20, 30).origin(AREA, 360, 80),
            (20, 30)
        );
        assert_eq!(
            Spot::parse("bottomLeft", 20, 30).origin(AREA, 360, 80),
            (20, 890)
        );
        assert_eq!(
            Spot::parse("bottomCenter", 99, 0).origin(AREA, 360, 80),
            (780, 920)
        );
        // Bilinmeyen değer: sağ üst.
        assert_eq!(Spot::parse("?", 12, 12).origin(AREA, 360, 80), (1548, 12));
    }

    #[test]
    fn stays_on_screen() {
        assert_eq!(
            Spot::parse("topRight", 5000, 5000).origin(AREA, 360, 80),
            (0, 920)
        );
        // Yığın alandan büyükse sol üste yaslanır.
        assert_eq!(
            Spot::parse("bottomRight", 0, 0).origin(AREA, 4000, 4000),
            (0, 0)
        );
    }
}
