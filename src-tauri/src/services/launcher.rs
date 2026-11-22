//! Uygulama başlatma ve Başlat menüsü (bölüm 5.5).

use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_A, VK_B, VK_CONTROL, VK_D, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_N, VK_RETURN, VK_RWIN, VK_S, VK_SHIFT, VK_SPACE, VK_TAB, VK_V,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, SW_SHOWNORMAL};
use windows::core::{PCWSTR, w};

use crate::error::{AppError, AppResult};
use crate::win::wide;

#[derive(Debug, Clone, PartialEq)]
pub enum LaunchTarget {
    Exe(String),
    /// Paketli (UWP / Mağaza) uygulama: `explorer.exe shell:AppsFolder\<AUMID>`.
    Aumid(String),
}

/// AUMID, Başlat'taki uygulamalar listesinde (`shell:AppsFolder`) varsa her zaman
/// onunla başlat. Paketli uygulamaların exe'si doğrudan çalışmaz; tarayıcıdaki web
/// uygulamalarının (Edge/Chrome PWA, Firefox web uygulaması) exe'si ise yalnızca
/// düz tarayıcıyı açar. Exe yalnızca AUMID çözülmezse kullanılır.
pub fn launch_target(
    exe: Option<&str>,
    aumid: Option<&str>,
    aumid_resolves: bool,
) -> Option<LaunchTarget> {
    match (exe, aumid) {
        (_, Some(aumid)) if aumid_resolves => Some(LaunchTarget::Aumid(aumid.to_string())),
        (Some(exe), _) => Some(LaunchTarget::Exe(exe.to_string())),
        _ => None,
    }
}

pub fn launch(target: &LaunchTarget) -> AppResult<()> {
    let (file, params) = match target {
        LaunchTarget::Exe(path) => (path.clone(), String::new()),
        LaunchTarget::Aumid(aumid) => (
            "explorer.exe".to_string(),
            format!("shell:AppsFolder\\{aumid}"),
        ),
    };
    let file_w = wide(&file);
    let params_w = wide(&params);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(file_w.as_ptr()),
            if params.is_empty() {
                PCWSTR::null()
            } else {
                PCWSTR(params_w.as_ptr())
            },
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute: 32'den büyük değer başarı demek.
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(AppError::Win32(format!(
            "başlatılamadı: {file} {params} (kod {})",
            result.0 as isize
        )))
    }
}

/// Programı bağımsız değişkenler ve çalışma klasörüyle açar (atlama listesi görevleri).
pub fn shell_open(file: &str, params: &str, dir: &str) -> AppResult<()> {
    let file_w = wide(file);
    let params_w = wide(params);
    let dir_w = wide(dir);
    let opt = |s: &str, w: &[u16]| {
        if s.is_empty() {
            PCWSTR::null()
        } else {
            PCWSTR(w.as_ptr())
        }
    };
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(file_w.as_ptr()),
            opt(params, &params_w),
            opt(dir, &dir_w),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(AppError::Win32(format!(
            "açılamadı (kod {})",
            result.0 as isize
        )))
    }
}

/// `ms-settings:` gibi bir adresi kabuğa açtırır.
pub fn open_uri(uri: &str) -> AppResult<()> {
    let uri_w = wide(uri);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(uri_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(AppError::Win32(format!("açılamadı: {uri}")))
    }
}

/// Windows'un kendi Başlat menüsü: Win tuşuna basıp bırak.
pub fn open_start() -> AppResult<()> {
    send_chord(&[VK_LWIN])
}

/// Sistem düğmeleri (8.2): Windows'un kendi panelini açan kısayol.
pub fn system_shortcut(action: &str) -> AppResult<()> {
    let keys: &[VIRTUAL_KEY] = match action {
        "notifications" => &[VK_LWIN, VK_N],
        "quickSettings" => &[VK_LWIN, VK_A],
        "taskView" => &[VK_LWIN, VK_TAB],
        "search" => &[VK_LWIN, VK_S],
        // Pano geçmişi kapalıysa Windows kendi panelinde "Aç" düğmesini gösterir.
        "clipboard" => &[VK_LWIN, VK_V],
        // `ms-screenclip:` (adsız) Mayıs 2025'te kaldırıldı; yenisi yanıt adresi
        // istiyor. Kısayol her sürümde aynı alıntı katmanını açar.
        "snip" => &[VK_LWIN, VK_LSHIFT, VK_S],
        "desktop" => &[VK_LWIN, VK_D],
        // Giriş dili / klavye düzeni değiştirici (8.8).
        "inputSwitch" => &[VK_LWIN, VK_SPACE],
        "tray" => return open_tray(),
        _ => return Err(AppError::Win32(format!("bilinmeyen kısayol: {action}"))),
    };
    send_chord(keys)
}

