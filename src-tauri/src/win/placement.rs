//! Dock penceresinin ekrandaki yeri.
//!
//! Ekranda ayrılan şerit (AppBar, bölüm 4.1) yalnızca bar yüksekliğindedir.
//! Pencere ise her zaman bar + büyütme payı yüksekliğindedir ve hiç yeniden
//! boyutlanmaz (bölüm 3.2).
//!
//! Neden Plan A değil: pencereyi yukarı doğru büyütmek alt kenarda hem ilk
//! karede titreme (WebView eski boyutta çiziliyor) hem de büyüyen ikonların
//! eski pencere sınırında kesilmesine yol açtı.
//!
//! Plan B'nin yoklamasız türevi: pencereye `SetWindowRgn` ile bölge veriyoruz.
//! Normalde bölge yalnızca bar şeridi; dışındaki şeffaf pay hem görünmez hem
//! de tıklamaları alttaki pencereye geçirir. İmleç bara girince (frontend
//! bildirir) bölge tüm pencereye açılır, çıkınca geri daralır. Bar her zaman
//! bölgenin içinde olduğundan girişi sayfa kendisi algılar; imleç yoklamaya gerek yok.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateRectRgn, DeleteObject, GetMonitorInfoW, HGDIOBJ, HRGN, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
    MONITORINFO, MonitorFromPoint, RDW_ALLCHILDREN, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow,
    SetWindowRgn,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos};

use super::appbar;
use crate::error::AppResult;

/// Ayrılan şerit (mantıksal piksel): 4 üst pay + 52 çubuk + 8 alt boşluk.
/// `src/styles/tokens.css` → `--bar-height` ile aynı tutulmalı.
pub const BAR_HEIGHT: f64 = 64.0;
/// Büyütülen ikonların taştığı ek pay (bölüm 3.2) → `--magnify-pad`.
pub const MAGNIFY_PAD: f64 = 44.0;
/// Otomatik gizlenmede kenardaki tetik şeridi (mantıksal piksel).
const TRIGGER: f64 = 2.0;

/// Kullanıcının boyut ayarı (7.9, `theme.scale`): sayfa CSS `zoom` ile aynı
/// çarpanla büyür, ayrılan şerit ve pay da. f64 bitleri olarak saklanır.
static UI_SCALE: AtomicU64 = AtomicU64::new(0x3FF0_0000_0000_0000); // 1.0

pub fn set_ui_scale(scale: f64) {
    UI_SCALE.store(scale.to_bits(), Ordering::SeqCst);
}

pub fn ui_scale() -> f64 {
    f64::from_bits(UI_SCALE.load(Ordering::SeqCst))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    #[allow(dead_code)] // Geliştirmede üst kenar kullanılmıştı; ayarlara açılacak.
    Top,
    Bottom,
}

/// Frontend'deki `DOCK_EDGE` ile aynı tutulmalı.
pub const EDGE: Edge = Edge::Bottom;

struct Geometry {
    expanded: bool,
    /// Otomatik gizlenme (8.10): bölge kenardaki tetik şeridi.
    hidden: bool,
    /// AppBar'ın ayırdığı şerit (fiziksel piksel, ekran koordinatı).
    reserved: Option<RECT>,
    /// Büyütme payı (fiziksel piksel).
    pad: i32,
}

static GEOMETRY: Mutex<Geometry> = Mutex::new(Geometry {
    expanded: false,
    hidden: false,
    reserved: None,
    pad: 0,
});

/// Ekranda ayrılan yeri yeniden hesaplar (açılış, çözünürlük/DPI değişimi,
/// başka bir appbar'ın değişmesi) ve pencereyi yerleştirir.
pub fn reposition(hwnd: HWND) -> AppResult<()> {
    let monitor = primary_monitor_rect();
    let scale = scale(hwnd) * ui_scale();
    let thickness = (BAR_HEIGHT * scale).round() as i32;
    let pad = (MAGNIFY_PAD * scale).round() as i32;
    let reserved =
        appbar::set_pos(hwnd, monitor, thickness).unwrap_or_else(|| edge_strip(monitor, thickness));
    {
        let mut g = GEOMETRY.lock().unwrap();
        g.reserved = Some(reserved);
        g.pad = pad;
    }

    // Pencere = ayrılan şerit + ekranın içine doğru pay.
    let window = match EDGE {
        Edge::Bottom => RECT {
            top: reserved.top - pad,
            ..reserved
        },
        Edge::Top => RECT {
            bottom: reserved.bottom + pad,
            ..reserved
        },
    };
    // Kilit bırakıldı: SetWindowPos pencere mesajı tetikler.
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            window.left,
            window.top,
            window.right - window.left,
            window.bottom - window.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
    }
    apply_region(hwnd);
    Ok(())
}

