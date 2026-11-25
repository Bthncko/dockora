use std::sync::mpsc;
use std::time::Duration;

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::config::{Config, ConfigStore};
use crate::services::apps::{AppItem, AppsService};
use crate::services::audio::{AudioCmd, AudioService, AudioState};
use crate::services::backdrop::{Backdrop, BackdropService};
use crate::services::media::{self, MediaCmd, MediaService, MediaState};
use crate::services::power::{PowerService, PowerState};
use crate::services::radios::{RadiosService, RadiosState, SetRadio};
use crate::services::spotify::{
    Device, Player, Playlist, Queue, QueueItem, Slot, Spotify, SpotifyStatus,
};
use crate::services::sysmon::SysState;
use crate::services::updater::{self, UpdateInfo, Updater};
use crate::services::{battery_settings, ics, launcher, wireless};
use crate::win::{placement, popup, preview};

type CmdResult<T = ()> = Result<T, String>;

// --- pencere ---------------------------------------------------------------

/// Fare dock'a girince görünür bölgeyi büyütme payına açar, çıkınca bara daraltır (3.2).
#[tauri::command]
pub fn set_dock_expanded(window: WebviewWindow, expanded: bool) -> CmdResult {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    placement::set_expanded(hwnd, expanded);
    Ok(())
}

/// Windows'un kendi ayar sayfaları (radyo değiştirilemezse yedek yol, bölüm 6.7).
#[tauri::command]
pub fn open_system_settings(page: String) -> CmdResult {
    let uri = match page.as_str() {
        "wifi" => "ms-settings:network-wifi",
        "ethernet" => "ms-settings:network-ethernet",
        "bluetooth" => "ms-settings:bluetooth",
        "sound" => "ms-settings:sound",
        "sound-mixer" => "ms-settings:apps-volume",
        "battery" => "ms-settings:batterysaver",
        "power" => "ms-settings:powersleep",
        "nightlight" => "ms-settings:nightlight",
        // Dış bağlantılar da yalnızca bilinen adreslerle (frontend keyfi URL açtıramaz).
        "spotify-dashboard" => "https://developer.spotify.com/dashboard",
        _ => return Err(format!("bilinmeyen sayfa: {page}")),
    };
    launcher::open_uri(uri).map_err(Into::into)
}

/// Windows görev çubuğundaki gibi sistem panelini açar.
/// Hızlı ayarlar adresi bu makinede görev çubuğu gizliyken denendi, sağ altta açılıyor.
/// `ms-availablenetworks:` Microsoft'un ms-settings listesinde Wi-Fi ağları paneli olarak geçiyor.
/// `.../bluetooth` belgelenmemiş (kullanıcı paylaşımlarından); hızlı ayarları Bluetooth
/// listesinde açar. Kullanıcı denedi diye işaretlenene kadar şüpheli say.
#[tauri::command]
pub fn open_quick_settings(panel: String) -> CmdResult {
    let uri = match panel.as_str() {
        "wifi" => "ms-availablenetworks:",
        "bluetooth" => "ms-actioncenter:controlcenter/bluetooth",
        _ => "ms-actioncenter:controlcenter/&showFooter=true",
    };
    launcher::open_uri(uri).map_err(Into::into)
}

// --- popup (bölüm 3.4) ------------------------------------------------------

/// Dock'taki widget tıklaması: paneli aç / kapat / değiştir.
#[tauri::command]
pub fn show_popup(app: AppHandle, widget_id: String, anchor: popup::Anchor) -> CmdResult {
    popup::toggle(&app, widget_id, anchor).map_err(Into::into)
}

/// Popup sayfası paneli çizdi; şimdi konumlanıp gösterilebilir.
#[tauri::command]
pub fn popup_ready(
    app: AppHandle,
    seq: u64,
    width: f64,
    height: f64,
) -> CmdResult<Option<popup::Placed>> {
    popup::ready(&app, seq, width, height).map_err(Into::into)
}

/// Kapanış animasyonu bitti. `seq` verilirse yalnızca o açılışı kapatır.
#[tauri::command]
pub fn hide_popup(app: AppHandle, seq: Option<u64>) -> CmdResult {
    popup::hide(&app, seq).map_err(Into::into)
}

