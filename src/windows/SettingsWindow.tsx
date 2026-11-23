import { useEffect, useState, type FC } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, events, report, type Config } from "../lib/ipc";
import { startConfigSync, useConfig } from "../lib/store";
import { useService } from "../lib/useService";
import { applyTheme } from "../lib/theme";
import { Appearance } from "./settings/Appearance";
import { Apps } from "./settings/Apps";
import { Calendar } from "./settings/Calendar";
import { Flyouts } from "./settings/Flyouts";
import { General } from "./settings/General";
import { Music } from "./settings/Music";
import { Weather } from "./settings/Weather";
import { Widgets } from "./settings/Widgets";
import styles from "./settings/Settings.module.css";
import { appName, t } from "../lib/i18n";

startConfigSync();

const PAGES: { id: string; label: string; Page: FC<{ config: Config }> }[] = [
  {
    id: "appearance",
    get label() {
      return t("Görünüm");
    },
    Page: Appearance,
  },
  {
    id: "widgets",
    get label() {
      return t("Widget'lar");
    },
    Page: Widgets,
  },
  {
    id: "apps",
    get label() {
      return t("Uygulamalar");
    },
    Page: Apps,
  },
  {
    id: "weather",
    get label() {
      return t("Hava durumu");
    },
    Page: Weather,
  },
  {
    id: "calendar",
    get label() {
      return t("Saat ve takvim");
    },
    Page: Calendar,
  },
  {
    id: "music",
    get label() {
      return t("Müzik");
    },
    Page: Music,
  },
  {
    id: "flyouts",
    get label() {
      return t("Göstergeler");
    },
    Page: Flyouts,
  },
  {
    id: "general",
    get label() {
      return t("Genel");
    },
    Page: General,
  },
];

/** Ayarlar penceresi (Aşama 6). Her değişiklik `set_config` ile hemen kaydedilir. */
export function SettingsWindow() {
  const config = useConfig();
  const [active, setActive] = useState(PAGES[0].id);
  const systemAccent = useService(commands.getSystemAccent, events.accent);

  useEffect(() => {
    if (config) applyTheme(config.theme, { systemAccent });
  }, [config?.theme, systemAccent]); // eslint-disable-line react-hooks/exhaustive-deps

  // Başka yerden belirli bir bölüm istenebilir (ör. hava panelindeki "Değiştir").
  useEffect(() => {
    const go = (page: string | null) => {
      if (page && PAGES.some((p) => p.id === page)) setActive(page);
    };
    commands.takeSettingsPage().then(go).catch(report("take_settings_page"));
    const unlisten = listen<string>(events.settingsPage, (e) => go(e.payload));
    return () => {
      unlisten.then((off) => off());
    };
  }, []);

  const { Page } = PAGES.find((p) => p.id === active) ?? PAGES[0];

  return (
    <div className={styles.root}>
      <nav className={styles.nav} aria-label={t("Ayar bölümleri")}>
        <h1 className={styles.appTitle}>{appName()}</h1>
        {PAGES.map((p) => (
          <button
            key={p.id}
            type="button"
            className={styles.navItem}
            aria-current={p.id === active ? "page" : undefined}
            onClick={() => setActive(p.id)}
          >
            {p.label}
          </button>
        ))}
      </nav>
      <main className={styles.main}>{config ? <Page config={config} /> : null}</main>
    </div>
  );
}
