//! Ses aygıtları (8.5): çıkış / giriş listesi ve varsayılanı değiştirme.
//!
//! Liste belgeli: `IMMDeviceEnumerator::EnumAudioEndpoints` + aygıt adı.
//!
//! Varsayılanı değiştirmenin açık bir API'si yok. Windows'un kendi ses
//! ayarlarının ve yaygın araçların kullandığı belgelenmemiş `IPolicyConfig`
//! arayüzü elle tanımlanır. Kimlikler ve sanal tablo sırası açık kaynak
//! EarTrumpet (`Interop/MMDeviceAPI/IPolicyConfig.cs`, `PolicyConfigClient.cs`)
//! ve SoundSwitch (`IPolicyConfig.cs`) ile karşılaştırıldı; ikisi aynı:
//! IID F8679F50-850A-41CF-9C72-430F290290C8 (Win7–8 ve Win10 RS1 sonrası),
//! CLSID 870AF99C-171D-4F9E-AF0D-E63DF40C2BC9, `SetDefaultEndpoint` 11. yöntem.
//! Yanlış sıra çökme demek; bu yüzden yalnızca bu yöntem çağrılır.
//! Başarısız olursa çağıran Windows'un ses ayarlarını açar.

use serde::Serialize;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    eCommunications, eConsole, eMultimedia,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
use windows::core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

use crate::error::{AppError, AppResult};
use crate::win::{take_pwstr, wide};

const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[allow(non_snake_case)]
mod policy {
    use super::*;

    /// Belgelenmemiş arayüz. İlk sekiz yöntem (biçim, süre, paylaşım modu)
    /// kullanılmaz; yalnızca sanal tablodaki yerleri için tanımlı.
    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self) -> HRESULT;
        fn GetDeviceFormat(&self) -> HRESULT;
        fn ResetDeviceFormat(&self) -> HRESULT;
        fn SetDeviceFormat(&self) -> HRESULT;
        fn GetProcessingPeriod(&self) -> HRESULT;
        fn SetProcessingPeriod(&self) -> HRESULT;
        fn GetShareMode(&self) -> HRESULT;
        fn SetShareMode(&self) -> HRESULT;
        fn GetPropertyValue(&self) -> HRESULT;
        fn SetPropertyValue(&self) -> HRESULT;
        fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: ERole) -> HRESULT;
        fn SetEndpointVisibility(&self) -> HRESULT;
    }

    /// Aygıtı üç rol için de (oyun/müzik, iletişim) varsayılan yapar.
    pub fn set_default(id: &str) -> AppResult<()> {
        let w = wide(id);
        unsafe {
            let policy: IPolicyConfig =
                CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)
                    .map_err(|e| AppError::Win32(format!("IPolicyConfig: {e}")))?;
            for role in [eConsole, eMultimedia, eCommunications] {
                policy
                    .SetDefaultEndpoint(PCWSTR(w.as_ptr()), role)
                    .ok()
                    .map_err(|e| {
                        AppError::Win32(format!("varsayılan aygıt değiştirilemedi: {e}"))
                    })?;
            }
        }
        Ok(())
    }
}
pub use policy::set_default;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Etkin aygıtlar; varsayılan işaretli. Çağıran iş parçacığında COM açık olmalı.
pub fn list(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> Vec<Device> {
    unsafe {
        let default = enumerator
            .GetDefaultAudioEndpoint(flow, eMultimedia)
            .ok()
            .and_then(|d| device_id(&d));
        let Ok(collection) = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else {
            return Vec::new();
        };
        let count = collection.GetCount().unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                let device = collection.Item(i).ok()?;
                let id = device_id(&device)?;
                Some(Device {
                    name: friendly_name(&device)
                        .unwrap_or_else(|| crate::i18n::t("Ses aygıtı").into()),
                    default: default.as_deref() == Some(id.as_str()),
                    id,
                })
            })
            .collect()
    }
}

pub fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() }
}

pub fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: GetId dizesi CoTaskMemAlloc ile ayrılır.
    unsafe { device.GetId().ok().map(|p| take_pwstr(p)) }
}

pub fn friendly_name(device: &IMMDevice) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        Some(take_pwstr(PropVariantToStringAlloc(&value).ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::Audio::{eCapture, eRender};

    /// Deneme (elle): `cargo test policy_config_noop -- --ignored --nocapture`.
    /// Varsayılan çıkışı yine kendisine atar: ses başka yere geçmez.
    #[test]
    #[ignore]
    fn policy_config_noop() {
        crate::win::com_mta();
        let e = enumerator().unwrap();
        let outputs = list(&e, eRender);
        let inputs = list(&e, eCapture);
        println!(
            "çıkış: {} aygıt, varsayılan var: {}; giriş: {} aygıt",
            outputs.len(),
            outputs.iter().any(|d| d.default),
            inputs.len()
        );
        let current = outputs.iter().find(|d| d.default).unwrap();
        set_default(&current.id).unwrap();
        let after = list(&e, eRender);
        assert_eq!(
            after.iter().find(|d| d.default).map(|d| &d.id),
            Some(&current.id)
        );
        println!("SetDefaultEndpoint (aynı aygıt) başarılı, varsayılan değişmedi");
    }
}
