//! Kurumsal (802.1X) Wi-Fi için Native WiFi (wlanapi) yedek yolu.
//!
//! WinRT `ConnectWithPasswordCredentialAsync` bazı ağlarda (UnspecifiedFailure,
//! Timeout, UnsupportedAuthenticationProtocol) sonuç vermez. Bu yol WPA2/WPA3
//! kurumsal profili (PEAP + MSCHAPv2, olmazsa EAP-TTLS + PAP) kendisi yazar, kullanıcı bilgilerini
//! EapHost kullanıcı verisi olarak verir ve bağlanır. Sunucu sertifikası
//! doğrulaması KAPATILMAZ: Windows, Ayarlar'dakiyle aynı şekilde güven sorar.
//!
//! Kullanıcı adı/parola içeren XML yalnızca bellekte kurulur; günlüğe yazılmaz,
//! diske yazılmaz, kullanımdan sonra sıfırlanır.

use std::ffi::c_void;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::time::Duration;

use crate::win::wide;
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_infrastructure, wlan_connection_mode_profile, wlan_interface_state_connected,
    wlan_intf_opcode_current_connection, wlan_notification_acm_connection_attempt_fail,
    wlan_notification_acm_connection_complete, WlanCloseHandle, WlanConnect, WlanDeleteProfile,
    WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList, WlanGetProfile,
    WlanOpenHandle, WlanQueryInterface, WlanReasonCodeToString, WlanRegisterNotification, WlanScan,
    WlanSetProfile, WlanSetProfileEapXmlUserData, DOT11_AUTH_ALGO_WPA3_ENT,
    DOT11_AUTH_ALGO_WPA3_ENT_192, L2_NOTIFICATION_DATA, WLAN_API_VERSION_2_0,
    WLAN_AVAILABLE_NETWORK_LIST, WLAN_CONNECTION_ATTRIBUTES, WLAN_CONNECTION_NOTIFICATION_DATA,
    WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST, WLAN_NOTIFICATION_SOURCE_ACM,
    WLAN_NOTIFICATION_SOURCE_NONE, WLAN_REASON_CODE_MSMSEC_AUTH_START_TIMEOUT,
    WLAN_REASON_CODE_MSMSEC_AUTH_SUCCESS_TIMEOUT, WLAN_REASON_CODE_MSMSEC_CANCELLED,
    WLAN_REASON_CODE_MSMSEC_FORCED_FAILURE, WLAN_REASON_CODE_MSMSEC_UI_REQUEST_FAILURE,
    WLAN_REASON_CODE_SUCCESS, WLAN_SET_EAPHOST_FLAGS,
};

const ERROR_SUCCESS: u32 = 0;
const ERROR_NOT_FOUND: u32 = 1168;
const WAIT: Duration = Duration::from_secs(20);
// 802.1X (OneX) neden kodları (L2_REASON_CODE_ONEX_BASE = 0x50000 tabanlı).
const ONEX_UI_DISABLED: u32 = 0x5_0003;
const ONEX_UI_FAILURE: u32 = 0x5_0004;
const ONEX_EAP_FAILURE_RECEIVED: u32 = 0x5_0005;
const ONEX_PROFILE_DISALLOWED_EAP_TYPE: u32 = 0x5_000A;
const ONEX_UI_CANCELLED: u32 = 0x5_0011;
const ONEX_UI_NOT_PERMITTED: u32 = 0x5_0014;

/// Native yolun sonucu; çağıran Türkçe iletiye çevirir.
#[derive(Debug, PartialEq, Eq)]
pub enum NativeError {
    /// Kullanıcı adı / parola reddedildi.
    Credentials,
    /// Sunucu sertifikası güvenilmedi ya da Windows'un güven sorusu yanıtlanmadı.
    CertTrust,
    /// Ağ görünmüyor.
    NotVisible,
    /// Sunucu EAP'yi açıkça reddetti: yanlış yöntem (PEAP/TTLS) ya da yanlış bilgi.
    EapRejected,
    /// Diğer hata (Türkçe ileti).
    Other(String),
}

