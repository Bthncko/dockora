import { useEffect, useRef, useState, type CSSProperties } from "react";
import {
  commands,
  events,
  report,
  type AudioSession,
  type FlyoutLockKey,
  type FlyoutsConfig,
  type MediaState,
  type MediaTimeline,
  type RepeatMode,
} from "../../lib/ipc";
import { pct, t } from "../../lib/i18n";
import { useService } from "../../lib/useService";
import { rgbCss, type Rgb } from "../../lib/color";
import {
  ChevronIcon,
  CloseIcon,
  KeyboardIcon,
  LockIcon,
  MusicIcon,
  NextIcon,
  PauseIcon,
  PlayIcon,
  PrevIcon,
  RepeatIcon,
  ShuffleIcon,
  SpeakerIcon,
  SunIcon,
} from "../../widgets/icons";
import { artAccent } from "./accent";
import { Slider } from "./Slider";
import styles from "./Flyout.module.css";

const KNOWN: Record<string, string> = {
  msedge: "Microsoft Edge",
  chrome: "Google Chrome",
  firefox: "Firefox",
  vlc: "VLC",
  zunemusic: "Medya Oynatıcı",
};

/** "SpotifyAB.SpotifyMusic_…!Spotify" / "chrome.exe" → okunur oynatıcı adı. */
function playerName(id: string): string {
  let n = id.includes("!") ? id.slice(id.lastIndexOf("!") + 1) : id;
  n = n.replace(/\.exe$/i, "").split(/[\\/]/).pop() ?? n;
  if (n.includes(".") && !id.includes("!")) n = n.split(".").pop() ?? n;
  const k = KNOWN[n.toLowerCase()];
  if (k) return k === "Medya Oynatıcı" ? t(k) : k;
  return n.charAt(0).toUpperCase() + n.slice(1);
}

const fmt = (ms: number) => {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};

/** Sürüklerken her harekette değil ~60 ms'de bir çağırır. */
function useThrottle<A extends unknown[]>(fn: (...a: A) => void, ms = 60) {
  const last = useRef(0);
  return (...a: A) => {
    const now = performance.now();
    if (now - last.current < ms) return;
    last.current = now;
    fn(...a);
  };
}

/**
 * Sürükleme bırakıldıktan `ms` sonrasına kadar açık kalan bayrak (sunucudan gelen
 * eski değer sürüklenen değerin üstüne yazılmasın). Yeni sürükleme önceki süreyi
 * iptal eder; kaldırılırken zamanlayıcı temizlenir.
 */
function useHold(ms: number) {
  const flag = useRef(false);
  const timer = useRef(0);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return {
    flag,
    hold() {
      window.clearTimeout(timer.current);
      flag.current = true;
    },
    release() {
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => (flag.current = false), ms);
    },
  };
}

// --- ses satırı ------------------------------------------------------------------

type Vol = { level: number; muted: boolean; device: string };

/** Canlı ses durumu: başlangıç olayın değeri, sonra `audio` olayı. */
function useVolume(init: Vol): [Vol, (v: Vol) => void] {
  const [vol, setVol] = useState(init);
  const audio = useService(commands.getAudio, events.audio);
  // Olay / ses durumu değişince değeri yeniden al (render sırasında türetme).
  const [seen, setSeen] = useState({ init, audio });
  if (
    seen.init.level !== init.level ||
    seen.init.muted !== init.muted ||
    seen.init.device !== init.device
  ) {
    setSeen({ ...seen, init });
    setVol({ level: init.level, muted: init.muted, device: init.device });
  } else if (seen.audio !== audio) {
    setSeen({ ...seen, audio });
    if (audio)
      setVol({ level: Math.round(audio.volume), muted: audio.muted, device: audio.device });
  }
  return [vol, setVol];
}

function VolumeRow({ init }: { init: Vol }) {
  const [vol, setVol] = useVolume(init);
  const send = useThrottle((v: number) => commands.setVolume(v).catch(report("set_volume")));
  return (
    <div className={styles.volRow}>
      <button
        type="button"
        className={styles.iconBtn}
        aria-label={t("Sessiz")}
        aria-pressed={vol.muted}
        onClick={() => {
          setVol({ ...vol, muted: !vol.muted });
          commands.setMute(!vol.muted).catch(report("set_mute"));
        }}
      >
        <SpeakerIcon level={vol.level} muted={vol.muted || vol.level === 0} size={18} />
      </button>
      <Slider
        label={t("Ses")}
        value={vol.muted ? 0 : vol.level / 100}
        onChange={(v) => {
          setVol({ ...vol, level: Math.round(v * 100), muted: false });
          send(v);
        }}
        onCommit={(v) => commands.setVolume(v).catch(report("set_volume"))}
      />
      <span className={styles.pct}>{vol.muted ? t("Sessiz") : pct(vol.level)}</span>
    </div>
  );
}