/// Windows vurgu rengi ("#rrggbb"); tema `accent: "auto"` iken kullanılır (7.9).
#[tauri::command]
pub fn get_system_accent(accent: State<crate::services::accent::AccentService>) -> Option<String> {
    accent.get()
}

// --- cam çubuk (7.2) ---------------------------------------------------------

#[tauri::command]
pub fn get_backdrop(backdrop: State<BackdropService>) -> Option<Backdrop> {
    backdrop.get()
}

// --- ayarlar ----------------------------------------------------------------

/// Pencere oluşturma Windows'ta eşzamanlı komutta kilitlenir → async.
#[tauri::command]
pub async fn open_settings(app: AppHandle, page: Option<String>) -> CmdResult {
    crate::win::settings::open_page(&app, page).map_err(Into::into)
}

/// Ayarlar sayfası yeni açıldıysa hangi bölümle başlayacağı (bir kez).
#[tauri::command]
pub fn take_settings_page() -> Option<String> {
    crate::win::settings::take_page()
}

#[tauri::command]
pub fn show_dock_menu(window: WebviewWindow) -> CmdResult {
    crate::show_dock_menu(&window).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct AutostartState {
    enabled: bool,
    /// Geliştirme sürümünde kapalı (Vite sunucusu olmadan açılamaz).
    available: bool,
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> AutostartState {
    AutostartState {
        enabled: crate::autostart_enabled(&app),
        available: crate::AUTOSTART_AVAILABLE,
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<AutostartState> {
    crate::set_autostart(&app, enabled)?;
    Ok(get_autostart(app))
}

// --- takvim -----------------------------------------------------------------

/// ICS aboneliğini indirir (metin olarak). Ağ beklemesi ana iş parçacığında olmasın.
#[tauri::command]
pub async fn fetch_ics(url: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || ics::fetch(&url))
        .await
        .map_err(|e| e.to_string())?
        .map_err(Into::into)
}

// --- Spotify (müzik paneli) -------------------------------------------------------

/// Arka planda: anahtar yenilenirken (ağ isteği) durum kilidi tutulur; ana
/// iş parçacığı beklerse dock donardı.
#[tauri::command]
pub async fn spotify_status(app: AppHandle) -> CmdResult<SpotifyStatus> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Spotify>().status())
        .await
        .map_err(|e| e.to_string())
}

/// Tarayıcıda onay ister; kullanıcı onaylayana kadar (en çok 3 dk) bekler.
#[tauri::command]
pub async fn spotify_connect(app: AppHandle, client_id: String) -> CmdResult<SpotifyStatus> {
    blocking(move || app.state::<Spotify>().connect(&app, &client_id)).await
}

#[tauri::command]
pub async fn spotify_disconnect(app: AppHandle) -> CmdResult<SpotifyStatus> {
    blocking(move || app.state::<Spotify>().disconnect(&app)).await
}

#[tauri::command]
pub async fn spotify_queue(app: AppHandle) -> CmdResult<Queue> {
    blocking(move || app.state::<Spotify>().queue(&app)).await
}

/// Sıradakiler listesindeki öğeyi doğrudan çal (`position`: Spotify sırasındaki yeri).
#[tauri::command]
pub async fn spotify_play_item(app: AppHandle, position: u32, uri: String) -> CmdResult {
    blocking(move || app.state::<Spotify>().play_item(&app, position, &uri)).await
}

/// Sıradan kaldır: listede gizlenir, çalmaya başlarsa geçilir.
#[tauri::command]
pub async fn spotify_queue_remove(
    app: AppHandle,
    uri: String,
    occurrence: u32,
    title: String,
) -> CmdResult {
    blocking(move || {
        app.state::<Spotify>()
            .queue_remove(&app, &uri, occurrence, &title)
    })
    .await
}

/// Sürükle-bırak / "sıradaki yap": sıranın başının istenen hâli.
#[tauri::command]
pub async fn spotify_queue_reorder(app: AppHandle, order: Vec<Slot>) -> CmdResult {
    blocking(move || app.state::<Spotify>().queue_reorder(&app, &order)).await
}

/// Panelden kaldırılanları geri getir.
#[tauri::command]
pub fn spotify_queue_reset(spotify: State<Spotify>) {
    spotify.queue_reset();
}

#[tauri::command]
pub async fn spotify_player(app: AppHandle) -> CmdResult<Option<Player>> {
    blocking(move || app.state::<Spotify>().player(&app)).await
}

#[tauri::command]
pub async fn spotify_set_volume(app: AppHandle, percent: u8) -> CmdResult<u8> {
    blocking(move || app.state::<Spotify>().set_volume(&app, percent)).await
}

/// Müzik karosunda tekerlek: yeni ses yüzdesi döner.
#[tauri::command]
pub async fn spotify_volume_step(app: AppHandle, delta: i32) -> CmdResult<u8> {
    blocking(move || app.state::<Spotify>().volume_step(&app, delta)).await
}

#[tauri::command]
pub async fn spotify_toggle_mute(app: AppHandle, current: u8) -> CmdResult<u8> {
    blocking(move || app.state::<Spotify>().toggle_mute(&app, current)).await
}

#[tauri::command]
pub async fn spotify_seek(app: AppHandle, position_ms: u64) -> CmdResult {
    blocking(move || app.state::<Spotify>().seek(&app, position_ms)).await
}

#[tauri::command]
pub async fn spotify_devices(app: AppHandle) -> CmdResult<Vec<Device>> {
    blocking(move || app.state::<Spotify>().devices(&app)).await
}

#[tauri::command]
pub async fn spotify_transfer(app: AppHandle, device_id: String) -> CmdResult {
    blocking(move || app.state::<Spotify>().transfer(&app, &device_id)).await
}

#[tauri::command]
pub async fn spotify_search(app: AppHandle, query: String) -> CmdResult<Vec<QueueItem>> {
    blocking(move || app.state::<Spotify>().search(&app, &query)).await
}

#[tauri::command]
pub async fn spotify_add_to_queue(app: AppHandle, uri: String) -> CmdResult {
    blocking(move || app.state::<Spotify>().add_to_queue(&app, &uri)).await
}

#[tauri::command]
pub async fn spotify_play_uri(app: AppHandle, uri: String) -> CmdResult {
    blocking(move || app.state::<Spotify>().play_uri(&app, &uri)).await
}

#[tauri::command]
pub async fn spotify_play_context(app: AppHandle, uri: String) -> CmdResult {
    blocking(move || app.state::<Spotify>().play_context(&app, &uri)).await
}

#[tauri::command]
pub async fn spotify_set_liked(app: AppHandle, uri: String, liked: bool) -> CmdResult {
    blocking(move || app.state::<Spotify>().set_liked(&app, &uri, liked)).await
}

#[tauri::command]
pub async fn spotify_playlists(app: AppHandle) -> CmdResult<Vec<Playlist>> {
    blocking(move || app.state::<Spotify>().playlists(&app)).await
}

/// Ağ beklemesi ana iş parçacığında olmasın.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> crate::error::AppResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(Into::into)
}

