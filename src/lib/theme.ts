import type { Config } from "./ipc";

/** `accent: "auto"` iken Windows rengi okunamazsa. tokens.css ile aynı. */
export const FALLBACK_ACCENT = "#3b9eff";
/** "Sistem" yazı tipi: Windows 11'in arayüz yazısı. */
const SYSTEM_FONT = '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';

/** Tema vurgu rengi: "auto" ise Windows'unki (7.9). */
export function resolveAccent(theme: Config["theme"], systemAccent?: string | null) {
  return theme.accent === "auto" ? (systemAccent ?? FALLBACK_ACCENT) : theme.accent;
}

/** Hex renk açık mı (göreli parlaklık > 0.5)? Okunamazsa koyu varsayılır. */
export function isLightColor(color: string) {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(color.trim());
  if (!m) return false;
  let h = m[1];
  if (h.length === 3) h = [...h].map((c) => c + c).join("");
  const ch = [0, 2, 4].map((i) => {
    const v = parseInt(h.slice(i, i + 2), 16) / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * ch[0] + 0.7152 * ch[1] + 0.0722 * ch[2] > 0.4;
}

/** Boyut ayarı, sınırlar içinde (Rust → `Theme::ui_scale` ile aynı). */
export function uiScale(theme: Config["theme"]) {
  return Number.isFinite(theme.scale) ? Math.min(Math.max(theme.scale, 0.85), 1.25) : 1;
}

/**
 * Buzlu camın (paneller, bildirim kartları; Rust `win/glass.rs`) örtüsü: arkası
 * açıksa koyulaşır (yazı okunsun), düz zeminde opak.
 */
export function glassAlpha(theme: { opacity: number; glass: string }, luma: number) {
  if (theme.glass === "flat") return 1;
  const base = 0.42 + 0.4 * Math.min(Math.max(luma, 0), 1);
  // Saydamlık ayarı yükseldikçe opaklığa yaklaşır.
  return Math.min(1, base + Math.max(0, theme.opacity - 0.6));
}

type Options = {
  /** Duvar kâğıdı şeridinin parlaklığı (0–1); yoksa (düz cam) örtü tam. */
  luma?: number;
  systemAccent?: string | null;
  /** Bu pencerenin tamamı boyut ayarıyla ölçeklensin mi (yalnızca dock). */
  zoomDocument?: boolean;
};

/**
 * Config'teki temayı :root değişkenlerine yazar (tokens.css).
 *
 * İki ayrı opaklık:
 * - `--glass-opacity` (ayarlardaki saydamlık): cam katmanlarının tamamı. 1 = tam
 *   buzlu cam, düştükçe arkadaki gerçek masaüstü görünür.
 * - `--glass-alpha` (otomatik): bulanık duvar kâğıdının üstündeki koyu örtü,
 *   şeridin parlaklığına (`luma`, 0–1, 7.2) göre. Plan `0.45 + 0.30 · luma`;
 *   beyaz duvar kâğıdında `--text-2` böyle 4.4:1 kalıyor (hedef ≥ 4.5:1),
 *   0.35 ile 5.0:1 (`--text` 9.1:1).
 * Duvar kâğıdı şeridi yoksa (luma yok) örtü tam, saydamlığı yalnızca ayar belirler.
 *
 * 7.9: yazı tipi, hareket ayarı (`data-motion`, global.css) ve dock'ta boyut (zoom).
 */
export function applyTheme(theme: Config["theme"], options: Options = {}) {
  const { luma, systemAccent, zoomDocument } = options;
  const html = document.documentElement;
  const root = html.style;
  const opacity = Math.min(Math.max(theme.opacity, 0.1), 1);
  const alpha = luma === undefined ? 1 : 0.45 + 0.35 * Math.min(Math.max(luma, 0), 1);
  const accent = resolveAccent(theme, systemAccent);
  root.setProperty("--accent", accent);
  root.setProperty("--on-accent-knob", isLightColor(accent) ? "#14181f" : "#ffffff");
  root.setProperty("--glass-opacity", opacity.toFixed(3));
  root.setProperty("--glass-alpha", alpha.toFixed(3));
  // Bulanıklığı olmayan yüzeyler (bildirim kartı, önizleme, görüntüsü alınamayan
  // panel): arkadaki pencerenin yazısı karışmasın diye koyu; düzde opak. Buzlu
  // camlı panel kendi örtüsünü arkasının parlaklığından hesaplar (PopupWindow).
  const flat = theme.glass === "flat";
  root.setProperty("--panel-alpha", (flat ? 1 : Math.max(0.88, opacity)).toFixed(3));
  html.dataset.glass = flat ? "flat" : "glass";
  root.setProperty("--r-bar", `${Math.max(theme.radius, 0)}px`);
  if (theme.font === "system") root.setProperty("--font", SYSTEM_FONT);
  else root.removeProperty("--font");
  html.dataset.motion = theme.reduceMotion ?? "system";
  if (zoomDocument) root.zoom = String(uiScale(theme));
}
