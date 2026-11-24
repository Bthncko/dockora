//! Atlama listesi: Windows görev çubuğunun sağ tık menüsündeki "Görevler",
//! "Son kullanılanlar" ve uygulamanın kendi kategorileri (ör. Firefox "Çok
//! ziyaret edilenler", Spotify "Yakın zamanda çalınanlar").
//!
//! Kaynaklar:
//! - Son / sık kullanılanlar: `IApplicationDocumentLists` (belgelenmiş API).
//! - Görevler ve özel kategoriler: uygulamanın `ICustomDestinationList` ile
//!   yazdığı `%APPDATA%\Microsoft\Windows\Recent\CustomDestinations\<kimlik>.customDestinations-ms`.
//!   Okuma API'si yok; biçim belgelenmemiş ama basit: başlık, kategoriler,
//!   her öğe = sınıf kimliği + `IPersistStream` ile kaydedilmiş nesne (çoğunlukla
//!   kısayol). Nesneyi COM'a yükletiyoruz, kısayol biçimini kendimiz çözmüyoruz.
//!   Okunamayan dosya menüyü bozmaz, yalnızca o kısım çıkmaz.
//!
//! Dosya adı: uygulama kimliğinin (AppUserModelID) büyük harfli UTF-16
//! hâlinin CRC-64'ü (Jones polinomu, başlangıç tüm bitler 1, son XOR yok).
//! Denendi (bu makinede): Spotify, WhatsApp, Claude, Firefox, VS Code, Explorer.
//! Kimliği pencereye yazmayan uygulamalarda (VS Code, Explorer) kimlik Başlat
//! menüsündeki kaydından (`shell:AppsFolder`) bulunur.
//!
//! İçerik (tarama geçmişi, dosya adları) günlüğe yazılmaz.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_AppUserModel_IsDestListSeparator, PKEY_Link_TargetParsingPath, PKEY_Title,
};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantToBoolean, PropVariantToStringAlloc,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistStream, STREAM_SEEK_CUR, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::Common::IObjectArray;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    ADLT_FREQUENT, ADLT_RECENT, APPDOCLISTTYPE, ApplicationDocumentLists, BHID_EnumItems,
    IApplicationDocumentLists, IEnumShellItems, IShellItem, IShellItem2, IShellLinkW,
    SHCreateItemFromParsingName, SHCreateMemStream, SHLoadIndirectString, SIGDN_FILESYSPATH,
    SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SLGP_RAWPATH,
};
use windows::core::{GUID, Interface, PCWSTR};

use super::launcher::{self, LaunchTarget};
use crate::win::{take_pwstr, wide};

/// Kategori başına en çok öğe (Windows'un varsayılanı da 10).
const MAX_ITEMS: usize = 10;
const FOOTER: u32 = 0xBABF_FBAB;

pub struct Category {
    pub title: String,
    pub items: Vec<JumpItem>,
}

pub struct JumpItem {
    pub title: String,
    pub target: Target,
}

#[derive(Clone)]
pub enum Target {
    /// Kısayol: program + bağımsız değişkenler.
    Link {
        path: String,
        args: String,
        dir: String,
    },
    /// Dosya / klasör (son kullanılanlar).
    File(String),
}

/// Uygulamanın atlama listesi; Windows'taki sırayla (kategoriler, en altta görevler).
pub fn load(exe: Option<&str>, aumid: Option<&str>) -> Vec<Category> {
    let Some(app_id) = app_id(exe, aumid) else {
        return Vec::new();
    };
    let hash = format!("{:x}", app_id_hash(&app_id));
    let recent_dir = recent_dir();
    let custom = std::fs::read(
        recent_dir
            .join("CustomDestinations")
            .join(format!("{hash}.customDestinations-ms")),
    )
    .ok();
    let has_automatic = recent_dir
        .join("AutomaticDestinations")
        .join(format!("{hash}.automaticDestinations-ms"))
        .exists();

    let mut out = Vec::new();
    let mut tasks = Vec::new();
    match custom.as_deref().map(parse_custom) {
        Some(sections) => {
            for section in sections {
                match section {
                    Section::Known(kind) => {
                        let (title, list) = match kind {
                            1 => (crate::i18n::t("Sık kullanılanlar"), ADLT_FREQUENT),
                            _ => (crate::i18n::t("Son kullanılanlar"), ADLT_RECENT),
                        };
                        push(&mut out, title.into(), automatic(&app_id, list));
                    }
                    Section::Custom(title, items) => push(&mut out, title, items),
                    Section::Tasks(items) => tasks = items,
                }
            }
        }
        // Uygulama listeyi özelleştirmemiş: Windows "Son kullanılanlar"ı gösterir.
        None if has_automatic => {
            push(
                &mut out,
                crate::i18n::t("Son kullanılanlar").into(),
                automatic(&app_id, ADLT_RECENT),
            );
        }
        None => {}
    }
    push(&mut out, crate::i18n::t("Görevler").into(), tasks);
    out
}

