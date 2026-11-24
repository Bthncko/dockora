pub mod accent;
pub mod app_events;
pub mod apps;
pub mod audio;
pub mod audio_devices;
pub mod audio_sessions;
pub mod backdrop;
pub mod battery_settings;
pub mod file_drop;
pub mod flylog;
pub mod flyouts;
pub mod http;
pub mod icons;
pub mod ics;
pub mod jumplist;
pub mod ai_usage;
pub mod keyboard;
pub mod launcher;
pub mod media;
pub mod notifications;
pub mod notify_sound;
pub mod power;
pub mod power_mode;
pub mod radios;
pub mod recycle;
pub mod runner;
pub mod spotify;
pub mod sysmon;
pub mod timer;
pub mod updater;
pub mod windows_list;
pub mod wireless;

use tauri::{AppHandle, Manager};

use crate::config::Config;

/// Hangi servisin çalışacağına yerleşime ve temaya göre karar verir (8.1).
/// Yerleşimde olmayan widget'ın servisi durur, eklenince yeniden başlar.
/// Açılışta ve config her değiştiğinde (`ConfigStore::update`) çağrılır.
pub fn sync_active(app: &AppHandle, config: &Config) {
    let on = |id: &str| config.layout.contains(id);
    // Ekran üstü kartlar da bu servislerin durumunu okur (medya, ses kartı).
    let fly = &config.flyouts;
    let fly_media =
        fly.enabled && (fly.media.enabled || fly.next_up.enabled || fly.media.pause_others);
    let fly_audio = fly.enabled && fly.volume.enabled;
    if let Some(s) = app.try_state::<media::MediaService>() {
        s.set_active(on("media") || fly_media);
    }
    if let Some(s) = app.try_state::<audio::AudioService>() {
        s.set_active(on("volume") || on("mic") || fly_audio);
    }
    flyouts::sync(config);
    if let Some(s) = app.try_state::<power::PowerService>() {
        s.set_active(on("battery"));
    }
    if let Some(s) = app.try_state::<radios::RadiosService>() {
        s.set_active(on("radios"));
    }
    if let Some(s) = app.try_state::<sysmon::SysmonService>() {
        s.set_active(on("sysmon"));
    }
    if let Some(s) = app.try_state::<recycle::RecycleService>() {
        s.set_active(on("recycleBin"));
    }
    if let Some(s) = app.try_state::<keyboard::KeyboardService>() {
        s.set_active(on("keyboard"));
    }
    if let Some(s) = app.try_state::<ai_usage::AiUsageService>() {
        s.set_active(on("aiUsage"));
    }
    // Düz zeminde duvar kâğıdı şeridi kullanılmaz.
    if let Some(s) = app.try_state::<backdrop::BackdropService>() {
        s.set_active(config.theme.glass != "flat");
    }
    // Vurgu rengi ayarlar penceresinde de gösterilir; olay güdümlü ve ucuz.
    if let Some(s) = app.try_state::<accent::AccentService>() {
        s.set_active(true);
    }
}
