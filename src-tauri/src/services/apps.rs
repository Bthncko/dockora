//! Uygulamalar servisi: açık pencereleri uygulamalara gruplar, sabitlenenlerle
//! birleştirir ve değişince `apps://changed` gönderir (bölüm 5).
//!
//! Tek bir iş parçacığı listeler. 7.8: pencere olayları (`app_events`) onu
//! hemen uyandırır; seyrek yoklama yalnızca olayı olmayan değişimler için yedek.
//! Komutlar son anlık görüntüyü okur; bir şey değiştirdiklerinde yoklayıcıyı
//! hemen uyandırırlar.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use super::icons::shell_info;
use super::jumplist;
use super::launcher::{self, LaunchTarget, launch_target};
use super::windows_list::{self, process_aumid, process_image, window_aumid};
use crate::config::{ConfigStore, PinnedApp};
use crate::error::{AppError, AppResult};
use crate::win::focus;

pub const APPS_CHANGED: &str = "apps://changed";
/// Yedek yoklama: asıl tetikleyici pencere olayları.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
pub const MENU_PREFIX: &str = "app-menu";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppItem {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub pinned: bool,
    pub windows: Vec<WindowInfo>,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub title: String,
}

/// Bir uygulama hakkında bilinen her şey (ikon hariç komutlara gereken kısmı).
#[derive(Debug, Clone)]
struct AppMeta {
    name: String,
    icon: Option<String>,
    exe: Option<String>,
    aumid: Option<String>,
    launch: Option<LaunchTarget>,
}

#[derive(Default)]
struct Snapshot {
    items: Vec<AppItem>,
    meta: HashMap<String, AppMeta>,
    /// Dock'a ait olmayan son ön plan penceresi (odak tuzağına karşı, bölüm 5.4).
    last_foreground: isize,
}

pub struct AppsService {
    snapshot: Mutex<Snapshot>,
    wake: Mutex<Sender<()>>,
}

impl AppsService {
    pub fn new(wake: Sender<()>) -> Self {
        Self {
            snapshot: Mutex::new(Snapshot::default()),
            wake: Mutex::new(wake),
        }
    }

    pub fn items(&self) -> Vec<AppItem> {
        self.snapshot.lock().unwrap().items.clone()
    }

    pub(crate) fn refresh_soon(&self) {
        let _ = self.wake.lock().unwrap().send(());
    }