/// Kurumsal EAP yöntemi. Kurumlar ikisinden birini kullanır (eduroam'da ikisi de yaygın).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// PEAP (25) + iç EAP-MSCHAPv2 (26).
    Peap,
    /// EAP-TTLS (21, Microsoft yazarı 311) + iç PAP.
    TtlsPap,
}

impl Method {
    fn other(self) -> Self {
        match self {
            Method::Peap => Method::TtlsPap,
            Method::TtlsPap => Method::Peap,
        }
    }

    /// Mevcut bir profilin XML'inden yöntemi çıkarır; desteklemediğimiz yöntemse None.
    fn detect(profile_xml: &str) -> Option<Self> {
        if profile_xml.contains("EapTtls") && profile_xml.contains("PAPAuthentication") {
            Some(Method::TtlsPap)
        } else if profile_xml.contains("MsPeapConnectionPropertiesV1")
            && profile_xml.contains(">26</")
        {
            Some(Method::Peap)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    Wpa2,
    Wpa3Ent,
}

// ---------------------------------------------------------------- XML kurucular

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\t' => {} // XML 1.0'da geçersiz
            c => out.push(c),
        }
    }
    out
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

const PEAP_CONFIG: &str = r#"<EAPConfig><EapHostConfig xmlns="http://www.microsoft.com/provisioning/EapHostConfig" xmlns:eapCommon="http://www.microsoft.com/provisioning/EapCommon" xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapConnectionPropertiesV1" xmlns:msPeap="http://www.microsoft.com/provisioning/MsPeapConnectionPropertiesV1" xmlns:msChapV2="http://www.microsoft.com/provisioning/MsChapV2ConnectionPropertiesV1"><EapMethod><eapCommon:Type>25</eapCommon:Type><eapCommon:AuthorId>0</eapCommon:AuthorId></EapMethod><Config><baseEap:Eap><baseEap:Type>25</baseEap:Type><msPeap:EapType><msPeap:ServerValidation><msPeap:DisableUserPromptForServerValidation>false</msPeap:DisableUserPromptForServerValidation><msPeap:ServerNames></msPeap:ServerNames></msPeap:ServerValidation><msPeap:FastReconnect>true</msPeap:FastReconnect><msPeap:InnerEapOptional>false</msPeap:InnerEapOptional><baseEap:Eap><baseEap:Type>26</baseEap:Type><msChapV2:EapType><msChapV2:UseWinLogonCredentials>false</msChapV2:UseWinLogonCredentials></msChapV2:EapType></baseEap:Eap><msPeap:EnableQuarantineChecks>false</msPeap:EnableQuarantineChecks><msPeap:RequireCryptoBinding>false</msPeap:RequireCryptoBinding></msPeap:EapType></baseEap:Eap></Config></EapHostConfig></EAPConfig>"#;

// Windows'un kendi kurduğu TTLS profilinin biçimi; sunucu adı / kök CA boş:
// Windows ilk bağlantıda sertifikayı gösterip güven sorar.
const TTLS_CONFIG: &str = r#"<EAPConfig><EapHostConfig xmlns="http://www.microsoft.com/provisioning/EapHostConfig"><EapMethod><Type xmlns="http://www.microsoft.com/provisioning/EapCommon">21</Type><VendorId xmlns="http://www.microsoft.com/provisioning/EapCommon">0</VendorId><VendorType xmlns="http://www.microsoft.com/provisioning/EapCommon">0</VendorType><AuthorId xmlns="http://www.microsoft.com/provisioning/EapCommon">311</AuthorId></EapMethod><Config xmlns="http://www.microsoft.com/provisioning/EapHostConfig"><EapTtls xmlns="http://www.microsoft.com/provisioning/EapTtlsConnectionPropertiesV1"><ServerValidation><ServerNames></ServerNames><DisablePrompt>false</DisablePrompt></ServerValidation><Phase2Authentication><PAPAuthentication/></Phase2Authentication><Phase1Identity><IdentityPrivacy>false</IdentityPrivacy></Phase1Identity></EapTtls></Config></EapHostConfig></EAPConfig>"#;

