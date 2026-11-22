import { useSyncExternalStore } from "react";
import { enGB, enUS, tr as trLocale } from "date-fns/locale";
import { EN } from "../locales/en";
import { useConfigStore } from "./store";

// Arayüz dili: Türkçe (kaynak metin) ve İngilizce. Metinler kodda Türkçe yazılır,
// `t("…")` seçili dile çevirir; çevirisi olmayan metin Türkçe kalır. Rust'ın
// çizdiği Windows menüleri / bildirimler için karşılığı `src-tauri/src/i18n.rs`.

export type Lang = "tr" | "en";
export type LanguageSetting = "auto" | Lang;
type Vars = Record<string, string | number>;

/** "auto": Windows'un görüntüleme dili Türkçe ise Türkçe, değilse İngilizce. */
const detect = (): Lang =>
  (navigator.language || "").toLowerCase().startsWith("tr") ? "tr" : "en";

export const resolveLang = (setting: string | undefined): Lang =>
  setting === "tr" || setting === "en" ? setting : detect();

let lang: Lang = detect();
const listeners = new Set<() => void>();

function apply(next: Lang) {
  if (next === lang) return;
  lang = next;
  document.documentElement.lang = next;
  listeners.forEach((l) => l());
}

// Ayar değişince (bu pencerede ya da başka bir pencerede) dil de değişir.
useConfigStore.subscribe((s) => apply(resolveLang(s.config?.language)));
// Config bu modülden önce gelmiş olabilir (ör. sıcak yenileme): şimdikini de uygula.
lang = resolveLang(useConfigStore.getState().config?.language);
document.documentElement.lang = lang;

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => listeners.delete(l);
};

/** Dil değişince bileşeni yeniden çizer; pencere kökleri çağırır. */
export const useLang = (): Lang => useSyncExternalStore(subscribe, () => lang);

export const currentLang = (): Lang => lang;

// `{ad}` yer tutuculu anahtarlar: "Sürüm {v}". Çalışma anında oluşan metinleri
// (Rust'tan gelen hata iletileri) eşleştirmek için desenlere de çevrilir.
const patterns: { re: RegExp; names: string[]; en: string }[] = [];
for (const [key, value] of Object.entries(EN)) {
  if (!key.includes("{")) continue;
  const names: string[] = [];
  const source = key
    .split(/\{(\w+)\}/)
    .map((part, i) => {
      if (i % 2 === 0) return part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      names.push(part);
      return "(.+?)";
    })
    .join("");
  patterns.push({ re: new RegExp(`^${source}$`), names, en: value });
}

const fill = (text: string, vars: Vars) =>
  text.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m));

function translate(tr: string): string {
  const hit = EN[tr];
  if (hit !== undefined) return hit;
  for (const p of patterns) {
    const m = p.re.exec(tr);
    if (m)
      return fill(p.en, Object.fromEntries(p.names.map((n, i) => [n, EN[m[i + 1]!] ?? m[i + 1]!])));
  }
  return tr;
}

/**
 * Türkçe kaynak metni seçili dile çevirir. Değişken: `t("Sürüm {v}", { v })`.
 * Rust'tan gelen hata iletileri de (`t(String(e))`) aynı yoldan geçer.
 */
export function t(tr: string, vars?: Vars): string {
  const text = lang === "en" ? translate(tr) : tr;
  return vars ? fill(text, vars) : text;
}

/**
 * Aynı Türkçe sözcüğün farklı anlamları için: `tc("hava", "Kapalı")` sözlükte
 * "Kapalı|hava" anahtarına bakar (Türkçede metnin kendisi görünür).
 */
export function tc(context: string, tr: string): string {
  if (lang !== "en") return tr;
  return EN[`${tr}|${context}`] ?? translate(tr);
}

/** Uygulamanın görünen adı (Rust'taki `app_name` ile aynı). */
export const appName = () => "Dockora";

// --- yerel biçimler ---------------------------------------------------------------

/** Tarih/sayı biçimi: Türkçede tr-TR; İngilizcede Windows'un İngilizce biçimi, yoksa en-GB (24 saat). */
export function locale(): string {
  if (lang === "tr") return "tr-TR";
  return /^en/i.test(navigator.language) ? navigator.language : "en-GB";
}

const dtfCache = new Map<string, Intl.DateTimeFormat>();
/** Seçili dilde `Intl.DateTimeFormat` (önbellekli; dil değişince yenisi). */
export function dtf(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${locale()}|${JSON.stringify(options)}`;
  let f = dtfCache.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(locale(), options);
    dtfCache.set(key, f);
  }
  return f;
}

const nfCache = new Map<string, Intl.NumberFormat>();
export function nf(options: Intl.NumberFormatOptions = {}): Intl.NumberFormat {
  const key = `${locale()}|${JSON.stringify(options)}`;
  let f = nfCache.get(key);
  if (!f) {
    f = new Intl.NumberFormat(locale(), options);
    nfCache.set(key, f);
  }
  return f;
}

/** date-fns için dil nesnesi. */
export const dateFnsLocale = () =>
  lang === "tr" ? trLocale : /^en-us/i.test(navigator.language) ? enUS : enGB;

/** Yüzde yazımı: Türkçede "%62", İngilizcede "62%". */
export const pct = (n: number | string) => (lang === "tr" ? `%${n}` : `${n}%`);

/** Dakikayı "2 sa 5 dk" / "2 h 5 min" yazar. */
export function fmtMinutes(min: number): string {
  const h = Math.floor(min / 60);
  const m = min % 60;
  if (h === 0) return t("{m} dk", { m });
  return m > 0 ? t("{h} sa {m} dk", { h, m }) : t("{h} sa", { h });
}

/** Rust'tan gelen hata iletisini (ya da Error nesnesini) ekranda gösterilecek metne çevirir. */
export const errText = (e: unknown): string => t(e instanceof Error ? e.message : String(e));
