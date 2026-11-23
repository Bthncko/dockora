pub mod appbar;
pub mod autohide;
pub mod broadcast;
pub mod flyout;
pub mod focus;
pub mod glass;
pub mod notify;
pub mod osd_guard;
pub mod placement;
pub mod popup;
pub mod preview;
pub mod settings;
pub mod window_fx;
pub mod taskbar;
pub mod tray;
pub mod tray_host;
pub mod window_style;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
use windows::core::PWSTR;

/// Sonu sıfırlı UTF-16 dize.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// COM'un ayırdığı dizeyi Rust'a kopyalar ve belleği serbest bırakır.
///
/// # Safety
/// `p`, `CoTaskMemAlloc` ile ayrılmış geçerli bir dize olmalı.
pub unsafe fn take_pwstr(p: PWSTR) -> String {
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

/// HWND'yi JSON'a ve geri taşımak için sayıya çevirir.
pub fn hwnd_to_id(h: HWND) -> isize {
    h.0 as isize
}

pub fn id_to_hwnd(id: isize) -> HWND {
    HWND(id as *mut _)
}

/// Pencere bu sürecin mi (ör. kendi `Shell_TrayWnd`'ımız, `tray_host`).
pub fn is_own_window(hwnd: HWND) -> bool {
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() }
}

/// Temiz çıkış (bölüm 11): AppBar'ı kaldır → Windows çubuğunu geri getir.
/// Normal çıkışta ve panik kancasında çağrılır; iki kez çağrılması zararsız.
pub fn shutdown() {
    // Önce kendi tepsimiz: görev çubuğu iletileri yine Explorer'a gitsin.
    tray_host::shutdown();
    // Dock yokken bildirimler yine Windows'un kutucuklarında görünsün.
    crate::services::notifications::shutdown();
    appbar::remove();
    taskbar::cleanup();
    // Uygulama pencerelerinde kapattığımız Windows geçişlerini geri aç.
    window_fx::shutdown();
}
