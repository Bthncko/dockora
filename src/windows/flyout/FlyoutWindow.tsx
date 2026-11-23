import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, events, report, type FlyoutShow, type FlyoutsConfig } from "../../lib/ipc";
import { startConfigSync, useConfig } from "../../lib/store";
import { applyTheme, uiScale } from "../../lib/theme";
import { useService } from "../../lib/useService";
import { BrightnessCard, LockCard, MediaCard, NextUpCard, VolumeCard } from "./Cards";
import styles from "./Flyout.module.css";

startConfigSync();

/** Kartın her yanındaki boşluk: gölge + kayma payı (pencere buna dahil boyutlanır). */
const PAD = 24;
const ENTER_MS = 320;
const EXIT_MS = 240;

const EASING: Record<FlyoutsConfig["easing"], string> = {
  linear: "linear",
  sine: "cubic-bezier(0.37, 0, 0.63, 1)",
  quad: "cubic-bezier(0.45, 0, 0.55, 1)",
  cubic: "cubic-bezier(0.65, 0, 0.35, 1)",
};

type Shown = { data: FlyoutShow; seq: number };

function widthOf(data: FlyoutShow, f: FlyoutsConfig) {
  switch (data.kind) {
    case "media":
      return f.media.compact ? 320 : 380;
    case "nextUp":
      return 320;
    case "lockKey":
      return 270;
    case "volume":
      return 320;
    case "brightness":
      return 300;
  }
}

/** Kartın görünme süresi; null: kapat düğmesine kadar. */
function lifeOf(data: FlyoutShow, f: FlyoutsConfig): number | null {
  switch (data.kind) {
    case "media":
      return f.media.alwaysShow ? null : f.media.durationMs;
    case "nextUp":
      return f.nextUp.durationMs;
    case "lockKey":
      return f.lockKeys.durationMs;
    case "volume":
      return f.volume.durationMs;
    case "brightness":
      return f.brightness.durationMs;
  }
}

/**
 * Ekran üstü kartlar (medya, sıradaki, kilit tuşları, ses, parlaklık). Rust
 * `flyout:show` gönderir; kart burada çizilir, süresi dolunca solar ve
 * `flyout_hide` ile pencere gizlenir. Boyut değişince `flyout_layout`.
 */
export function FlyoutWindow() {
  const config = useConfig();
  const systemAccent = useService(commands.getSystemAccent, events.accent);
  const [shown, setShown] = useState<Shown | null>(null);
  const [leaving, setLeaving] = useState(false);
  const [hover, setHover] = useState(false);
  const frame = useRef<HTMLDivElement>(null);
  const seq = useRef(0);
  const zoom = config ? uiScale(config.theme) : 1;
  const f = config?.flyouts;

  useEffect(() => {
    if (config) applyTheme(config.theme, { systemAccent });
  }, [config?.theme, systemAccent]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const un = listen<FlyoutShow>(events.flyoutShow, (e) => {
      setShown({ data: e.payload, seq: ++seq.current });
      setLeaving(false);
    });
    return () => {
      un.then((off) => off());
    };
  }, []);

  const speed = f ? Math.max(0, f.animationSpeed) : 1;
  const exitMs = speed > 0 ? Math.round(EXIT_MS / speed) : 0;
  const enterMs = speed > 0 ? Math.round(ENTER_MS / speed) : 0;

  // Süre: yeni olay yeniden başlatır, üstündeyken durur.
  const life = shown && f ? lifeOf(shown.data, f) : null;
  const shownSeq = shown?.seq;
  useEffect(() => {
    if (shownSeq === undefined || life === null || hover) return;
    const id = window.setTimeout(() => setLeaving(true), life);
    return () => window.clearTimeout(id);
  }, [shownSeq, life, hover]);

  // Solma bitince pencere gizlenir.
  useEffect(() => {
    if (!leaving) return;
    const id = window.setTimeout(() => {
      setShown(null);
      setLeaving(false);
      setHover(false);
      commands.flyoutHold(false).catch(report("flyout_hold"));
      commands.flyoutHide().catch(report("flyout_hide"));
    }, exitMs + 20);
    return () => window.clearTimeout(id);
  }, [leaving, exitMs]);

  // Ayar flyout'ları kapatırsa açık kart gider.
  const enabled = f?.enabled;
  useEffect(() => {
    if (enabled !== false || shownSeq === undefined) return;
    const id = window.setTimeout(() => setLeaving(true));
    return () => window.clearTimeout(id);
  }, [enabled, shownSeq]);

  useEffect(() => {
    if (shownSeq === undefined) return;
    commands.flyoutHold(hover).catch(report("flyout_hold"));
  }, [hover]); // eslint-disable-line react-hooks/exhaustive-deps

  // İçerik boyu → pencere boyu (tam kutu, CSS px).
  const visible = shown !== null;
  const kind = shown?.data.kind;
  useLayoutEffect(() => {
    const el = frame.current;
    if (!el) return;
    const send = () =>
      commands
        .flyoutLayout(el.offsetWidth * zoom, el.offsetHeight * zoom)
        .catch(report("flyout_layout"));
    send();
    const ro = new ResizeObserver(send);
    ro.observe(el);
    return () => ro.disconnect();
  }, [visible, kind, zoom]);

  if (!shown || !f || !config) return null;
  const { data } = shown;
  const bottom = f.placement.anchor.startsWith("bottom");
  const vars = {
    "--fly-enter": `${enterMs}ms`,
    "--fly-exit": `${exitMs}ms`,
    "--fly-ease": EASING[f.easing] ?? EASING.cubic,
    "--fly-slide": `${bottom ? 18 : -18}px`,
    padding: PAD,
    width: widthOf(data, f) + PAD * 2,
    zoom,
  } as CSSProperties;

  return (
    <div
      ref={frame}
      className={styles.frame}
      style={vars}
      data-leaving={leaving || undefined}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
    >
      <div className={styles.card} data-solid={!f.glass || undefined} data-kind={data.kind}>
        {data.kind === "media" && (
          <MediaCard
            cfg={f.media}
            albumAccent={f.albumAccent}
            trigger={data.trigger}
            aboveMedia={f.volume.aboveMedia}
            onClose={() => setLeaving(true)}
          />
        )}
        {data.kind === "nextUp" && (
          <NextUpCard title={data.title} artist={data.artist} art={data.art} />
        )}
        {data.kind === "lockKey" && (
          <LockCard cfg={f.lockKeys} lockKey={data.key} on={data.on} seq={shown.seq} />
        )}
        {data.kind === "volume" && (
          <VolumeCard
            cfg={f.volume}
            init={{ level: data.level, muted: data.muted, device: data.device }}
          />
        )}
        {data.kind === "brightness" && <BrightnessCard level={data.level} />}
      </div>
    </div>
  );
}
