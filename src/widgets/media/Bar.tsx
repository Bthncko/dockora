import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { commands, events, report, type MediaState } from "../../lib/ipc";
import { useService } from "../../lib/useService";
import { togglePopup } from "../../lib/popup";
import { NextIcon, PauseIcon, PlayIcon, PrevIcon } from "../icons";
import { useCoverTint } from "./useCoverTint";
import { IconButton, Tile } from "../ui";
import styles from "./Bar.module.css";
import { errText, t } from "../../lib/i18n";

/** Kayan yazı hızı (px/sn). */
const MARQUEE_SPEED = 30;

type Shown = NonNullable<MediaState>;

/**
 * Oturum kapanınca karo hemen kaybolmaz: son durum çıkış animasyonu bitene
 * kadar gösterilir. (Türetilmiş durum: render sırasında güncellenir.)
 */
function useLingering(media: MediaState | undefined) {
  const [state, setState] = useState<{ last: Shown | null; leaving: boolean }>({
    last: null,
    leaving: false,
  });
  if (media && media !== state.last) setState({ last: media, leaving: false });
  if (!media && state.last && !state.leaving) setState({ last: state.last, leaving: true });
  const done = () => setState({ last: null, leaving: false });
  return [state.last, state.leaving, done] as const;
}

/** Şarkı değişince eski etiket solarken yenisi belirir. */
function useCrossfade(label: string) {
  const [state, setState] = useState<{ cur: string; prev: string | null }>({
    cur: label,
    prev: null,
  });
  if (label !== state.cur) setState({ cur: label, prev: state.cur });
  const clear = () => setState((s) => ({ ...s, prev: null }));
  return [state.cur, state.prev, clear] as const;
}

/** Tekerlek adımı (yüzde) ve göstergenin kalma süresi. */
const WHEEL_STEP = 5;
const OSD_MS = 1200;

/**
 * Spotify çalarken (ve bağlıyken) tekerlek Spotify sesini değiştirir. Hızlı
 * çevirmede adımlar birikir; aynı anda tek istek gider.
 */
function useWheelVolume(enabled: boolean) {
  const [osd, setOsd] = useState<{ text: string; key: number } | null>(null);
  const pending = useRef(0);
  const busy = useRef(false);
  const hide = useRef(0);
  useEffect(() => () => window.clearTimeout(hide.current), []);

  const show = (text: string) => {
    setOsd((o) => ({ text, key: (o?.key ?? 0) + 1 }));
    window.clearTimeout(hide.current);
    hide.current = window.setTimeout(() => setOsd(null), OSD_MS);
  };

  const flush = () => {
    if (busy.current || pending.current === 0) return;
    const delta = pending.current;
    pending.current = 0;
    busy.current = true;
    commands
      .spotifyVolumeStep(delta)
      .then((v) => show(t("Spotify sesi %{v}", { v })))
      .catch((e) => show(errText(e)))
      .finally(() => {
        busy.current = false;
        flush();
      });
  };

  const onWheel = (e: React.WheelEvent) => {
    if (!enabled || e.deltaY === 0) return;
    pending.current += e.deltaY < 0 ? WHEEL_STEP : -WHEEL_STEP;
    flush();
  };

  return { osd, onWheel };
}

export function MediaBar() {
  const live = useService(commands.getMedia, events.media);
  const spotify = useService(commands.spotifyStatus, events.spotify);
  const [media, leaving, gone] = useLingering(live);
  const tint = useCoverTint(media?.artwork ?? null);
  const wheel = useWheelVolume(!!spotify?.connected && !!media && /spotify/i.test(media.sourceApp));

  if (!media) return null;

  return (
    <Tile
      variant="tinted"
      color={tint}
      className={styles.card}
      data-leaving={leaving || undefined}
      onWheel={wheel.onWheel}
      // Kontrollerin dışına tıklama: müzik paneli (sıradakiler).
      onClick={(e) => {
        if (!(e.target as Element).closest("button")) togglePopup("media", e.currentTarget);
      }}
      onAnimationEnd={(e) => {
        if (leaving && e.target === e.currentTarget) gone();
      }}
    >
      <Title title={media.title} artist={media.artist} />
      {wheel.osd && (
        <span key={wheel.osd.key} className={styles.osd} role="status">
          {wheel.osd.text}
        </span>
      )}
      <div className={styles.controls}>
        <IconButton
          className={styles.control}
          title={t("Önceki")}
          aria-label={t("Önceki")}
          disabled={!media.canPrev}
          onClick={() => commands.mediaPrev().catch(report("media_prev"))}
        >
          <PrevIcon size={14} />
        </IconButton>
        <IconButton
          className={styles.control}
          title={media.playing ? t("Duraklat") : t("Oynat")}
          aria-label={media.playing ? t("Duraklat") : t("Oynat")}
          onClick={() => commands.mediaToggle().catch(report("media_toggle"))}
        >
          {/* Durum değişince simge çapraz geçişle değişir (7.7). */}
          <span key={media.playing ? "pause" : "play"} className={styles.swap}>
            {media.playing ? <PauseIcon size={17} /> : <PlayIcon size={17} />}
          </span>
        </IconButton>
        <IconButton
          className={styles.control}
          title={t("Sonraki")}
          aria-label={t("Sonraki")}
          disabled={!media.canNext}
          onClick={() => commands.mediaNext().catch(report("media_next"))}
        >
          <NextIcon size={14} />
        </IconButton>
      </div>
    </Tile>
  );
}

/** Şarkı ile sanatçıyı tek anahtarda tutar (başlıkta geçemeyecek ayırıcıyla). */
const SEP = "\u0000";

function Title({ title, artist }: { title: string; artist: string }) {
  const full = artist ? `${title} · ${artist}` : title;
  const [cur, prev, clearPrev] = useCrossfade(`${title}${SEP}${artist}`);
  const boxRef = useRef<HTMLDivElement>(null);
  const textRef = useRef<HTMLSpanElement>(null);
  const [overflow, setOverflow] = useState(0);

  // Sığmayan başlık: kenarlar soluklaşır, hover'da yavaşça kayar.
  useLayoutEffect(() => {
    const box = boxRef.current;
    const text = textRef.current;
    if (!box || !text) return;
    const measure = () => setOverflow(Math.max(0, Math.ceil(text.scrollWidth - box.clientWidth)));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(box);
    return () => ro.disconnect();
  }, [cur]);

  const style = {
    "--marquee": `${-overflow}px`,
    "--marquee-time": `${Math.max(2, overflow / MARQUEE_SPEED)}s`,
  } as CSSProperties;

  return (
    <div
      ref={boxRef}
      className={styles.titleBox}
      data-overflow={overflow > 0 || undefined}
      title={full}
      style={style}
    >
      {prev !== null && (
        <span key={`prev:${prev}`} className={styles.label} data-out onAnimationEnd={clearPrev}>
          <Label text={prev} />
        </span>
      )}
      <span key={cur} ref={textRef} className={styles.label} data-in={prev !== null || undefined}>
        <Label text={cur} />
      </span>
    </div>
  );
}

/** "Şarkı · Sanatçı": şarkı kalın, sanatçı soluk. */
function Label({ text }: { text: string }) {
  const [title, artist] = text.split(SEP);
  return (
    <>
      <b>{title}</b>
      {artist && <span className={styles.artist}> · {artist}</span>}
    </>
  );
}