/// WLAN profil XML'i (PEAP-MSCHAPv2 ya da TTLS-PAP), sunucu doğrulaması açık.
pub fn build_profile_xml(ssid: &str, auth: Auth, auto: bool, method: Method) -> String {
    let name = xml_escape(ssid);
    let hex = hex_upper(ssid.as_bytes());
    let mode = if auto { "auto" } else { "manual" };
    let auth = match auth {
        Auth::Wpa2 => "WPA2",
        Auth::Wpa3Ent => "WPA3ENT",
    };
    let eap = match method {
        Method::Peap => PEAP_CONFIG,
        Method::TtlsPap => TTLS_CONFIG,
    };
    format!(
        r#"<?xml version="1.0"?><WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1"><name>{name}</name><SSIDConfig><SSID><hex>{hex}</hex><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>{mode}</connectionMode><MSM><security><authEncryption><authentication>{auth}</authentication><encryption>AES</encryption><useOneX>true</useOneX></authEncryption><OneX xmlns="http://www.microsoft.com/networking/OneX/v1"><authMode>user</authMode>{eap}</OneX></security></MSM></WLANProfile>"#
    )
}

/// EapHost kullanıcı kimlik bilgileri. Gizli veri içerir.
fn build_user_data_xml(username: &str, password: &str, method: Method) -> String {
    let u = xml_escape(username);
    let p = xml_escape(password);
    if method == Method::TtlsPap {
        return format!(
            r#"<?xml version="1.0"?><EapHostUserCredentials xmlns="http://www.microsoft.com/provisioning/EapHostUserCredentials" xmlns:eapCommon="http://www.microsoft.com/provisioning/EapCommon" xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapMethodUserCredentials"><EapMethod><eapCommon:Type>21</eapCommon:Type><eapCommon:AuthorId>311</eapCommon:AuthorId></EapMethod><Credentials><EapTtls xmlns="http://www.microsoft.com/provisioning/EapTtlsUserPropertiesV1"><Username>{u}</Username><Password>{p}</Password></EapTtls></Credentials></EapHostUserCredentials>"#
        );
    }
    format!(
        r#"<?xml version="1.0"?><EapHostUserCredentials xmlns="http://www.microsoft.com/provisioning/EapHostUserCredentials" xmlns:eapCommon="http://www.microsoft.com/provisioning/EapCommon" xmlns:baseEap="http://www.microsoft.com/provisioning/BaseEapUserPropertiesV1" xmlns:MsPeap="http://www.microsoft.com/provisioning/MsPeapUserPropertiesV1" xmlns:MsChapV2="http://www.microsoft.com/provisioning/MsChapV2UserPropertiesV1"><EapMethod><eapCommon:Type>25</eapCommon:Type><eapCommon:AuthorId>0</eapCommon:AuthorId></EapMethod><Credentials><baseEap:Eap><baseEap:Type>25</baseEap:Type><MsPeap:EapType><baseEap:Eap><baseEap:Type>26</baseEap:Type><MsChapV2:EapType><MsChapV2:Username>{u}</MsChapV2:Username><MsChapV2:Password>{p}</MsChapV2:Password><MsChapV2:LogonDomain></MsChapV2:LogonDomain></MsChapV2:EapType></baseEap:Eap></MsPeap:EapType></baseEap:Eap></Credentials></EapHostUserCredentials>"#
    )
}

fn wipe_string(s: &mut String) {
    // SAFETY: tüm baytlar 0 (geçerli UTF-8) olur.
    unsafe { s.as_bytes_mut().fill(0) };
    s.clear();
}

