//! Wi-Fi / Bluetooth: WinRT `Windows.Devices.Radios` + ağ adı (bölüm 6.7).
//!
//! Doğrulandı (bu makinede, paketsiz süreçten): radyo listesi okunuyor,
//! `RequestAccessAsync` → Allowed. Belgelere göre "radios" yeteneği gerekiyor;
//! paketsiz uygulamada buna rağmen çalıştı. Değiştirme reddedilirse frontend
//! `ms-settings:` sayfasını açar.
//!
//! Kablolu bağlantı: internet profilinin bağdaştırıcısı Ethernet (IANA 6) ise
//! `ethernet` (Wi-Fi karosu kablo simgesini gösterir).
//!
//! Ağ adı: Microsoft'un konum izni gerektirmeyen yol olarak önerdiği
//! `WlanConnectionProfileDetails.GetConnectedSsid` (WlanQueryInterface değil).
//!
//! 7.8: yoklama yerine olay. `Radio.StateChanged` (aç/kapat, uçak modu, donanım
//! anahtarı) ve `NetworkInformation.NetworkStatusChanged` (bağlantı değişimi)
//! servisi uyandırır. Sinyal gücünün olayı yok; seyrek yoklama onu ve kaçan
//! olayları yakalar.

use std::sync::mpsc::Sender;
use std::time::Duration;

use serde::Serialize;
use tauri::AppHandle;
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Foundation::TypedEventHandler;
use windows::Networking::Connectivity::{
    ConnectionProfile, NetworkInformation, NetworkStatusChangedEventHandler,
};
use windows::core::IInspectable;

use super::runner::{self, Waker};

