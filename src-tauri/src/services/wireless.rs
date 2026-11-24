//! Wi-Fi ağları ve Bluetooth aygıtları (dock tarzı paneller için).
//!
//! Wi-Fi: WinRT `Windows.Devices.WiFi` (tarama, bağlanma, kesme). Belgeye göre
//! paketli uygulamada `wiFiControl` yeteneği ve kullanıcının konum izni
//! gerekiyor; izin yoksa "erişim reddedildi" döner. Denendi (bu makinede,
//! paketsiz): erişim Allowed, tarama 31 ağ döndürdü. Hata olursa panel
//! Windows'un Wi-Fi sayfasını açar.
//!
//! Bluetooth: eşleşmiş aygıtlar ve bağlantı durumu (`BluetoothDevice`,
//! `BluetoothLEDevice`). Klasik aygıtlar (kulaklık, hoparlör, klavye) için
//! `BluetoothSetServiceState` (Win32, bthprops) ile ses/HID hizmetleri
//! açılıp kapatılarak bağlanır / ayrılır. (`AudioPlaybackConnection` tersine,
//! PC'yi hoparlör yapar; kulaklığa bağlanmak için uygun değildir.) Yalnızca-LE
//! aygıt GATT hizmetleri sorgulanarak bağlanır; ayrılması mümkün değilse
//! "use_settings" hatası döner ve panel Windows ayarlarını açar.
//!
//! İşlevler engelleyicidir (tarama birkaç saniye sürer): komutlar ayrı iş
//! parçacığında çağırır. Ağ adları ve aygıt adları günlüğe yazılmaz.

use std::collections::HashMap;

mod wlan_native;

