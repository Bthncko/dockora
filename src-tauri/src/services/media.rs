//! Müzik: WinRT `Windows.Media.Control` (bölüm 6.2).
//!
//! Spotify, tarayıcılar ve çoğu oynatıcı bu arayüzle görünür.
//!
//! 7.8: yoklama yerine olay. Yönetici `CurrentSessionChanged` / `SessionsChanged`,
//! o an izlenen oturum `MediaPropertiesChanged` / `PlaybackInfoChanged` ile
//! servisi uyandırır; oturum değişince oturum olayları yenisine taşınır.
//! Seyrek yoklama yedektir (bazı oynatıcılar olay göndermeyebilir).

use std::cell::RefCell;
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Manager as _};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    CurrentSessionChangedEventArgs, GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    MediaPropertiesChangedEventArgs, PlaybackInfoChangedEventArgs, SessionsChangedEventArgs,
};
use windows::Media::MediaPlaybackAutoRepeatMode;
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::core::Interface;
use windows_future::IAsyncOperation;

use super::runner::{self, Waker};
use crate::config::ConfigStore;

pub const MEDIA_CHANGED: &str = "media://changed";
/// Yedek yoklama: olay göndermeyen oynatıcılar için.
const POLL: Duration = Duration::from_secs(5);
/// Kapak resmi bundan büyükse alma (bayt).
const MAX_ARTWORK: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaState {
    pub title: String,
    pub artist: String,
    pub artwork: Option<String>,
    pub playing: bool,
    pub can_prev: bool,
    pub can_next: bool,
    pub source_app: String,
    /// Karıştırma açık mı; oynatıcı desteklemiyorsa yok.
    pub shuffle: Option<bool>,
    /// "off" | "track" | "list"; oynatıcı desteklemiyorsa yok.
    pub repeat: Option<&'static str>,
}

pub enum MediaCmd {
    Toggle,
    Next,
    Prev,
    Shuffle(bool),
    Repeat(MediaPlaybackAutoRepeatMode),
}

pub type MediaService = runner::ServiceHandle<Option<MediaState>, MediaCmd>;

/// Olaylarına abone olunan oturum ve abonelik jetonları.
struct Watched {
    session: Session,
    props: i64,
    playback: i64,
}

impl Watched {
    fn subscribe(session: Session, waker: &Waker<MediaCmd>) -> Option<Self> {
        let w = waker.clone();
        let props = session
            .MediaPropertiesChanged(
                &TypedEventHandler::<Session, MediaPropertiesChangedEventArgs>::new(move |_, _| {
                    w.wake();
                    Ok(())
                }),
            )
            .ok()?;
        let w = waker.clone();
        let playback = session
            .PlaybackInfoChanged(
                &TypedEventHandler::<Session, PlaybackInfoChangedEventArgs>::new(move |_, _| {
                    w.wake();
                    Ok(())
                }),
            )
            .ok()?;
        Some(Self {
            session,
            props,
            playback,
        })
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.props);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
    }
}

/// Oturum yöneticisi ve olay jetonları; bırakılınca (servis durunca, 8.1) abonelikler silinir.
struct Opened {
    manager: Manager,
    current: Option<i64>,
    sessions: Option<i64>,
}

impl Drop for Opened {
    fn drop(&mut self) {
        if let Some(t) = self.current {
            let _ = self.manager.RemoveCurrentSessionChanged(t);
        }
        if let Some(t) = self.sessions {
            let _ = self.manager.RemoveSessionsChanged(t);
        }
    }
}

/// Yöneticiyi alır ve oturum listesi / geçerli oturum değişimlerine abone olur.
fn open_manager(waker: &Waker<MediaCmd>) -> Option<Opened> {
    let manager = Manager::RequestAsync().and_then(|op| op.join()).ok()?;
    let w = waker.clone();
    let current = manager
        .CurrentSessionChanged(
            &TypedEventHandler::<Manager, CurrentSessionChangedEventArgs>::new(move |_, _| {
                w.wake();
                Ok(())
            }),
        )
        .ok();
    let w = waker.clone();
    let sessions = manager
        .SessionsChanged(
            &TypedEventHandler::<Manager, SessionsChangedEventArgs>::new(move |_, _| {
                w.wake();
                Ok(())
            }),
        )
        .ok();
    Some(Opened {
        manager,
        current,
        sessions,
    })
}