// --- ses kartı (+ karıştırıcı) -------------------------------------------------

export function VolumeCard({ cfg, init }: { cfg: FlyoutsConfig["volume"]; init: Vol }) {
  const [open, setOpen] = useState(false);
  const [vol] = useVolume(init);
  return (
    <div className={styles.volCard}>
      <VolumeRow init={init} />
      <div className={styles.subRow}>
        <span className={styles.device}>{vol.device || t("Ses")}</span>
        {cfg.mixer && (
          <button
            type="button"
            className={styles.chev}
            data-open={open || undefined}
            aria-expanded={open}
            aria-label={t("Uygulama sesleri")}
            onClick={() => setOpen(!open)}
          >
            <ChevronIcon size={14} />
          </button>
        )}
      </div>
      {cfg.mixer && open && <Mixer highlight={cfg.highlightActive} />}
    </div>
  );
}

function Mixer({ highlight }: { highlight: boolean }) {
  const [rows, setRows] = useState<AudioSession[] | null>(null);
  const { flag: editing, hold, release } = useHold(400);
  useEffect(() => {
    let alive = true;
    const load = () => {
      if (editing.current) return;
      commands
        .audioSessions()
        .then((r) => alive && !editing.current && setRows(r))
        .catch(report("audio_sessions"));
    };
    load();
    const id = window.setInterval(load, 1500);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [editing]);
  const patch = (id: string, p: Partial<AudioSession>) =>
    setRows((rs) => rs?.map((r) => (r.id === id ? { ...r, ...p } : r)) ?? rs);
  const send = useThrottle((id: string, v: number) =>
    commands.setSessionVolume(id, v).catch(report("set_session_volume")),
  );
  if (!rows) return null;
  if (rows.length === 0) return <p className={styles.note}>{t("Ses çalan uygulama yok")}</p>;
  return (
    <div className={styles.mixer}>
      {rows.map((r) => (
        <div
          key={r.id}
          className={styles.session}
          data-active={(highlight && r.active) || undefined}
        >
          {r.icon ? (
            <img className={styles.appIcon} src={r.icon} alt="" draggable={false} />
          ) : (
            <span className={styles.appIcon} data-fallback>
              <MusicIcon size={12} />
            </span>
          )}
          <span className={styles.appName}>
            {r.name}
            {highlight && r.active && <i className={styles.dot} />}
          </span>
          <Slider
            thin
            label={r.name}
            value={r.muted ? 0 : r.volume / 100}
            onChange={(v) => {
              hold();
              patch(r.id, { volume: Math.round(v * 100), muted: false });
              send(r.id, Math.round(v * 100));
            }}
            onCommit={(v) => {
              commands
                .setSessionVolume(r.id, Math.round(v * 100))
                .catch(report("set_session_volume"));
              release();
            }}
          />
          <button
            type="button"
            className={styles.iconBtn}
            data-small
            aria-label={t("Sessiz")}
            aria-pressed={r.muted}
            onClick={() => {
              patch(r.id, { muted: !r.muted });
              commands.setSessionMute(r.id, !r.muted).catch(report("set_session_mute"));
            }}
          >
            <SpeakerIcon level={r.volume} muted={r.muted} size={15} />
          </button>
        </div>
      ))}
    </div>
  );
}

// --- parlaklık ---------------------------------------------------------------

export function BrightnessCard({ level }: { level: number }) {
  const [v, setV] = useState(level);
  const [seen, setSeen] = useState(level);
  if (seen !== level) {
    setSeen(level);
    setV(level);
  }
  const send = useThrottle((p: number) =>
    commands.setBrightness(p).catch(report("set_brightness")),
  );
  return (
    <div className={styles.volRow}>
      <span className={styles.iconBtn} data-static>
        <SunIcon size={18} />
      </span>
      <Slider
        label={t("Parlaklık")}
        value={v / 100}
        onChange={(x) => {
          setV(Math.round(x * 100));
          send(Math.round(x * 100));
        }}
        onCommit={(x) =>
          commands.setBrightness(Math.round(x * 100)).catch(report("set_brightness"))
        }
      />
      <span className={styles.pct}>{v}%</span>
    </div>
  );
}

// --- kilit tuşları ---------------------------------------------------------------

function lockText(key: FlyoutLockKey, on: boolean) {
  switch (key) {
    case "caps":
      return on ? t("Caps Lock açık") : t("Caps Lock kapalı");
    case "num":
      return on ? t("Num Lock açık") : t("Num Lock kapalı");
    case "scroll":
      return on ? t("Scroll Lock açık") : t("Scroll Lock kapalı");
    case "insert":
      return on ? t("Üzerine yazma modu") : t("Ekleme modu");
  }
}

export function LockCard({
  cfg,
  lockKey,
  on,
  seq,
}: {
  cfg: FlyoutsConfig["lockKeys"];
  lockKey: FlyoutLockKey;
  on: boolean;
  seq: number;
}) {
  return (
    <div className={styles.lock} data-bold={cfg.bold || undefined}>
      <span
        key={cfg.animated ? seq : "static"}
        className={styles.lockBadge}
        data-on={on || undefined}
        data-animated={cfg.animated || undefined}
      >
        {lockKey === "insert" ? <KeyboardIcon size={20} /> : <LockIcon size={20} />}
      </span>
      <span className={styles.lockText}>{lockText(lockKey, on)}</span>
    </div>
  );
}

// --- sıradaki -------------------------------------------------------------------

function Art({ src, size }: { src?: string | null; size: number }) {
  return src ? (
    <img
      className={styles.art}
      src={src}
      alt=""
      draggable={false}
      style={{ width: size, height: size }}
    />
  ) : (
    <span className={styles.art} data-fallback style={{ width: size, height: size }}>
      <MusicIcon size={size * 0.45} />
    </span>
  );
}

export function NextUpCard({
  title,
  artist,
  art,
}: {
  title: string;
  artist: string;
  art?: string;
}) {
  return (
    <div className={styles.nextUp}>
      <Art src={art} size={52} />
      <div className={styles.text}>
        <span className={styles.kicker}>{t("Sıradaki")}</span>
        <span className={styles.title}>{title}</span>
        <span className={styles.artist}>{artist}</span>
      </div>
    </div>
  );
}

// --- medya -------------------------------------------------------------------------

const NEXT_REPEAT: Record<RepeatMode, RepeatMode> = { off: "list", list: "track", track: "off" };

export function MediaCard({
  cfg,
  albumAccent,
  trigger,
  aboveMedia,
  onClose,
}: {
  cfg: FlyoutsConfig["media"];
  albumAccent: boolean;
  trigger: "mediaKey" | "volumeKey";
  aboveMedia: boolean;
  onClose: () => void;
}) {
  const media = useService<MediaState>(commands.getMedia, events.media) ?? null;
  const audio = useService(commands.getAudio, events.audio);
  const [art, setArt] = useState<Rgb | null>(null);
  const artSrc = media?.artwork ?? null;
  useEffect(() => {
    let alive = true;
    artAccent(artSrc).then((c) => alive && setArt(c));
    return () => {
      alive = false;
    };
  }, [artSrc]);

  // Zaman çizelgesi: kart görünürken ~500 ms'de bir.
  const [tl, setTl] = useState<MediaTimeline>(null);
  const { flag: seeking, hold: holdSeek, release: releaseSeek } = useHold(600);
  useEffect(() => {
    if (!cfg.showSeekbar) return;
    let alive = true;
    const poll = () =>
      commands
        .mediaTimeline()
        .then((x) => alive && !seeking.current && setTl(x))
        .catch(report("media_timeline"));
    poll();
    const id = window.setInterval(poll, 500);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [cfg.showSeekbar, seeking]);
  const [dragPos, setDragPos] = useState<number | null>(null);

  const compact = cfg.compact;
  const useArt = art && artSrc ? art : null;
  const style = {
    ...(albumAccent && useArt
      ? { "--accent": rgbCss(useArt), "--accent-soft": `rgb(${useArt.join(" ")} / 0.2)` }
      : null),
    "--glow": useArt ? rgbCss(useArt) : "var(--accent)",
  } as CSSProperties;

  const volRow =
    trigger === "volumeKey" && audio ? (
      <div className={styles.mediaVol}>
        <VolumeRow
          init={{
            level: Math.round(audio.volume),
            muted: audio.muted,
            device: audio.device,
          }}
        />
      </div>
    ) : null;

  const repeat = media?.repeat ?? null;
  const dur = tl?.durationMs ?? 0;
  const pos = dragPos ?? tl?.positionMs ?? 0;
  const bg = cfg.background;
  const artSize = compact ? 46 : 64;

  return (
    <div
      className={styles.media}
      data-compact={compact || undefined}
      data-center={cfg.centerText || undefined}
      style={style}
    >
      {bg === "blur" && artSrc && (
        <img className={styles.bgBlur} src={artSrc} alt="" draggable={false} />
      )}
      {bg === "coverGlow" && artSrc && (
        <img className={styles.bgCover} src={artSrc} alt="" draggable={false} />
      )}
      {bg === "risingGlow" && <div className={styles.bgRising} />}
      {cfg.alwaysShow && (
        <button type="button" className={styles.close} aria-label={t("Kapat")} onClick={onClose}>
          <CloseIcon size={10} />
        </button>
      )}
      <div className={styles.mediaBody}>
        {aboveMedia && volRow}
        <div className={styles.mediaTop}>
          <Art src={artSrc} size={artSize} />
          <div className={styles.text}>
            {cfg.showPlayer && !compact && media?.sourceApp && (
              <span className={styles.kicker}>{playerName(media.sourceApp)}</span>
            )}
            <span className={styles.title}>
              {media ? media.title || t("Bilinmeyen parça") : t("Çalan medya yok")}
            </span>
            {media && <span className={styles.artist}>{media.artist}</span>}
          </div>
        </div>
        {media && cfg.showSeekbar && tl && dur > 0 && (
          <div className={styles.seek}>
            <span className={styles.time}>{fmt(pos)}</span>
            <Slider
              thin
              label={t("Konum")}
              value={pos / dur}
              onChange={(v) => {
                holdSeek();
                setDragPos(v * dur);
              }}
              onCommit={(v) => {
                commands.mediaSeek(Math.round(v * dur)).catch(report("media_seek"));
                setTl({ ...tl, positionMs: v * dur });
                setDragPos(null);
                releaseSeek();
              }}
            />
            <span className={styles.time}>{fmt(dur)}</span>
          </div>
        )}
        {media && (
          <div className={styles.controls}>
            {cfg.showShuffle && !compact && media.shuffle !== null && (
              <button
                type="button"
                className={styles.ctl}
                data-small
                data-on={media.shuffle || undefined}
                aria-label={t("Karıştır")}
                aria-pressed={media.shuffle}
                onClick={() =>
                  commands.mediaSetShuffle(!media.shuffle).catch(report("media_set_shuffle"))
                }
              >
                <ShuffleIcon size={16} />
              </button>
            )}
            <button
              type="button"
              className={styles.ctl}
              disabled={!media.canPrev}
              aria-label={t("Önceki")}
              onClick={() => commands.mediaPrev().catch(report("media_prev"))}
            >
              <PrevIcon size={20} />
            </button>
            <button
              type="button"
              className={styles.ctl}
              data-main
              aria-label={media.playing ? t("Duraklat") : t("Oynat")}
              onClick={() => commands.mediaToggle().catch(report("media_toggle"))}
            >
              {media.playing ? <PauseIcon size={22} /> : <PlayIcon size={22} />}
            </button>
            <button
              type="button"
              className={styles.ctl}
              disabled={!media.canNext}
              aria-label={t("Sonraki")}
              onClick={() => commands.mediaNext().catch(report("media_next"))}
            >
              <NextIcon size={20} />
            </button>
            {cfg.showRepeat && !compact && repeat !== null && (
              <button
                type="button"
                className={styles.ctl}
                data-small
                data-on={repeat !== "off" || undefined}
                aria-label={t("Tekrar")}
                onClick={() =>
                  commands.mediaSetRepeat(NEXT_REPEAT[repeat]).catch(report("media_set_repeat"))
                }
              >
                <RepeatIcon one={repeat === "track"} size={16} />
              </button>
            )}
          </div>
        )}
        {!aboveMedia && volRow}
      </div>
    </div>
  );
}
