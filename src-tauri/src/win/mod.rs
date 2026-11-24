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

thread_local! {
    /// Bu iş parçacığında COM zaten başlatıldı (havuz iş parçacıklarında her
    /// komutta yeniden başlatıp sayacı şişirmemek için).
    static COM_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Çağıran iş parçacığında COM'u çok iş parçacıklı daireyle (MTA) başlatır.
/// Zaten başlatılmışsa (ya da başka dairedeyse) hata yok sayılır.
pub fn com_mta() {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    COM_READY.with(|c| {
        if !c.replace(true) {
            // SAFETY: yalnızca çağıran iş parçacığının COM durumunu değiştirir.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
        }
    });
}

/// Tek iş parçacıklı daire (STA): kabuk nesneleri (geri dönüşüm kutusu, JumpList,
/// IShellLink) için.
pub fn com_sta() {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    COM_READY.with(|c| {
        if !c.replace(true) {
            // SAFETY: yalnızca çağıran iş parçacığının COM durumunu değiştirir.
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
        }
    });
}

/// Pencerenin sınıf adı (en çok 256 UTF-16 birimi; boşsa pencere yok demektir).
pub fn class_name(hwnd: HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;
    let mut buf = [0u16; 256];
    // SAFETY: tampon yerel ve yeterli büyüklükte.
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
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