/// Hata kodundan kimlik/sertifika sınıflandırması.
fn classify_reason(reason: u32) -> NativeError {
    match reason {
        // Kullanıcı sertifika sorusunu yanıtlamadı / iptal etti / UI isteği başarısız.
        WLAN_REASON_CODE_MSMSEC_UI_REQUEST_FAILURE
        | WLAN_REASON_CODE_MSMSEC_CANCELLED
        | WLAN_REASON_CODE_MSMSEC_AUTH_START_TIMEOUT => NativeError::CertTrust,
        // Sunucu kimlik doğrulamayı reddetti / başarı beklenirken zaman aşımı.
        WLAN_REASON_CODE_MSMSEC_AUTH_SUCCESS_TIMEOUT | WLAN_REASON_CODE_MSMSEC_FORCED_FAILURE => {
            NativeError::Credentials
        }
        // Sunucu EAP'yi açıkça reddetti ya da profildeki yöntemi kabul etmiyor.
        ONEX_EAP_FAILURE_RECEIVED | ONEX_PROFILE_DISALLOWED_EAP_TYPE => NativeError::EapRejected,
        // Windows'un sertifika / kimlik penceresi açılamadı ya da kapatıldı.
        ONEX_UI_CANCELLED | ONEX_UI_FAILURE | ONEX_UI_DISABLED | ONEX_UI_NOT_PERMITTED => {
            NativeError::CertTrust
        }
        r => NativeError::Other(format!("Bağlanılamadı: {}", reason_text(r))),
    }
}

fn reason_text(reason: u32) -> String {
    let mut buf = vec![0u16; 256];
    // SAFETY: tampon 256 geniş karakter. Sarmalayıcı dilim alıyor ama API tampona
    // yazıyor; dilim yazılabilir işaretçiden türetilir.
    let rc = unsafe {
        WlanReasonCodeToString(
            reason,
            std::slice::from_raw_parts(buf.as_mut_ptr(), buf.len()),
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return format!("kod {reason:#x}");
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let text = String::from_utf16_lossy(&buf[..len]);
    if text.trim().is_empty() {
        format!("kod {reason:#x}")
    } else {
        text.trim().to_string()
    }
}

// ---------------------------------------------------------------- Win32 yardımcıları

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: WlanOpenHandle'dan gelen geçerli tanıtıcı.
        unsafe { WlanCloseHandle(self.0, None) };
    }
}

/// WlanFreeMemory ile serbest bırakılan işaretçi.
struct WlanMem<T>(*mut T);
impl<T> Drop for WlanMem<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: wlanapi'nin ayırdığı bellek.
            unsafe { WlanFreeMemory(self.0 as *const c_void) };
        }
    }
}

/// SSID'yi gören arayüz ve ağın yetkilendirme algoritması.
/// Ağı bulur; listede yoksa taze tarama ister ve birkaç saniye bekler (Windows'un
/// önbelleğindeki liste eski olabilir).
fn find_interface(h: HANDLE, ssid: &str) -> Result<(GUID, Auth), NativeError> {
    match lookup(h, ssid) {
        Err(NativeError::NotVisible) => {}
        other => return other,
    }
    request_scan(h);
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        match lookup(h, ssid) {
            Err(NativeError::NotVisible) => {}
            other => return other,
        }
    }
    Err(NativeError::NotVisible)
}

/// Bütün Wi-Fi arayüzlerinde tarama başlatır (sonucu beklemez).
fn request_scan(h: HANDLE) {
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    // SAFETY: çıktı işaretçisi geçerli.
    if unsafe { WlanEnumInterfaces(h, None, &mut list) } != ERROR_SUCCESS {
        return;
    }
    let list = WlanMem(list);
    // SAFETY: dwNumberOfItems kadar öğe ardışık.
    let ifaces = unsafe {
        std::slice::from_raw_parts(
            (*list.0).InterfaceInfo.as_ptr(),
            (*list.0).dwNumberOfItems as usize,
        )
    };
    for iface in ifaces {
        // SAFETY: geçerli tanıtıcı ve GUID; isteğe bağlı parametreler boş.
        let _ = unsafe { WlanScan(h, &iface.InterfaceGuid, None, None, None) };
    }
}