// --- güncelleme ----------------------------------------------------------------------

#[tauri::command]
pub fn get_update_info(app: AppHandle, updater: State<Updater>) -> UpdateInfo {
    updater.info(&app)
}

/// "Şimdi denetle": sonuç `update://status` olaylarıyla gelir.
#[tauri::command]
pub fn check_update(app: AppHandle) {
    std::thread::spawn(move || updater::check_now(&app));
}

// --- config ----------------------------------------------------------------

#[tauri::command]
pub fn get_config(config: State<ConfigStore>) -> Config {
    config.get()
}

#[tauri::command]
pub fn set_config(app: AppHandle, config: State<ConfigStore>, value: Config) -> CmdResult {
    let before = config.get();
    let hide_after = value.hide_windows_taskbar;
    let fx_after = value.minimize_to_dock;
    let notify_after = value.own_notifications;
    let language_after = value.language.clone();
    let auto_hide_after = value.auto_hide.clone();
    let scale_after = value.theme.ui_scale();
    config.update(|c| *c = value).map_err(String::from)?;
    if before.theme.ui_scale() != scale_after {
        crate::win::placement::set_ui_scale(scale_after);
        if let Some(dock) = app.get_webview_window("dock")
            && let Ok(hwnd) = dock.hwnd()
        {
            crate::win::appbar::post_reposition(hwnd);
        }
    }
    if before.hide_windows_taskbar != hide_after {
        crate::apply_hide_taskbar(&app, hide_after);
    }
    if before.minimize_to_dock != fx_after {
        crate::win::window_fx::set_enabled(fx_after);
    }
    if before.language != language_after && crate::i18n::set(&language_after) {
        crate::apply_language(&app);
    }
    if before.own_notifications != notify_after {
        crate::services::notifications::set_enabled(notify_after);
    }
    if before.auto_hide != auto_hide_after {
        crate::win::autohide::set_mode(crate::win::autohide::Mode::parse(&auto_hide_after));
        // Şerit ayırma değişti: yerleşimi yenile (pencereler alanı alır / bırakır).
        if let Some(dock) = app.get_webview_window("dock")
            && let Ok(hwnd) = dock.hwnd()
        {
            crate::win::appbar::post_reposition(hwnd);
        }
    }
    Ok(())
}

