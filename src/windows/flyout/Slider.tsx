import { useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import styles from "./Flyout.module.css";

type Props = {
  /** 0–1 */
  value: number;
  onChange?: (v: number) => void;
  /** Sürükleme bitince (arama çubuğu gibi yalnızca sonda iş yapanlar için). */
  onCommit?: (v: number) => void;
  /** Yalnızca gösterge. */
  readOnly?: boolean;
  label: string;
  thin?: boolean;
};

/** Sürüklenebilir çubuk (işaretçi yakalamalı); sürerken kendi değerini gösterir. */
export function Slider({ value, onChange, onCommit, readOnly, label, thin }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<number | null>(null);
  const shown = Math.min(1, Math.max(0, drag ?? value));

  const at = (e: { clientX: number }) => {
    const r = ref.current!.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
  };

  const key = (e: KeyboardEvent) => {
    if (readOnly) return;
    const step =
      e.key === "ArrowRight" || e.key === "ArrowUp"
        ? 0.05
        : e.key === "ArrowLeft" || e.key === "ArrowDown"
          ? -0.05
          : 0;
    if (!step) return;
    e.preventDefault();
    const v = Math.min(1, Math.max(0, shown + step));
    onChange?.(v);
    onCommit?.(v);
  };

  return (
    <div
      ref={ref}
      className={styles.slider}
      data-thin={thin || undefined}
      data-readonly={readOnly || undefined}
      data-drag={drag !== null || undefined}
      role="slider"
      tabIndex={readOnly ? -1 : 0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(shown * 100)}
      style={{ "--v": shown } as CSSProperties}
      onKeyDown={key}
      onPointerDown={(e) => {
        if (readOnly || e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        const v = at(e);
        setDrag(v);
        onChange?.(v);
      }}
      onPointerMove={(e) => {
        if (drag === null) return;
        const v = at(e);
        setDrag(v);
        onChange?.(v);
      }}
      onPointerUp={(e) => {
        if (drag === null) return;
        const v = at(e);
        setDrag(null);
        onCommit?.(v);
      }}
      onPointerCancel={() => setDrag(null)}
    >
      <div className={styles.track}>
        <div className={styles.fill} />
      </div>
      {!readOnly && <div className={styles.knob} />}
    </div>
  );
}
