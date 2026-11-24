//! Dockora — yalnızca Windows 11 için.

mod commands;
mod config;
mod error;
mod i18n;
mod services;
mod win;

use std::sync::mpsc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, RunEvent, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use config::ConfigStore;
use error::AppError;
use services::apps::{self, AppsService};

const MENU_SETTINGS: &str = "tray:settings";
const MENU_HIDE_TASKBAR: &str = "tray:hide-taskbar";
const MENU_AUTOSTART: &str = "tray:autostart";
const MENU_QUIT: &str = "tray:quit";

/// Tepsi menüsündeki onay kutuları (durumlarını güncelleyebilmek için).
struct TrayMenu {
    /// Dock'a sağ tıklayınca da aynı menü açılır: Windows çubuğu gizliyken
    /// tepsi simgesi de onunla birlikte gizlenir.
    menu: Menu<Wry>,
    settings: MenuItem<Wry>,
    hide_taskbar: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// Dock'un boş yerine sağ tık: tepsi menüsü imlecin olduğu yerde.
pub(crate) fn show_dock_menu(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let app = window.app_handle();
    if let Some(tray) = app.try_state::<TrayMenu>() {
        // Menü kapanana dek döner (TrackPopupMenu); bu sırada dock gizlenmesin.
        win::autohide::set_menu_open(true);
        let shown = window.popup_menu(&tray.menu);
        win::autohide::set_menu_open(false);
        shown?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Çökmede de sistemi geri ver: AppBar'ı kaldır, Windows çubuğunu göster.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        win::shutdown();
        default_hook(info);
    }));

    let app = tauri::Builder::default()
        // Tek örnek: dokümana göre ilk kaydedilen eklenti olmalı.
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            // İkinci kopya açılmaz; ilk kopya zaten ekranda.
        }))
        .invoke_handler(tauri::generate_handler![
            commands::set_dock_expanded,
            commands::dock_pointer,
            commands::open_system_settings,
            commands::open_quick_settings,
            commands::show_popup,
            commands::popup_ready,
            commands::hide_popup,
            commands::fetch_ics,
            commands::open_settings,
            commands::take_settings_page,
            commands::get_system_accent,
            commands::spotify_status,
            commands::spotify_connect,
            commands::spotify_disconnect,
            commands::spotify_queue,
            commands::spotify_play_item,
            commands::spotify_queue_remove,
            commands::spotify_queue_reorder,
            commands::spotify_queue_reset,
            commands::spotify_player,
            commands::spotify_set_volume,
            commands::spotify_volume_step,
            commands::spotify_toggle_mute,
            commands::spotify_seek,
            commands::spotify_devices,
            commands::spotify_transfer,
            commands::spotify_search,
            commands::spotify_add_to_queue,
            commands::spotify_play_uri,
            commands::spotify_play_context,
            commands::spotify_set_liked,
            commands::spotify_playlists,
            commands::get_update_info,
            commands::check_update,
            commands::show_dock_menu,
            commands::get_backdrop,
            commands::set_app_icon_rects,
            commands::get_autostart,
            commands::set_autostart,
            commands::get_config,
            commands::set_config,
            commands::get_apps,
            commands::activate_app,
            commands::launch_app,
            commands::close_window,
            commands::preview_enter,
            commands::preview_leave,
            commands::preview_hover,
            commands::preview_sizes,
            commands::preview_layout,
            commands::hide_preview,
            commands::activate_window,
            commands::pin_app,
            commands::unpin_app,
            commands::open_files_with_app,
            commands::pin_path,
            commands::reorder_pinned,
            commands::show_app_menu,
            commands::open_start,
            commands::get_sysmon,
            commands::sysmon_detail,
            commands::open_task_manager,
            commands::system_shortcut,
            commands::open_tray,
            commands::get_media,
            commands::media_toggle,
            commands::media_next,
            commands::media_prev,
            commands::media_set_shuffle,
            commands::media_set_repeat,
            commands::get_audio,
            commands::set_volume,
            commands::set_mute,
            commands::set_mic_volume,
            commands::set_mic_mute,
            commands::set_default_output,
            commands::get_power,
            commands::get_radios,
            commands::set_power_mode,
            commands::get_battery_settings,
            commands::set_energy_saver,
            commands::set_brightness,
            commands::set_refresh_rate,
            commands::set_power_timeout,
            commands::get_tray,
            commands::tray_click,
            commands::timer_get,
            commands::timer_control,
            commands::get_recycle,
            commands::open_recycle_bin,
            commands::empty_recycle_bin,
            commands::recycle_files,
            commands::get_keyboard,
            commands::get_ai_usage,
            commands::ai_usage_refresh,
            commands::get_notification_sound,
            commands::set_notification_sound,
            commands::preview_notification_sound,
            commands::get_brightness,
            commands::flyout_layout,
            commands::flyout_ready,
            commands::flyout_hide,
            commands::flyout_hold,
            commands::audio_sessions,
            commands::set_session_volume,
            commands::set_session_mute,
            commands::media_timeline,
            commands::media_seek,
            commands::notify_layout,
            commands::notify_test,
            commands::notify_activate,
            commands::wifi_scan,
            commands::wifi_connect,
            commands::wifi_connect_enterprise,
            commands::wifi_disconnect,
            commands::bt_devices,
            commands::bt_set_connected,
            commands::set_radio,
        ])
        // Tüm menü olayları (tepsi + sağ tık menüleri) buraya gelir.
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_QUIT => app.exit(0),
            MENU_SETTINGS => {
                // Olay işleyicisinde pencere oluşturmak Windows'ta kilitlenir.
                let app = app.clone();
                std::thread::spawn(move || {
                    if let Err(e) = win::settings::open(&app) {
                        eprintln!("ayarlar açılamadı: {e}");
                    }
                });
            }
            MENU_HIDE_TASKBAR => toggle_hide_taskbar(app),
            MENU_AUTOSTART => toggle_autostart(app),
            id => apps::handle_menu_event(app, id),
        })
        .setup(|app| {
            app.handle()
                .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))?;
            app.handle()
                .plugin(tauri_plugin_updater::Builder::new().build())?;

            let config_dir = app.path().app_config_dir()?;
            services::flylog::init(config_dir.clone());
            let config = ConfigStore::load(config_dir.clone());
            i18n::set(&config.get().language);
            config.attach(app.handle().clone());
            let hide_taskbar = config.get().hide_windows_taskbar;
            let minimize_to_dock = config.get().minimize_to_dock;
            let own_notifications = config.get().own_notifications;
            let auto_hide = win::autohide::Mode::parse(&config.get().auto_hide);
            let first_run = config.is_first_run();
            win::placement::set_ui_scale(config.get().theme.ui_scale());
            app.manage(config);
            app.manage(services::spotify::Spotify::load(config_dir.clone()));
            app.manage(services::updater::Updater::new());
            services::updater::spawn(app.handle());

            let (wake_tx, wake_rx) = mpsc::channel();
            app.manage(AppsService::new(wake_tx));
            apps::spawn_poller(app.handle().clone(), wake_rx);
            services::jumplist::prewarm();
            services::app_events::spawn(app.handle());
            services::media::spawn(app.handle());
            services::audio::spawn(app.handle());
            services::flyouts::spawn(app.handle());
            services::battery_settings::init(config_dir.clone());
            services::power::spawn(app.handle());
            services::radios::spawn(app.handle());
            services::sysmon::spawn(app.handle());
            services::recycle::spawn(app.handle());
            services::keyboard::spawn(app.handle());
            services::ai_usage::spawn(app.handle());
            services::timer::init(app.handle());
            services::backdrop::spawn(app.handle());
            services::accent::spawn(app.handle());
            // Servisler kapalı başlar; yerleşimdeki widget'larınki açılır (8.1).
            services::sync_active(app.handle(), &app.state::<ConfigStore>().get());

            let dock = app
                .get_webview_window("dock")
                .ok_or(AppError::NoDockWindow)?;
            let hwnd = dock.hwnd()?;
            win::window_style::apply_dock_style(&dock)?;
            // Yerleşimden önce: mod açıksa AppBar şerit ayırmaz (8.10).
            win::autohide::spawn(app.handle(), hwnd, auto_hide);
            win::appbar::register(app.handle(), hwnd)?;
            win::placement::reposition(hwnd)?;
            dock.show()?;
            win::popup::init(app.handle())?;
            win::preview::init(app.handle())?;
            win::notify::init(app.handle())?;
            win::flyout::init(app.handle())?;
            services::notifications::spawn(app.handle(), config_dir.clone(), own_notifications);
            win::window_fx::set_enabled(minimize_to_dock);
            win::window_fx::spawn(app.handle());

            // Önceki çökmeden kalan gizli çubuğu da burada onarır.
            win::taskbar::init(config_dir, hide_taskbar);
            // Tepsi simgelerini kendi panelimizde göstermek için (8.6).
            win::tray_host::init(app.handle());

            // Bu bilgisayarda ilk kez açıldı (kurulumdan sonra): açılışta başlasın.
            if first_run
                && AUTOSTART_AVAILABLE
                && !autostart_enabled(app.handle())
                && let Err(e) = set_autostart(app.handle(), true)
            {
                eprintln!("açılışta başlatma açılamadı: {e}");
            }

            create_tray(app.handle(), hide_taskbar)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Dockora başlatılamadı");

    app.run(|_app, event| {
        if let RunEvent::Exit = event {
            win::shutdown();
        }
    });
}