    fn find(&self, id: &str) -> AppResult<(AppItem, AppMeta, isize)> {
        let snap = self.snapshot.lock().unwrap();
        let item = snap
            .items
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or_else(|| AppError::UnknownApp(id.to_string()))?;
        let meta = snap
            .meta
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::UnknownApp(id.to_string()))?;
        Ok((item, meta, snap.last_foreground))
    }

    /// Sol tık (bölüm 5.4 tablosu).
    pub fn activate(&self, id: &str) -> AppResult<()> {
        let (item, meta, last_fg) = self.find(id)?;
        if item.windows.is_empty() {
            return self.launch_meta(&meta);
        }
        let own_pid = std::process::id();
        let fg = focus::foreground();
        let fg = if focus::window_pid(fg) == own_pid {
            last_fg
        } else {
            fg
        };
        let hwnds: Vec<isize> = item.windows.iter().map(|w| w.hwnd).collect();
        if hwnds.contains(&fg) {
            if hwnds.len() > 1 {
                // Sırayla dolaş: en alttakini öne getir → hepsi sırayla gelir.
                focus::activate(*hwnds.last().unwrap());
            } else {
                focus::minimize(fg);
            }
        } else {
            focus::activate(hwnds[0]);
        }
        self.refresh_soon();
        Ok(())
    }

    /// Bildirim kartına tıklama: uygulamanın penceresi varsa öne getir, yoksa
    /// AUMID ile başlat (bildirimi gönderen uygulama).
    pub fn bring_aumid(&self, aumid: &str) -> AppResult<()> {
        let hwnd = {
            let snap = self.snapshot.lock().unwrap();
            snap.items
                .iter()
                .find(|i| i.id.eq_ignore_ascii_case(aumid))
                .and_then(|i| i.windows.first().map(|w| w.hwnd))
        };
        if let Some(hwnd) = hwnd {
            focus::activate(hwnd);
            self.refresh_soon();
            return Ok(());
        }
        super::launcher::shell_open("explorer.exe", &format!("shell:AppsFolder\\{aumid}"), "")
    }

    /// Orta tık / "Yeni pencere".
    pub fn launch(&self, id: &str) -> AppResult<()> {
        let (_, meta, _) = self.find(id)?;
        self.launch_meta(&meta)
    }

    fn launch_meta(&self, meta: &AppMeta) -> AppResult<()> {
        let target = meta
            .launch
            .as_ref()
            .ok_or_else(|| AppError::UnknownApp(meta.name.clone()))?;
        launcher::launch(target)?;
        self.refresh_soon();
        Ok(())
    }

    pub fn close_window(&self, hwnd: isize) {
        focus::close(hwnd);
        self.refresh_soon();
    }

    pub fn close_all(&self, id: &str) -> AppResult<()> {
        let (item, _, _) = self.find(id)?;
        for w in &item.windows {
            focus::close(w.hwnd);
        }
        self.refresh_soon();
        Ok(())
    }

    pub fn pin(&self, config: &ConfigStore, id: &str) -> AppResult<()> {
        let (_, meta, _) = self.find(id)?;
        config.update(|c| {
            let pinned = &mut c.widgets.apps.pinned;
            if !pinned.iter().any(|p| p.id == id) {
                pinned.push(PinnedApp {
                    id: id.to_string(),
                    name: meta.name.clone(),
                    exe: meta.exe.clone(),
                    aumid: meta.aumid.clone(),
                });
            }
        })?;
        self.refresh_soon();
        Ok(())
    }

    /// Dock'a sürüklenen dosyalar uygulamanın ikonuna bırakıldı (8.7).
    pub fn open_files(&self, id: &str, paths: &[String]) -> AppResult<()> {
        let (_, meta, _) = self.find(id)?;
        super::file_drop::open_with(meta.exe.as_deref(), meta.aumid.as_deref(), paths)?;
        self.refresh_soon();
        Ok(())
    }

    /// Boş alana bırakılan `.exe` / `.lnk` dock'a sabitlenir (8.7).
    pub fn pin_path(&self, config: &ConfigStore, path: &str) -> AppResult<()> {
        let lower = path.to_lowercase();
        let (exe, aumid) = if lower.ends_with(".lnk") {
            super::file_drop::resolve_link(path)
                .ok_or_else(|| AppError::Win32("kısayol bir uygulamayı göstermiyor".into()))?
        } else if lower.ends_with(".exe") {
            (Some(path.to_string()), None)
        } else {
            return Err(AppError::Win32(
                "yalnızca uygulamalar sabitlenebilir".into(),
            ));
        };
        // Çalışan pencerelerle aynı gruplama anahtarı (`poll`): AUMID, yoksa exe.
        let id = match (&aumid, &exe) {
            (Some(a), _) => a.clone(),
            (None, Some(e)) => e.to_lowercase(),
            (None, None) => return Err(AppError::Win32("uygulama bulunamadı".into())),
        };
        let stem = std::path::Path::new(path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let meta = resolve_meta(aumid.as_deref(), exe.as_deref(), &stem);
        config.update(|c| {
            let pinned = &mut c.widgets.apps.pinned;
            if !pinned.iter().any(|p| p.id.eq_ignore_ascii_case(&id)) {
                pinned.push(PinnedApp {
                    id: id.clone(),
                    name: meta.name.clone(),
                    exe: meta.exe.clone(),
                    aumid: meta.aumid.clone(),
                });
            }
        })?;
        self.refresh_soon();
        Ok(())
    }

    /// Sabit ikonlar sürüklenerek sıralandı: `ids` yeni sıra (8.7).
    pub fn reorder_pinned(&self, config: &ConfigStore, ids: &[String]) -> AppResult<()> {
        config.update(|c| {
            let pinned = &mut c.widgets.apps.pinned;
            // Listede olmayan (ör. bu arada eklenen) sona.
            pinned.sort_by_key(|p| ids.iter().position(|id| *id == p.id).unwrap_or(usize::MAX));
        })?;
        self.refresh_soon();
        Ok(())
    }

    pub fn unpin(&self, config: &ConfigStore, id: &str) -> AppResult<()> {
        config.update(|c| c.widgets.apps.pinned.retain(|p| p.id != id))?;
        self.refresh_soon();
        Ok(())
    }

    /// Sağ tık: yerel (native) bağlam menüsü, imlecin olduğu yerde. Windows
    /// görev çubuğundaki gibi: önce uygulamanın atlama listesi (kategoriler,
    /// görevler), sonra uygulamanın adı (yeni örnek), sabitleme, kapatma.
    pub fn show_menu(&self, window: &WebviewWindow, id: &str) -> AppResult<()> {
        let (item, meta, _) = self.find(id)?;
        let app = window.app_handle();
        let mid = |action: &str| format!("{MENU_PREFIX}:{action}:{id}");
        let menu = Menu::new(app)?;

        let categories = jumplist::load(meta.exe.as_deref(), meta.aumid.as_deref());
        let mut targets = Vec::new();
        for cat in &categories {
            // Başlık: tıklanamaz satır.
            menu.append(&MenuItem::new(
                app,
                menu_text(&cat.title),
                false,
                None::<&str>,
            )?)?;
            for entry in &cat.items {
                let n = targets.len();
                targets.push(entry.target.clone());
                menu.append(&MenuItem::with_id(
                    app,
                    format!("{MENU_PREFIX}:jump:{n}"),
                    format!("    {}", menu_text(&entry.title)),
                    true,
                    None::<&str>,
                )?)?;
            }
            menu.append(&PredefinedMenuItem::separator(app)?)?;
        }
        *JUMP_TARGETS.lock().unwrap() = (meta.exe.clone(), targets);

        menu.append(&MenuItem::with_id(
            app,
            mid("new"),
            menu_text(&meta.name),
            meta.launch.is_some(),
            None::<&str>,
        )?)?;
        let pin = if item.pinned {
            MenuItem::with_id(
                app,
                mid("unpin"),
                crate::i18n::t("Dock'tan kaldır"),
                true,
                None::<&str>,
            )?
        } else {
            MenuItem::with_id(
                app,
                mid("pin"),
                crate::i18n::t("Dock'a sabitle"),
                true,
                None::<&str>,
            )?
        };
        menu.append(&pin)?;
        if !item.windows.is_empty() {
            let close_label = if item.windows.len() == 1 {
                crate::i18n::t("Pencereyi kapat").to_string()
            } else if crate::i18n::english() {
                format!("Close all windows ({})", item.windows.len())
            } else {
                format!("Tüm pencereleri kapat ({})", item.windows.len())
            };
            menu.append(&MenuItem::with_id(
                app,
                mid("close"),
                close_label,
                true,
                None::<&str>,
            )?)?;
        }
        window.popup_menu(&menu)?;
        Ok(())
    }
}

