import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
// Inter Variable (SIL OFL 1.1), uygulamaya gömülü; ağdan çekilmez.
import "@fontsource-variable/inter";
import "./styles/tokens.css";
import "./styles/global.css";
import { DockWindow } from "./windows/DockWindow";
import { FlyoutWindow } from "./windows/flyout/FlyoutWindow";
import { NotifyWindow } from "./windows/notify/NotifyWindow";
import { PopupWindow } from "./windows/PopupWindow";
import { PreviewWindow } from "./windows/PreviewWindow";
import { SettingsWindow } from "./windows/SettingsWindow";
import { LangRoot } from "./lib/LangRoot";

// Her Tauri penceresi aynı paketi yükler; hangisi olduğunu pencere etiketi söyler
// (tauri.conf.json). Etiket adresten güvenilir: pencere adresine # eklemeye gerek yok.
function pickWindow() {
  switch (getCurrentWindow().label) {
    case "popup":
      return <PopupWindow />;
    case "preview":
      return <PreviewWindow />;
    case "notify":
      return <NotifyWindow />;
    case "flyout":
      return <FlyoutWindow />;
    case "settings":
      return <SettingsWindow />;
    default:
      return <DockWindow />;
  }
}

// Tarayıcı sağ tık menüsü kapalı (bölüm 10).
window.addEventListener("contextmenu", (e) => e.preventDefault());

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <LangRoot render={pickWindow} />
  </React.StrictMode>,
);
