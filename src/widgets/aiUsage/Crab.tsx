import styles from "./Crab.module.css";

/**
 * Claude Code maskotu (piksel yengeç). 15x10'luk ızgara, <rect>'lerle; sakin
 * boşta animasyonları Crab.module.css'te (hareket azaltma global.css'ten gelir).
 */
export function Crab({ className }: { className?: string }) {
  return (
    <svg
      className={`${styles.crab} ${className ?? ""}`}
      width={20}
      height={13}
      viewBox="0 0 15 10"
      shapeRendering="crispEdges"
      aria-hidden="true"
      focusable="false"
    >
      <g className={styles.legs} fill="#D97757">
        <rect x="3" y="7" width="1" height="2" />
        <rect x="5" y="7" width="1" height="2" />
        <rect x="9" y="7" width="1" height="2" />
        <rect x="11" y="7" width="1" height="2" />
      </g>
      <g className={styles.clawL} fill="#D97757">
        <rect x="0" y="3" width="2" height="1" />
        <rect className={styles.pincer} x="0" y="5" width="2" height="1" />
        <rect x="1" y="4" width="1" height="1" />
      </g>
      <g className={styles.clawR} fill="#D97757">
        <rect x="13" y="3" width="2" height="1" />
        <rect className={styles.pincer} x="13" y="5" width="2" height="1" />
        <rect x="13" y="4" width="1" height="1" />
      </g>
      <rect x="2" y="1" width="11" height="6" fill="#D97757" />
      <g className={styles.eyes} fill="#2B1A14">
        <rect x="4" y="3" width="1" height="2" />
        <rect x="10" y="3" width="1" height="2" />
      </g>
    </svg>
  );
}
