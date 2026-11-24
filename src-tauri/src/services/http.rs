//! Küçük HTTP istemcisi (Spotify, AI kullanımı): WinRT `HttpClient`, ek
//! bağımlılık yok. Önbellek kapalı. Bekleyen bir iş parçacığında çağrılmalı.
//!
//! Zaman aşımı (`TIMEOUT`): WinRT `HttpClient`'ın kendi süresi yok; Windows
//! açılırken ağ hazır değilken gönderilen istek hiç bitmeyebiliyordu ve onu
//! bekleyen servis (AI kullanımı) bir daha güncellenmiyordu. Süre dolunca işlem
//! iptal edilir, hata döner, servis sonraki turda yeniden dener.

use std::time::{Duration, Instant};

use windows::Foundation::Uri;
use windows::Storage::Streams::UnicodeEncoding;
use windows::Web::Http::Filters::{
    HttpBaseProtocolFilter, HttpCacheReadBehavior, HttpCacheWriteBehavior,
};
use windows::Web::Http::{HttpClient, HttpMethod, HttpRequestMessage, HttpStringContent};
use windows::core::{Error, HSTRING, Interface};
use windows_future::{AsyncStatus, IAsyncInfo};

/// Bir isteğin tamamı (gönderme + gövdeyi okuma) en çok bu kadar sürer.
const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(25);
/// `ERROR_TIMEOUT` (0x800705B4).
const E_TIMEOUT: windows::core::HRESULT = windows::core::HRESULT(0x8007_05B4_u32 as i32);

pub struct Response {
    pub status: u16,
    pub body: String,
}

/// İstek gövdesi.
pub enum Body<'a> {
    Form(&'a str),
    Json(&'a str),
}

/// `headers`: ek başlıklar (ör. `Authorization`). Hata: bağlantı kurulamadı.
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<Body>,
) -> windows::core::Result<Response> {
    crate::win::com_mta();
    let filter = HttpBaseProtocolFilter::new()?;
    let cache = filter.CacheControl()?;
    cache.SetReadBehavior(HttpCacheReadBehavior::NoCache)?;
    cache.SetWriteBehavior(HttpCacheWriteBehavior::NoCache)?;
    let client = HttpClient::Create(&filter)?;
    let has_body = method != "GET";
    let method = match method {
        "POST" => HttpMethod::Post()?,
        "PUT" => HttpMethod::Put()?,
        "DELETE" => HttpMethod::Delete()?,
        _ => HttpMethod::Get()?,
    };
    let request = HttpRequestMessage::Create(&method, &Uri::CreateUri(&HSTRING::from(url))?)?;
    for (name, value) in headers {
        request
            .Headers()?
            .TryAppendWithoutValidation(&HSTRING::from(*name), &HSTRING::from(*value))?;
    }
    if let Some(body) = body {
        let (text, media_type) = match body {
            Body::Form(t) => (t, "application/x-www-form-urlencoded"),
            Body::Json(t) => (t, "application/json"),
        };
        let content = HttpStringContent::CreateFromStringWithEncodingAndMediaType(
            &HSTRING::from(text),
            UnicodeEncoding::Utf8,
            &HSTRING::from(media_type),
        )?;
        request.SetContent(&content)?;
    } else if has_body {
        // Gövdesiz POST/PUT: Spotify Content-Length ister (yoksa 411).
        request.SetContent(&HttpStringContent::CreateFromString(&HSTRING::new())?)?;
    }
    let deadline = Instant::now() + TIMEOUT;
    let send = client.SendRequestAsync(&request)?;
    wait(&send.cast()?, deadline)?;
    let response = send.GetResults()?;
    let status = response.StatusCode()?.0 as u16;
    let read = response.Content()?.ReadAsStringAsync()?;
    wait(&read.cast()?, deadline)?;
    let body = read.GetResults()?.to_string_lossy();
    Ok(Response { status, body })
}

/// İşlem bitene ya da süre dolana kadar bekler; süre dolarsa iptal eder.
fn wait(op: &IAsyncInfo, deadline: Instant) -> windows::core::Result<()> {
    loop {
        match op.Status()? {
            AsyncStatus::Started => {}
            AsyncStatus::Completed => return Ok(()),
            AsyncStatus::Error => return Err(Error::from_hresult(op.ErrorCode()?)),
            _ => return Err(Error::from_hresult(E_TIMEOUT)), // iptal edildi
        }
        if Instant::now() >= deadline {
            let _ = op.Cancel();
            return Err(Error::from_hresult(E_TIMEOUT));
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Yanıt vermeyen adres: süre dolunca hata döner, takılmaz. Ağ gerektirir.
    #[test]
    #[ignore]
    fn times_out() {
        let started = Instant::now();
        let res = request("GET", "http://10.255.255.1/", &[], None);
        assert!(res.is_err());
        assert!(started.elapsed() < TIMEOUT + Duration::from_secs(3));
    }

    /// Gerçek istek hâlâ çalışıyor. Ağ gerektirir.
    #[test]
    #[ignore]
    fn fetches() {
        let res = request("GET", "https://api.anthropic.com/", &[], None).unwrap();
        assert!(res.status > 0);
    }
}
