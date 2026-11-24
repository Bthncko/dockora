//! Pil panelindeki ayarlar: enerji tasarrufu (tek tık), parlaklık, ekran
//! yenileme hızı, pildeyken ekranı kapatma / uyku süreleri.
//!
//! - Enerji tasarrufunu "şimdi aç" için genel bir API yok. Windows'un kendi
//!   "Pilde şu yüzdede aç" eşiği (`ESBATTTHRESHOLD`, belgelenmiş güç ayarı)
//!   100'e çekilince tasarruf pildeyken hemen açılır. Asıl eşik diske yazılır,
//!   kapatınca ya da prize takılınca geri yüklenir.
//! - Parlaklık: WMI `root\WMI` → `WmiMonitorBrightness` / `WmiSetBrightness`
//!   (yalnızca dizüstü panelleri; harici monitörde yok → `None`).
//! - Yenileme hızı: `EnumDisplaySettingsW` + `ChangeDisplaySettingsExW`, ana ekran.
//! - Süreler: `PowerReadDCValueIndex` / `PowerWriteDCValueIndex` (etkin düzen).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{HLOCAL, LocalFree, WIN32_ERROR};
use windows::Win32::Graphics::Gdi::{
    CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL,
    DM_DISPLAYFREQUENCY, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsW,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Power::{
    PowerGetActiveScheme, PowerReadDCValueIndex, PowerSetActiveScheme, PowerWriteDCValueIndex,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE, WbemLocator,
};
use windows::core::{BSTR, GUID, PCWSTR, w};

// learn.microsoft.com/windows-hardware/customize/power-settings
const SUB_ENERGYSAVER: GUID = GUID::from_u128(0xde830923_a562_41af_a086_e3a2c6bad2da);
const ESBATTTHRESHOLD: GUID = GUID::from_u128(0xe69653ca_cf7f_4f05_aa73_cb833fa90ad4);
const SUB_VIDEO: GUID = GUID::from_u128(0x7516b95f_f776_4464_8c53_06167f40cc99);
const VIDEOIDLE: GUID = GUID::from_u128(0x3c0bc021_c8a8_4e07_a973_6b14cbcb2b7e);
const SUB_SLEEP: GUID = GUID::from_u128(0x238c9fa8_0aad_41ed_83f4_97be242c8f20);
const STANDBYIDLE: GUID = GUID::from_u128(0x29f6c1db_86da_48c5_9fdb_f2b67b1f44da);

/// "Her zaman" eşiği: pildeyken tasarruf hemen açılır.
const ALWAYS: u32 = 100;
const SAVED_FILE: &str = "energy-saver.json";

static SAVED_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Eşik okuma-yazma sırası karışmasın (panel + pil servisi).
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct Saved {
    /// Kullanıcının asıl eşiği (%).
    threshold: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatterySettings {
    /// Parlaklık (%); ayarlanamıyorsa `None`.
    pub brightness: Option<u8>,
    /// Ana ekranın şu anki ve seçilebilir yenileme hızları (Hz).
    pub refresh: Option<Refresh>,
    /// Pildeyken ekranı kapat / uyku (sn, 0 = hiçbir zaman).
    pub screen_off: Option<u32>,
    pub sleep: Option<u32>,
    /// Enerji tasarrufunun otomatik açıldığı yüzde (asıl ayar, bizim 100'ümüz değil).
    pub saver_threshold: Option<u32>,
}

#[derive(Serialize)]
pub struct Refresh {
    pub current: u32,
    pub options: Vec<u32>,
}

pub fn init(config_dir: PathBuf) {
    let _ = SAVED_PATH.set(config_dir.join(SAVED_FILE));
}

pub fn read() -> BatterySettings {
    let saved = read_saved().map(|s| s.threshold);
    BatterySettings {
        brightness: brightness().ok(),
        refresh: refresh(),
        screen_off: read_dc(&SUB_VIDEO, &VIDEOIDLE).ok(),
        sleep: read_dc(&SUB_SLEEP, &STANDBYIDLE).ok(),
        saver_threshold: saved.or_else(|| read_dc(&SUB_ENERGYSAVER, &ESBATTTHRESHOLD).ok()),
    }
}

// --- enerji tasarrufu ------------------------------------------------------------------

/// Pildeyken tasarrufu aç / kapat. `percent`: şu anki pil yüzdesi.
pub fn set_saver(on: bool, percent: u8) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let current = read_dc(&SUB_ENERGYSAVER, &ESBATTTHRESHOLD)?;
    let saved = read_saved();
    if on {
        if saved.is_none() && current != ALWAYS {
            write_saved(current)?;
        }
        return write_dc(&SUB_ENERGYSAVER, &ESBATTTHRESHOLD, ALWAYS);
    }
    let had_saved = saved.is_some();
    let original = saved
        .map(|s| s.threshold)
        .unwrap_or(current.min(ALWAYS - 1));
    // Pil asıl eşiğin altındaysa eşiği geri koymak tasarrufu açık bırakır:
    // prize takılana dek 0'da tut, asıl değer dosyada kalır (`restore_if_plugged`).
    if u32::from(percent) <= original {
        if !had_saved {
            write_saved(original)?;
        }
        write_dc(&SUB_ENERGYSAVER, &ESBATTTHRESHOLD, 0)
    } else {
        write_dc(&SUB_ENERGYSAVER, &ESBATTTHRESHOLD, original)?;
        remove_saved();
        Ok(())
    }
}

/// Prize takılınca asıl eşiği geri yükle (pil servisi her okumada çağırır).
pub fn restore_if_plugged(plugged: bool) {
    if !plugged || !SAVED_PATH.get().is_some_and(|p| p.exists()) {
        return;
    }
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(saved) = read_saved()
        && write_dc(&SUB_ENERGYSAVER, &ESBATTTHRESHOLD, saved.threshold).is_ok()
    {
        remove_saved();
    }
}

fn read_saved() -> Option<Saved> {
    let text = std::fs::read_to_string(SAVED_PATH.get()?).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_saved(threshold: u32) -> Result<(), String> {
    let path = SAVED_PATH.get().ok_or("ayar klasörü yok")?;
    let text = serde_json::to_string(&Saved { threshold }).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("eşik kaydedilemedi: {e}"))
}

fn remove_saved() {
    if let Some(path) = SAVED_PATH.get() {
        let _ = std::fs::remove_file(path);
    }
}

// --- güç ayarları ----------------------------------------------------------------------

fn active_scheme() -> Result<GUID, String> {
    let mut ptr: *mut GUID = std::ptr::null_mut();
    check(
        unsafe { PowerGetActiveScheme(None, &mut ptr) },
        "güç düzeni okunamadı",
    )?;
    let guid = unsafe { *ptr };
    unsafe { LocalFree(Some(HLOCAL(ptr as *mut _))) };
    Ok(guid)
}

fn read_dc(sub: &GUID, setting: &GUID) -> Result<u32, String> {
    let scheme = active_scheme()?;
    let mut value = 0u32;
    check(
        WIN32_ERROR(unsafe {
            PowerReadDCValueIndex(None, Some(&scheme), Some(sub), Some(setting), &mut value)
        }),
        "güç ayarı okunamadı",
    )?;
    Ok(value)
}

fn write_dc(sub: &GUID, setting: &GUID, value: u32) -> Result<(), String> {
    let scheme = active_scheme()?;
    check(
        WIN32_ERROR(unsafe {
            PowerWriteDCValueIndex(None, &scheme, Some(sub), Some(setting), value)
        }),
        "güç ayarı yazılamadı",
    )?;
    // Yazılan değer etkin düzen yeniden etkinleştirilince uygulanır.
    check(
        unsafe { PowerSetActiveScheme(None, Some(&scheme)) },
        "güç düzeni uygulanamadı",
    )
}

/// Pildeyken ekranı kapatma ("screen") ya da uyku ("sleep") süresi (sn).
pub fn set_timeout(kind: &str, seconds: u32) -> Result<(), String> {
    match kind {
        "screen" => write_dc(&SUB_VIDEO, &VIDEOIDLE, seconds),
        "sleep" => write_dc(&SUB_SLEEP, &STANDBYIDLE, seconds),
        _ => Err(format!("bilinmeyen süre: {kind}")),
    }
}

fn check(err: WIN32_ERROR, what: &str) -> Result<(), String> {
    if err.0 == 0 {
        Ok(())
    } else {
        Err(format!("{what} (kod {})", err.0))
    }
}

// --- yenileme hızı ---------------------------------------------------------------------

fn devmode() -> DEVMODEW {
    DEVMODEW {
        dmSize: size_of::<DEVMODEW>() as u16,
        ..Default::default()
    }
}

fn refresh() -> Option<Refresh> {
    let mut cur = devmode();
    unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut cur) }
        .as_bool()
        .then_some(())?;
    let mut options = Vec::new();
    let mut mode = devmode();
    let mut i = 0;
    while unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_DISPLAY_SETTINGS_MODE(i), &mut mode) }
        .as_bool()
    {
        i += 1;
        let hz = mode.dmDisplayFrequency;
        if mode.dmPelsWidth == cur.dmPelsWidth
            && mode.dmPelsHeight == cur.dmPelsHeight
            && mode.dmBitsPerPel == cur.dmBitsPerPel
            && hz > 1
            && !options.contains(&hz)
        {
            options.push(hz);
        }
    }
    options.sort_unstable();
    Some(Refresh {
        current: cur.dmDisplayFrequency,
        options,
    })
}