pub fn spawn(app: &AppHandle) {
    let handle = app.clone();
    runner::spawn(
        app,
        "media",
        MEDIA_CHANGED,
        POLL,
        None,
        move |waker: Waker<MediaCmd>| {
            let app = handle.clone();
            let mut manager: Option<Opened> = None;
            let mut watched: Option<Watched> = None;
            // Kapak resmini parça değişmedikçe yeniden okuma.
            let mut artwork_key = String::new();
            let mut artwork: Option<String> = None;
            // Şarkı kendiliğinden mi değişti (aynı uygulama), kim çalıyor (diğerlerini duraklatma).
            let mut last_app = String::new();
            let mut playing_app: Option<String> = None;
            move |cmd: Option<MediaCmd>| {
                if manager.is_none() {
                    manager = open_manager(&waker);
                }
                let Some(session) = manager.as_ref()?.manager.GetCurrentSession().ok() else {
                    watched = None;
                    return None;
                };
                // Geçerli oturum değiştiyse olayları yenisine taşı (eskisi Drop ile bırakılır).
                if watched.as_ref().map(|w| w.session.as_raw()) != Some(session.as_raw()) {
                    watched = None;
                    watched = Watched::subscribe(session.clone(), &waker);
                }
                if let Some(cmd) = cmd {
                    run_command(&session, cmd);
                }
                let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
                let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
                if title.is_empty() {
                    return None;
                }
                let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
                let source_app = session
                    .SourceAppUserModelId()
                    .map(|s| s.to_string())
                    .unwrap_or_default();

                let key = format!("{source_app}\u{1}{title}\u{1}{artist}");
                let mut track_changed = false;
                if key != artwork_key {
                    track_changed = !artwork_key.is_empty() && last_app == source_app;
                    last_app = source_app.clone();
                    artwork = props.Thumbnail().ok().and_then(|t| read_artwork(&t));
                    artwork_key = key;
                    // Panelden sıradan kaldırılan şarkı başladıysa geç.
                    if source_app.to_lowercase().contains("spotify") {
                        super::spotify::Spotify::track_changed(&app, &title);
                    }
                }

                let playback = session.GetPlaybackInfo().ok();
                let playing = playback
                    .as_ref()
                    .and_then(|p| p.PlaybackStatus().ok())
                    .is_some_and(|s| s == PlaybackStatus::Playing);
                if track_changed && playing {
                    let album = props.AlbumTitle().ok().map(|s| s.to_string());
                    super::flyouts::track_changed(
                        &app,
                        &title,
                        &artist,
                        album,
                        artwork.clone(),
                        &source_app,
                    );
                }
                // Bir oturum çalmaya başladı: ayara göre diğerlerini duraklat.
                if playing {
                    if playing_app.as_deref() != Some(source_app.as_str()) {
                        playing_app = Some(source_app.clone());
                        let pause = app.try_state::<ConfigStore>().is_some_and(|c| {
                            let f = c.get().flyouts;
                            f.enabled && f.media.pause_others
                        });
                        if pause && let Some(m) = manager.as_ref() {
                            pause_others(&m.manager, &source_app);
                        }
                    }
                } else {
                    playing_app = None;
                }
                let controls = playback.as_ref().and_then(|p| p.Controls().ok());
                let can = |f: fn(&_) -> windows::core::Result<bool>| {
                    controls.as_ref().and_then(|c| f(c).ok()).unwrap_or(false)
                };

                // Karıştır / tekrarla: Spotify dahil çoğu oynatıcı bildiriyor
                // (bu makinede Spotify ile denendi). Kapalıysa düğme gösterilmez.
                let info = playback.as_ref();
                let shuffle = can(|c| c.IsShuffleEnabled())
                    .then(|| info.and_then(|p| p.IsShuffleActive().ok()?.Value().ok()))
                    .flatten();
                let repeat = can(|c| c.IsRepeatEnabled())
                    .then(|| info.and_then(|p| p.AutoRepeatMode().ok()?.Value().ok()))
                    .flatten()
                    .map(|m| match m {
                        MediaPlaybackAutoRepeatMode::Track => "track",
                        MediaPlaybackAutoRepeatMode::List => "list",
                        _ => "off",
                    });

                Some(MediaState {
                    title,
                    artist,
                    artwork: artwork.clone(),
                    playing,
                    can_prev: can(|c| c.IsPreviousEnabled()),
                    can_next: can(|c| c.IsNextEnabled()),
                    source_app,
                    shuffle,
                    repeat,
                })
            }
        },
    );
}

