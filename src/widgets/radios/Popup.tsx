import { useEffect, useRef, useState } from "react";
import { commands, events, report, type BtDevice, type WifiNetwork } from "../../lib/ipc";
import { useService } from "../../lib/useService";
import { Toggle } from "../../windows/settings/controls";
import {
  BluetoothIcon,
  EthernetIcon,
  HeadphonesIcon,
  KeyboardIcon,
  LockIcon,
  PhoneIcon,
  RefreshIcon,
  WifiIcon,
} from "../icons";
import { IconButton } from "../ui";
import { LinkButton, PanelHeader } from "../ui/panel";
import { usePopupClose } from "../ui/popupClose";
import styles from "./Popup.module.css";
import { errText, t } from "../../lib/i18n";

/** Bluetooth aygıtlarının bağlantı durumu panel açıkken bu aralıkla tazelenir. */
const BT_REFRESH_MS = 4000;

/** Wi-Fi ya da Bluetooth paneli (sayfa: "radios/wifi", "radios/bluetooth"). */
export function RadiosPopup({ page }: { page?: string }) {
  return page === "bluetooth" ? <BluetoothPanel /> : <WifiPanel />;
}

function useSettings(page: "wifi" | "bluetooth") {
  const close = usePopupClose();
  return () => {
    commands.openSystemSettings(page).catch(report("open_system_settings"));
    close();
  };
}

// --- Wi-Fi -------------------------------------------------------------------------

/** Son tamamlanan tarama; `key` hangi istek için olduğunu söyler. */
type Scan = { key: string; list: WifiNetwork[] | null; error: string | null };

function WifiPanel() {
  const radios = useService(commands.getRadios, events.radios);
  const wifi = radios?.wifi;
  const openSettings = useSettings("wifi");
  const [result, setResult] = useState<Scan>({ key: "", list: null, error: null });
  const [attempt, setAttempt] = useState(0);
  const [open, setOpen] = useState<string | null>(null);

  const on = !!wifi?.on;
  // Bağlanınca (ad değişince) ve "yeniden tara"da liste tazelenir.
  const key = `${attempt}|${radios?.ssid ?? ""}`;
  useEffect(() => {
    if (!on) return;
    let alive = true;
    commands
      .wifiScan()
      .then((list) => alive && setResult({ key, list, error: null }))
      .catch((e) => alive && setResult((r) => ({ key, list: r.list, error: errText(e) })));
    return () => {
      alive = false;
    };
  }, [on, key]);

  const loading = result.key !== key;
  const scan =
    result.error && !loading
      ? ({ kind: "error", message: result.error } as const)
      : ({ kind: loading ? "loading" : "ready", list: result.list } as const);

  const list = scan.kind === "error" ? null : scan.list;

  return (
    <div className={styles.root}>
      <PanelHeader
        title="Wi-Fi"
        actions={
          <>
            {on && (
              <IconButton
                aria-label={t("Yeniden tara")}
                title={t("Yeniden tara")}
                className={styles.refresh}
                data-spin={scan.kind === "loading" || undefined}
                onClick={() => setAttempt((n) => n + 1)}
              >
                <RefreshIcon size={15} />
              </IconButton>
            )}
            {wifi && (
              <Toggle
                label="Wi-Fi"
                checked={on}
                disabled={wifi.disabled}
                onChange={(v) => commands.setRadio("wifi", v).catch(report("set_radio"))}
              />
            )}
          </>
        }
      />
      {radios?.ethernet && (
        <button
          type="button"
          className={styles.wired}
          title={t("Ethernet ayarları")}
          onClick={() =>
            commands.openSystemSettings("ethernet").catch(report("open_system_settings"))
          }
        >
          <EthernetIcon size={16} />
          <span>
            <span className={styles.wiredTitle}>{t("Kablolu bağlantı")}</span>
            <span className={styles.wiredHint}>{t("İnternet kablodan (Ethernet) geliyor")}</span>
          </span>
        </button>
      )}
      {!wifi ? (
        <p className={styles.note}>{t("Wi-Fi bağdaştırıcısı yok.")}</p>
      ) : !on ? (
        <p className={styles.note}>
          {wifi.disabled ? t("Wi-Fi donanım anahtarından kapalı.") : t("Wi-Fi kapalı.")}
        </p>
      ) : scan.kind === "error" ? (
        <div className={styles.error}>
          <span>
            {t("Ağlar okunamadı:")} {t(scan.message)}
          </span>
          <LinkButton onClick={openSettings}>{t("Windows ayarları")}</LinkButton>
        </div>
      ) : !list ? (
        <Skeleton />
      ) : list.length === 0 ? (
        <p className={styles.note}>{t("Yakında ağ bulunamadı.")}</p>
      ) : (
        <ul className={styles.list}>
          {list.map((n) => (
            <li key={n.ssid}>
              <Network
                network={n}
                expanded={open === n.ssid}
                onToggle={() => setOpen((o) => (o === n.ssid ? null : n.ssid))}
                onSettings={openSettings}
              />
            </li>
          ))}
        </ul>
      )}
      <footer className={styles.footer}>
        <LinkButton onClick={openSettings}>{t("Daha fazla Wi-Fi ayarı")}</LinkButton>
      </footer>
    </div>
  );
}