/// Otomatik gizlenme (8.10): imleç dock penceresine girdi / çıktı.
#[tauri::command]
pub fn dock_pointer(inside: bool) {
    crate::win::autohide::pointer(inside);
}

/// Uygulama ikonlarının dinlenme konumları (dock CSS pikseli); küçültme animasyonu hedefi.
#[tauri::command]
pub fn set_app_icon_rects(rects: Vec<crate::win::window_fx::IconRect>) {
    crate::win::window_fx::set_icon_rects(rects);
}

// --- uygulamalar -----------------------------------------------------------

#[tauri::command]
pub fn get_apps(apps: State<AppsService>) -> Vec<AppItem> {
    apps.items()
}

#[tauri::command]
pub fn activate_app(apps: State<AppsService>, id: String) -> CmdResult {
    apps.activate(&id).map_err(Into::into)
}

#[tauri::command]
pub fn launch_app(apps: State<AppsService>, id: String) -> CmdResult {
    apps.launch(&id).map_err(Into::into)
}

// --- pencere önizlemesi (8.3) ------------------------------------------------

#[tauri::command]
pub fn preview_enter(app: AppHandle, app_id: String, anchor: preview::Anchor) {
    preview::enter(&app, app_id, anchor);
}

#[tauri::command]
pub fn preview_leave(app: AppHandle) {
    preview::leave(&app);
}

#[tauri::command]
pub fn preview_hover(app: AppHandle, inside: bool) {
    preview::hover(&app, inside);
}

#[tauri::command]
pub fn preview_sizes(
    app: AppHandle,
    seq: u64,
    hwnds: Vec<isize>,
) -> CmdResult<Vec<preview::SourceSize>> {
    preview::sizes(&app, seq, hwnds).map_err(Into::into)
}

#[tauri::command]
pub fn preview_layout(
    app: AppHandle,
    seq: u64,
    width: f64,
    height: f64,
    thumbs: Vec<preview::ThumbRect>,
    clip: preview::Clip,
) -> CmdResult {
    preview::layout(&app, seq, width, height, thumbs, clip).map_err(Into::into)
}

#[tauri::command]
pub fn hide_preview(app: AppHandle) {
    preview::hide(&app);
    if cfg!(debug_assertions) {
        eprintln!(
            "[preview] kapandı, kayıtlı küçük resim: {}",
            preview::thumb_count()
        );
    }
}

/// Önizlemedeki karta tıklama: o pencere öne gelir.
#[tauri::command]
pub fn activate_window(app: AppHandle, apps: State<AppsService>, hwnd: isize) {
    preview::hide(&app);
    crate::win::focus::activate(hwnd);
    apps.refresh_soon();
}

#[tauri::command]
pub fn close_window(apps: State<AppsService>, hwnd: isize) {
    apps.close_window(hwnd);
}

#[tauri::command]
pub fn pin_app(apps: State<AppsService>, config: State<ConfigStore>, id: String) -> CmdResult {
    apps.pin(&config, &id).map_err(Into::into)
}

#[tauri::command]
pub fn unpin_app(apps: State<AppsService>, config: State<ConfigStore>, id: String) -> CmdResult {
    apps.unpin(&config, &id).map_err(Into::into)
}

/// Dosyalar uygulama ikonuna bırakıldı (8.7). COM ve ShellExecute ana iş
/// parçacığını bekletmesin diye arka planda.
#[tauri::command]
pub async fn open_files_with_app(app: AppHandle, id: String, paths: Vec<String>) -> CmdResult {
    tauri::async_runtime::spawn_blocking(move || app.state::<AppsService>().open_files(&id, &paths))
        .await
        .map_err(|e| e.to_string())?
        .map_err(Into::into)
}

