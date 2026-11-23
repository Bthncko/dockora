import { useRef, useState, type CSSProperties, type PointerEvent } from "react";
import {
  commands,
  report,
  type Config,
  type NotifyAnchor,
  type NotifyPosition,
} from "../../lib/ipc";
import { updateConfig } from "../../lib/store";
import { uiScale } from "../../lib/theme";
import { Card, Row, Slider } from "./controls";
import styles from "./NotifyPlacement.module.css";
import settings from "./Settings.module.css";
import { t } from "../../lib/i18n";

/** Kart (NotifyWindow → WIDTH) ve tek kartlık yığının yaklaşık boyu (CSS piksel). */
const CARD_W = 360;
const CARD_H = 68;
/** Dock'un ayırdığı şerit (placement.rs → BAR_HEIGHT). */
const BAR_H = 64;
/** Kaydırıcı bittikten sonra yazılır (Görünüm sayfasıyla aynı). */
const DRAG_DELAY_MS = 120;
/** Ayar yazıldıktan sonra örnek kart (yazma `updateConfig` içinde zamanlayıcıyla). */
const TEST_DELAY_MS = 200;

const NAMES: Record<NotifyAnchor, string> = {
  get topLeft() {
    return t("Sol üst");
  },
  get topCenter() {
    return t("Üst orta");
  },
  get topRight() {
    return t("Sağ üst");
  },
  get bottomLeft() {
    return t("Sol alt");
  },
  get bottomCenter() {
    return t("Alt orta");
  },
  get bottomRight() {
    return t("Sağ alt");
  },
};

type Screen = { w: number; h: number; card: { w: number; h: number } };
type Point = { x: number; y: number };

/** Kartın sol üst köşesi (mantıksal piksel); Rust `notify.rs → Spot::origin` ile aynı. */
function origin(pos: NotifyPosition, s: Screen): Point {
  const { w, h } = s.card;
  const x = pos.anchor.endsWith("Left")
    ? pos.x
    : pos.anchor.endsWith("Center")
      ? (s.w - w) / 2
      : s.w - pos.x - w;
  const y = pos.anchor.startsWith("bottom") ? s.h - pos.y - h : pos.y;
  return { x: clamp(x, 0, s.w - w), y: clamp(y, 0, s.h - h) };
}

/** Serbest konum → en yakın köşe / kenar ortası ve o kenarlardan uzaklık. */
function snap(p: Point, s: Screen, keepX: number): NotifyPosition {
  const { w, h } = s.card;
  const cx = p.x + w / 2;
  const side = cx < s.w / 3 ? "Left" : cx > (s.w * 2) / 3 ? "Right" : "Center";
  const bottom = p.y + h / 2 > s.h / 2;
  const x = side === "Left" ? p.x : side === "Right" ? s.w - p.x - w : keepX;
  const y = bottom ? s.h - p.y - h : p.y;
  return {
    anchor: `${bottom ? "bottom" : "top"}${side}` as NotifyAnchor,
    x: Math.max(0, Math.round(x)),
    y: Math.max(0, Math.round(y)),
  };
}

function clamp(v: number, min: number, max: number) {
  return Math.min(Math.max(v, min), Math.max(min, max));
}

const percentOf = (v: number, of: number) => `${(v / of) * 100}%`;

/**
 * Ayarlar → Genel: bildirim kartlarının yeri. Ekranın küçük bir kopyası:
 * kart sürüklenir (ya da ekranda bir yere tıklanır), bırakınca en yakın köşeye /
 * kenar ortasına oturur ve gerçek ekranda örnek kart orada görünür. Uzaklıklar
 * kaydırıcılarla piksel piksel ayarlanır.
 */
