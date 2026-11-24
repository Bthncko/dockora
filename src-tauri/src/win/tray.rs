//! Tepsi taşma penceresini ("gizli simgeler", 8.6) dock'taki tepsi düğmesinin
//! tam üstüne taşır.
//!
//! Pencere Explorer'ın; Windows onu gizli çubuktaki okun yerine göre açar.
//! Açıldıktan sonra konumu `SetWindowPos` ile değiştirilir (aynı bütünlük
//! düzeyindeki başka sürecin penceresi: izin var). Görünümü değiştirilemez;
//! içerik ve menüler Windows'un.

use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
};
use windows::core::w;

use super::placement::{self, EDGE, Edge};
use crate::error::{AppError, AppResult};

/// Bar ile pencere arası (popup ile aynı, mantıksal piksel).
const GAP: f64 = 10.0;
const MARGIN: f64 = 8.0;
/// Pencere açıldıktan sonra Windows'un kendi konumlamasını bitirmesi için.
const SETTLE: Duration = Duration::from_millis(120);
const WAIT: Duration = Duration::from_millis(900);

/// Düğmenin dock penceresindeki yatay konumu (CSS piksel).
#[derive(Deserialize, Clone, Copy)]
pub struct Anchor {
    x: f64,
    width: f64,
}

/// Taşma penceresini aç ve düğmenin üstüne taşı.
/// Düğme yoksa (popup'taki "Windows tepsisi" bağlantısı) pencere dock'un sağ ucuna gelir.
pub fn open(app: &AppHandle, anchor: Option<Anchor>) -> AppResult<()> {
    crate::services::launcher::open_tray()?;
    let start = Instant::now();
    while start.elapsed() < WAIT {
        let hwnd = unsafe { FindWindowW(w!("TopLevelWindowForOverflowXamlIsland"), None) };
        if let Ok(hwnd) = hwnd
            && !hwnd.is_invalid()
        {
            std::thread::sleep(SETTLE);
            return place(app, hwnd, anchor);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Ok(())
}

fn place(
    app: &AppHandle,
    overflow: windows::Win32::Foundation::HWND,
    anchor: Option<Anchor>,
) -> AppResult<()> {
    let dock = app
        .get_webview_window("dock")
        .ok_or(AppError::NoDockWindow)?;
    let dock_hwnd = dock.hwnd()?;
    let scale = placement::scale(dock_hwnd);
    let mut dock_rect = RECT::default();
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(dock_hwnd, &mut dock_rect)?;
        GetWindowRect(overflow, &mut rect)?;
    }
    let bar = placement::reserved_rect().unwrap_or(dock_rect);
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    let gap = (GAP * scale).round() as i32;
    let margin = (MARGIN * scale).round() as i32;
    let center = match anchor {
        Some(a) => dock_rect.left + ((a.x + a.width / 2.0) * scale).round() as i32,
        None => dock_rect.right - margin - w / 2,
    };
    let monitor = placement::monitor_rect_at(center, (bar.top + bar.bottom) / 2);
    let x = (center - w / 2).clamp(
        monitor.left + margin,
        (monitor.right - margin - w).max(monitor.left + margin),
    );
    let y = match EDGE {
        Edge::Bottom => bar.top - gap - h,
        Edge::Top => bar.bottom + gap,
    };
    unsafe {
        SetWindowPos(
            overflow,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
    }
    Ok(())
}

/// Windows 11 görev çubuğundaki "Gizli simgeleri göster" okunu UI Automation ile
/// tetikler (çubuk gizli olsa da çalışır; tuş göndermez, Başlat'ı açmaz).
/// `true`: ok bulundu ve tetiklendi.
pub fn invoke_chevron() -> bool {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationInvokePattern, TreeScope_Descendants,
        UIA_InvokePatternId,
    };

    let Some(taskbar) = super::tray_host::explorer_tray() else {
        return false;
    };
    unsafe {
        super::com_mta();
        let Ok(auto) =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        else {
            return false;
        };
        let Ok(root) = auto.ElementFromHandle(taskbar) else {
            return false;
        };
        let Ok(all) = auto.CreateTrueCondition() else {
            return false;
        };
        let Ok(found) = root.FindAll(TreeScope_Descendants, &all) else {
            return false;
        };
        for i in 0..found.Length().unwrap_or(0) {
            let Ok(el) = found.GetElement(i) else {
                continue;
            };
            let id = el
                .CurrentAutomationId()
                .map(|b| b.to_string())
                .unwrap_or_default();
            let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            if !is_chevron(&id, &name) {
                continue;
            }
            if let Ok(p) = el.GetCurrentPattern(UIA_InvokePatternId)
                && let Ok(invoke) = windows::core::Interface::cast::<IUIAutomationInvokePattern>(&p)
                && invoke.Invoke().is_ok()
            {
                return true;
            }
        }
    }
    false
}

/// Win11: AutomationId "NotificationChevron"; ad yerelleşir ("Show hidden icons" /
/// "Gizli simgeleri göster"), yedek olarak ona da bakılır.
fn is_chevron(id: &str, name: &str) -> bool {
    let id = id.to_lowercase();
    let name = name.to_lowercase();
    id.contains("notificationchevron")
        || name.contains("hidden icons")
        || name.contains("gizli simge")
}

#[cfg(test)]
mod tests {
    use super::is_chevron;

    #[test]
    fn finds_chevron() {
        assert!(is_chevron("NotificationChevron", ""));
        assert!(is_chevron("", "Show Hidden Icons"));
        assert!(is_chevron("", "Gizli simgeleri göster"));
        assert!(!is_chevron("SystemTrayIcon", "Volume"));
    }
}
