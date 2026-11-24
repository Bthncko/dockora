//! Ses karıştırıcısı: varsayılan çıkış aygıtındaki uygulama oturumları
//! (Core Audio `IAudioSessionManager2`). Yoklamasız: sayfa açıkken istedikçe
//! sorar (`audio_sessions`), değiştirmek için kimlik verir.
//!
//! Oturum kimliği `IAudioSessionControl2::GetSessionInstanceIdentifier`
//! (oturum başına benzersiz; aynı uygulamanın birden çok oturumu ayrışır).
//! Uygulama adı / ikonu exe yolundan bir kez çözülür ve önbelleğe alınır.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{
    AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl, IAudioSessionControl2,
    IAudioSessionManager2, ISimpleAudioVolume, eMultimedia, eRender,
};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::core::Interface;

use super::{audio_devices, icons, windows_list};
use crate::i18n;
use crate::win::take_pwstr;

/// Bu tepe değerinin üstü "şu an ses çıkarıyor" sayılır.
const ACTIVE_PEAK: f32 = 0.01;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioSession {
    pub id: String,
    pub pid: u32,
    pub name: String,
    /// data URL
    pub icon: Option<String>,
    /// 0–100
    pub volume: u8,
    pub muted: bool,
    pub active: bool,
}

/// (ad, ikon) exe yoluna göre.
type AppInfo = (String, Option<String>);
static APPS: Mutex<Option<HashMap<String, AppInfo>>> = Mutex::new(None);

thread_local! {
    static COM: Cell<bool> = const { Cell::new(false) };
}

/// Komutlar havuz iş parçacıklarında çalışır: COM bir kez başlatılır.
pub fn ensure_com() {
    COM.with(|c| {
        if !c.replace(true) {
            // Başka kipte başlatılmışsa hata döner; kullanılabilir kalır.
            crate::win::com_mta();
        }
    });
}

fn manager() -> Result<IAudioSessionManager2, String> {
    ensure_com();
    let enumerator = audio_devices::enumerator().ok_or("ses aygıtları açılamadı")?;
    unsafe {
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .map_err(|e| e.to_string())?;
        device
            .Activate::<IAudioSessionManager2>(CLSCTX_ALL, None)
            .map_err(|e| e.to_string())
    }
}

/// Oturumları dolaşır; `f` oturum denetimlerini alır.
fn each(
    mut f: impl FnMut(&IAudioSessionControl, &IAudioSessionControl2) -> bool,
) -> Result<(), String> {
    let manager = manager()?;
    unsafe {
        let sessions = manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
        let count = sessions.GetCount().map_err(|e| e.to_string())?;
        for i in 0..count {
            let Ok(control) = sessions.GetSession(i) else {
                continue;
            };
            let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if !f(&control, &control2) {
                break;
            }
        }
    }
    Ok(())
}

fn session_id(control2: &IAudioSessionControl2) -> Option<String> {
    // SAFETY: dize CoTaskMemAlloc ile ayrılır.
    unsafe {
        control2
            .GetSessionInstanceIdentifier()
            .ok()
            .map(|p| take_pwstr(p))
    }
}

fn app_info(pid: u32) -> Option<AppInfo> {
    let exe = windows_list::process_image(pid)?;
    let key = exe.to_lowercase();
    if let Some(hit) = APPS
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .get(&key)
    {
        return Some(hit.clone());
    }
    let info = icons::shell_info(None, Some(&exe));
    let name = info.name.unwrap_or_else(|| {
        std::path::Path::new(&exe)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let entry = (name, info.icon);
    APPS.lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, entry.clone());
    Some(entry)
}

pub fn list() -> Result<Vec<AudioSession>, String> {
    let mut out = Vec::new();
    each(|control, control2| {
        unsafe {
            if control
                .GetState()
                .is_ok_and(|s| s == AudioSessionStateExpired)
            {
                return true;
            }
            let Some(id) = session_id(control2) else {
                return true;
            };
            let system = control2.IsSystemSoundsSession() == S_OK;
            let pid = control2.GetProcessId().unwrap_or(0);
            let (name, icon) = if system {
                (i18n::t("Sistem sesleri").to_string(), None)
            } else if pid == 0 {
                return true;
            } else if let Some(info) = app_info(pid) {
                info
            } else {
                return true;
            };
            let Ok(volume) = control.cast::<ISimpleAudioVolume>() else {
                return true;
            };
            let level = volume.GetMasterVolume().unwrap_or(0.0);
            let muted = volume.GetMute().is_ok_and(|m| m.as_bool());
            let active = match control
                .cast::<IAudioMeterInformation>()
                .and_then(|m| m.GetPeakValue())
            {
                Ok(peak) => peak > ACTIVE_PEAK,
                Err(_) => control
                    .GetState()
                    .is_ok_and(|s| s == AudioSessionStateActive),
            };
            out.push(AudioSession {
                id,
                pid,
                name,
                icon,
                volume: (level * 100.0).round().clamp(0.0, 100.0) as u8,
                muted,
                active,
            });
        }
        true
    })?;
    Ok(out)
}

/// Kimliği verilen oturumun `ISimpleAudioVolume`'u üzerinde `f` çalıştırır.
fn with_session(id: &str, f: impl FnOnce(&ISimpleAudioVolume)) -> Result<(), String> {
    let mut found = false;
    let mut f = Some(f);
    each(|control, control2| {
        if session_id(control2).as_deref() == Some(id)
            && let Ok(volume) = control.cast::<ISimpleAudioVolume>()
        {
            if let Some(f) = f.take() {
                f(&volume);
            }
            found = true;
            return false;
        }
        true
    })?;
    if found {
        Ok(())
    } else {
        Err("ses oturumu bulunamadı".into())
    }
}

pub fn set_volume(id: &str, volume: u8) -> Result<(), String> {
    with_session(id, |v| unsafe {
        let _ = v.SetMasterVolume(f32::from(volume.min(100)) / 100.0, std::ptr::null());
        // Ses açılınca sessizden de çık.
        if volume > 0 {
            let _ = v.SetMute(false, std::ptr::null());
        }
    })
}

pub fn set_mute(id: &str, muted: bool) -> Result<(), String> {
    with_session(id, |v| unsafe {
        let _ = v.SetMute(muted, std::ptr::null());
    })
}

#[cfg(test)]
mod tests {
    /// Bu makinede (yerel deneme): oturumlar okunuyor (ses değiştirilmez).
    #[test]
    #[ignore]
    fn lists_sessions() {
        let sessions = super::list().unwrap();
        println!("{sessions:#?}");
    }
}