/// Büyütme sırasında görünür/tıklanır bölgeyi tüm pencereye açar / bara daraltır.
pub fn set_expanded(hwnd: HWND, expanded: bool) {
    GEOMETRY.lock().unwrap().expanded = expanded;
    apply_region(hwnd);
    if !expanded {
        appbar::poke_autohide();
    }
}

/// Büyütme / sürükleme sürüyor (otomatik gizlenme beklesin).
pub fn is_expanded() -> bool {
    GEOMETRY.lock().unwrap().expanded
}

/// Otomatik gizlenme (8.10): bölgeyi kenardaki tetik şeridine indir / geri aç.
pub fn set_hidden(hwnd: HWND, hidden: bool) {
    GEOMETRY.lock().unwrap().hidden = hidden;
    apply_region(hwnd);
}

/// Bölgeyi pencereye verir. Başarılıysa sahibi sistemdir; başarısızsa biz bırakırız.
unsafe fn set_region(hwnd: HWND, region: HRGN) {
    unsafe {
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
        }
    }
}

fn apply_region(hwnd: HWND) {
    let (expanded, hidden, reserved, pad) = {
        let g = GEOMETRY.lock().unwrap();
        (g.expanded, g.hidden, g.reserved, g.pad)
    };
    let Some(reserved) = reserved else { return };
    unsafe {
        if hidden {
            // Ekran kenarında ince tetik şeridi: imleç değince sayfa haber verir.
            let width = reserved.right - reserved.left;
            let height = reserved.bottom - reserved.top;
            let trigger = (TRIGGER * scale(hwnd)).round().max(2.0) as i32;
            let region = match EDGE {
                Edge::Bottom => CreateRectRgn(0, pad + height - trigger, width, pad + height),
                Edge::Top => CreateRectRgn(0, 0, width, trigger),
            };
            set_region(hwnd, region);
        } else if expanded {
            // Bölge yok = tüm pencere.
            SetWindowRgn(hwnd, None, true);
            // WebView2 bölge dışındaki alanı çizmeyi atlıyor; bölge açılınca
            // yeni kare üretmezse büyüyen ikonların üstü kesik kalıyordu.
            let _ = RedrawWindow(
                Some(hwnd),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW,
            );
        } else {
            // Pencere koordinatında bar şeridi. Bölgenin sahibi artık sistem.
            let width = reserved.right - reserved.left;
            let height = reserved.bottom - reserved.top;
            let region = match EDGE {
                Edge::Bottom => CreateRectRgn(0, pad, width, pad + height),
                Edge::Top => CreateRectRgn(0, 0, width, height),
            };
            set_region(hwnd, region);
        }
    }
}

/// Bar şeridi (fiziksel piksel, ekran koordinatı); popup'lar bunun üstüne açılır.
pub fn reserved_rect() -> Option<RECT> {
    GEOMETRY.lock().unwrap().reserved
}

/// AppBar yoksa: monitörün kenarında bar kalınlığında şerit.
fn edge_strip(monitor: RECT, thickness: i32) -> RECT {
    match EDGE {
        Edge::Bottom => RECT {
            top: monitor.bottom - thickness,
            ..monitor
        },
        Edge::Top => RECT {
            bottom: monitor.top + thickness,
            ..monitor
        },
    }
}

/// v1 yalnızca birincil ekran (bölüm 11).
pub fn primary_monitor_rect() -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        info.rcMonitor
    }
}

/// Noktanın bulunduğu (ya da en yakın) monitörün dikdörtgeni (8.1: çoklu monitöre hazırlık).
pub fn monitor_rect_at(x: i32, y: i32) -> RECT {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        info.rcMonitor
    }
}

pub fn scale(hwnd: HWND) -> f64 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 1.0,
        dpi => dpi as f64 / 96.0,
    }
}
