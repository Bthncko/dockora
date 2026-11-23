//! Dock'un kendi Windows arayüzü için iki dil: Türkçe (kaynak metin) ve
//! İngilizce. Tepsi menüsü, sağ tık menüleri, bildirim kartları, pencere
//! başlıkları gibi sayfada (React) çizilmeyenler buradan geçer; sayfa metinleri
//! `src/lib/i18n.ts` + `src/locales/en.ts`'te.
//!
//! `Config.language`: "auto" (Windows'un görüntüleme dili Türkçe ise Türkçe,
//! değilse İngilizce) | "tr" | "en". Çevirisi olmayan metin Türkçe kalır.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Globalization::GetUserDefaultUILanguage;

static ENGLISH: AtomicBool = AtomicBool::new(false);

/// `LANG_TURKISH` (birincil dil kimliği, LANGID'in alt 10 biti).
const LANG_TURKISH: u16 = 0x1F;

/// Ayardaki dili uygular. Dönüş: dil değişti mi.
pub fn set(language: &str) -> bool {
    let english = match language {
        "tr" => false,
        "en" => true,
        _ => system_primary_language() != LANG_TURKISH,
    };
    ENGLISH.swap(english, Ordering::SeqCst) != english
}

/// Windows'un görüntüleme dili (birincil dil kimliği).
fn system_primary_language() -> u16 {
    // SAFETY: parametresiz sistem çağrısı.
    unsafe { GetUserDefaultUILanguage() & 0x3FF }
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::SeqCst)
}

/// Uygulamanın görünen adı (her dilde aynı).
pub fn app_name() -> &'static str {
    "Dockora"
}

/// Türkçe kaynak metin → seçili dildeki karşılığı.
pub fn t(tr: &'static str) -> &'static str {
    if english() { en(tr) } else { tr }
}

fn en(tr: &'static str) -> &'static str {
    match tr {
        // Tepsi menüsü
        "Ayarlar…" => "Settings…",
        "Windows görev çubuğunu gizle" => "Hide the Windows taskbar",
        "Açılışta başlat" => "Start at login",
        "Açılışta başlat (yalnızca kurulu sürüm)" => {
            "Start at login (installed version only)"
        }
        "Çıkış" => "Quit",
        // Uygulama simgesine sağ tık
        "Dock'tan kaldır" => "Remove from Dock",
        "Dock'a sabitle" => "Pin to Dock",
        "Pencereyi kapat" => "Close window",
        "Sık kullanılanlar" => "Frequent",
        "Son kullanılanlar" => "Recent",
        "Görevler" => "Tasks",
        // Bildirimler
        "Zamanlayıcı" => "Timer",
        "Süre doldu" => "Time's up",
        "Bildirimler böyle görünür" => "Notifications look like this",
        "Uygulamaların bildirimleri bu kartlarla gelir. Tıklayınca uygulama açılır." => {
            "App notifications arrive as cards like this. Click one to open the app."
        }
        // Diğer
        "Ayarlar" => "Settings",
        "Ses aygıtı" => "Audio device",
        "Sistem sesleri" => "System sounds",
        _ => tr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_language() {
        set("en");
        assert_eq!(t("Çıkış"), "Quit");
        assert_eq!(app_name(), "Dockora");
        assert!(!set("en"), "aynı dil: değişmedi");
        assert!(set("tr"), "dil değişti");
        assert_eq!(t("Çıkış"), "Çıkış");
        assert_eq!(app_name(), "Dockora");
        // Çevirisi olmayan metin olduğu gibi döner.
        set("en");
        assert_eq!(t("Başka bir şey"), "Başka bir şey");
        set("tr");
    }
}