fn push(out: &mut Vec<Category>, title: String, mut items: Vec<JumpItem>) {
    items.truncate(MAX_ITEMS);
    if !items.is_empty() {
        out.push(Category { title, items });
    }
}

/// Öğeyi açar. Dosyalar, uygulamanın exe'si biliniyorsa onunla açılır (Windows
/// da listeyi tutan uygulamayla açar); bilinmiyorsa varsayılan uygulamayla.
pub fn open(target: &Target, exe: Option<&str>) -> crate::error::AppResult<()> {
    match target {
        Target::Link { path, args, dir } => launcher::shell_open(path, args, dir),
        Target::File(path) => match exe {
            Some(exe) if !exe.to_lowercase().contains("\\windowsapps\\") => {
                launcher::shell_open(exe, &format!("\"{path}\""), "")
            }
            _ => launcher::launch(&LaunchTarget::Exe(path.clone())),
        },
    }
}

// --- uygulama kimliği ----------------------------------------------------------

/// Pencereye yazılmış kimlik; yoksa Başlat menüsündeki kaydın kimliği (exe ile
/// eşleşen). İkisi de yoksa liste yok.
fn app_id(exe: Option<&str>, aumid: Option<&str>) -> Option<String> {
    if let Some(a) = aumid {
        return Some(a.to_string());
    }
    let exe = exe?.to_lowercase();
    apps_folder_ids(false)
        .get(&exe)
        .cloned()
        .or_else(|| apps_folder_ids(true).get(&exe).cloned())
}

struct IdCache {
    map: Option<HashMap<String, String>>,
    built: Option<Instant>,
}

static IDS: Mutex<IdCache> = Mutex::new(IdCache {
    map: None,
    built: None,
});

/// exe (küçük harf) → `shell:AppsFolder` kimliği. `refresh`: bulunamadıysa
/// yeniden tara (en çok dakikada bir; yeni kurulan uygulamalar için).
fn apps_folder_ids(refresh: bool) -> HashMap<String, String> {
    let mut cache = IDS.lock().unwrap();
    let stale = match cache.built {
        None => true,
        Some(at) => refresh && at.elapsed() > Duration::from_secs(60),
    };
    if stale {
        cache.map = Some(scan_apps_folder());
        cache.built = Some(Instant::now());
    }
    cache.map.clone().unwrap_or_default()
}

/// Açılışta bir kez (arka planda): ilk sağ tık beklemesin.
pub fn prewarm() {
    let _ = std::thread::Builder::new()
        .name("jumplist-prewarm".into())
        .spawn(|| {
            crate::win::com_sta();
            apps_folder_ids(false);
        });
}

fn scan_apps_folder() -> HashMap<String, String> {
    let mut map = HashMap::new();
    unsafe {
        let folder = wide("shell:AppsFolder");
        let root: windows::core::Result<IShellItem> =
            SHCreateItemFromParsingName(PCWSTR(folder.as_ptr()), None);
        let Ok(root) = root else {
            return map;
        };
        let Ok(items) = root.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else {
            return map;
        };
        loop {
            let mut batch = [None];
            let mut fetched = 0;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            let Ok(item2) = item.cast::<IShellItem2>() else {
                continue;
            };
            let Ok(target) = item2.GetString(&PKEY_Link_TargetParsingPath) else {
                continue;
            };
            let target = take_pwstr(target).to_lowercase();
            let Ok(id) = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING) else {
                continue;
            };
            let id = take_pwstr(id);
            if target.ends_with(".exe") && !id.is_empty() {
                // Aynı exe'ye birden çok kısayol olabilir: ilki kalsın.
                map.entry(target).or_insert(id);
            }
        }
    }
    map
}

/// Kimliğin dosya adındaki karşılığı (bkz. modül açıklaması).
fn app_id_hash(app_id: &str) -> u64 {
    const POLY: u64 = 0x92C6_4265_D321_39A4;
    let mut crc = u64::MAX;
    for unit in app_id.to_uppercase().encode_utf16() {
        for byte in unit.to_le_bytes() {
            crc ^= u64::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ POLY
                } else {
                    crc >> 1
                };
            }
        }
    }
    crc
}

fn recent_dir() -> PathBuf {
    let appdata = std::env::var_os("APPDATA").unwrap_or_default();
    PathBuf::from(appdata).join(r"Microsoft\Windows\Recent")
}

// --- özel liste dosyası -------------------------------------------------------