fn lookup(h: HANDLE, ssid: &str) -> Result<(GUID, Auth), NativeError> {
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    // SAFETY: çıktı işaretçisi geçerli.
    let rc = unsafe { WlanEnumInterfaces(h, None, &mut list) };
    if rc != ERROR_SUCCESS {
        return Err(NativeError::Other("Wi-Fi arayüzü bulunamadı".into()));
    }
    let list = WlanMem(list);
    // SAFETY: dwNumberOfItems kadar öğe ardışık.
    let ifaces = unsafe {
        std::slice::from_raw_parts(
            (*list.0).InterfaceInfo.as_ptr(),
            (*list.0).dwNumberOfItems as usize,
        )
    };
    for iface in ifaces {
        let mut nets: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        // SAFETY: geçerli tanıtıcı, GUID ve çıktı işaretçisi.
        let rc =
            unsafe { WlanGetAvailableNetworkList(h, &iface.InterfaceGuid, 0, None, &mut nets) };
        if rc != ERROR_SUCCESS {
            continue;
        }
        let nets = WlanMem(nets);
        // SAFETY: yukarıdaki gibi.
        let items = unsafe {
            std::slice::from_raw_parts(
                (*nets.0).Network.as_ptr(),
                (*nets.0).dwNumberOfItems as usize,
            )
        };
        for n in items {
            let len = (n.dot11Ssid.uSSIDLength as usize).min(32);
            if &n.dot11Ssid.ucSSID[..len] != ssid.as_bytes() {
                continue;
            }
            let algo = n.dot11DefaultAuthAlgorithm;
            // RSNA ve bilinmeyenler WPA2/AES; WPA3-Enterprise raporlanırsa WPA3ENT.
            let auth = if algo == DOT11_AUTH_ALGO_WPA3_ENT || algo == DOT11_AUTH_ALGO_WPA3_ENT_192 {
                Auth::Wpa3Ent
            } else {
                Auth::Wpa2
            };
            return Ok((iface.InterfaceGuid, auth));
        }
    }
    Err(NativeError::NotVisible)
}

/// Kayıtlı profilin XML'i (gizli alanlar olmadan); profil yoksa None.
fn profile_xml(h: HANDLE, guid: &GUID, name: &[u16]) -> Option<String> {
    let mut xml = PWSTR::null();
    // SAFETY: ad sıfır sonlu; çıktı işaretçisi geçerli.
    let rc = unsafe { WlanGetProfile(h, guid, PCWSTR(name.as_ptr()), None, &mut xml, None, None) };
    let text = if rc == ERROR_SUCCESS && !xml.is_null() {
        // SAFETY: wlanapi sıfır sonlu geniş dizge döndürdü.
        unsafe { xml.to_string() }.ok()
    } else {
        None
    };
    if !xml.is_null() {
        // SAFETY: wlanapi belleği.
        unsafe { WlanFreeMemory(xml.0 as *const c_void) };
    }
    if rc == ERROR_NOT_FOUND {
        None
    } else {
        Some(text.unwrap_or_default())
    }
}

struct Ctx {
    tx: Mutex<Sender<u32>>,
    guid: GUID,
    /// Sıfır sonlu profil adı.
    profile: Vec<u16>,
}

unsafe extern "system" fn on_notify(data: *mut L2_NOTIFICATION_DATA, context: *mut c_void) {
    if data.is_null() || context.is_null() {
        return;
    }
    // SAFETY: bağlam, kayıt süresince yaşayan Ctx'i gösterir.
    let (d, ctx) = unsafe { (&*data, &*(context as *const Ctx)) };
    if d.NotificationSource != WLAN_NOTIFICATION_SOURCE_ACM
        || d.InterfaceGuid != ctx.guid
        || (d.NotificationCode != wlan_notification_acm_connection_complete.0 as u32
            && d.NotificationCode != wlan_notification_acm_connection_attempt_fail.0 as u32)
        || d.pData.is_null()
        || (d.dwDataSize as usize) < std::mem::size_of::<WLAN_CONNECTION_NOTIFICATION_DATA>() - 2
    {
        return;
    }
    // SAFETY: boyut denetlendi; veri yalnızca okunur.
    let n = unsafe { &*(d.pData as *const WLAN_CONNECTION_NOTIFICATION_DATA) };
    let len = n.strProfileName.iter().position(|&c| c == 0).unwrap_or(256);
    if n.strProfileName[..len] != ctx.profile[..ctx.profile.len() - 1] {
        return;
    }
    if let Ok(tx) = ctx.tx.lock() {
        let _ = tx.send(n.wlanReasonCode);
    }
}

