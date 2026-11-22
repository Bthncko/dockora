import { commands, events } from "../../lib/ipc";
import { togglePopup } from "../../lib/popup";
import { useService } from "../../lib/useService";
import { Meter, Tile } from "../ui";
import { current, remaining, resetTime, useMinute } from "./format";
import styles from "./Bar.module.css";
import { Crab } from "./Crab";
import { pct, t } from "../../lib/i18n";

/**
 * AI kullanımı (8.9): Claude hesabındaki oturum (5 saat) ve haftalık limit
 * yüzdeleri. Ne Claude oturumu ne Claude Code kaydı varsa gizli.
 */
export function AiUsageBar() {
  const usage = useService(commands.getAiUsage, events.aiUsage);
  const now = useMinute();
  if (!usage) return null;
  const session = current(usage.limits?.session, now);
  const weekly = current(usage.limits?.weekly, now);

  const line = (name: string, l: typeof session) =>
    l
      ? `${name}: ${pct(l.percent)}${l.resetsAt ? ` · ${t("sıfırlanma {at} ({left})", { at: resetTime(l.resetsAt, now), left: remaining(l.resetsAt, now) })}` : ""}`
      : `${name}: —`;

  return (
    <Tile
      as="button"
      variant="soft"
      interactive
      className={styles.tile}
      title={[
        line(t("Oturum (5 saat)"), session),
        line(t("Haftalık"), weekly),
        ...(usage.limitsError ? [t(usage.limitsError)] : []),
      ].join("\n")}
      aria-label={t("AI kullanımı: oturum {s}, hafta {w}", {
        s: session ? t("yüzde {p}", { p: session.percent }) : t("bilinmiyor"),
        w: weekly ? t("yüzde {p}", { p: weekly.percent }) : t("bilinmiyor"),
      })}
      onClick={(e) => togglePopup("aiUsage", e.currentTarget)}
    >
      <Crab className={styles.icon} />
      <span className={styles.rows}>
        <Row label={t("Oturum")} limit={session} />
        <Row label={t("Hafta")} limit={weekly} />
      </span>
    </Tile>
  );
}

function Row({ label, limit }: { label: string; limit: ReturnType<typeof current> }) {
  return (
    <span className={styles.row} data-high={limit?.warning || undefined}>
      <span className={styles.label}>{label}</span>
      <Meter value={limit?.percent ?? 0} muted={!limit} className={styles.meter} />
      <span className={styles.value}>{limit ? pct(limit.percent) : "—"}</span>
    </span>
  );
}