enum Section {
    /// Windows'un otomatik listesi: 1 sık, 2 son kullanılanlar.
    Known(i32),
    Custom(String, Vec<JumpItem>),
    Tasks(Vec<JumpItem>),
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u16(&mut self) -> Option<u16> {
        let b = self.data.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn utf16(&mut self, chars: usize) -> Option<String> {
        let units: Vec<u16> = (0..chars).map(|_| self.u16()).collect::<Option<_>>()?;
        Some(String::from_utf16_lossy(&units))
    }
}

/// Biçim: sürüm, kategori sayısı, ayrılmış (3 × u32); her kategori: tür (u32),
/// türe göre içerik ve `FOOTER`. Tür 0 özel (ad + öğeler), 1 bilinen (kimlik),
/// 2 görevler (öğeler). Bozuk yerde okuma durur, okunanlar döner.
fn parse_custom(data: &[u8]) -> Vec<Section> {
    let mut r = Reader { data, pos: 0 };
    let mut out = Vec::new();
    let (Some(_version), Some(count), Some(_)) = (r.u32(), r.u32(), r.u32()) else {
        return out;
    };
    let stream = unsafe { SHCreateMemStream(Some(data)) };
    let Some(stream) = stream else { return out };
    for _ in 0..count.min(32) {
        let section = match r.u32() {
            Some(0) => {
                let Some(len) = r.u16() else { break };
                let Some(name) = r.utf16(len as usize) else {
                    break;
                };
                let Some(items) = read_items(&mut r, &stream) else {
                    break;
                };
                Section::Custom(indirect(&name), items)
            }
            Some(1) => {
                let Some(id) = r.u32() else { break };
                Section::Known(id as i32)
            }
            Some(2) => {
                let Some(items) = read_items(&mut r, &stream) else {
                    break;
                };
                Section::Tasks(items)
            }
            _ => break,
        };
        if r.u32() != Some(FOOTER) {
            break;
        }
        out.push(section);
    }
    out
}

fn read_items(
    r: &mut Reader,
    stream: &windows::Win32::System::Com::IStream,
) -> Option<Vec<JumpItem>> {
    let count = r.u32()?;
    let mut items = Vec::new();
    for _ in 0..count.min(200) {
        let clsid_bytes: [u8; 16] = r.data.get(r.pos..r.pos + 16)?.try_into().ok()?;
        r.pos += 16;
        let clsid = GUID::from_u128(u128::from_be_bytes(guid_be(clsid_bytes)));
        // Nesneyi COM yükler; okuduğu bayt kadar ilerleriz.
        let (object, end) = unsafe {
            stream.Seek(r.pos as i64, STREAM_SEEK_SET, None).ok()?;
            let persist: IPersistStream =
                CoCreateInstance(&clsid, None, CLSCTX_INPROC_SERVER).ok()?;
            persist.Load(stream).ok()?;
            let mut end = 0u64;
            stream.Seek(0, STREAM_SEEK_CUR, Some(&mut end)).ok()?;
            (persist, end as usize)
        };
        r.pos = end;
        if let Some(item) = to_item(&object) {
            items.push(item);
        }
    }
    Some(items)
}

/// Dosyadaki GUID bayt sırası (ilk üç alan küçük uçlu) → `from_u128` sırası.
fn guid_be(b: [u8; 16]) -> [u8; 16] {
    [
        b[3], b[2], b[1], b[0], b[5], b[4], b[7], b[6], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15],
    ]
}

fn to_item(object: &IPersistStream) -> Option<JumpItem> {
    unsafe {
        if let Ok(link) = object.cast::<IShellLinkW>() {
            let store = object.cast::<IPropertyStore>().ok();
            if let Some(store) = &store
                && prop_bool(store, &PKEY_AppUserModel_IsDestListSeparator)
            {
                return None; // ayraç
            }
            let mut buf = [0u16; 1024];
            link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
                .ok()?;
            let path = expand(&from_buf(&buf));
            let mut args = [0u16; 2048];
            let _ = link.GetArguments(&mut args);
            let mut dir = [0u16; 1024];
            let _ = link.GetWorkingDirectory(&mut dir);
            let title = store
                .as_ref()
                .and_then(|s| prop_string(s, &PKEY_Title))
                .map(|t| indirect(&t))
                .filter(|t| !t.is_empty())
                .or_else(|| {
                    let mut desc = [0u16; 512];
                    link.GetDescription(&mut desc).ok()?;
                    Some(from_buf(&desc)).filter(|d| !d.is_empty())
                })
                .unwrap_or_else(|| file_stem(&path));
            return Some(JumpItem {
                title,
                target: Target::Link {
                    path,
                    args: from_buf(&args),
                    dir: expand(&from_buf(&dir)),
                },
            });
        }
        let item = object.cast::<IShellItem>().ok()?;
        shell_item(&item)
    }
}

