import { invoke } from "@tauri-apps/api/core";

// Rust komutlarının tipli sarmalayıcıları. Komut ve olay adları
// src-tauri/src/commands.rs ve servis dosyalarıyla aynı.

// --- tipler -------------------------------------------------------------------

export type WidgetRef = { id: string };

export type PinnedApp = { id: string; name: string; exe?: string | null; aumid?: string | null };

export type AutostartState = { enabled: boolean; available: boolean };

/** Bulanık duvar kâğıdı şeridi (7.2). Yoksa düz cam. */
export type Backdrop = { image: string; luma: number } | null;

/** Bildirim kartlarının köşesi / kenarı. */
export type NotifyAnchor =
  "topLeft" | "topCenter" | "topRight" | "bottomLeft" | "bottomCenter" | "bottomRight";

/** `x`/`y`: köşenin kenarlarından uzaklık (mantıksal piksel); ortada `x` yok sayılır. */
export type NotifyPosition = { anchor: NotifyAnchor; x: number; y: number };

export type FlyoutMonitor = "primary" | "cursor" | "focused";
export type FlyoutEasing = "linear" | "sine" | "quad" | "cubic";
export type FlyoutMediaBackground = "none" | "coverGlow" | "risingGlow" | "blur";
export type FlyoutLockKey = "caps" | "num" | "scroll" | "insert";

/** Ekran üstü kartlar (flyout). `placement`: alt kenarda `y` dock'un üstünden ölçülür. */
export type FlyoutsConfig = {
  enabled: boolean;
  placement: NotifyPosition;
  monitor: FlyoutMonitor;
  hideInFullscreen: boolean;
  /** Hız çarpanı; 0 = animasyonsuz. */
  animationSpeed: number;
  easing: FlyoutEasing;
  albumAccent: boolean;
  glass: boolean;
  /** `apps`: oynatıcı kimlikleri (medya oturumunun `sourceApp`'i). */
  appFilter: { mode: "off" | "allow" | "block"; apps: string[] };
  media: {
    enabled: boolean;
    durationMs: number;
    alwaysShow: boolean;
    compact: boolean;
    centerText: boolean;
    showPlayer: boolean;
    showRepeat: boolean;
    showShuffle: boolean;
    showSeekbar: boolean;
    background: FlyoutMediaBackground;
    /** Ses tuşları medya kartını açmasın. */
    excludeVolumeKeys: boolean;
    /** Bir oturum çalmaya başlayınca diğerlerini duraklat. */
    pauseOthers: boolean;
  };
  nextUp: { enabled: boolean; durationMs: number };
  lockKeys: {
    enabled: boolean;
    durationMs: number;
    caps: boolean;
    num: boolean;
    scroll: boolean;
    insert: boolean;
    bold: boolean;
    animated: boolean;
  };
  volume: {
    enabled: boolean;
    durationMs: number;
    aboveMedia: boolean;
    mixer: boolean;
    highlightActive: boolean;
  };
  brightness: { enabled: boolean; durationMs: number };
};

/** Yalnızca "flyout" penceresine, `events.flyoutShow` ile. */
export type FlyoutShow =
  | { kind: "media"; trigger: "mediaKey" | "volumeKey" }
  | {
      kind: "nextUp";
      title: string;
      artist: string;
      album?: string;
      /** Kapak: MediaState.artwork ile aynı biçim (data URL). */
      art?: string;
      appId: string;
    }
  | { kind: "lockKey"; key: FlyoutLockKey; on: boolean }
  | { kind: "volume"; level: number; muted: boolean; device: string }
  | { kind: "brightness"; level: number };

/** Ses karıştırıcısı satırı (varsayılan çıkış aygıtındaki bir uygulama oturumu). */
export type AudioSession = {
  id: string;
  pid: number;
  name: string;
  /** data URL */
  icon: string | null;
  /** 0–100 */
  volume: number;
  muted: boolean;
  /** Şu an ses çıkarıyor. */
  active: boolean;
};

