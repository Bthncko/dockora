//! Geri dönüşüm kutusu (8.8): öğe sayısı ve boyutu (`SHQueryRecycleBinW`),
//! açma ve boşaltma (`SHEmptyRecycleBinW`, Windows'un kendi onay penceresiyle).

use std::time::Duration;

use serde::Serialize;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::Shell::{
    FILEOPERATION_FLAGS, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_WANTNUKEWARNING,
    FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IShellItem,
    SHCreateItemFromParsingName, SHEmptyRecycleBinW, SHQUERYRBINFO, SHQueryRecycleBinW,
};
use windows::core::PCWSTR;

use super::runner;
use crate::error::{AppError, AppResult};
use crate::win::wide;

pub const RECYCLE_CHANGED: &str = "recycle://changed";
const POLL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecycleState {
    pub items: u64,
    pub bytes: u64,
}

pub type RecycleService = runner::ServiceHandle<Option<RecycleState>, ()>;

pub fn spawn(app: &tauri::AppHandle) {
    runner::spawn(app, "recycle", RECYCLE_CHANGED, POLL, None, |_| {
        |_: Option<()>| query()
    });
}

/// Tüm sürücülerin geri dönüşüm kutuları.
fn query() -> Option<RecycleState> {
    let mut info = SHQUERYRBINFO {
        cbSize: size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.ok()?;
    Some(RecycleState {
        items: info.i64NumItems.max(0) as u64,
        bytes: info.i64Size.max(0) as u64,
    })
}

pub fn open() -> AppResult<()> {
    super::launcher::shell_open("explorer.exe", "shell:RecycleBinFolder", "")
}

/// Windows onay sorar; kullanıcı vazgeçerse de hata dönmez.
pub fn empty() {
    unsafe {
        let _ = SHEmptyRecycleBinW(None, PCWSTR::null(), 0);
    }
}

/// Dock'taki çöp kutusuna bırakılan dosyalar → geri dönüşüm kutusu.
///
/// Gezgin'in kendi işlemi (`IFileOperation`, belgelenmiş): ilerleme penceresi,
/// kullanımdaki dosya uyarısı ve geri al (Ctrl+Z) Windows'ta olduğu gibi.
/// "Sil onayı" sorulmaz (Gezgin'de de varsayılan kapalı) ama öğe kutuya
/// sığmıyorsa / ağ sürücüsündeyse Windows "kalıcı silinecek" diye sorar
/// (`FOF_WANTNUKEWARNING`); sessizce kalıcı silme yok. Sürücü kökleri atlanır.
/// Döner: kutuya gönderilmek üzere sıraya giren öğe sayısı.
pub fn recycle(paths: Vec<String>) -> AppResult<usize> {
    let paths: Vec<String> = paths.into_iter().filter(|p| !is_root(p)).collect();
    if paths.is_empty() {
        return Ok(0);
    }
    // Kabuk işlemleri STA ister; komut iş parçacığı havuzu MTA olabilir.
    std::thread::Builder::new()
        .name("recycle-files".into())
        .spawn(move || unsafe { recycle_sta(&paths) })
        .map_err(|e| AppError::Win32(e.to_string()))?
        .join()
        .map_err(|_| AppError::Win32("çöp kutusu işlemi çöktü".into()))?
}

/// "C:\" / "C:" / "\\sunucu\paylaşım" gibi kökler.
fn is_root(path: &str) -> bool {
    let p = path.trim_end_matches(['\\', '/']);
    p.len() <= 2 || (p.starts_with(r"\\") && p[2..].matches('\\').count() <= 1)
}

unsafe fn recycle_sta(paths: &[String]) -> AppResult<usize> {
    let fail =
        |e: windows::core::Error| AppError::Win32(format!("çöpe atılamadı: {}", e.message()));
    unsafe {
        crate::win::com_sta();
        let op: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(fail)?;
        op.SetOperationFlags(FILEOPERATION_FLAGS(
            FOF_ALLOWUNDO.0
                | FOF_NOCONFIRMATION.0
                | FOF_WANTNUKEWARNING.0
                | FOFX_RECYCLEONDELETE.0
                | FOFX_ADDUNDORECORD.0,
        ))
        .map_err(fail)?;
        let mut queued = 0;
        for path in paths {
            let item: windows::core::Result<IShellItem> =
                SHCreateItemFromParsingName(PCWSTR(wide(path).as_ptr()), None);
            if let Ok(item) = item
                && op.DeleteItem(&item, None).is_ok()
            {
                queued += 1;
            }
        }
        if queued == 0 {
            return Err(AppError::Win32("dosya bulunamadı".into()));
        }
        op.PerformOperations().map_err(fail)?;
        Ok(queued)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn skips_roots() {
        for root in [r"C:\", "C:", r"\\sunucu\pay", r"\\sunucu\pay\"] {
            assert!(super::is_root(root), "{root}");
        }
        for path in [r"C:\a.txt", r"D:\klasör", r"\\sunucu\pay\dosya.txt"] {
            assert!(!super::is_root(path), "{path}");
        }
    }

    /// Bu makinede (yerel deneme): kutu okunabiliyor.
    #[test]
    #[ignore]
    fn reads_recycle_bin() {
        let state = super::query();
        println!("{state:?}");
        assert!(state.is_some());
    }
}
