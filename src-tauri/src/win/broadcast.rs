//! Sistem geneli yayın (`HWND_BROADCAST`) yerine üst düzey pencerelere tek tek
//! gönderme: bazı pencereler atlanır.
//!
//! Neden (2026-11-20, Wallpaper Engine ile denendi): hareketli duvar kâğıdı
//! uygulaması iki yayında duvar kâğıdını baştan kuruyor, arada Windows'un kendi
//! duvar kâğıdı görünüyordu:
//! - "TaskbarCreated": Wallpaper Engine'in olay penceresi (`WPEEventWindow`)
//!   bunu Explorer yeniden başladı sanıyor. Tepsi penceresi (`WPETrayWindow`)
//!   simgesini yeniden ekliyor, duvar kâğıdına dokunmuyor → o alır.
//! - `WM_SETTINGCHANGE(SPI_SETWORKAREA)`: masaüstü (`Progman`) bunu alınca
//!   duvar kâğıdı katmanını (`WorkerW`) yeniden kuruyor; hareketli duvar
//!   kâğıtları da onunla. Masaüstü simgeleri zaten çalışma alanına göre
//!   yerleşiyor (alan Explorer'ın kendi değeri).
//!
//! `HWND_BROADCAST` da yalnızca üst düzey pencerelere gider; burada da öyle.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, SendNotifyMessageW};
use windows::core::BOOL;

/// Her yayında atlananlar.
const SKIP_ALWAYS: &[&str] = &["WPEEventWindow"];
/// Çalışma alanı bildiriminde ayrıca atlananlar (masaüstü).
const SKIP_DESKTOP: &[&str] = &["Progman", "WorkerW"];

/// Kime gönderilmeyeceği.
#[derive(Clone, Copy)]
pub enum Skip {
    /// Duvar kâğıdını sıfırlayan pencereler.
    Wallpaper,
    /// Bunlar + masaüstü (çalışma alanı bildirimi).
    WallpaperAndDesktop,
}

/// `msg`'yi üst düzey pencerelere beklemeden gönderir (`SendNotifyMessage`).
/// İşaretçi taşıyan sistem iletileri için değil (`lp` başka sürece taşınmaz).
pub fn send(msg: u32, wp: WPARAM, lp: LPARAM, skip: Skip) {
    for hwnd in top_level() {
        let class = super::class_name(hwnd);
        let skipped = SKIP_ALWAYS.contains(&class.as_str())
            || (matches!(skip, Skip::WallpaperAndDesktop)
                && SKIP_DESKTOP.contains(&class.as_str()));
        if !skipped {
            unsafe {
                let _ = SendNotifyMessageW(hwnd, msg, wp, lp);
            }
        }
    }
}

fn top_level() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
        // SAFETY: lp, `top_level`'deki Vec'e işaret ediyor; EnumWindows dönene kadar geçerli.
        unsafe { (*(lp.0 as *mut Vec<HWND>)).push(hwnd) };
        true.into()
    }
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut out as *mut Vec<HWND> as isize));
    }
    out
}