/** Medya oturumunun zaman çizelgesi (ms); oturum yoksa null. */
export type MediaTimeline = { positionMs: number; durationMs: number; playing: boolean } | null;

/** "smart": yalnızca bir pencere dock'un alanına girince gizlenir. */
export type AutoHide = "never" | "always" | "smart";

export type Config = {
  version: number;
  position: "top" | "bottom";
  hideWindowsTaskbar: boolean;
  /** Küçültülen pencere dock ikonuna girer / dock'tan açılınca çıkar. */
  minimizeToDock: boolean;
  /** Yeni sürüm çıkınca kendiliğinden indirip kur. */
  autoUpdate: boolean;
  /** Bildirimler dock tarzı kartlarla (Windows'un kutucukları kapanır). */
  ownNotifications: boolean;
  /** Bildirim kartlarının yeri; alt kenarda uzaklık dock'un üstünden ölçülür. */
  notifyPosition: NotifyPosition;
  /** Ekran üstü kartlar: medya, sıradaki şarkı, kilit tuşları, ses, parlaklık. */
  flyouts: FlyoutsConfig;
  /** Arayüz dili: "auto" Windows'un diline uyar. */
  language: "auto" | "tr" | "en";
  /** Otomatik gizlenme (8.10). */
  autoHide: AutoHide;
  theme: {
    /** "#rrggbb" ya da "auto" (Windows vurgu rengi). */
    accent: string;
    opacity: number;
    radius: number;
    glass: "wallpaper" | "flat";
    /** 0.85–1.25: dock ve paneller CSS zoom ile, ayrılan şerit Rust'ta. */
    scale: number;
    font: "inter" | "system";
    reduceMotion: "system" | "reduce" | "full";
  };
  layout: { left: WidgetRef[]; center: WidgetRef[]; right: WidgetRef[] };
  widgets: {
    apps: { pinned: PinnedApp[]; magnify: number; previews: boolean; previewDelayMs: number };
    weather: { lat: number; lon: number; city: string; unit: "C" | "F" };
    calendar: { icsUrls: string[] };
    clock: { showSeconds: boolean };
    system: { buttons: SystemButton[] };
    sysmon: { view: SysmonView };
    worldClock: { cities: WorldCity[] };
  } & Record<string, unknown>;
};

/** Sistem düğmeleri (8.2); her biri Windows'un kendi panelini açar. */
export type SystemButton =
  | "notifications"
  | "quickSettings"
  | "taskView"
  | "search"
  | "clipboard"
  | "snip"
  | "desktop"
  | "tray"
  | "nightLight"
  | "inputSwitch";

/** Dünya saati (8.8): IANA saat dilimi ("Europe/London"). */
export type WorldCity = { label: string; tz: string };

/** Zamanlayıcı (8.8). Çalışırken `endsAt` (Unix ms) dolu; geri sayım sayfada. */
export type TimerState = {
  total: number;
  remainingMs: number;
  endsAt: number | null;
  finished: boolean;
};
export type RecycleState = { items: number; bytes: number };
export type KeyboardState = { short: string; name: string };

/** Pencere önizlemesi (8.3). */
export type PreviewOpen = { appId: string; seq: number };
export type SourceSize = { hwnd: number; width: number; height: number };
export type ThumbRect = { hwnd: number; x: number; y: number; width: number; height: number };
export type Rect = { x: number; y: number; width: number; height: number };

/** Sistem izleyici (8.4). Baytlar; `disks`/`processes` yalnızca panel açıkken dolu. */
export type SysmonView = "compact" | "bars" | "net";
export type SysState = {
  cpu: number;
  ramUsed: number;
  ramTotal: number;
  netDown: number;
  netUp: number;
  history: number[];
  disks: { root: string; used: number; total: number }[];
  processes: { name: string; cpu: number; memory: number }[];
} | null;

