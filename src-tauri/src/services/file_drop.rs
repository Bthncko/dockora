//! Dock'a sürüklenen dosyalar (8.7): uygulamayla açma ve kısayol çözme.
//!
//! - Masaüstü uygulaması: `ShellExecuteW(exe, "dosya1" "dosya2")`.
//! - Paketli (Mağaza) uygulama: exe doğrudan çalışmaz;
//!   `IApplicationActivationManager::ActivateForFile` (belgelenmiş) ile açılır.
//!   Uygulama o dosya türünü desteklemiyorsa Windows hata döndürür.
//! - `.lnk`: kabuk özellikleriyle hedef exe ve kısayolun AUMID'si (varsa
//!   çalışan pencereyle aynı ikonda gruplanır).

use windows::Win32::Storage::EnhancedStorage::{PKEY_AppUserModel_ID, PKEY_Link_TargetParsingPath};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, IApplicationActivationManager, ILCreateFromPathW, ILFree,
    IShellItem2, SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists,
};
use windows::core::{PCWSTR, w};

use super::launcher;
use crate::error::{AppError, AppResult};
use crate::win::{take_pwstr, wide};

fn com() {
    crate::win::com_sta();
}

/// Paketli uygulamaların exe'si `WindowsApps` altında; doğrudan başlatılamaz.
fn packaged(exe: &str) -> bool {
    exe.to_lowercase().contains("\\windowsapps\\")
}

/// Dosyaları uygulamayla aç.
pub fn open_with(exe: Option<&str>, aumid: Option<&str>, paths: &[String]) -> AppResult<()> {
    if paths.is_empty() {
        return Ok(());
    }
    match (exe, aumid) {
        (Some(exe), _) if !packaged(exe) => {
            let params = paths
                .iter()
                .map(|p| format!("\"{p}\""))
                .collect::<Vec<_>>()
                .join(" ");
            launcher::shell_open(exe, &params, "")
        }
        (_, Some(aumid)) => activate_for_files(aumid, paths),
        _ => Err(AppError::Win32("bu uygulama dosyayla açılamıyor".into())),
    }
}

fn activate_for_files(aumid: &str, paths: &[String]) -> AppResult<()> {
    com();
    unsafe {
        let pidls: Vec<*mut ITEMIDLIST> = paths
            .iter()
            .map(|p| ILCreateFromPathW(PCWSTR(wide(p).as_ptr())))
            .filter(|p| !p.is_null())
            .collect();
        if pidls.is_empty() {
            return Err(AppError::Win32("dosya bulunamadı".into()));
        }
        let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
        let result = (|| -> windows::core::Result<()> {
            let items = SHCreateShellItemArrayFromIDLists(&consts)?;
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_INPROC_SERVER)?;
            let id = wide(aumid);
            manager.ActivateForFile(PCWSTR(id.as_ptr()), &items, w!("open"))?;
            Ok(())
        })();
        for p in pidls {
            ILFree(Some(p as *const _));
        }
        result.map_err(|e| AppError::Win32(format!("dosya açılamadı: {}", e.message())))
    }
}

/// Kısayolun hedefi: (exe, AUMID). Hedef bir uygulama değilse `None`.
/// Kabuğun `System.Link.TargetParsingPath` özelliği MSI'ın "ilan edilmiş"
/// kısayollarını da (Office) çözer; `IShellLinkW::GetPath` onlarda boş döner.
pub fn resolve_link(path: &str) -> Option<(Option<String>, Option<String>)> {
    com();
    unsafe {
        let item: IShellItem2 =
            SHCreateItemFromParsingName(PCWSTR(wide(path).as_ptr()), None).ok()?;
        let read = |key| {
            item.GetString(key)
                .ok()
                .map(|p| take_pwstr(p))
                .filter(|s| !s.trim().is_empty())
        };
        let target =
            read(&PKEY_Link_TargetParsingPath).filter(|t| t.to_lowercase().ends_with(".exe"));
        let aumid = read(&PKEY_AppUserModel_ID);
        // Mağaza uygulaması kısayolu: hedef exe yok ama AUMID var.
        match (target, aumid) {
            (None, None) => None,
            pair => Some(pair),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bu makinedeki Başlat kısayollarıyla (yerel deneme).
    #[test]
    #[ignore]
    fn resolves_start_menu_links() {
        let dir = r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs";
        for name in ["Excel.lnk", "Firefox.lnk", "Microsoft Edge.lnk"] {
            let r = resolve_link(&format!("{dir}\\{name}"));
            println!("{name}: {r:?}");
            assert!(r.is_some_and(|(exe, _)| exe.is_some()), "{name}");
        }
    }
}
