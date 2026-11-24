//! Pil: `GetSystemPowerStatus` (bölüm 6.6).
//! Pil yoksa (masaüstü PC) durum `None` olur ve widget gizlenir.
//!
//! Prize takma/çıkarma, yüzde ve tasarruf değişimi `PowerSettingRegisterNotification`
//! bildirimleriyle hemen okunur; 30 sn yoklama yalnızca yedek.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, GetSystemPowerStatus, PowerSettingRegisterNotification,
    SYSTEM_POWER_STATUS,
};
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK;
use windows::core::GUID;

use super::runner;

pub const POWER_CHANGED: &str = "power://changed";
const POLL: Duration = Duration::from_secs(30);
/// `BatteryFlag`: sistemde pil yok.
const NO_BATTERY: u8 = 128;
/// `BatteryLifePercent` / `ACLineStatus`: bilinmiyor.
const UNKNOWN: u8 = 255;

// learn.microsoft.com/windows/win32/power/power-setting-guids (ilk üçü windows
// crate'in SystemServices modülüyle aynı; o özelliği yalnızca bunun için açmıyoruz).
const GUID_ACDC_POWER_SOURCE: GUID = GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);
const GUID_BATTERY_PERCENTAGE_REMAINING: GUID =
    GUID::from_u128(0xa7ad8041_b45a_4cae_87a3_eecbb468a9e1);
/// Eski pil tasarrufu (Windows 10): Data 0 kapalı, 1 açık.
const GUID_POWER_SAVING_STATUS: GUID = GUID::from_u128(0xe00958c0_c213_4ace_ac77_fecced2eeea5);
/// Windows 11 Enerji tasarrufu (belgede "ön sürüm"): Data `ENERGY_SAVER_STATUS`
/// (0 kapalı, 1 standart, 2 yüksek tasarruf).
const GUID_ENERGY_SAVER_STATUS: GUID = GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000);

static APP: OnceLock<AppHandle> = OnceLock::new();
/// Son bildirilen enerji tasarrufu durumu (`GetSystemPowerStatus`'ta yok).
static ENERGY_SAVER: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PowerState {
    pub percent: u8,
    /// Prize takılı.
    pub plugged: bool,
    /// Şarj oluyor (takılı ve dolmamış).
    pub charging: bool,
    /// Enerji / pil tasarrufu açık.
    pub saver: bool,
    /// Pilde kalan süre (sn); prizde ya da Windows bilmiyorsa `None`.
    pub seconds_left: Option<u32>,
    /// Güç modu: "efficiency" / "balanced" / "performance" (`power_mode`).
    pub mode: Option<&'static str>,
}

pub type PowerService = runner::ServiceHandle<Option<PowerState>, ()>;

pub fn spawn(app: &AppHandle) {
    runner::spawn(app, "power", POWER_CHANGED, POLL, None, |_| {
        |_: Option<()>| {
            let mut status = SYSTEM_POWER_STATUS::default();
            unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
            if status.BatteryFlag & NO_BATTERY != 0 || status.BatteryLifePercent == UNKNOWN {
                return None;
            }
            let plugged = status.ACLineStatus == 1;
            // Bizim açtığımız enerji tasarrufu: prize takılınca asıl eşik geri gelir.
            super::battery_settings::restore_if_plugged(plugged);
            Some(PowerState {
                percent: status.BatteryLifePercent.min(100),
                plugged,
                // BatteryFlag 8 = şarj oluyor.
                charging: status.BatteryFlag & 8 != 0,
                // Hangisinin Windows 11 Enerji tasarrufunu yansıttığı belgede yok: ikisi de.
                saver: status.SystemStatusFlag == 1 || ENERGY_SAVER.load(Ordering::Relaxed) > 0,
                // u32::MAX: bilinmiyor (prizde ya da yeni çıkarıldı).
                seconds_left: (!plugged && status.BatteryLifeTime != u32::MAX)
                    .then_some(status.BatteryLifeTime),
                mode: super::power_mode::get(),
            })
        }
    });
    let _ = APP.set(app.clone());
    subscribe();
}

/// Bildirim gelince servise "şimdi oku" der. Sistem iş parçacığında çağrılır:
/// yalnızca kanala yazar, hemen döner.
unsafe extern "system" fn on_power_setting(
    _context: *const core::ffi::c_void,
    _kind: u32,
    setting: *const core::ffi::c_void,
) -> u32 {
    // `setting`: POWERBROADCAST_SETTING { GUID (16 bayt), DataLength u32, Data }.
    if !setting.is_null() {
        let base = setting as *const u8;
        let guid = unsafe { std::ptr::read_unaligned(base as *const GUID) };
        let len = unsafe { std::ptr::read_unaligned(base.add(16) as *const u32) };
        if len >= 4 {
            let data = unsafe { std::ptr::read_unaligned(base.add(20) as *const u32) };
            if guid == GUID_ENERGY_SAVER_STATUS {
                ENERGY_SAVER.store(data, Ordering::Relaxed);
            }
        }
    }
    if let Some(app) = APP.get() {
        app.state::<PowerService>().wake();
    }
    0
}

fn subscribe() {
    // Belge yapının ne kadar yaşaması gerektiğini söylemiyor: süreç boyunca tut.
    let params: &'static DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
        Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power_setting),
            Context: std::ptr::null_mut(),
        }));
    for (name, guid) in [
        ("güç kaynağı", GUID_ACDC_POWER_SOURCE),
        ("pil yüzdesi", GUID_BATTERY_PERCENTAGE_REMAINING),
        ("pil tasarrufu", GUID_POWER_SAVING_STATUS),
        ("enerji tasarrufu", GUID_ENERGY_SAVER_STATUS),
    ] {
        let mut handle = std::ptr::null_mut();
        let err = unsafe {
            PowerSettingRegisterNotification(
                &guid,
                DEVICE_NOTIFY_CALLBACK,
                HANDLE(params as *const _ as *mut _),
                &mut handle,
            )
        };
        // Kayıt başarısızsa yoklama yine çalışır (yalnızca gecikmeli).
        if err.0 != 0 {
            eprintln!("[power] {name} bildirimi kaydedilemedi: {}", err.0);
        }
    }
}
