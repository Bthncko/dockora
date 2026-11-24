//! Takvim aboneliği (ICS) indirme (bölüm 6.4).
//!
//! Google Takvim'in ICS yanıtında CORS başlığı yok (denendi), bu yüzden dosya
//! frontend'den değil buradan indirilir. Ayrıştırma frontend'de (`ical.js`).
//! Ek bağımlılık olmasın diye WinRT `Windows.Web.Http.HttpClient`.
//!
//! Adresler gizlidir (bölüm 6.4): günlüğe yazılmaz.

use windows::Foundation::Uri;
use windows::Web::Http::Filters::{HttpBaseProtocolFilter, HttpCacheReadBehavior};
use windows::Web::Http::HttpClient;
use windows::core::HSTRING;

use crate::error::{AppError, AppResult};

/// Bekleyen bir iş parçacığında çağrılmalı (ağ isteği bitene kadar bekler).
pub fn fetch(url: &str) -> AppResult<String> {
    let url = normalize(url)?;
    crate::win::com_mta();
    let filter = HttpBaseProtocolFilter::new()?;
    // Önbellekteki eski kopya değil, sunucudaki güncel takvim.
    filter
        .CacheControl()?
        .SetReadBehavior(HttpCacheReadBehavior::MostRecent)?;
    let client = HttpClient::Create(&filter)?;
    let uri = Uri::CreateUri(&HSTRING::from(url))?;
    let text = client.GetStringAsync(&uri)?.join()?;
    Ok(text.to_string_lossy())
}

/// `webcal://` → `https://`; yalnızca http(s).
fn normalize(url: &str) -> AppResult<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("webcal://") {
        // Küçük harfe çevrilmiş kopyadan değil, özgün adresten (yol büyük/küçük harfe duyarlı).
        return Ok(format!("https://{}", &url[url.len() - rest.len()..]));
    }
    if lower.starts_with("https://") || lower.starts_with("http://") {
        Ok(url.to_string())
    } else {
        Err(AppError::InvalidInput("takvim adresi http(s) ya da webcal olmalı".into()))
    }
}