fn shell_item(item: &IShellItem) -> Option<JumpItem> {
    unsafe {
        let path = take_pwstr(item.GetDisplayName(SIGDN_FILESYSPATH).ok()?);
        let title = item
            .GetDisplayName(SIGDN_NORMALDISPLAY)
            .map(|p| take_pwstr(p))
            .unwrap_or_else(|_| file_stem(&path));
        Some(JumpItem {
            title,
            target: Target::File(path),
        })
    }
}

/// Son / sık kullanılanlar (Windows'un otomatik listesi).
fn automatic(app_id: &str, kind: APPDOCLISTTYPE) -> Vec<JumpItem> {
    let id = wide(app_id);
    unsafe {
        let Ok(lists) = CoCreateInstance::<_, IApplicationDocumentLists>(
            &ApplicationDocumentLists,
            None,
            CLSCTX_INPROC_SERVER,
        ) else {
            return Vec::new();
        };
        if lists.SetAppID(PCWSTR(id.as_ptr())).is_err() {
            return Vec::new();
        }
        let Ok(array) = lists.GetList::<IObjectArray>(kind, MAX_ITEMS as u32) else {
            return Vec::new();
        };
        let count = array.GetCount().unwrap_or(0);
        (0..count)
            .filter_map(|i| {
                if let Ok(item) = array.GetAt::<IShellItem>(i) {
                    return shell_item(&item);
                }
                let link = array.GetAt::<IShellLinkW>(i).ok()?;
                to_item(&link.cast::<IPersistStream>().ok()?)
            })
            .collect()
    }
}

// --- küçük yardımcılar ---------------------------------------------------------

fn prop_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let value = store.GetValue(key).ok()?;
        let s = take_pwstr(PropVariantToStringAlloc(&value).ok()?);
        (!s.is_empty()).then_some(s)
    }
}

fn prop_bool(store: &IPropertyStore, key: &PROPERTYKEY) -> bool {
    unsafe {
        store
            .GetValue(key)
            .ok()
            .and_then(|v| PropVariantToBoolean(&v).ok())
            .is_some_and(|b| b.as_bool())
    }
}

/// "@shell32.dll,-21781" gibi kaynak dizelerini çözer; düz metin aynen döner.
fn indirect(s: &str) -> String {
    if !s.starts_with('@') {
        return s.to_string();
    }
    let src = wide(s);
    let mut buf = [0u16; 512];
    match unsafe { SHLoadIndirectString(PCWSTR(src.as_ptr()), &mut buf, None) } {
        Ok(()) => from_buf(&buf),
        Err(_) => s.to_string(),
    }
}

fn from_buf(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// `%ProgramFiles%\...` gibi ortam değişkenlerini açar.
fn expand(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn file_stem(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_hash_matches_windows_file_names() {
        // Bu makinedeki dosya adlarıyla doğrulandı.
        assert_eq!(
            format!("{:x}", app_id_hash("Microsoft.VisualStudioCode")),
            "1ced32d74a95c7bc"
        );
        assert_eq!(
            format!("{:x}", app_id_hash("308046B0AF4A39CB")),
            "6824f4a902c78fbd"
        );
    }

    #[test]
    fn guid_bytes_from_file_order() {
        // CLSID_ShellLink {00021401-0000-0000-C000-000000000046} dosyada böyle durur.
        let file = [
            0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x46,
        ];
        let guid = GUID::from_u128(u128::from_be_bytes(guid_be(file)));
        assert_eq!(
            guid,
            GUID::from_u128(0x00021401_0000_0000_c000_000000000046)
        );
    }

    #[test]
    fn known_and_custom_sections_parse() {
        // Spotify dosyasının başı: bilinen kategori + boş özel kategori.
        let mut data = Vec::new();
        for v in [2u32, 2, 0, 1, 2, FOOTER, 0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        data.extend_from_slice(&2u16.to_le_bytes());
        for c in "Ab".encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&0u32.to_le_bytes()); // öğe yok
        data.extend_from_slice(&FOOTER.to_le_bytes());
        let sections = parse_custom(&data);
        assert!(matches!(sections[0], Section::Known(2)));
        assert!(matches!(&sections[1], Section::Custom(n, items) if n == "Ab" && items.is_empty()));
    }

    #[test]
    fn expands_env_vars() {
        let windir = std::env::var("windir").unwrap();
        assert_eq!(expand(r"%windir%\a.exe"), format!(r"{windir}\a.exe"));
        assert_eq!(expand("100%"), "100%");
    }
}