/// Son açılan menünün atlama listesi öğeleri (menü kimliğindeki sıra) ve exe.
static JUMP_TARGETS: Mutex<(Option<String>, Vec<jumplist::Target>)> =
    Mutex::new((None, Vec::new()));

/// Menü metni: `&` kısayol işareti sayılmasın, çok uzunsa kısalsın.
fn menu_text(s: &str) -> String {
    const MAX: usize = 60;
    let s = s.trim();
    let short: String = if s.chars().count() > MAX {
        s.chars()
            .take(MAX - 1)
            .chain(std::iter::once('…'))
            .collect()
    } else {
        s.to_string()
    };
    short.replace('&', "&&")
}

/// Menü olayları: `app-menu:<eylem>:<id>`. id içinde ':' olabilir (exe yolu).
pub fn handle_menu_event(app: &AppHandle, menu_id: &str) {
    let mut parts = menu_id.splitn(3, ':');
    let (Some(MENU_PREFIX), Some(action), Some(id)) = (parts.next(), parts.next(), parts.next())
    else {
        return;
    };
    if action == "jump" {
        let (exe, target) = {
            let jump = JUMP_TARGETS.lock().unwrap();
            (
                jump.0.clone(),
                id.parse::<usize>()
                    .ok()
                    .and_then(|n| jump.1.get(n).cloned()),
            )
        };
        if let Some(target) = target
            && let Err(e) = jumplist::open(&target, exe.as_deref())
        {
            eprintln!("atlama listesi öğesi açılamadı: {e}");
        }
        return;
    }
    let apps = app.state::<AppsService>();
    let config = app.state::<ConfigStore>();
    let result = match action {
        "pin" => apps.pin(&config, id),
        "unpin" => apps.unpin(&config, id),
        "new" => apps.launch(id),
        "close" => apps.close_all(id),
        _ => Ok(()),
    };
    if let Err(e) = result {
        eprintln!("menü eylemi '{action}' başarısız: {e}");
    }
}

// ---------------------------------------------------------------------------
// Yoklayıcı
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PollerCache {
    /// (hwnd, pid) → (exe, aumid). Pencere yaşadıkça bunlar değişmez.
    windows: HashMap<(isize, u32), (Option<String>, Option<String>)>,
    meta: HashMap<String, AppMeta>,
    /// Sabitlenmemiş açık uygulamaların ilk görülme sırası (ikonlar zıplamasın).
    order: Vec<String>,
    last_foreground: isize,
}