fn is_connected(h: HANDLE, guid: &GUID, profile: &[u16]) -> bool {
    let mut size = 0u32;
    let mut data: *mut c_void = std::ptr::null_mut();
    // SAFETY: çıktı işaretçileri geçerli.
    let rc = unsafe {
        WlanQueryInterface(
            h,
            guid,
            wlan_intf_opcode_current_connection,
            None,
            &mut size,
            &mut data,
            None,
        )
    };
    if rc != ERROR_SUCCESS || data.is_null() {
        return false;
    }
    let mem = WlanMem(data as *mut WLAN_CONNECTION_ATTRIBUTES);
    // SAFETY: opcode bu yapıyı döndürür.
    let a = unsafe { &*mem.0 };
    let len = a.strProfileName.iter().position(|&c| c == 0).unwrap_or(256);
    a.isState == wlan_interface_state_connected
        && a.strProfileName[..len] == profile[..profile.len() - 1]
}

/// Kurumsal ağa Native WiFi ile bağlanır (profil + kullanıcı verisi + WlanConnect).
pub fn connect(ssid: &str, username: &str, password: &str, auto: bool) -> Result<(), NativeError> {
    let mut version = 0u32;
    let mut raw = HANDLE::default();
    // SAFETY: çıktı işaretçileri geçerli.
    let rc = unsafe { WlanOpenHandle(WLAN_API_VERSION_2_0, None, &mut version, &mut raw) };
    if rc != ERROR_SUCCESS {
        return Err(NativeError::Other("Wi-Fi hizmetine erişilemedi".into()));
    }
    let handle = Handle(raw);
    let h = handle.0;

    let (guid, auth) = find_interface(h, ssid)?;
    let name = wide(ssid);
    let existing = profile_xml(h, &guid, &name);

    // Windows'ta bu ağ için tanıdığımız bir profil varsa (ör. Windows'un kendi
    // algıladığı TTLS profili) onu koru: içindeki öğrenilmiş sunucu adı / kök CA
    // sayesinde sertifika sorusu çıkmaz. Yalnızca kullanıcı bilgisi önbelleğini aç
    // (Windows'un profili `cacheUserData=false` ile gelir; o hâlde kullanıcı verisi
    // kabul edilmez). Sunucu EAP'yi reddederse diğer yöntemle kendi profilimizi yaz.
    let plan: Vec<(Method, String)> = match existing
        .as_deref()
        .and_then(|x| Method::detect(x).map(|m| (m, x)))
    {
        Some((m, x)) => vec![
            (m, patch_existing(x, auto)),
            (m.other(), build_profile_xml(ssid, auth, auto, m.other())),
        ],
        None => vec![
            (
                Method::Peap,
                build_profile_xml(ssid, auth, auto, Method::Peap),
            ),
            (
                Method::TtlsPap,
                build_profile_xml(ssid, auth, auto, Method::TtlsPap),
            ),
        ],
    };
    let mut outcome = Err(NativeError::EapRejected);
    for (method, profile) in &plan {
        outcome = attempt(h, &guid, &name, *method, profile, username, password);
        super::super::flylog::log(&format!("wifi kurumsal: {method:?} sonucu {outcome:?}"));
        if outcome != Err(NativeError::EapRejected) {
            break;
        }
    }
    // Her yöntem açıkça reddedildiyse sorun kullanıcı adı / parola.
    if outcome == Err(NativeError::EapRejected) {
        outcome = Err(NativeError::Credentials);
    }
    if outcome.is_err() {
        match &existing {
            // Üzerine yazdığımız eski profili geri koy.
            Some(xml) if !xml.is_empty() => {
                let w = wide(xml);
                let mut reason = 0u32;
                // SAFETY: dizgiler sıfır sonlu.
                unsafe {
                    WlanSetProfile(
                        h,
                        &guid,
                        0,
                        PCWSTR(w.as_ptr()),
                        PCWSTR::null(),
                        true,
                        None,
                        &mut reason,
                    )
                };
            }
            // Yeni açtığımız profili sil: sonraki deneme temiz başlasın.
            None => {
                // SAFETY: ad sıfır sonlu.
                unsafe { WlanDeleteProfile(h, &guid, PCWSTR(name.as_ptr()), None) };
            }
            _ => {}
        }
    }
    outcome
}