pub fn set_refresh(hz: u32) -> Result<(), String> {
    let mut dm = devmode();
    unsafe { EnumDisplaySettingsW(PCWSTR::null(), ENUM_CURRENT_SETTINGS, &mut dm) }
        .as_bool()
        .then_some(())
        .ok_or("ekran ayarı okunamadı")?;
    dm.dmDisplayFrequency = hz;
    dm.dmFields = DM_DISPLAYFREQUENCY;
    let result = unsafe {
        ChangeDisplaySettingsExW(PCWSTR::null(), Some(&dm), None, CDS_UPDATEREGISTRY, None)
    };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(format!("yenileme hızı değiştirilemedi (kod {})", result.0))
    }
}

// --- parlaklık (WMI) -------------------------------------------------------------------

fn wmi() -> Result<IWbemServices, String> {
    unsafe {
        crate::win::com_mta();
        let locator: IWbemLocator =
            CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(err)?;
        let services = locator
            .ConnectServer(
                &BSTR::from("ROOT\\WMI"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )
            .map_err(err)?;
        CoSetProxyBlanket(
            &services,
            10, // RPC_C_AUTHN_WINNT
            0,  // RPC_C_AUTHZ_NONE
            None,
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
        .map_err(err)?;
        Ok(services)
    }
}

fn err(e: windows::core::Error) -> String {
    e.message()
}

/// Sorgunun ilk nesnesi.
fn first(services: &IWbemServices, query: &str) -> Result<IWbemClassObject, String> {
    unsafe {
        let rows = services
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .map_err(err)?;
        let mut row = [None];
        let mut returned = 0;
        rows.Next(WBEM_INFINITE, &mut row, &mut returned)
            .ok()
            .map_err(err)?;
        row[0]
            .take()
            .ok_or_else(|| "parlaklık denetimi yok".to_string())
    }
}

fn get(object: &IWbemClassObject, name: PCWSTR) -> Result<VARIANT, String> {
    let mut value = VARIANT::default();
    unsafe { object.Get(name, 0, &mut value, None, None) }.map_err(err)?;
    Ok(value)
}

pub fn brightness() -> Result<u8, String> {
    let services = wmi()?;
    let row = first(
        &services,
        "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE",
    )?;
    let value = get(&row, w!("CurrentBrightness"))?;
    let percent = i32::try_from(&value).map_err(err)?;
    Ok(percent.clamp(0, 100) as u8)
}

pub fn set_brightness(percent: u8) -> Result<(), String> {
    let services = wmi()?;
    let target = first(
        &services,
        "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE",
    )?;
    let path = BSTR::try_from(&get(&target, w!("__PATH"))?).map_err(err)?;
    unsafe {
        let mut class = None;
        services
            .GetObject(
                &BSTR::from("WmiMonitorBrightnessMethods"),
                Default::default(),
                None,
                Some(&mut class),
                None,
            )
            .map_err(err)?;
        let class = class.ok_or("parlaklık sınıfı yok")?;
        let mut params_def = None;
        class
            .GetMethod(
                w!("WmiSetBrightness"),
                0,
                &mut params_def,
                std::ptr::null_mut(),
            )
            .map_err(err)?;
        let params = params_def
            .ok_or("parlaklık yöntemi yok")?
            .SpawnInstance(0)
            .map_err(err)?;
        params
            .Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0)
            .map_err(err)?;
        params
            .Put(w!("Brightness"), 0, &VARIANT::from(percent.min(100)), 0)
            .map_err(err)?;
        services
            .ExecMethod(
                &path,
                &BSTR::from("WmiSetBrightness"),
                Default::default(),
                None,
                &params,
                None,
                None,
            )
            .map_err(err)?;
    }
    Ok(())
}
