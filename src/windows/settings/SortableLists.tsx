import { useState, type DragEvent, type ReactNode } from "react";
import styles from "./Settings.module.css";
import { t } from "../../lib/i18n";

// Bir ya da birkaç liste arasında sürükle-bırak (HTML5). Ayarlar penceresinde
// WebView2'nin dosya bırakma işleyicisi kapalı (win/settings.rs), yoksa
// Windows'ta bu olaylar hiç gelmez.

export type SortItem = { id: string; label: string; icon?: string | null };
export type SortList = { key: string; title: string; items: SortItem[]; empty?: string };

type Drag = { id: string; from: string };
type Over = { list: string; index: number };

export function SortableLists({
  lists,
  onChange,
  renderActions,
}: {
  lists: SortList[];
  /** Her listenin yeni id sırası. */
  onChange: (next: Record<string, string[]>) => void;
  renderActions?: (item: SortItem, listKey: string) => ReactNode;
}) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [over, setOver] = useState<Over | null>(null);

  const end = () => {
    setDrag(null);
    setOver(null);
  };

  /** İmlecin altındaki ekleme noktası: öğe ortalarına göre. */
  const indexAt = (listEl: HTMLElement, y: number) => {
    const items = listEl.querySelectorAll<HTMLElement>("[data-sort-item]");
    for (let i = 0; i < items.length; i++) {
      const r = items[i].getBoundingClientRect();
      if (y < r.top + r.height / 2) return i;
    }
    return items.length;
  };

  const onDragOver = (list: string) => (e: DragEvent<HTMLUListElement>) => {
    if (!drag) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    const index = indexAt(e.currentTarget, e.clientY);
    if (over?.list !== list || over.index !== index) setOver({ list, index });
  };

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    if (drag && over) {
      const next = Object.fromEntries(lists.map((l) => [l.key, l.items.map((i) => i.id)]));
      const source = next[drag.from];
      const fromIndex = source.indexOf(drag.id);
      let toIndex = over.index;
      if (drag.from === over.list && fromIndex < toIndex) toIndex--;
      if (!(drag.from === over.list && fromIndex === toIndex)) {
        source.splice(fromIndex, 1);
        next[over.list].splice(toIndex, 0, drag.id);
        onChange(next);
      }
    }
    end();
  };

  return (
    <div className={styles.lists} style={{ gridTemplateColumns: `repeat(${lists.length}, minmax(0, 1fr))` }}>
      {lists.map((list) => {
        const indicatorAt = over?.list === list.key ? over.index : -1;
        const indicator = <li key="__indicator" className={styles.dropLine} aria-hidden />;
        return (
          <div key={list.key} className={styles.listBox}>
            <h3 className={styles.listTitle}>{list.title}</h3>
            <ul
              className={styles.list}
              data-over={indicatorAt >= 0 || undefined}
              onDragOver={onDragOver(list.key)}
              onDragLeave={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(null);
              }}
              onDrop={onDrop}
            >
              {list.items.map((item, i) => [
                i === indicatorAt ? indicator : null,
                <li
                  key={item.id}
                  data-sort-item
                  className={styles.listItem}
                  data-dragging={drag?.id === item.id || undefined}
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData("text/plain", item.id);
                    e.dataTransfer.effectAllowed = "move";
                    setDrag({ id: item.id, from: list.key });
                  }}
                  onDragEnd={end}
                >
                  <span className={styles.grip} aria-hidden>
                    ⠿
                  </span>
                  {item.icon && <img className={styles.itemIcon} src={item.icon} alt="" />}
                  <span className={styles.itemLabel}>{item.label}</span>
                  {renderActions?.(item, list.key)}
                </li>,
              ])}
              {indicatorAt === list.items.length && indicator}
              {list.items.length === 0 && indicatorAt < 0 && (
                <li className={styles.listEmpty}>{list.empty ?? t("Buraya sürükle")}</li>
              )}
            </ul>
          </div>
        );
      })}
    </div>
  );
}