pub fn spawn_poller(app: AppHandle, wake: Receiver<()>) {
    let spawned = std::thread::Builder::new()
        .name("apps-poller".into())
        .spawn(move || {
            // Kabuk ve özellik deposu API'leri COM ister.
            crate::win::com_sta();
            let mut cache = PollerCache::default();
            loop {
                let (items, meta) = build(&app, &mut cache);
                super::app_events::set_known(
                    items.iter().flat_map(|i| i.windows.iter().map(|w| w.hwnd)),
                );
                let service = app.state::<AppsService>();
                let changed = {
                    let mut snap = service.snapshot.lock().unwrap();
                    let changed = snap.items != items;
                    snap.items = items.clone();
                    snap.meta = meta;
                    snap.last_foreground = cache.last_foreground;
                    changed
                };
                if changed {
                    let _ = app.emit(APPS_CHANGED, &items);
                }
                match wake.recv_timeout(POLL_INTERVAL) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => {
                        // Komutların ardından gelen olayların yerine oturması için kısa bekleme.
                        std::thread::sleep(Duration::from_millis(60));
                        while wake.try_recv().is_ok() {}
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    if let Err(e) = spawned {
        eprintln!("[apps] yoklayıcı başlatılamadı: {e}");
    }
}

fn build(app: &AppHandle, cache: &mut PollerCache) -> (Vec<AppItem>, HashMap<String, AppMeta>) {
    let own_pid = std::process::id();
    let pinned = app.state::<ConfigStore>().get().widgets.apps.pinned;

    let fg = focus::foreground();
    if fg != 0 && focus::window_pid(fg) != own_pid {
        cache.last_foreground = fg;
    }

    // 1) Pencereleri uygulamalara grupla (z sırası korunur).
    let raws = windows_list::list(own_pid);
    let alive: HashSet<(isize, u32)> = raws.iter().map(|r| (r.hwnd, r.pid)).collect();
    // Dock'taki pencerelerde Windows'un küçültme geçişini kapat (ikona girme animasyonu).
    crate::win::window_fx::track(&raws.iter().map(|r| r.hwnd).collect::<Vec<_>>());
    cache.windows.retain(|k, _| alive.contains(k));

    let mut groups: HashMap<String, Vec<WindowInfo>> = HashMap::new();
    let mut seen_order: Vec<String> = Vec::new();
    for raw in raws {
        let (exe, aumid) = cache
            .windows
            .entry((raw.hwnd, raw.pid))
            .or_insert_with(|| {
                // Önce pencereye yazılmış AUMID, yoksa paketli sürecin kimliği.
                let aumid = window_aumid(raw.hwnd).or_else(|| process_aumid(raw.pid));
                (process_image(raw.pid), aumid)
            })
            .clone();
        // Gruplama anahtarı: AUMID varsa o, yoksa küçük harfli exe yolu.
        let id = match (&aumid, &exe) {
            (Some(a), _) => a.clone(),
            (None, Some(e)) => e.to_lowercase(),
            (None, None) => format!("pid:{}", raw.pid),
        };
        if !cache.meta.contains_key(&id) {
            let meta = resolve_meta(aumid.as_deref(), exe.as_deref(), &raw.title);
            cache.meta.insert(id.clone(), meta);
        }
        if !groups.contains_key(&id) {
            seen_order.push(id.clone());
        }
        groups.entry(id).or_default().push(WindowInfo {
            hwnd: raw.hwnd,
            title: raw.title,
        });
    }

    // 2) Sabitlenenlerin bilgisini (kapalı olsalar da) hazırla.
    for p in &pinned {
        if !cache.meta.contains_key(&p.id) {
            let mut meta = resolve_meta(p.aumid.as_deref(), p.exe.as_deref(), &p.name);
            meta.name = p.name.clone();
            cache.meta.insert(p.id.clone(), meta);
        }
    }

    // 3) Sıra: sabitlenenler (config sırası), sonra diğer açıklar ilk görülme sırasıyla.
    let pinned_ids: HashSet<&str> = pinned.iter().map(|p| p.id.as_str()).collect();
    cache
        .order
        .retain(|id| groups.contains_key(id) && !pinned_ids.contains(id.as_str()));
    for id in seen_order {
        if !pinned_ids.contains(id.as_str()) && !cache.order.contains(&id) {
            cache.order.push(id);
        }
    }

    let ids = pinned
        .iter()
        .map(|p| p.id.clone())
        .chain(cache.order.iter().cloned());
    let mut items = Vec::new();
    let mut meta_out = HashMap::new();
    for id in ids {
        let Some(meta) = cache.meta.get(&id) else {
            continue;
        };
        let windows = groups.remove(&id).unwrap_or_default();
        let focused = windows.iter().any(|w| w.hwnd == cache.last_foreground);
        items.push(AppItem {
            id: id.clone(),
            name: meta.name.clone(),
            icon: meta.icon.clone(),
            pinned: pinned_ids.contains(id.as_str()),
            windows,
            focused,
        });
        meta_out.insert(id, meta.clone());
    }
    (items, meta_out)
}

fn resolve_meta(aumid: Option<&str>, exe: Option<&str>, fallback_name: &str) -> AppMeta {
    let info = shell_info(aumid, exe);
    AppMeta {
        name: info.name.unwrap_or_else(|| fallback_name.to_string()),
        icon: info.icon,
        exe: exe.map(str::to_string),
        aumid: aumid.map(str::to_string),
        launch: launch_target(exe, aumid, info.aumid_resolves),
    }
}