fn run_command(session: &Session, cmd: MediaCmd) {
    if matches!(cmd, MediaCmd::Next | MediaCmd::Prev) {
        super::flyouts::mark_user_skip();
    }
    let op = match cmd {
        MediaCmd::Toggle => session.TryTogglePlayPauseAsync(),
        MediaCmd::Next => session.TrySkipNextAsync(),
        MediaCmd::Prev => session.TrySkipPreviousAsync(),
        MediaCmd::Shuffle(on) => session.TryChangeShuffleActiveAsync(on),
        MediaCmd::Repeat(mode) => session.TryChangeAutoRepeatModeAsync(mode),
    };
    let _ = op.and_then(|op| op.join());
    // Oynatıcının durumunu güncellemesine kısa süre tanı.
    std::thread::sleep(Duration::from_millis(150));
}

/// Kapak akışını baytlara çevirip data URL yapar.
fn read_artwork(reference: &IRandomAccessStreamReference) -> Option<String> {
    let stream = reference.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_ARTWORK {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let load: IAsyncOperation<u32> = reader.LoadAsync(size as u32).ok()?.cast().ok()?;
    load.join().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    let content_type = stream
        .ContentType()
        .map(|s| s.to_string())
        .ok()
        .filter(|s| s.starts_with("image/"))
        .unwrap_or_else(|| "image/png".into());
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{content_type};base64,{b64}"))
}

/// Çalan diğer oturumları duraklatır (beklemeden).
fn pause_others(manager: &Manager, current_app: &str) {
    let Ok(sessions) = manager.GetSessions() else {
        return;
    };
    for other in sessions {
        let id = other
            .SourceAppUserModelId()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if id == current_app {
            continue;
        }
        if is_playing(&other) {
            let _ = other.TryPauseAsync();
        }
    }
}

pub fn is_playing(session: &Session) -> bool {
    session
        .GetPlaybackInfo()
        .and_then(|p| p.PlaybackStatus())
        .is_ok_and(|s| s == PlaybackStatus::Playing)
}

thread_local! {
    static MANAGER: RefCell<Option<Manager>> = const { RefCell::new(None) };
}

/// Çağıran iş parçacığında COM (MTA) başlatılmış olmalı; yönetici iş parçacığı
/// başına bir kez alınır. Geçerli oturum ve uygulama kimliği.
pub fn current_session() -> Option<(Session, String)> {
    let manager = MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        if m.is_none() {
            *m = Manager::RequestAsync().and_then(|op| op.join()).ok();
        }
        m.clone()
    })?;
    let session = manager.GetCurrentSession().ok()?;
    let id = session
        .SourceAppUserModelId()
        .map(|s| s.to_string())
        .unwrap_or_default();
    Some((session, id))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
}

/// 100 ns birimleri.
const TICKS_PER_MS: i64 = 10_000;
/// 1601-01-01 → 1970-01-01 (100 ns).
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

/// Geçerli oturumun konumu / süresi; oynatıcı son güncellemeden bu yana geçen
/// süreyi ekler (çoğu oynatıcı konumu yalnızca değişimde bildirir).
pub fn timeline() -> Option<Timeline> {
    super::audio_sessions::ensure_com();
    let (session, _) = current_session()?;
    let t = session.GetTimelineProperties().ok()?;
    let start = t.StartTime().ok()?.Duration;
    let end = t.EndTime().ok()?.Duration;
    let duration = (end - start).max(0);
    let mut position = t.Position().ok()?.Duration - start;
    let playing = is_playing(&session);
    if playing && let Ok(updated) = t.LastUpdatedTime() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_nanos() / 100) as i64)
            .unwrap_or(0)
            + UNIX_EPOCH_TICKS;
        let rate = session
            .GetPlaybackInfo()
            .ok()
            .and_then(|p| p.PlaybackRate().ok()?.Value().ok())
            .unwrap_or(1.0);
        let elapsed = (now - updated.UniversalTime).max(0);
        position += (elapsed as f64 * rate) as i64;
    }
    let position = position.clamp(0, duration);
    Some(Timeline {
        position_ms: (position / TICKS_PER_MS) as u64,
        duration_ms: (duration / TICKS_PER_MS) as u64,
        playing,
    })
}

/// Geçerli oturumda konuma atla (ms).
pub fn seek(position_ms: u64) -> Result<(), String> {
    super::audio_sessions::ensure_com();
    let (session, _) = current_session().ok_or("medya oturumu yok")?;
    let start = session
        .GetTimelineProperties()
        .ok()
        .and_then(|t| t.StartTime().ok())
        .map(|d| d.Duration)
        .unwrap_or(0);
    let ticks = start + (position_ms as i64).saturating_mul(TICKS_PER_MS);
    session
        .TryChangePlaybackPositionAsync(ticks)
        .and_then(|op| op.join())
        .map_err(|e| e.to_string())?;
    Ok(())
}