/// Tepsi (bölüm 11).
fn create_tray(app: &AppHandle, hide_taskbar: bool) -> tauri::Result<()> {
    let settings_item =
        MenuItem::with_id(app, MENU_SETTINGS, i18n::t("Ayarlar…"), true, None::<&str>)?;
    let hide_item = CheckMenuItem::with_id(
        app,
        MENU_HIDE_TASKBAR,
        i18n::t("Windows görev çubuğunu gizle"),
        true,
        hide_taskbar,
        None::<&str>,
    )?;
    let autostart_item = CheckMenuItem::with_id(
        app,
        MENU_AUTOSTART,
        autostart_label(),
        AUTOSTART_AVAILABLE,
        autostart_enabled(app),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, MENU_QUIT, i18n::t("Çıkış"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &settings_item,
            &PredefinedMenuItem::separator(app)?,
            &hide_item,
            &autostart_item,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(TrayMenu {
        menu: menu.clone(),
        settings: settings_item,
        hide_taskbar: hide_item,
        autostart: autostart_item,
        quit: quit.clone(),
    });

    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip(i18n::app_name())
        .menu(&menu)
        .show_menu_on_left_click(true);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn autostart_label() -> &'static str {
    i18n::t(if AUTOSTART_AVAILABLE {
        "Açılışta başlat"
    } else {
        "Açılışta başlat (yalnızca kurulu sürüm)"
    })
}

/// Dil değişince tepsi menüsü, simge ipucu ve açık ayarlar penceresi yeni dile geçer.
pub(crate) fn apply_language(app: &AppHandle) {
    if let Some(tray) = app.try_state::<TrayMenu>() {
        let _ = tray.settings.set_text(i18n::t("Ayarlar…"));
        let _ = tray
            .hide_taskbar
            .set_text(i18n::t("Windows görev çubuğunu gizle"));
        let _ = tray.autostart.set_text(autostart_label());
        let _ = tray.quit.set_text(i18n::t("Çıkış"));
    }
    if let Some(icon) = app.tray_by_id("main") {
        let _ = icon.set_tooltip(Some(i18n::app_name()));
    }
    win::settings::refresh_title(app);
}

fn toggle_hide_taskbar(app: &AppHandle) {
    let config = app.state::<ConfigStore>();
    let hide = !config.get().hide_windows_taskbar;
    if let Err(e) = config.update(|c| c.hide_windows_taskbar = hide) {
        eprintln!("ayar kaydedilemedi: {e}");
    }
    apply_hide_taskbar(app, hide);
}

/// Windows çubuğunu gizle/göster ve tepsideki onay kutusunu eşitle.
/// (Ayar kaydını çağıran yapar: tepsi ya da `set_config`.)
pub(crate) fn apply_hide_taskbar(app: &AppHandle, hide: bool) {
    win::taskbar::set_hidden(hide);
    if let Some(tray) = app.try_state::<TrayMenu>() {
        let _ = tray.hide_taskbar.set_checked(hide);
    }
}

/// Geliştirme sürümü Vite sunucusu olmadan açılamaz; açılışa yalnızca
/// `tauri build` ile üretilen sürüm kaydedilsin.
pub(crate) const AUTOSTART_AVAILABLE: bool = !cfg!(debug_assertions);

pub(crate) fn autostart_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

fn toggle_autostart(app: &AppHandle) {
    if let Err(e) = set_autostart(app, !autostart_enabled(app)) {
        eprintln!("{e}");
    }
}

/// Tepsi ve ayarlar penceresi ortak yolu; tepsideki onay kutusunu da eşitler.
pub(crate) fn set_autostart(app: &AppHandle, enable: bool) -> Result<(), String> {
    if !AUTOSTART_AVAILABLE {
        return Err("açılışta başlatma yalnızca kurulu sürümde".into());
    }
    let autolaunch = app.autolaunch();
    let result = if enable {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    if let Some(tray) = app.try_state::<TrayMenu>() {
        let _ = tray.autostart.set_checked(autostart_enabled(app));
    }
    result.map_err(|e| format!("açılışta başlatma ayarlanamadı: {e}"))
}