export type AppItem = {
  id: string; // gruplama anahtarı
  name: string;
  icon: string | null; // data URL
  pinned: boolean;
  windows: { hwnd: number; title: string }[];
  focused: boolean;
};

export type MediaState = {
  title: string;
  artist: string;
  artwork: string | null;
  playing: boolean;
  canPrev: boolean;
  canNext: boolean;
  sourceApp: string;
  /** Karıştırma; oynatıcı desteklemiyorsa null. */
  shuffle: boolean | null;
  /** Tekrar kipi; oynatıcı desteklemiyorsa null. */
  repeat: RepeatMode | null;
} | null;

export type RepeatMode = "off" | "list" | "track";

export type AudioDevice = { id: string; name: string; default: boolean };
export type AudioState = {
  volume: number;
  muted: boolean;
  device: string;
  /** Etkin çıkış aygıtları (8.5). */
  outputs: AudioDevice[];
  /** Varsayılan mikrofon. */
  mic: { name: string; volume: number; muted: boolean } | null;
} | null;

export type PowerState = {
  percent: number;
  plugged: boolean;
  charging: boolean;
  /** Enerji / pil tasarrufu açık. */
  saver: boolean;
  /** Pilde kalan süre (sn); prizde ya da bilinmiyorsa null. */
  secondsLeft: number | null;
  mode: PowerMode | null;
} | null;

export type RadioInfo = { on: boolean; disabled: boolean };
export type RadiosState = {
  wifi: RadioInfo | null;
  bluetooth: RadioInfo | null;
  ssid: string | null;
  signal: number | null;
  /** İnternet kablodan (Ethernet) geliyor. */
  ethernet: boolean;
};

export type SpotifyStatus = {
  clientId: string;
  connected: boolean;
  /** Bağlı hesabın görünen adı. */
  account: string | null;
  /** Spotify panelinde kayıtlı olması gereken yönlendirme adresi. */
  redirectUri: string;
  /** Eski bağlantı: yeni özelliklerin (beğen, listeler) izinleri yok. */
  needsReconnect: boolean;
};

export type QueueItem = {
  uri: string;
  title: string;
  artist: string;
  image: string | null;
  durationMs: number;
  /** Spotify sırasındaki yeri (gizlenenler dahil). */
  position: number;
  /** Aynı şarkının sıradaki kaçıncı geçişi. */
  occurrence: number;
};

export type SpotifyQueue = {
  current: QueueItem | null;
  items: QueueItem[];
  /** Panelden kaldırılıp gizlenenler. */
  hidden: number;
};

export type SpotifyDevice = {
  id: string;
  name: string;
  /** "Computer", "Smartphone", "Speaker" ... */
  kind: string;
  active: boolean;
  volume: number | null;
  supportsVolume: boolean;
};

export type SpotifyPlayer = {
  uri: string | null;
  playing: boolean;
  progressMs: number;
  durationMs: number;
  /** Ölçüm anı (epoch ms). */
  at: number;
  device: SpotifyDevice | null;
  liked: boolean | null;
} | null;

export type SpotifyPlaylist = {
  uri: string;
  name: string;
  owner: string;
  image: string | null;
  total: number | null;
};

export type UpdateStatus =
  | { state: "idle" | "checking" | "upToDate" }
  | { state: "available"; version: string }
  | { state: "downloading"; version: string; percent: number }
  | { state: "installing"; version: string }
  | { state: "error"; message: string };

export type UpdateInfo = { current: string; status: UpdateStatus };

// --- olaylar ------------------------------------------------------------------