/// `.exe` / `.lnk` boş alana bırakıldı: sabitle (8.7).
#[tauri::command]
pub async fn pin_path(app: AppHandle, path: String) -> CmdResult {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppsService>()
            .pin_path(&app.state::<ConfigStore>(), &path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(Into::into)
}

/// Sabit ikonların yeni sırası (sürükleyerek sıralama, 8.7).
#[tauri::command]
pub fn reorder_pinned(
    apps: State<AppsService>,
    config: State<ConfigStore>,
    ids: Vec<String>,
) -> CmdResult {
    apps.reorder_pinned(&config, &ids).map_err(Into::into)
}

#[tauri::command]
pub fn show_app_menu(window: WebviewWindow, apps: State<AppsService>, id: String) -> CmdResult {
    apps.show_menu(&window, &id).map_err(Into::into)
}

#[tauri::command]
pub fn open_start() -> CmdResult {
    launcher::open_start().map_err(Into::into)
}

// --- sistem izleyici (8.4) ---------------------------------------------------

#[tauri::command]
pub fn get_sysmon(sysmon: State<crate::services::sysmon::SysmonService>) -> Option<SysState> {
    sysmon.get()
}

/// Panel açıldı / kapandı: açıkken her saniye, diskler ve süreçlerle.
#[tauri::command]
pub fn sysmon_detail(sysmon: State<crate::services::sysmon::SysmonService>, on: bool) {
    sysmon.send(crate::services::sysmon::Detail(on));
}

#[tauri::command]
pub fn open_task_manager() -> CmdResult {
    launcher::shell_open("taskmgr.exe", "", "").map_err(Into::into)
}

/// Tepsi düğmesi (8.6): Windows'un taşma penceresini açar, düğmenin üstüne taşır.
#[tauri::command]
pub async fn open_tray(app: AppHandle, anchor: Option<crate::win::tray::Anchor>) -> CmdResult {
    tauri::async_runtime::spawn_blocking(move || crate::win::tray::open(&app, anchor))
        .await
        .map_err(|e| e.to_string())?
        .map_err(Into::into)
}

/// Sistem düğmeleri (8.2). Değiştirici tuş bekleyebileceği için ana iş
/// parçacığında değil.
#[tauri::command]
pub async fn system_shortcut(action: String) -> CmdResult {
    tauri::async_runtime::spawn_blocking(move || launcher::system_shortcut(&action))
        .await
        .map_err(|e| e.to_string())?
        .map_err(Into::into)
}

// --- müzik -----------------------------------------------------------------

#[tauri::command]
pub fn get_media(media: State<MediaService>) -> Option<MediaState> {
    media.get()
}

#[tauri::command]
pub fn media_toggle(media: State<MediaService>) {
    media.send(MediaCmd::Toggle);
}

#[tauri::command]
pub fn media_next(media: State<MediaService>) {
    media.send(MediaCmd::Next);
}

#[tauri::command]
pub fn media_prev(media: State<MediaService>) {
    media.send(MediaCmd::Prev);
}

#[tauri::command]
pub fn media_set_shuffle(media: State<MediaService>, on: bool) {
    media.send(MediaCmd::Shuffle(on));
}

/// `mode`: "off" | "track" | "list".
#[tauri::command]
pub fn media_set_repeat(media: State<MediaService>, mode: String) -> CmdResult {
    use windows::Media::MediaPlaybackAutoRepeatMode as M;
    let mode = match mode.as_str() {
        "off" => M::None,
        "track" => M::Track,
        "list" => M::List,
        _ => return Err(format!("bilinmeyen tekrar kipi: {mode}")),
    };
    media.send(MediaCmd::Repeat(mode));
    Ok(())
}

// --- ses -------------------------------------------------------------------

#[tauri::command]
pub fn get_audio(audio: State<AudioService>) -> Option<AudioState> {
    audio.get()
}

/// `v`: 0–1.
#[tauri::command]
pub fn set_volume(audio: State<AudioService>, v: f32) {
    audio.send(AudioCmd::Volume(v));
}

#[tauri::command]
pub fn set_mute(audio: State<AudioService>, muted: bool) {
    audio.send(AudioCmd::Mute(muted));
}

#[tauri::command]
pub fn set_mic_volume(audio: State<AudioService>, v: f32) {
    audio.send(AudioCmd::MicVolume(v));
}

#[tauri::command]
pub fn set_mic_mute(audio: State<AudioService>, muted: bool) {
    audio.send(AudioCmd::MicMute(muted));
}

/// Çıkış aygıtını değiştir (8.5, belgelenmemiş API). Hata dönerse panel
/// Windows'un ses ayarlarını açar.
#[tauri::command]
pub async fn set_default_output(audio: State<'_, AudioService>, id: String) -> CmdResult {
    let (tx, rx) = mpsc::channel();
    audio.send(AudioCmd::DefaultOutput(id, tx));
    tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(5)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "ses servisi yanıt vermedi".to_string())?
}

