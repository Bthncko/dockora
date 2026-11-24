//! Görev çubuğunda görünmesi gereken üst düzey pencereleri listeler (bölüm 5.1).

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND, LPARAM};
use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

use crate::win::{hwnd_to_id, take_pwstr};

#[derive(Debug, Clone)]
pub struct RawWindow {
    pub hwnd: isize,
    pub title: String,
    pub pid: u32,
}

/// Kabuğa ait, görev çubuğunda gösterilmeyen sınıflar.
const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

/// Z sırasıyla (en üstteki önce) görev çubuğu pencereleri.
pub fn list(own_pid: u32) -> Vec<RawWindow> {
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut Vec<HWND> as isize));
    }
    all.into_iter()
        .filter_map(|h| inspect(h, own_pid))
        .collect()
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam, `list` içindeki yerel Vec'in adresi; EnumWindows senkron.
    let all = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    all.push(hwnd);
    true.into()
}

fn inspect(hwnd: HWND, own_pid: u32) -> Option<RawWindow> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        // Sahibi olan pencereler (iletişim kutuları vb.) ayrı düğme almaz.
        if matches!(GetWindow(hwnd, GW_OWNER), Ok(owner) if !owner.is_invalid()) {
            return None;
        }
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let app_window = ex & WS_EX_APPWINDOW.0 != 0;
        if !app_window && ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 {
            return None;
        }
        // Gizlenmiş (cloaked) pencereler: UWP hayaletleri, diğer sanal masaüstleri.
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return None;
        }
        let title = window_text(hwnd);
        if title.trim().is_empty() {
            return None;
        }
        if SHELL_CLASSES.contains(&crate::win::class_name(hwnd).as_str()) {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == own_pid {
            return None;
        }
        Some(RawWindow {
            hwnd: hwnd_to_id(hwnd),
            title,
            pid,
        })
    }
}

fn window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

/// Sürecin exe yolu.
pub fn process_image(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        result.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Paketli (MSIX / Mağaza) sürecin AUMID'si, ör. `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`.
/// Paketsiz süreçte `None`. Bu uygulamaların exe'si `WindowsApps` altında korumalıdır;
/// ancak AUMID ile başlatılabilirler.
pub fn process_aumid(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let result = GetApplicationUserModelId(
            process,
            &mut len,
            Some(windows::core::PWSTR(buf.as_mut_ptr())),
        );
        let _ = CloseHandle(process);
        if result != ERROR_SUCCESS || len == 0 {
            return None;
        }
        // len sondaki sıfırı da sayar.
        let s = String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]);
        (!s.is_empty()).then_some(s)
    }
}

/// Pencerenin açıkça atanmış AppUserModelID'si (çoğu klasik uygulamada yoktur).
/// Çağıran iş parçacığında COM başlatılmış olmalı.
pub fn window_aumid(hwnd: isize) -> Option<String> {
    unsafe {
        let store: IPropertyStore =
            SHGetPropertyStoreForWindow(crate::win::id_to_hwnd(hwnd)).ok()?;
        let value = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let s = take_pwstr(PropVariantToStringAlloc(&value).ok()?);
        (!s.is_empty()).then_some(s)
    }
}