pub const RADIOS_CHANGED: &str = "radios://changed";
/// Yedek yoklama: sinyal çubukları ve kaçan olaylar için.
const POLL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RadioInfo {
    pub on: bool,
    /// Donanım anahtarı / ürün yazılımı kapatmış: programla açılamaz.
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RadiosState {
    pub wifi: Option<RadioInfo>,
    pub bluetooth: Option<RadioInfo>,
    /// Bağlı Wi-Fi ağının adı.
    pub ssid: Option<String>,
    /// 0–5 sinyal çubuğu.
    pub signal: Option<u8>,
    /// İnternet kablodan (Ethernet) geliyor.
    pub ethernet: bool,
}

pub struct SetRadio {
    pub wifi: bool,
    pub on: bool,
    pub reply: Sender<Result<(), String>>,
}

pub type RadiosService = runner::ServiceHandle<RadiosState, SetRadio>;

/// Olay abonelikleri; bırakılınca (servis durunca, 8.1) silinir.
#[derive(Default)]
struct Subscriptions {
    radios: Vec<(Radio, i64)>,
    network: Option<i64>,
}

impl Drop for Subscriptions {
    fn drop(&mut self) {
        for (radio, token) in &self.radios {
            let _ = radio.RemoveStateChanged(*token);
        }
        if let Some(t) = self.network {
            let _ = NetworkInformation::RemoveNetworkStatusChanged(t);
        }
    }
}

pub fn spawn(app: &AppHandle) {
    runner::spawn(
        app,
        "radios",
        RADIOS_CHANGED,
        POLL,
        RadiosState::default(),
        |waker: Waker<SetRadio>| {
            let mut access_requested = false;
            let mut subs = Subscriptions::default();
            let w = waker.clone();
            match NetworkInformation::NetworkStatusChanged(&NetworkStatusChangedEventHandler::new(
                move |_| {
                    w.wake();
                    Ok(())
                },
            )) {
                Ok(t) => subs.network = Some(t),
                Err(e) => eprintln!("[radios] ağ bildirimi kaydedilemedi: {e}"),
            }
            move |cmd: Option<SetRadio>| {
                if !access_requested {
                    // Yalnızca değiştirmek için gerekir; okuma her zaman serbest.
                    let _ = Radio::RequestAccessAsync().and_then(|op| op.join());
                    access_requested = true;
                }
                if subs.radios.is_empty() {
                    subs.radios = subscribe(&waker);
                }
                let radios = list_radios();
                if let Some(cmd) = cmd {
                    let kind = if cmd.wifi {
                        RadioKind::WiFi
                    } else {
                        RadioKind::Bluetooth
                    };
                    let result = set_state(&radios, kind, cmd.on);
                    let _ = cmd.reply.send(result);
                    return read_state(&list_radios());
                }
                read_state(&radios)
            }
        },
    );
}

/// Her radyonun durum değişimine abone olur; radyo ve jetonu döndürür.
fn subscribe(waker: &Waker<SetRadio>) -> Vec<(Radio, i64)> {
    list_radios()
        .into_iter()
        .filter_map(|radio| {
            let w = waker.clone();
            let token = radio
                .StateChanged(&TypedEventHandler::<Radio, IInspectable>::new(
                    move |_, _| {
                        w.wake();
                        Ok(())
                    },
                ))
                .ok()?;
            Some((radio, token))
        })
        .collect()
}

fn list_radios() -> Vec<Radio> {
    let Ok(view) = Radio::GetRadiosAsync().and_then(|op| op.join()) else {
        return Vec::new();
    };
    let count = view.Size().unwrap_or(0);
    (0..count).filter_map(|i| view.GetAt(i).ok()).collect()
}

fn info(radios: &[Radio], kind: RadioKind) -> Option<RadioInfo> {
    let radio = radios.iter().find(|r| r.Kind().ok() == Some(kind))?;
    let state = radio.State().ok()?;
    Some(RadioInfo {
        on: state == RadioState::On,
        disabled: state == RadioState::Disabled,
    })
}

fn read_state(radios: &[Radio]) -> RadiosState {
    let profile = NetworkInformation::GetInternetConnectionProfile().ok();
    let (ssid, signal) = profile.as_ref().map_or((None, None), wifi_connection);
    RadiosState {
        wifi: info(radios, RadioKind::WiFi),
        bluetooth: info(radios, RadioKind::Bluetooth),
        ssid,
        signal,
        ethernet: profile.as_ref().is_some_and(is_ethernet),
    }
}

/// IANA arayüz türü 6: Ethernet (ifType, RFC 1213).
const IANA_ETHERNET: u32 = 6;

/// İnternet profilinin bağdaştırıcısı gerçek, takılı kablolu bir Ethernet mi?
/// WinRT yalnızca ifType'a bakar; Hyper-V/WSL vEthernet, TAP/VPN, VirtualBox gibi
/// sanal bağdaştırıcılar da 6 döner. Bu yüzden `MIB_IF_ROW2` ile fiziksel arayüz,
/// ortam bağlı ve çalışır durumda olması da aranır.
fn is_ethernet(profile: &ConnectionProfile) -> bool {
    if profile.IsWlanConnectionProfile().unwrap_or(false)
        || profile.IsWwanConnectionProfile().unwrap_or(false)
    {
        return false;
    }
    let Ok(adapter) = profile.NetworkAdapter() else {
        return false;
    };
    if adapter.IanaInterfaceType().ok() != Some(IANA_ETHERNET) {
        return false;
    }
    adapter
        .NetworkAdapterId()
        .is_ok_and(|guid| is_physical_wired_up(&guid))
}

fn is_physical_wired_up(guid: &windows::core::GUID) -> bool {
    use windows::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceGuidToLuid, GetIfEntry2, IF_TYPE_ETHERNET_CSMACD, MIB_IF_ROW2,
    };
    use windows::Win32::NetworkManagement::Ndis::{
        IfOperStatusUp, MediaConnectStateConnected, NET_LUID_LH,
    };
    let mut luid = NET_LUID_LH::default();
    if unsafe { ConvertInterfaceGuidToLuid(guid, &mut luid) }.is_err() {
        return false;
    }
    let mut row = MIB_IF_ROW2 {
        InterfaceLuid: luid,
        ..Default::default()
    };
    if unsafe { GetIfEntry2(&mut row) }.is_err() {
        return false;
    }
    // Bit alanı: bit 0 HardwareInterface, bit 1 FilterInterface.
    let flags = row.InterfaceAndOperStatusFlags._bitfield;
    let hardware = flags & 0x01 != 0;
    let filter = flags & 0x02 != 0;
    row.Type == IF_TYPE_ETHERNET_CSMACD
        && hardware
        && !filter
        && row.OperStatus == IfOperStatusUp
        && row.MediaConnectState == MediaConnectStateConnected
}

fn wifi_connection(profile: &ConnectionProfile) -> (Option<String>, Option<u8>) {
    if !profile.IsWlanConnectionProfile().unwrap_or(false) {
        return (None, None);
    }
    let ssid = profile
        .WlanConnectionProfileDetails()
        .and_then(|d| d.GetConnectedSsid())
        .map(|s| s.to_string())
        .ok()
        .filter(|s| !s.is_empty());
    let signal = profile.GetSignalBars().and_then(|bars| bars.Value()).ok();
    (ssid, signal)
}

fn set_state(radios: &[Radio], kind: RadioKind, on: bool) -> Result<(), String> {
    let radio = radios
        .iter()
        .find(|r| r.Kind().ok() == Some(kind))
        .ok_or("radyo bulunamadı")?;
    let target = if on { RadioState::On } else { RadioState::Off };
    let status = radio
        .SetStateAsync(target)
        .and_then(|op| op.join())
        .map_err(|e| e.message())?;
    if status == RadioAccessStatus::Allowed {
        Ok(())
    } else {
        Err(format!("erişim reddedildi ({})", status.0))
    }
}