// --- pil -------------------------------------------------------------------

#[tauri::command]
pub fn get_power(power: State<PowerService>) -> Option<PowerState> {
    power.get()
}

// --- radyolar --------------------------------------------------------------

// --- Wi-Fi / Bluetooth panelleri ---------------------------------------------

/// Hata metni doğrudan panele gider (ör. "password_required").
async fn blocking_str<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn wifi_scan(fresh: Option<bool>) -> CmdResult<Vec<wireless::WifiNetwork>> {
    let fresh = fresh.unwrap_or(true);
    blocking_str(move || wireless::scan(fresh)).await
}

/// `password` yoksa ve ağ şifre isterse hata "password_required".
#[tauri::command]
pub async fn wifi_connect(
    radios: State<'_, RadiosService>,
    ssid: String,
    password: Option<String>,
) -> CmdResult {
    let result = blocking_str(move || wireless::connect(&ssid, password.as_deref())).await;
    radios.wake();
    result
}

/// Kurumsal (802.1X) ağ: kullanıcı adı + parola. Hata "cert_trust" ise sunucu
/// sertifikası Windows'ta onaylanmalı. Parola günlüğe yazılmaz.
#[tauri::command]
pub async fn wifi_connect_enterprise(
    radios: State<'_, RadiosService>,
    ssid: String,
    username: String,
    password: String,
    auto: bool,
) -> CmdResult {
    let result =
        blocking_str(move || wireless::connect_enterprise(&ssid, &username, &password, auto)).await;
    radios.wake();
    result
}

#[tauri::command]
pub async fn wifi_disconnect(radios: State<'_, RadiosService>) -> CmdResult {
    let result = blocking_str(wireless::disconnect).await;
    radios.wake();
    result
}

#[tauri::command]
pub async fn bt_devices() -> CmdResult<Vec<wireless::BtDevice>> {
    blocking_str(wireless::bt_devices).await
}

/// Eşleşmiş Bluetooth aygıtını bağlar / ayırır. Hata "use_settings" ise panel
/// Windows ayarlarını açar. Bitince radyo servisi tazelenir.
#[tauri::command]
pub async fn bt_set_connected(
    radios: State<'_, RadiosService>,
    id: String,
    le: bool,
    connect: bool,
) -> CmdResult {
    let result = blocking_str(move || wireless::bt_set_connected(&id, le, connect)).await;
    radios.wake();
    result
}

/// Güç modu (pil paneli): değiştir ve pil servisini hemen okut.
#[tauri::command]
pub fn set_power_mode(power: State<PowerService>, mode: String) -> CmdResult {
    crate::services::power_mode::set(&mode)?;
    power.wake();
    Ok(())
}

/// Pil panelindeki ayarlar (parlaklık, yenileme hızı, süreler). WMI yavaş olabilir.
#[tauri::command]
pub async fn get_battery_settings() -> CmdResult<battery_settings::BatterySettings> {
    blocking_str(|| Ok(battery_settings::read())).await
}

/// Enerji tasarrufu (pildeyken tek tık): aç / kapat.
#[tauri::command]
pub async fn set_energy_saver(power: State<'_, PowerService>, on: bool) -> CmdResult {
    let percent = power.get().map_or(100, |p| p.percent);
    let result = blocking_str(move || battery_settings::set_saver(on, percent)).await;
    power.wake();
    result
}

#[tauri::command]
pub async fn set_brightness(percent: u8) -> CmdResult {
    crate::services::flyouts::mark_own_brightness();
    blocking_str(move || battery_settings::set_brightness(percent)).await
}