export const events = {
  config: "config://changed",
  apps: "apps://changed",
  media: "media://changed",
  audio: "audio://changed",
  power: "power://changed",
  radios: "radios://changed",
  backdrop: "backdrop://changed",
  /** Windows vurgu rengi değişti ("#rrggbb" | null). */
  accent: "accent://changed",
  /** Tam ekran uygulama açıldı (true) / kapandı (false). */
  fullscreen: "fullscreen://changed",
  /** Otomatik gizlenme: `true` = çubuk kenara kaysın (8.10). */
  autohide: "autohide://changed",
  /** Yalnızca popup penceresine: hangi panel çizilecek. */
  popupOpen: "popup://open",
  popupClosed: "popup://closed",
  /** Rust kapanma istiyor (odak kaybı, ikinci tıklama): animasyonla kapat. */
  popupClose: "popup://close",
  sysmon: "sysmon://changed",
  timer: "timer://changed",
  recycle: "recycle://changed",
  keyboard: "keyboard://changed",
  aiUsage: "ai-usage://changed",
  /** Yalnızca bildirim penceresine: yeni kart (Notice) / kaldırılan bildirim (id). */
  notifyShow: "notify://show",
  notifyGone: "notify://gone",
  /** Bildirim kartlarının buzlu camı (NotifyBackdrop), kart olayından hemen önce. */
  notifyBackdrop: "notify://backdrop",
  /** Yalnızca "flyout" penceresine: gösterilecek kart (FlyoutShow). */
  flyoutShow: "flyout:show",
  /** Kendi tepsimizdeki simgeler (TrayState). */
  tray: "tray://changed",
  /** Yalnızca önizleme penceresine: hangi uygulamanın pencereleri (PreviewOpen). */
  previewOpen: "preview://open",
  previewClosed: "preview://closed",
  /** Önizlemesi açık uygulama (`null`: kapalı); dock isim balonunu gizler. */
  previewState: "preview://state",
  /** Açık ayarlar penceresi: bu bölüme geç. */
  settingsPage: "settings://page",
  /** Spotify bağlandı / bağlantı kesildi ya da düştü (SpotifyStatus). */
  spotify: "spotify://changed",
  /** Güncelleme durumu (UpdateInfo). */
  update: "update://status",
} as const;

/** `widgetId` "radios/wifi" gibi sayfa içerebilir (bir widget'ın birden çok paneli). */
export type PopupOpen = { widgetId: string; seq: number };

export type PowerMode = "efficiency" | "balanced" | "performance";
/** Pil panelindeki ayarlar; okunamayan alan `null`. Süreler saniye (0 = hiçbir zaman). */
export type BatterySettings = {
  brightness: number | null;
  refresh: { current: number; options: number[] } | null;
  screenOff: number | null;
  sleep: number | null;
  saverThreshold: number | null;
};
/** Uygulama bildirimi (kendi kartımız). */
export type Notice = { id: number; app: string; icon: string | null; title: string; body: string };
/** Kartların arkasındaki ekranın bulanık görüntüsü; alan CSS pikseli, yığınla aynı köşeye yaslı. */
export type NotifyBackdrop = { image: string; luma: number; width: number; height: number };
export type TrayIcon = { id: string; tip: string; icon: string | null };
/** `active: false`: simgeler Windows'ta (kendi tepsimiz açılamadı). */
/** AI kullanımı (8.9): Claude hesabındaki limitler (`/usage` ile aynı). */
export type AiLimit = {
  label: string;
  /** 0–100. */
  percent: number;
  /** Sıfırlanma (epoch ms); pencere başlamadıysa null. */
  resetsAt: number | null;
  /** Claude'un kendi uyarı eşiği aşıldı. */
  warning: boolean;
};
export type AiUsage = {
  /** Hiç alınamadıysa null. */
  limits: {
    session: AiLimit | null;
    weekly: AiLimit | null;
    other: AiLimit[];
    fetchedAt: number;
  } | null;
  /** Son deneme başarısızsa nedeni (önceki değer gösterilmeye devam eder). */
  limitsError: string | null;
  /** Bu bilgisayardaki Claude Code kayıtları (son 7 gün); yoksa null. */
  activity: {
    /** Son 8 günün saatlik ağırlıkları: [saat başı (epoch ms), ağırlık]; yalnızca oran için. */
    hourly: [number, number][];
    /** share: son 7 günkü pay (%). */
    models: { name: string; share: number; requests: number }[];
    tokens: { input: number; output: number; cacheRead: number; cacheWrite: number };
    requests: number;
  } | null;
} | null;