use serde::Serialize;
use windows::Devices::Bluetooth::BluetoothCacheMode;
use windows::Devices::Bluetooth::GenericAttributeProfile::GattCommunicationStatus;
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::WiFi::{WiFiAdapter, WiFiConnectionStatus, WiFiReconnectionKind};
use windows::Networking::Connectivity::{NetworkAuthenticationType, NetworkInformation};
use windows::Security::Credentials::PasswordCredential;
use windows::Win32::Devices::Bluetooth::{
    BLUETOOTH_DEVICE_INFO, BluetoothGetDeviceInfo, BluetoothSetServiceState,
};
use windows::core::{GUID, HSTRING};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiNetwork {
    pub ssid: String,
    /// 0–4 çubuk.
    pub bars: u8,
    /// Şifre ister (kurumsal ağlar hariç).
    pub secured: bool,
    /// Kullanıcı adı + parola isteyen kurumsal (802.1X) ağ.
    pub enterprise: bool,
    /// Daha önce bağlanılmış (kayıtlı profil var).
    pub saved: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BtDevice {
    pub name: String,
    pub connected: bool,
    /// "audio", "input", "phone" ya da "other" (ikon için).
    pub kind: &'static str,
    /// `DeviceInformation` kimliği (bağlanmak için).
    pub id: String,
    /// Yalnızca düşük enerjili (LE) aygıt.
    pub le: bool,
}

fn com() {
    crate::win::com_mta();
}

fn adapter() -> Result<WiFiAdapter, String> {
    let list = WiFiAdapter::FindAllAdaptersAsync()
        .and_then(|o| o.join())
        .map_err(|e| e.message())?;
    list.GetAt(0)
        .map_err(|_| "Wi-Fi bağdaştırıcısı yok".to_string())
}

/// Tarar ve ağları (aynı ad bir kez, en güçlü sinyalle) döndürür.
pub fn scan() -> Result<Vec<WifiNetwork>, String> {
    com();
    let adapter = adapter()?;
    adapter
        .ScanAsync()
        .and_then(|o| o.join())
        .map_err(|e| e.message())?;
    let report = adapter
        .NetworkReport()
        .and_then(|r| r.AvailableNetworks())
        .map_err(|e| e.message())?;

    let connected = NetworkInformation::GetInternetConnectionProfile()
        .ok()
        .filter(|p| p.IsWlanConnectionProfile().unwrap_or(false))
        .and_then(|p| p.WlanConnectionProfileDetails().ok())
        .and_then(|d| d.GetConnectedSsid().ok())
        .map(|s| s.to_string());
    let saved: Vec<String> = NetworkInformation::GetConnectionProfiles()
        .map(|list| {
            list.into_iter()
                .filter(|p| p.IsWlanConnectionProfile().unwrap_or(false))
                .filter_map(|p| p.ProfileName().ok().map(|n| n.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let mut by_ssid: HashMap<String, WifiNetwork> = HashMap::new();
    for i in 0..report.Size().unwrap_or(0) {
        let Ok(net) = report.GetAt(i) else { continue };
        let ssid = net.Ssid().map(|s| s.to_string()).unwrap_or_default();
        if ssid.is_empty() || net.IsWiFiDirect().unwrap_or(false) {
            continue;
        }
        let auth = net
            .SecuritySettings()
            .and_then(|s| s.NetworkAuthenticationType())
            .unwrap_or(NetworkAuthenticationType::Unknown);
        let enterprise = matches!(
            auth,
            NetworkAuthenticationType::Wpa
                | NetworkAuthenticationType::Rsna
                | NetworkAuthenticationType::Wpa3Enterprise
                | NetworkAuthenticationType::Wpa3Enterprise192Bits
        );
        let open = matches!(
            auth,
            NetworkAuthenticationType::None
                | NetworkAuthenticationType::Open80211
                | NetworkAuthenticationType::Owe
        );
        let entry = WifiNetwork {
            bars: net.SignalBars().unwrap_or(0).min(4),
            secured: !open && !enterprise,
            enterprise,
            saved: saved.contains(&ssid),
            connected: connected.as_deref() == Some(ssid.as_str()),
            ssid: ssid.clone(),
        };
        match by_ssid.get(&ssid) {
            Some(prev) if prev.bars >= entry.bars => {}
            _ => {
                by_ssid.insert(ssid, entry);
            }
        }
    }
    let mut list: Vec<WifiNetwork> = by_ssid.into_values().collect();
    // Bağlı en üstte, sonra kayıtlılar, sonra sinyale göre.
    list.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.saved.cmp(&a.saved))
            .then(b.bars.cmp(&a.bars))
            .then(a.ssid.to_lowercase().cmp(&b.ssid.to_lowercase()))
    });
    Ok(list)
}

/// Son taramadaki ağa bağlanır. Şifreli ve kayıtsız ağda `password` gerekir;
/// yoksa "password_required" hatası döner (panel şifre sorar).
pub fn connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
    connect_with(ssid, None, password, true)
}

/// Kurumsal (802.1X, ör. PEAP-MSCHAPv2) ağa kullanıcı adı + parola ile bağlanır.
/// Parola yalnızca Windows'a verilir (profili Windows saklar); günlüğe yazılmaz.
/// Sunucu sertifikası doğrulaması Windows'un varsayılanıyla kalır.
pub fn connect_enterprise(
    ssid: &str,
    username: &str,
    password: &str,
    auto: bool,
) -> Result<(), String> {
    if username.trim().is_empty() || password.is_empty() {
        return Err("Kullanıcı adı ve parola gerekli".into());
    }
    let username = username.trim();
    // Önce WinRT; bu yol sonuç vermezse (belirsiz hata, zaman aşımı, desteklenmeyen
    // protokol ya da WinRT hatası) Native WiFi yedek yoluna düşülür. Kimlik
    // bilgisi reddi ve erişim engeli kesin sonuçtur, yedek denenmez.
    match winrt_connect(ssid, Some(username), Some(password), auto) {
        Ok(
            status @ (WiFiConnectionStatus::Success
            | WiFiConnectionStatus::InvalidCredential
            | WiFiConnectionStatus::AccessRevoked
            | WiFiConnectionStatus::NetworkNotAvailable),
        ) => status_result(status, false, true),
        Ok(_) | Err(_) => match wlan_native::connect(ssid, username, password, auto) {
            Ok(()) => Ok(()),
            Err(wlan_native::NativeError::Credentials) => {
                Err("Kullanıcı adı ya da parola yanlış".into())
            }
            Err(wlan_native::NativeError::CertTrust) => Err("cert_trust".into()),
            Err(wlan_native::NativeError::NotVisible) => Err("Ağ artık görünmüyor".into()),
            Err(wlan_native::NativeError::Other(m)) => Err(m),
        },
    }
}

fn connect_with(
    ssid: &str,
    username: Option<&str>,
    password: Option<&str>,
    auto: bool,
) -> Result<(), String> {
    let status = winrt_connect(ssid, username, password, auto)?;
    status_result(status, password.is_none(), username.is_some())
}

fn status_result(
    status: WiFiConnectionStatus,
    no_password: bool,
    enterprise: bool,
) -> Result<(), String> {
    match status {
        WiFiConnectionStatus::Success => Ok(()),
        WiFiConnectionStatus::InvalidCredential if no_password => {
            Err("password_required".into())
        }
        WiFiConnectionStatus::InvalidCredential if enterprise => {
            Err("Kullanıcı adı ya da parola yanlış".into())
        }
        WiFiConnectionStatus::InvalidCredential => Err("Şifre yanlış".into()),
        WiFiConnectionStatus::NetworkNotAvailable => Err("Ağ artık görünmüyor".into()),
        // Kurumsal ağda zaman aşımı / belirsiz hata çoğunlukla sunucu sertifikasının
        // Windows'ta henüz onaylanmamasıdır; ayrı hata kodu panelde yönlendirir.
        WiFiConnectionStatus::Timeout | WiFiConnectionStatus::UnspecifiedFailure if enterprise => {
            Err("cert_trust".into())
        }
        WiFiConnectionStatus::Timeout => Err("Bağlantı zaman aşımına uğradı".into()),
        WiFiConnectionStatus::AccessRevoked => Err("Erişim reddedildi".into()),
        WiFiConnectionStatus::UnsupportedAuthenticationProtocol => {
            Err("Bu ağın güvenlik türü desteklenmiyor".into())
        }
        _ => Err("Bağlanılamadı".into()),
    }
}

/// WinRT ile bağlanır ve ham durumu döndürür.
fn winrt_connect(
    ssid: &str,
    username: Option<&str>,
    password: Option<&str>,
    auto: bool,
) -> Result<WiFiConnectionStatus, String> {
    com();
    let adapter = adapter()?;
    let report = adapter
        .NetworkReport()
        .and_then(|r| r.AvailableNetworks())
        .map_err(|e| e.message())?;
    let network = (0..report.Size().unwrap_or(0))
        .filter_map(|i| report.GetAt(i).ok())
        .filter(|n| n.Ssid().map(|s| s.to_string()).as_deref() == Ok(ssid))
        .max_by_key(|n| n.SignalBars().unwrap_or(0))
        .ok_or("ağ artık görünmüyor")?;
    let kind = if auto {
        WiFiReconnectionKind::Automatic
    } else {
        WiFiReconnectionKind::Manual
    };
    let op = match password {
        Some(p) => {
            let cred = PasswordCredential::new().map_err(|e| e.message())?;
            if let Some(u) = username {
                cred.SetUserName(&HSTRING::from(u))
                    .map_err(|e| e.message())?;
            }
            cred.SetPassword(&HSTRING::from(p))
                .map_err(|e| e.message())?;
            adapter.ConnectWithPasswordCredentialAsync(&network, kind, &cred)
        }
        None => adapter.ConnectAsync(&network, kind),
    };
    let status = op
        .and_then(|o| o.join())
        .and_then(|r| r.ConnectionStatus())
        .map_err(|e| e.message())?;
    Ok(status)
}

pub fn disconnect() -> Result<(), String> {
    com();
    adapter()?.Disconnect().map_err(|e| e.message())
}

/// Eşleşmiş Bluetooth aygıtları (klasik ve düşük enerji), bağlılar önce.
pub fn bt_devices() -> Result<Vec<BtDevice>, String> {
    com();
    let mut out: Vec<BtDevice> = Vec::new();
    let classic = BluetoothDevice::GetDeviceSelectorFromPairingState(true)
        .and_then(|s| DeviceInformation::FindAllAsyncAqsFilter(&s))
        .and_then(|o| o.join())
        .map_err(|e| e.message())?;
    for info in classic {
        let Ok(id) = info.Id() else { continue };
        let Ok(device) = BluetoothDevice::FromIdAsync(&id).and_then(|o| o.join()) else {
            continue;
        };
        let kind = match device
            .ClassOfDevice()
            .and_then(|c| c.MajorClass())
            .map(|m| m.0)
        {
            Ok(4) => "audio", // AudioVideo
            Ok(5) => "input", // Peripheral (klavye, fare)
            Ok(2) => "phone", // Phone
            _ => "other",
        };
        out.push(BtDevice {
            name: device.Name().map(|n| n.to_string()).unwrap_or_default(),
            connected: device.ConnectionStatus() == Ok(BluetoothConnectionStatus::Connected),
            kind,
            id: id.to_string(),
            le: false,
        });
    }
    if let Ok(le) = BluetoothLEDevice::GetDeviceSelectorFromPairingState(true)
        .and_then(|s| DeviceInformation::FindAllAsyncAqsFilter(&s))
        .and_then(|o| o.join())
    {
        for info in le {
            let Ok(id) = info.Id() else { continue };
            let Ok(device) = BluetoothLEDevice::FromIdAsync(&id).and_then(|o| o.join()) else {
                continue;
            };
            let name = device.Name().map(|n| n.to_string()).unwrap_or_default();
            let connected = device.ConnectionStatus() == Ok(BluetoothConnectionStatus::Connected);
            // Aynı aygıt iki listede de olabilir (çift modlu): bağlıysa işaretle.
            if let Some(existing) = out.iter_mut().find(|d| d.name == name) {
                existing.connected |= connected;
                continue;
            }
            out.push(BtDevice {
                name,
                connected,
                kind: "input",
                id: id.to_string(),
                le: true,
            });
        }
    }
    out.retain(|d| !d.name.is_empty());
    out.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// Klasik Bluetooth'ta ses / hands-free / HID hizmetleri (UUID kısa biçimleri).
const BT_SERVICES: [u32; 5] = [0x110B, 0x1108, 0x111E, 0x1124, 0x110E];
const SERVICE_DISABLE: u32 = 0;
const SERVICE_ENABLE: u32 = 1;

fn bt_uuid(short: u32) -> GUID {
    GUID::from_u128(((short as u128) << 96) | 0x0000_1000_8000_0080_5F9B_34FB)
}

/// Eşleşmiş bir aygıtı bağlar (`connect`) ya da ayırır. Engelleyicidir (birkaç sn).
pub fn bt_set_connected(id: &str, le: bool, connect: bool) -> Result<(), String> {
    com();
    let id = HSTRING::from(id);
    if le {
        return bt_le_connect(&id, connect);
    }
    let device = BluetoothDevice::FromIdAsync(&id)
        .and_then(|o| o.join())
        .map_err(|e| e.message())?;
    let address = device.BluetoothAddress().map_err(|e| e.message())?;
    let mut info = BLUETOOTH_DEVICE_INFO {
        dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
        ..Default::default()
    };
    info.Address.Anonymous.ullLong = address;
    // Sınıf / ad bilgisini doldurur; başarısızsa yine de denenir.
    unsafe { BluetoothGetDeviceInfo(None, &mut info) };
    let flag = if connect {
        SERVICE_ENABLE
    } else {
        SERVICE_DISABLE
    };
    let mut ok = false;
    for short in BT_SERVICES {
        let guid = bt_uuid(short);
        // 0 = ERROR_SUCCESS; aygıtın desteklemediği hizmet hata verir, atlanır.
        if unsafe { BluetoothSetServiceState(None, &info, &guid, flag) } == 0 {
            ok = true;
        }
    }
    if !ok {
        return Err(if connect {
            "Bağlanılamadı".into()
        } else {
            "use_settings".into()
        });
    }
    if connect {
        // Bağlantı kurulması birkaç saniye sürebilir.
        for _ in 0..20 {
            if device.ConnectionStatus() == Ok(BluetoothConnectionStatus::Connected) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        return Err("Bağlanılamadı".into());
    }
    Ok(())
}

/// LE aygıtta hizmetleri önbelleksiz sorgulamak bağlantıyı tetikler.
fn bt_le_connect(id: &HSTRING, connect: bool) -> Result<(), String> {
    if !connect {
        return Err("use_settings".into());
    }
    let device = BluetoothLEDevice::FromIdAsync(id)
        .and_then(|o| o.join())
        .map_err(|e| e.message())?;
    let status = device
        .GetGattServicesWithCacheModeAsync(BluetoothCacheMode::Uncached)
        .and_then(|o| o.join())
        .and_then(|r| r.Status())
        .map_err(|e| e.message())?;
    if status == GattCommunicationStatus::Success
        && device.ConnectionStatus() == Ok(BluetoothConnectionStatus::Connected)
    {
        Ok(())
    } else {
        Err("Bağlanılamadı".into())
    }
}