#[tauri::command]
pub async fn set_refresh_rate(hz: u32) -> CmdResult {
    blocking_str(move || battery_settings::set_refresh(hz)).await
}

/// `kind`: "screen" ya da "sleep"; süre saniye (0 = hiçbir zaman), pildeyken.
#[tauri::command]
pub async fn set_power_timeout(kind: String, seconds: u32) -> CmdResult {
    blocking_str(move || battery_settings::set_timeout(&kind, seconds)).await
}

/// Ayarlar → "Dene": örnek bildirim kartı (seçili sesle; `silent` ise sessiz).
#[tauri::command]
pub fn notify_test(app: AppHandle, silent: bool) {
    use crate::i18n::{app_name, t};
    use crate::services::notifications;
    let (name, title, body) = (
        app_name(),
        t("Bildirimler böyle görünür"),
        t("Uygulamaların bildirimleri bu kartlarla gelir. Tıklayınca uygulama açılır."),
    );
    if silent {
        notifications::show_card(&app, name, title, body);
    } else {
        notifications::show_local(&app, name, title, body);
    }
}

/// Bildirim kartları penceresi: yığının boyu değişti (0 → gizle).
#[tauri::command]
pub fn notify_layout(app: AppHandle, width: f64, height: f64) -> CmdResult {
    crate::win::notify::layout(&app, width, height)?;
    Ok(())
}

/// Bildirim kartına tıklandı: gönderen uygulamayı öne getir / aç.
#[tauri::command]
pub fn notify_activate(apps: State<AppsService>, id: u32) -> CmdResult {
    match crate::services::notifications::take_sender(id) {
        Some(aumid) => apps.bring_aumid(&aumid).map_err(Into::into),
        None => Ok(()),
    }
}

// --- küçük widget'lar (8.8) --------------------------------------------------------

#[tauri::command]
pub fn timer_get() -> crate::services::timer::TimerState {
    crate::services::timer::get()
}

/// `action`: "start" (`seconds` ile), "pause", "resume", "reset".
#[tauri::command]
pub fn timer_control(action: String, seconds: Option<u32>) -> CmdResult {
    use crate::services::timer;
    match action.as_str() {
        "start" => timer::start(seconds.unwrap_or(25 * 60)),
        "pause" => timer::pause(),
        "resume" => timer::resume(),
        "reset" => timer::reset(),
        _ => return Err(format!("bilinmeyen zamanlayıcı eylemi: {action}")),
    }
    Ok(())
}

#[tauri::command]
pub fn get_recycle(
    recycle: State<crate::services::recycle::RecycleService>,
) -> Option<crate::services::recycle::RecycleState> {
    recycle.get()
}

#[tauri::command]
pub fn open_recycle_bin() -> CmdResult {
    crate::services::recycle::open()?;
    Ok(())
}

/// Windows'un onay penceresi açılır; arka planda beklenir.
#[tauri::command]
pub async fn empty_recycle_bin(app: AppHandle) -> CmdResult {
    tauri::async_runtime::spawn_blocking(crate::services::recycle::empty)
        .await
        .map_err(|e| e.to_string())?;
    app.state::<crate::services::recycle::RecycleService>()
        .wake();
    Ok(())
}

/// Dock'taki çöp kutusuna bırakılan dosyalar (kutuya gidince sayaç yenilenir).
#[tauri::command]
pub async fn recycle_files(app: AppHandle, paths: Vec<String>) -> CmdResult<usize> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::services::recycle::recycle(paths).map_err(String::from)
    })
    .await
    .map_err(|e| e.to_string())?;
    app.state::<crate::services::recycle::RecycleService>()
        .wake();
    result
}

#[tauri::command]
pub fn get_keyboard(
    keyboard: State<crate::services::keyboard::KeyboardService>,
) -> Option<crate::services::keyboard::KeyboardState> {
    keyboard.get()
}

/// Parlaklık widget'ı (8.8): dizüstü ekranı (WMI); yoksa `None`.
#[tauri::command]
pub async fn get_brightness() -> CmdResult<Option<u8>> {
    blocking_str(|| Ok(battery_settings::brightness().ok())).await
}