function Network({
  network: n,
  expanded,
  onToggle,
  onSettings,
}: {
  network: WifiNetwork;
  expanded: boolean;
  onToggle: () => void;
  onSettings: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [askPassword, setAskPassword] = useState(n.secured && !n.saved);
  const [password, setPassword] = useState("");
  const [username, setUsername] = useState("");
  const [showPw, setShowPw] = useState(false);
  const [auto, setAuto] = useState(true);
  const [certTrust, setCertTrust] = useState(false);

  const connectEnterprise = () => {
    if (!username.trim() || !password) return;
    setBusy(true);
    setError(null);
    setCertTrust(false);
    commands
      .wifiConnectEnterprise(n.ssid, username.trim(), password, auto)
      .then(() => {
        setUsername("");
      })
      .catch((e) => {
        const raw = e instanceof Error ? e.message : String(e);
        if (raw === "cert_trust") {
          setCertTrust(true);
          setError(
            t(
              "Bağlanılamadı. Ağın sunucu sertifikasını Windows'ta bir kez onaylamanız gerekebilir.",
            ),
          );
        } else setError(errText(e));
      })
      .finally(() => {
        // Parola bellekte tutulmasın.
        setPassword("");
        setShowPw(false);
        setBusy(false);
      });
  };

  const connect = (pw?: string) => {
    setBusy(true);
    setError(null);
    commands
      .wifiConnect(n.ssid, pw)
      .then(() => setPassword(""))
      .catch((e) => {
        const msg = errText(e);
        if (msg === "password_required") setAskPassword(true);
        else setError(msg);
      })
      .finally(() => setBusy(false));
  };

  const state = n.connected
    ? t("Bağlı")
    : n.saved
      ? t("Kayıtlı")
      : n.enterprise
        ? t("Kurumsal")
        : n.secured
          ? t("Güvenli")
          : t("Açık ağ");

  return (
    <div className={styles.item} data-expanded={expanded || undefined}>
      <button
        type="button"
        className={styles.row}
        data-active={n.connected || undefined}
        aria-expanded={expanded}
        onClick={onToggle}
      >
        <span className={styles.icon}>
          <WifiIcon bars={Math.max(1, Math.round((n.bars / 4) * 5))} off={false} size={18} />
        </span>
        <span className={styles.text}>
          <span className={styles.name}>{n.ssid}</span>
          <span className={styles.sub}>{state}</span>
        </span>
        {(n.secured || n.enterprise) && (
          <span className={styles.lock} aria-label={t("Şifreli")}>
            <LockIcon size={13} />
          </span>
        )}
      </button>
      {expanded && (
        <div className={styles.actions}>
          {n.connected ? (
            <button
              type="button"
              className={styles.secondary}
              disabled={busy}
              onClick={() => {
                setBusy(true);
                commands
                  .wifiDisconnect()
                  .catch((e) => setError(errText(e)))
                  .finally(() => setBusy(false));
              }}
            >
              {t("Bağlantıyı kes")}
            </button>
          ) : n.enterprise ? (
            <form
              className={styles.enterprise}
              onSubmit={(e) => {
                e.preventDefault();
                connectEnterprise();
              }}
            >
              <input
                type="text"
                autoFocus
                autoComplete="off"
                spellCheck={false}
                placeholder={t("E-posta / kullanıcı adı")}
                aria-label={t("{ssid} kullanıcı adı", { ssid: n.ssid })}
                value={username}
                onChange={(e) => setUsername(e.currentTarget.value)}
              />
              <div className={styles.pwRow}>
                <input
                  type={showPw ? "text" : "password"}
                  autoComplete="off"
                  placeholder={t("Parola")}
                  aria-label={t("{ssid} parolası", { ssid: n.ssid })}
                  value={password}
                  onChange={(e) => setPassword(e.currentTarget.value)}
                />
                <button
                  type="button"
                  className={styles.secondary}
                  aria-pressed={showPw}
                  onClick={() => setShowPw((v) => !v)}
                >
                  {showPw ? t("Parolayı gizle") : t("Parolayı göster")}
                </button>
              </div>
              <label className={styles.check}>
                <input
                  type="checkbox"
                  checked={auto}
                  onChange={(e) => setAuto(e.currentTarget.checked)}
                />
                {t("Bağlantıyı otomatik kur")}
              </label>
              <button
                type="submit"
                className={styles.primary}
                disabled={busy || !username.trim() || !password}
              >
                {busy && <span className={styles.spinner} aria-hidden="true" />}
                {busy ? t("Bağlanıyor…") : t("Bağlan")}
              </button>
            </form>
          ) : askPassword ? (
            <form
              className={styles.password}
              onSubmit={(e) => {
                e.preventDefault();
                if (password) connect(password);
              }}
            >
              <input
                type="password"
                autoFocus
                placeholder={t("Ağ şifresi")}
                aria-label={t("{ssid} şifresi", { ssid: n.ssid })}
                value={password}
                onChange={(e) => setPassword(e.currentTarget.value)}
              />
              <button type="submit" className={styles.primary} disabled={busy || !password}>
                {busy ? t("Bağlanıyor…") : t("Bağlan")}
              </button>
            </form>
          ) : (
            <button
              type="button"
              className={styles.primary}
              disabled={busy}
              onClick={() => connect()}
            >
              {busy ? t("Bağlanıyor…") : t("Bağlan")}
            </button>
          )}
          {error && <p className={styles.errorText}>{error}</p>}
          {certTrust && (
            <button type="button" className={styles.secondary} onClick={onSettings}>
              {t("Windows ayarlarında aç")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// --- Bluetooth ---------------------------------------------------------------------

const BT_ICONS = {
  audio: HeadphonesIcon,
  input: KeyboardIcon,
  phone: PhoneIcon,
  other: (p: { size?: number }) => <BluetoothIcon off={false} {...p} />,
} as const;

function BluetoothPanel() {
  const radios = useService(commands.getRadios, events.radios);
  const bt = radios?.bluetooth;
  const openSettings = useSettings("bluetooth");
  const [devices, setDevices] = useState<BtDevice[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** Bağlanma / ayrılma süren aygıt kimliği ve son işlem hatası. */
  const [busyId, setBusyId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<{ id: string; msg: string } | null>(null);
  const on = !!bt?.on;
  const busyRef = useRef(false);
  useEffect(() => {
    busyRef.current = busyId !== null;
  }, [busyId]);

  const toggleDevice = (d: BtDevice) => {
    if (busyId) return;
    setBusyId(d.id);
    setActionError(null);
    commands
      .btSetConnected(d.id, d.le, !d.connected)
      .catch((e) => {
        const msg = errText(e);
        if (msg === "use_settings") openSettings();
        else setActionError({ id: d.id, msg });
      })
      .finally(() => {
        setBusyId(null);
        commands
          .btDevices()
          .then(setDevices)
          .catch(() => {});
      });
  };

  useEffect(() => {
    if (!on) return;
    let alive = true;
    const load = () =>
      !busyRef.current &&
      commands
        .btDevices()
        .then((d) => {
          if (!alive) return;
          setDevices(d);
          setError(null);
        })
        .catch((e) => alive && setError(errText(e)));
    load();
    const timer = window.setInterval(load, BT_REFRESH_MS);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [on]);

  return (
    <div className={styles.root}>
      <PanelHeader
        title="Bluetooth"
        actions={
          bt && (
            <Toggle
              label="Bluetooth"
              checked={on}
              disabled={bt.disabled}
              onChange={(v) => commands.setRadio("bluetooth", v).catch(report("set_radio"))}
            />
          )
        }
      />
      {!bt ? (
        <p className={styles.note}>{t("Bluetooth yok.")}</p>
      ) : !on ? (
        <p className={styles.note}>{t("Bluetooth kapalı.")}</p>
      ) : error ? (
        <p className={styles.errorText}>
          {t("Aygıtlar okunamadı:")} {error}
        </p>
      ) : !devices ? (
        <Skeleton rows={3} />
      ) : devices.length === 0 ? (
        <p className={styles.note}>{t("Eşleşmiş aygıt yok.")}</p>
      ) : (
        <ul className={styles.list}>
          {devices.map((d) => {
            const Icon = BT_ICONS[d.kind];
            return (
              <li key={d.id}>
                <button
                  type="button"
                  className={styles.row}
                  data-active={d.connected || undefined}
                  aria-busy={busyId === d.id || undefined}
                  disabled={busyId !== null && busyId !== d.id}
                  title={d.connected ? t("Bağlantıyı kes") : t("Bağlan")}
                  onClick={() => toggleDevice(d)}
                >
                  <span className={styles.icon}>
                    <Icon size={18} />
                  </span>
                  <span className={styles.text}>
                    <span className={styles.name}>{d.name}</span>
                    <span className={styles.sub}>
                      {busyId === d.id
                        ? d.connected
                          ? t("Ayrılıyor…")
                          : t("Bağlanıyor…")
                        : d.connected
                          ? t("Bağlı")
                          : t("Eşleşmiş")}
                    </span>
                  </span>
                </button>
                {actionError?.id === d.id && (
                  <p className={styles.errorText}>{t(actionError.msg)}</p>
                )}
              </li>
            );
          })}
        </ul>
      )}
      <footer className={styles.footer}>
        <LinkButton onClick={openSettings}>{t("Aygıt ekle / Bluetooth ayarları")}</LinkButton>
      </footer>
    </div>
  );
}

function Skeleton({ rows = 5 }: { rows?: number }) {
  return (
    <ul className={styles.list} aria-busy="true">
      {Array.from({ length: rows }, (_, i) => (
        <li key={i} className={styles.skeleton} style={{ animationDelay: `${i * 80}ms` }} />
      ))}
    </ul>
  );
}