/// Tepsi taşma penceresi ("gizli simgeler", 8.6). Denendi (bu makinede,
/// Windows çubuğu gizliyken): Win+B klavye odağını gizli çubuktaki "Gizli
/// simgeleri göster" okuna getirir; ok Enter ile aç / kapa çalışır. Pencere
/// dock'un üstünde, sağda açılır ve fareyle kullanılır. Bazen Win+B pencereyi
/// kendisi açıp öne getirir; o zaman Enter gönderilmez (yoksa içindeki ilk
/// simge çalışır). Pencerenin açık olup olmadığı görünürlüğünden anlaşılmıyor
/// (kapalıyken de "görünür"), bu yüzden ön plandaki pencereye bakılır.
pub fn open_tray() -> AppResult<()> {
    const WAIT: Duration = Duration::from_millis(450);
    // Önce UI Automation ile ok düğmesi (Win+B'ye bağlı değil, çubuk gizliyken de olur).
    if crate::win::tray::invoke_chevron() {
        return Ok(());
    }
    send_chord(&[VK_LWIN, VK_B])?;
    let start = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let class = foreground_class();
        if class == "TopLevelWindowForOverflowXamlIsland" {
            return Ok(()); // kendisi açıldı
        }
        if start.elapsed() >= WAIT {
            if class == "Shell_TrayWnd" {
                send_chord(&[VK_RETURN])?; // odak okta: aç
            }
            return Ok(());
        }
    }
}

fn foreground_class() -> String {
    let mut buf = [0u16; 128];
    let len = unsafe { GetClassNameW(GetForegroundWindow(), &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

/// Kullanıcının basılı tuttuğu Ctrl / Shift / Alt / Win kombinasyonu bozar
/// (Win+N, Ctrl+Win+N olur): bırakılmasını en çok `MAX` kadar bekle.
fn wait_for_modifiers() {
    const MAX: Duration = Duration::from_millis(1500);
    let held = || {
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            // En yüksek bit: tuş şu an basılı.
            .any(|&vk| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0)
    };
    let start = Instant::now();
    while held() && start.elapsed() < MAX {
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Tuş kombinasyonu gönderir (8.1): tuşlar sırayla basılır, ters sırayla
/// bırakılır. Ör. bildirim merkezi `send_chord(&[VK_LWIN, VK_N])`. Basılı
/// değiştirici tuş varsa önce bırakılması beklenir (8.2).
pub fn send_chord(keys: &[VIRTUAL_KEY]) -> AppResult<()> {
    wait_for_modifiers();
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&vk| key(vk, KEYBD_EVENT_FLAGS(0)))
        .chain(keys.iter().rev().map(|&vk| key(vk, KEYEVENTF_KEYUP)))
        .collect();
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(AppError::Win32("SendInput başarısız".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_app_launches_by_aumid_not_browser() {
        let edge = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe";
        assert_eq!(
            launch_target(Some(edge), Some("Facebook.InstagramBeta_x!App"), true),
            Some(LaunchTarget::Aumid("Facebook.InstagramBeta_x!App".into()))
        );
    }

    #[test]
    fn falls_back_to_exe_when_aumid_unknown() {
        let code = r"C:\Code\Code.exe";
        assert_eq!(
            launch_target(Some(code), Some("Some.Window.Id"), false),
            Some(LaunchTarget::Exe(code.into()))
        );
        assert_eq!(
            launch_target(Some(code), None, false),
            Some(LaunchTarget::Exe(code.into()))
        );
        assert_eq!(launch_target(None, Some("x"), false), None);
    }
}