export type TrayState = { active: boolean; icons: TrayIcon[] };
export type WifiNetwork = {
  ssid: string;
  bars: number;
  secured: boolean;
  enterprise: boolean;
  saved: boolean;
  connected: boolean;
};
export type BtDevice = {
  name: string;
  connected: boolean;
  kind: "audio" | "input" | "phone" | "other";
  id: string;
  /** Yalnızca düşük enerjili (LE) aygıt. */
  le: boolean;
};
/** `popup_ready` yanıtı: tıklanan widget'ın ortası, panelin sol kenarından (CSS px). */
export type PopupPlaced = {
  originX: number;
  /** Panelin arkasındaki ekranın bulanık görüntüsü (buzlu cam). */
  backdrop: string | null;
  /** O görüntünün ortalama parlaklığı (0–1). */
  luma: number | null;
} | null;

// --- komutlar -----------------------------------------------------------------

export const commands = {
  /** Büyütme sırasında görünür bölgeyi açar / bara daraltır (3.2). */
  setDockExpanded: (expanded: boolean) => invoke<void>("set_dock_expanded", { expanded }),
  /** Otomatik gizlenme (8.10): imleç dock penceresine girdi / çıktı. */
  dockPointer: (inside: boolean) => invoke<void>("dock_pointer", { inside }),
  openSystemSettings: (
    page:
      | "wifi"
      | "ethernet"
      | "bluetooth"
      | "sound"
      | "sound-mixer"
      | "battery"
      | "power"
      | "nightlight"
      | "spotify-dashboard",
  ) => invoke<void>("open_system_settings", { page }),
  /** Windows'un hızlı ayarlar paneli; "wifi" doğrudan ağ listesini açar. */
  openQuickSettings: (panel: "wifi" | "bluetooth" | "battery") =>
    invoke<void>("open_quick_settings", { panel }),

  /** Widget'ın panelini aç / kapat. `anchor`: widget'ın dock'taki yatay konumu (CSS px). */
  showPopup: (widgetId: string, anchor: { x: number; width: number }) =>
    invoke<void>("show_popup", { widgetId, anchor }),
  /** Popup sayfası paneli çizdi; boyut mantıksal piksel. */
  popupReady: (seq: number, width: number, height: number) =>
    invoke<PopupPlaced>("popup_ready", { seq, width, height }),
  /** Kapanış animasyonu bitti; `seq` verilirse yalnızca o açılış kapanır. */
  hidePopup: (seq?: number) => invoke<void>("hide_popup", { seq: seq ?? null }),

  /** ICS aboneliğini metin olarak indirir (CORS yüzünden Rust'ta). */
  fetchIcs: (url: string) => invoke<string>("fetch_ics", { url }),

  /** `page`: açılınca gösterilecek bölüm (ör. "weather"). */
  openSettings: (page?: string) => invoke<void>("open_settings", { page: page ?? null }),
  takeSettingsPage: () => invoke<string | null>("take_settings_page"),
  /** Dock'un boş yerine sağ tık: tepsi menüsü. */
  showDockMenu: () => invoke<void>("show_dock_menu"),
  getBackdrop: () => invoke<Backdrop>("get_backdrop"),
  getSystemAccent: () => invoke<string | null>("get_system_accent"),
  getAutostart: () => invoke<AutostartState>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<AutostartState>("set_autostart", { enabled }),
  /** Bildirim sesi: kendi seslerimizden biri, "windows" ya da "custom" (Windows'tan seçilmiş başka ses). */
  getNotificationSound: () => invoke<string>("get_notification_sound"),
  setNotificationSound: (id: string) => invoke<void>("set_notification_sound", { id }),
  /** Bir kez çal; hemen döner. */
  previewNotificationSound: (id: string) => invoke<void>("preview_notification_sound", { id }),

  getConfig: () => invoke<Config>("get_config"),
  setConfig: (value: Config) => invoke<void>("set_config", { value }),

  getApps: () => invoke<AppItem[]>("get_apps"),
  activateApp: (id: string) => invoke<void>("activate_app", { id }),
  launchApp: (id: string) => invoke<void>("launch_app", { id }),
  closeWindow: (hwnd: number) => invoke<void>("close_window", { hwnd }),
  pinApp: (id: string) => invoke<void>("pin_app", { id }),
  unpinApp: (id: string) => invoke<void>("unpin_app", { id }),
  /** Dosyalar uygulama ikonuna bırakıldı (8.7). */
  openFilesWithApp: (id: string, paths: string[]) =>
    invoke<void>("open_files_with_app", { id, paths }),
  /** `.exe` / `.lnk` dock'a bırakıldı: sabitle. */
  pinPath: (path: string) => invoke<void>("pin_path", { path }),
  reorderPinned: (ids: string[]) => invoke<void>("reorder_pinned", { ids }),
  showAppMenu: (id: string) => invoke<void>("show_app_menu", { id }),
  /** İkonların dinlenme konumları (dock CSS pikseli): küçültme animasyonunun hedefi. */
  setAppIconRects: (rects: { id: string; x: number; y: number; size: number }[]) =>
    invoke<void>("set_app_icon_rects", { rects }),
  openStart: () => invoke<void>("open_start"),
  getSysmon: () => invoke<SysState>("get_sysmon"),
  /** Panel açık: her saniye, diskler ve süreçlerle. */
  sysmonDetail: (on: boolean) => invoke<void>("sysmon_detail", { on }),
  openTaskManager: () => invoke<void>("open_task_manager"),

  /** İmleç açık penceresi olan bir ikona girdi; `anchor`: ikonun dock'taki yeri (CSS px). */
  previewEnter: (appId: string, anchor: { x: number; width: number }) =>
    invoke<void>("preview_enter", { appId, anchor }),
  previewLeave: () => invoke<void>("preview_leave"),
  previewHover: (inside: boolean) => invoke<void>("preview_hover", { inside }),
  previewSizes: (seq: number, hwnds: number[]) =>
    invoke<SourceSize[]>("preview_sizes", { seq, hwnds }),
  /** Kartlar dizildi: pencereyi konumla, küçük resimleri yerleştir, göster. */
  previewLayout: (seq: number, width: number, height: number, thumbs: ThumbRect[], clip: Rect) =>
    invoke<void>("preview_layout", { seq, width, height, thumbs, clip }),
  hidePreview: () => invoke<void>("hide_preview"),
  activateWindow: (hwnd: number) => invoke<void>("activate_window", { hwnd }),
  systemShortcut: (action: SystemButton) => invoke<void>("system_shortcut", { action }),
  /** Tepsi taşma penceresi, düğmenin üstünde (8.6). */
  openTray: (anchor?: { x: number; width: number }) =>
    invoke<void>("open_tray", { anchor: anchor ?? null }),

  getMedia: () => invoke<MediaState>("get_media"),
  mediaToggle: () => invoke<void>("media_toggle"),
  mediaNext: () => invoke<void>("media_next"),
  mediaPrev: () => invoke<void>("media_prev"),
  mediaSetShuffle: (on: boolean) => invoke<void>("media_set_shuffle", { on }),
  mediaSetRepeat: (mode: RepeatMode) => invoke<void>("media_set_repeat", { mode }),

  spotifyStatus: () => invoke<SpotifyStatus>("spotify_status"),
  /** Tarayıcıda Spotify onayını bekler (en çok 3 dk). */
  spotifyConnect: (clientId: string) => invoke<SpotifyStatus>("spotify_connect", { clientId }),
  spotifyDisconnect: () => invoke<SpotifyStatus>("spotify_disconnect"),
  spotifyQueue: () => invoke<SpotifyQueue>("spotify_queue"),
  /** Sıradakilerdeki öğeyi doğrudan çal (`position`: Spotify sırasındaki yeri). */
  spotifyPlayItem: (position: number, uri: string) =>
    invoke<void>("spotify_play_item", { position, uri }),
  /** Listede gizle; çalmaya başlarsa hemen geçilir. */
  spotifyQueueRemove: (item: QueueItem) =>
    invoke<void>("spotify_queue_remove", {
      uri: item.uri,
      occurrence: item.occurrence,
      title: item.title,
    }),
  /** Sıranın başının istenen hâli (sürükle-bırak, "sıradaki yap"). */
  spotifyQueueReorder: (order: QueueItem[]) =>
    invoke<void>("spotify_queue_reorder", {
      order: order.map((i) => ({ position: i.position, uri: i.uri, title: i.title })),
    }),
  spotifyQueueReset: () => invoke<void>("spotify_queue_reset"),
  spotifyPlayer: () => invoke<SpotifyPlayer>("spotify_player"),
  spotifySetVolume: (percent: number) => invoke<number>("spotify_set_volume", { percent }),
  spotifyVolumeStep: (delta: number) => invoke<number>("spotify_volume_step", { delta }),
  spotifyToggleMute: (current: number) => invoke<number>("spotify_toggle_mute", { current }),
  spotifySeek: (positionMs: number) => invoke<void>("spotify_seek", { positionMs }),
  spotifyDevices: () => invoke<SpotifyDevice[]>("spotify_devices"),
  spotifyTransfer: (deviceId: string) => invoke<void>("spotify_transfer", { deviceId }),
  spotifySearch: (query: string) => invoke<QueueItem[]>("spotify_search", { query }),
  spotifyAddToQueue: (uri: string) => invoke<void>("spotify_add_to_queue", { uri }),
  spotifyPlayUri: (uri: string) => invoke<void>("spotify_play_uri", { uri }),
  spotifyPlayContext: (uri: string) => invoke<void>("spotify_play_context", { uri }),
  spotifySetLiked: (uri: string, liked: boolean) =>
    invoke<void>("spotify_set_liked", { uri, liked }),
  spotifyPlaylists: () => invoke<SpotifyPlaylist[]>("spotify_playlists"),

  getUpdateInfo: () => invoke<UpdateInfo>("get_update_info"),
  /** Sonuç `events.update` ile gelir; yeni sürüm varsa kurulur (uygulama yeniden açılır). */
  checkUpdate: () => invoke<void>("check_update"),

  getAudio: () => invoke<AudioState>("get_audio"),
  /** v: 0–1 */
  setVolume: (v: number) => invoke<void>("set_volume", { v }),
  setMute: (muted: boolean) => invoke<void>("set_mute", { muted }),
  setMicVolume: (v: number) => invoke<void>("set_mic_volume", { v }),
  setMicMute: (muted: boolean) => invoke<void>("set_mic_mute", { muted }),
  /** Belgelenmemiş API; hata verirse Windows ses ayarları açılmalı. */
  setDefaultOutput: (id: string) => invoke<void>("set_default_output", { id }),

  getPower: () => invoke<PowerState>("get_power"),

  getRadios: () => invoke<RadiosState>("get_radios"),
  setRadio: (kind: "wifi" | "bluetooth", on: boolean) => invoke<void>("set_radio", { kind, on }),
  setPowerMode: (mode: PowerMode) => invoke<void>("set_power_mode", { mode }),
  getBatterySettings: () => invoke<BatterySettings>("get_battery_settings"),
  setEnergySaver: (on: boolean) => invoke<void>("set_energy_saver", { on }),
  setBrightness: (percent: number) => invoke<void>("set_brightness", { percent }),
  setRefreshRate: (hz: number) => invoke<void>("set_refresh_rate", { hz }),
  setPowerTimeout: (kind: "screen" | "sleep", seconds: number) =>
    invoke<void>("set_power_timeout", { kind, seconds }),
  getTray: () => invoke<TrayState>("get_tray"),
  timerGet: () => invoke<TimerState>("timer_get"),
  timerControl: (action: "start" | "pause" | "resume" | "reset", seconds?: number) =>
    invoke<void>("timer_control", { action, seconds }),
  getRecycle: () => invoke<RecycleState | null>("get_recycle"),
  openRecycleBin: () => invoke<void>("open_recycle_bin"),
  emptyRecycleBin: () => invoke<void>("empty_recycle_bin"),
  /** Dosyaları geri dönüşüm kutusuna gönder; döner: sıraya giren öğe sayısı. */
  recycleFiles: (paths: string[]) => invoke<number>("recycle_files", { paths }),
  getKeyboard: () => invoke<KeyboardState | null>("get_keyboard"),
  getAiUsage: () => invoke<AiUsage>("get_ai_usage"),
  /** Beklemeden yeniden oku; sonuç `events.aiUsage` ile gelir. */
  aiUsageRefresh: () => invoke<void>("ai_usage_refresh"),
  getBrightness: () => invoke<number | null>("get_brightness"),
  notifyLayout: (width: number, height: number) => invoke<void>("notify_layout", { width, height }),
  /** Flyout penceresi: içerik boyutu (CSS px, tam pencere); konumlar ve gösterir. 0 → gizle. */
  flyoutLayout: (width: number, height: number) =>
    invoke<void>("flyout_layout", { width, height }),
  flyoutHide: () => invoke<void>("flyout_hide"),
  /** İmleç kartın üzerinde: Rust tarafına bilgi (otomatik gizleme sayfada). */
  flyoutHold: (hold: boolean) => invoke<void>("flyout_hold", { hold }),
  /** Varsayılan çıkış aygıtındaki uygulama oturumları (ses karıştırıcı). */
  audioSessions: () => invoke<AudioSession[]>("audio_sessions"),
  setSessionVolume: (id: string, volume: number) =>
    invoke<void>("set_session_volume", { id, volume }),
  setSessionMute: (id: string, muted: boolean) => invoke<void>("set_session_mute", { id, muted }),
  /** Geçerli medya oturumunun konumu / süresi. */
  mediaTimeline: () => invoke<MediaTimeline>("media_timeline"),
  mediaSeek: (positionMs: number) => invoke<void>("media_seek", { positionMs }),
  notifyActivate: (id: number) => invoke<void>("notify_activate", { id }),
  /** Ayarlar → "Dene": örnek kart; `silent` ise ses çalmaz. */
  notifyTest: (silent = false) => invoke<void>("notify_test", { silent }),
  trayClick: (id: string, button: "left" | "double" | "right") =>
    invoke<void>("tray_click", { id, button }),
  /** Tarama birkaç saniye sürer. */
  wifiScan: () => invoke<WifiNetwork[]>("wifi_scan"),
  /** Şifre isteyen kayıtsız ağda `password` yoksa hata "password_required". */
  wifiConnect: (ssid: string, password?: string) =>
    invoke<void>("wifi_connect", { ssid, password: password ?? null }),
  wifiDisconnect: () => invoke<void>("wifi_disconnect"),
  btDevices: () => invoke<BtDevice[]>("bt_devices"),
  /** Hata "use_settings" ise bu aygıt programla ayrılamaz: Windows ayarlarını aç. */
  btSetConnected: (id: string, le: boolean, connect: boolean) =>
    invoke<void>("bt_set_connected", { id, le, connect }),
};

/** Komut hatalarını yutmadan konsola yazar; widget ayakta kalır. */
export function report(label: string) {
  return (e: unknown) => console.error(label, e);
}