/// Kendi tepsimizdeki simgeler (`win::tray_host`).
#[tauri::command]
pub fn get_tray() -> crate::win::tray_host::TrayState {
    crate::win::tray_host::state()
}

/// `button`: "left", "double" ya da "right".
#[tauri::command]
pub fn tray_click(id: String, button: String) -> CmdResult {
    crate::win::tray_host::click(&id, &button)?;
    Ok(())
}

#[tauri::command]
pub fn get_radios(radios: State<RadiosService>) -> RadiosState {
    radios.get()
}

/// `kind`: "wifi" ya da "bluetooth". Radyo değişimi bir saniye sürebilir;
/// ana iş parçacığını bekletmemek için async.
#[tauri::command]
pub async fn set_radio(radios: State<'_, RadiosService>, kind: String, on: bool) -> CmdResult {
    let wifi = match kind.as_str() {
        "wifi" => true,
        "bluetooth" => false,
        _ => return Err(format!("bilinmeyen radyo: {kind}")),
    };
    let (reply, result) = mpsc::channel();
    radios.send(SetRadio { wifi, on, reply });
    result
        .recv_timeout(Duration::from_secs(8))
        .map_err(|_| "radyo yanıt vermedi".to_string())?
}

// --- AI kullanımı (8.9) ---------------------------------------------------------

#[tauri::command]
pub fn get_ai_usage(
    usage: State<crate::services::ai_usage::AiUsageService>,
) -> Option<crate::services::ai_usage::AiUsage> {
    usage.get()
}

/// Panel açıldı: beklemeden yeniden oku (sonuç `ai-usage://changed` ile).
#[tauri::command]
pub fn ai_usage_refresh(usage: State<crate::services::ai_usage::AiUsageService>) {
    usage.send(crate::services::ai_usage::Refresh);
}

// --- bildirim sesi ---------------------------------------------------------

/// Seçili bildirim sesi: kendi seslerimizden biri, "windows" ya da "custom".
#[tauri::command]
pub fn get_notification_sound() -> String {
    crate::services::notify_sound::current()
}

#[tauri::command]
pub fn set_notification_sound(id: String) -> CmdResult {
    crate::services::notify_sound::set(&id)
}

#[tauri::command]
pub fn preview_notification_sound(id: String) {
    crate::services::notify_sound::preview(&id);
}

// --- ekran üstü kartlar (flyout) ---------------------------------------------

/// Kart penceresi: sayfa içeriğinin boyu (CSS piksel). Konumlar ve gösterir; 0 → gizle.
#[tauri::command]
pub fn flyout_layout(app: AppHandle, width: f64, height: f64) -> CmdResult {
    crate::win::flyout::layout(&app, width, height)?;
    Ok(())
}

/// Kart sayfası `flyout:show` dinleyicisini kaydetti; bekleyen kart varsa gider.
#[tauri::command]
pub fn flyout_ready(app: AppHandle) {
    crate::win::flyout::ready(&app);
}

#[tauri::command]
pub fn flyout_hide(app: AppHandle) -> CmdResult {
    crate::win::flyout::hide(&app)?;
    Ok(())
}

/// İmleç kartın üstünde mi (otomatik gizleme sayfada).
#[tauri::command]
pub fn flyout_hold(hold: bool) {
    crate::win::flyout::hold(hold);
}

/// Ses karıştırıcı: varsayılan çıkıştaki uygulama oturumları.
#[tauri::command]
pub async fn audio_sessions() -> CmdResult<Vec<crate::services::audio_sessions::AudioSession>> {
    blocking_str(crate::services::audio_sessions::list).await
}

#[tauri::command]
pub async fn set_session_volume(id: String, volume: u8) -> CmdResult {
    blocking_str(move || crate::services::audio_sessions::set_volume(&id, volume)).await
}

#[tauri::command]
pub async fn set_session_mute(id: String, muted: bool) -> CmdResult {
    blocking_str(move || crate::services::audio_sessions::set_mute(&id, muted)).await
}

/// Geçerli medya oturumunun konumu / süresi (ms); oturum yoksa `null`.
#[tauri::command]
pub async fn media_timeline() -> CmdResult<Option<media::Timeline>> {
    blocking_str(|| Ok(media::timeline())).await
}

#[tauri::command]
pub async fn media_seek(position_ms: u64) -> CmdResult {
    blocking_str(move || media::seek(position_ms)).await
}