export function NotifyPlacement({
  config,
  value,
  onChange,
  disabled: disabledProp,
  title,
  card = { w: CARD_W, h: CARD_H },
  onTest,
  hint,
  cardLabel,
}: {
  config: Config;
  /** Verilmezse bildirim konumu (varsayılan davranış). */
  value?: NotifyPosition;
  onChange?: (next: NotifyPosition, delay?: number) => void;
  disabled?: boolean;
  title?: string;
  card?: { w: number; h: number };
  /** Verilmezse "Göster" düğmesi ve örnek kart yok. */
  onTest?: (() => void) | null;
  hint?: string;
  cardLabel?: string;
}) {
  const custom = value !== undefined;
  const pos = value ?? config.notifyPosition;
  const zoom = uiScale(config.theme);
  const disabled = disabledProp ?? !config.ownNotifications;
  const center = pos.anchor.endsWith("Center");
  const bottom = pos.anchor.startsWith("bottom");

  // Kartların durabileceği alan: ekran, dock'un üstüne kadar (mantıksal piksel).
  const sw = window.screen.width || 1920;
  const sh = window.screen.height || 1080;
  const s: Screen = {
    w: sw,
    h: sh - BAR_H * zoom,
    card: { w: card.w * zoom, h: card.h * zoom },
  };

  const screenRef = useRef<HTMLDivElement>(null);
  const grab = useRef<Point | null>(null);
  const [drag, setDrag] = useState<Point | null>(null);

  const set = (next: NotifyPosition, delay = 0) =>
    onChange
      ? onChange(next, delay)
      : updateConfig((c) => ({ ...c, notifyPosition: next }), delay);

  const show = () => {
    if (custom) onTest?.();
    else commands.notifyTest(true).catch(report("notify_test"));
  };
  const canTest = !custom || !!onTest;

  const place = (next: NotifyPosition) => {
    set(next);
    if (canTest) window.setTimeout(show, TEST_DELAY_MS);
  };

  /** İmleç → ekran (mantıksal piksel). */
  const toScreen = (e: PointerEvent): Point | null => {
    const r = screenRef.current?.getBoundingClientRect();
    if (!r || r.width === 0) return null;
    const k = sw / r.width;
    return { x: (e.clientX - r.left) * k, y: (e.clientY - r.top) * k };
  };

  const free = (p: Point): Point => ({
    x: clamp(p.x, 0, s.w - s.card.w),
    y: clamp(p.y, 0, s.h - s.card.h),
  });

  const onCardDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    const p = toScreen(e);
    if (!p) return;
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    const at = origin(pos, s);
    grab.current = { x: p.x - at.x, y: p.y - at.y };
    setDrag(at);
  };

  const onCardMove = (e: PointerEvent) => {
    const p = toScreen(e);
    if (!p || !grab.current) return;
    setDrag(free({ x: p.x - grab.current.x, y: p.y - grab.current.y }));
  };

  const onCardUp = () => {
    if (drag) place(snap(drag, s, pos.x));
    grab.current = null;
    setDrag(null);
  };

  // Ekranda boş bir yere tıklama: kart oraya (ortası imleçte).
  const onScreenDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    const p = toScreen(e);
    if (!p) return;
    place(snap(free({ x: p.x - s.card.w / 2, y: p.y - s.card.h / 2 }), s, pos.x));
  };

  const at = drag ?? origin(pos, s);
  const ghost = drag ? origin(snap(drag, s, pos.x), s) : null;
  const box = (p: Point): CSSProperties => ({
    left: percentOf(p.x, s.w),
    top: percentOf(p.y, sh),
    width: percentOf(s.card.w, s.w),
    height: percentOf(s.card.h, sh),
  });

  const target = drag ? snap(drag, s, pos.x).anchor : pos.anchor;

  return (
    <>
      <h3 className={settings.subTitle}>{title ?? t("Bildirim konumu")}</h3>
      <Card>
        <Row
          label={NAMES[target]}
          hint={
            hint ??
            (disabled
              ? t(
                  "Yalnızca dock tarzı bildirimlerde; yukarıdan açınca buradan yerini seçebilirsin.",
                )
              : t(
                  "Kartı sürükle ya da ekranda bir yere tıkla: en yakın köşeye ya da kenarın ortasına oturur, örnek kart gerçek yerinde görünür.",
                ))
          }
        >
          {canTest && (
            <button type="button" className={settings.button} disabled={disabled} onClick={show}>
              {t("Göster")}
            </button>
          )}
        </Row>
        <div className={styles.stage} data-disabled={disabled || undefined}>
          <div
            ref={screenRef}
            className={styles.screen}
            style={{ aspectRatio: `${sw} / ${sh}` }}
            onPointerDown={onScreenDown}
            role="group"
            aria-label={t("Ekran önizlemesi")}
          >
            <div
              className={styles.dock}
              style={{ top: percentOf(s.h + 4 * zoom, sh), bottom: percentOf(8 * zoom, sh) }}
            />
            {ghost && <div className={styles.ghost} style={box(ghost)} />}
            <div
              className={styles.card}
              data-dragging={drag ? "" : undefined}
              style={box(at)}
              onPointerDown={onCardDown}
              onPointerMove={onCardMove}
              onPointerUp={onCardUp}
              onPointerCancel={onCardUp}
              role="img"
              aria-label={
                cardLabel
                  ? `${cardLabel}: ${NAMES[pos.anchor]}`
                  : t("Bildirim kartı: {name}", { name: NAMES[pos.anchor] })
              }
            >
              <span className={styles.cardIcon} />
              <span className={styles.cardLines}>
                <span />
                <span />
              </span>
            </div>
          </div>
        </div>
        <Row
          label={t("Yatay uzaklık")}
          hint={
            center
              ? t("Ortadayken kart tam ortalanır.")
              : pos.anchor.endsWith("Left")
                ? t("Ekranın sol kenarından.")
                : t("Ekranın sağ kenarından.")
          }
        >
          <Slider
            label={t("Yatay uzaklık")}
            min={0}
            max={Math.round(s.w / 2)}
            value={Math.min(pos.x, Math.round(s.w / 2))}
            format={(v) => `${v} px`}
            disabled={disabled || center}
            onChange={(x) => set({ ...pos, x }, DRAG_DELAY_MS)}
          />
        </Row>
        <Row
          label={t("Dikey uzaklık")}
          hint={bottom ? t("Dock'un üst kenarından.") : t("Ekranın üst kenarından.")}
        >
          <Slider
            label={t("Dikey uzaklık")}
            min={0}
            max={Math.round(s.h / 2)}
            value={Math.min(pos.y, Math.round(s.h / 2))}
            format={(v) => `${v} px`}
            disabled={disabled}
            onChange={(y) => set({ ...pos, y }, DRAG_DELAY_MS)}
          />
        </Row>
      </Card>
    </>
  );
}