/// Mevcut profili korur; yalnızca kullanıcı verisi önbelleğini ve bağlanma kipini ayarlar.
fn patch_existing(xml: &str, auto: bool) -> String {
    let mode = if auto { "auto" } else { "manual" };
    xml.replace(
        "<cacheUserData>false</cacheUserData>",
        "<cacheUserData>true</cacheUserData>",
    )
    .replace(
        "<connectionMode>auto</connectionMode>",
        &format!("<connectionMode>{mode}</connectionMode>"),
    )
    .replace(
        "<connectionMode>manual</connectionMode>",
        &format!("<connectionMode>{mode}</connectionMode>"),
    )
}

/// Tek deneme: profili yaz, kullanıcı verisini ver, bağlan, sonucu bekle.
fn attempt(
    h: HANDLE,
    guid: &GUID,
    name: &[u16],
    method: Method,
    profile: &str,
    username: &str,
    password: &str,
) -> Result<(), NativeError> {
    {
        let profile_w = wide(profile);
        let mut reason = 0u32;
        // SAFETY: dizgiler sıfır sonlu ve çağrı süresince yaşıyor. Aynı adlı profil
        // varsa üzerine yazılır.
        let rc = unsafe {
            WlanSetProfile(
                h,
                guid,
                0,
                PCWSTR(profile_w.as_ptr()),
                PCWSTR::null(),
                true,
                None,
                &mut reason,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(NativeError::Other(format!(
                "Wi-Fi profili oluşturulamadı: {}",
                reason_text(reason)
            )));
        }
    }

    let mut user_xml = build_user_data_xml(username, password, method);
    let mut user_w = wide(&user_xml);
    wipe_string(&mut user_xml);
    // SAFETY: dizgiler sıfır sonlu; bayrak 0 = yalnızca geçerli kullanıcı.
    let rc = unsafe {
        WlanSetProfileEapXmlUserData(
            h,
            guid,
            PCWSTR(name.as_ptr()),
            WLAN_SET_EAPHOST_FLAGS(0),
            PCWSTR(user_w.as_ptr()),
            None,
        )
    };
    user_w.fill(0);
    if rc != ERROR_SUCCESS {
        super::super::flylog::log(&format!("wifi kurumsal: kullanıcı verisi hatası {rc}"));
        return Err(NativeError::Other(
            "Kullanıcı bilgileri kaydedilemedi".into(),
        ));
    }

    let (tx, rx) = mpsc::channel();
    let ctx = Box::new(Ctx {
        tx: Mutex::new(tx),
        guid: *guid,
        profile: name.to_vec(),
    });
    // SAFETY: ctx, kayıt silinene kadar yaşar (aşağıda NONE ile kayıt kaldırılır).
    let rc = unsafe {
        WlanRegisterNotification(
            h,
            WLAN_NOTIFICATION_SOURCE_ACM,
            true,
            Some(on_notify),
            Some(&*ctx as *const Ctx as *const c_void),
            None,
            None,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(NativeError::Other("Bağlantı olayları dinlenemedi".into()));
    }

    let params = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: PCWSTR(name.as_ptr()),
        pDot11Ssid: std::ptr::null_mut(),
        pDesiredBssidList: std::ptr::null_mut(),
        dot11BssType: dot11_BSS_type_infrastructure,
        dwFlags: 0,
    };
    // SAFETY: parametreler çağrı süresince geçerli.
    let rc = unsafe { WlanConnect(h, guid, &params, None) };
    let outcome = if rc != ERROR_SUCCESS {
        Err(NativeError::Other("Bağlantı başlatılamadı".into()))
    } else {
        match rx.recv_timeout(WAIT) {
            Ok(WLAN_REASON_CODE_SUCCESS) => Ok(()),
            Ok(r) => Err(classify_reason(r)),
            // Olay gelmedi: durumu sorgula; yoksa kullanıcı güven sorusunu yanıtlamadı.
            Err(_) if is_connected(h, guid, name) => Ok(()),
            Err(_) => Err(NativeError::CertTrust),
        }
    };
    // Kaydı kaldır (süren geri çağrıları bekler), sonra bağlamı bırak.
    // SAFETY: geçerli tanıtıcı.
    unsafe {
        WlanRegisterNotification(
            h,
            WLAN_NOTIFICATION_SOURCE_NONE,
            true,
            None,
            None,
            None,
            None,
        )
    };
    drop(ctx);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_ssid() {
        let x = build_profile_xml("A&B <\"x\"> 'y'", Auth::Wpa2, true, Method::Peap);
        assert!(x.contains("<name>A&amp;B &lt;&quot;x&quot;&gt; &apos;y&apos;</name>"));
        assert!(!x.contains("A&B"));
    }

    #[test]
    fn hex_is_uppercase_utf8() {
        let x = build_profile_xml("aÇ", Auth::Wpa2, true, Method::Peap);
        assert!(x.contains("<hex>61C387</hex>"));
    }

    #[test]
    fn wpa2_vs_wpa3() {
        let a = build_profile_xml("n", Auth::Wpa2, true, Method::Peap);
        let b = build_profile_xml("n", Auth::Wpa3Ent, true, Method::Peap);
        assert!(a.contains("<authentication>WPA2</authentication>"));
        assert!(b.contains("<authentication>WPA3ENT</authentication>"));
        assert!(a.contains("<useOneX>true</useOneX>"));
    }

    #[test]
    fn connection_mode_follows_auto() {
        assert!(build_profile_xml("n", Auth::Wpa2, true, Method::Peap)
            .contains("<connectionMode>auto<"));
        assert!(build_profile_xml("n", Auth::Wpa2, false, Method::Peap)
            .contains("<connectionMode>manual<"));
    }

    #[test]
    fn server_validation_stays_on() {
        let x = build_profile_xml("n", Auth::Wpa2, true, Method::Peap);
        assert!(x.contains(">false</msPeap:DisableUserPromptForServerValidation>"));
        assert!(x.contains("<msChapV2:UseWinLogonCredentials>false<"));
        assert!(x.contains("<baseEap:Type>26</baseEap:Type>"));
    }

    #[test]
    fn user_data_escapes_and_wipes() {
        let mut x = build_user_data_xml("a<b", "p&\"w", Method::Peap);
        assert!(x.contains("<MsChapV2:Username>a&lt;b<"));
        assert!(x.contains("<MsChapV2:Password>p&amp;&quot;w<"));
        assert!(x.contains("<MsChapV2:LogonDomain></MsChapV2:LogonDomain>"));
        wipe_string(&mut x);
        assert!(x.is_empty());
    }

    #[test]
    fn ttls_profile_and_user_data() {
        let x = build_profile_xml("n", Auth::Wpa2, true, Method::TtlsPap);
        assert!(x.contains(">21</Type>") && x.contains("<PAPAuthentication/>"));
        assert!(x.contains("<DisablePrompt>false</DisablePrompt>"));
        assert_eq!(Method::detect(&x), Some(Method::TtlsPap));
        assert_eq!(
            Method::detect(&build_profile_xml("n", Auth::Wpa2, true, Method::Peap)),
            Some(Method::Peap)
        );
        let u = build_user_data_xml("a@b", "p<", Method::TtlsPap);
        assert!(u.contains("<Username>a@b</Username><Password>p&lt;</Password>"));
    }

    #[test]
    fn patch_keeps_trust_and_enables_cache() {
        let x = "<connectionMode>auto</connectionMode><cacheUserData>false</cacheUserData><TrustedRootCAHash>c4 10</TrustedRootCAHash>";
        let y = patch_existing(x, false);
        assert!(y.contains("<cacheUserData>true</cacheUserData>"));
        assert!(y.contains("<connectionMode>manual</connectionMode>"));
        assert!(y.contains("<TrustedRootCAHash>c4 10</TrustedRootCAHash>"));
    }

    #[test]
    fn reason_mapping() {
        assert_eq!(
            classify_reason(WLAN_REASON_CODE_MSMSEC_UI_REQUEST_FAILURE),
            NativeError::CertTrust
        );
        assert_eq!(
            classify_reason(WLAN_REASON_CODE_MSMSEC_FORCED_FAILURE),
            NativeError::Credentials
        );
        assert_eq!(
            classify_reason(ONEX_EAP_FAILURE_RECEIVED),
            NativeError::EapRejected
        );
    }
}
