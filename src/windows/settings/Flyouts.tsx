import { useState, type ReactNode } from "react";
import {
  commands,
  events,
  type Config,
  type FlyoutEasing,
  type FlyoutMediaBackground,
  type FlyoutMonitor,
  type FlyoutsConfig,
} from "../../lib/ipc";
import { updateConfig } from "../../lib/store";
import { useService } from "../../lib/useService";
import { Card, Row, Section, Segmented, Slider, Toggle } from "./controls";
import { NotifyPlacement } from "./NotifyPlacement";
import styles from "./Settings.module.css";
import { t } from "../../lib/i18n";

type Flyouts = FlyoutsConfig;
type SectionKey = "media" | "nextUp" | "lockKeys" | "volume" | "brightness";

const patch = (change: (f: Flyouts) => Flyouts, delay = 0) =>
  updateConfig((c) => ({ ...c, flyouts: change(c.flyouts) }), delay);

/** Bir bölümün tek anahtarı. */
function Opt<S extends SectionKey>({
  f,
  section,
  field,
  label,
  hint,
  off,
}: {
  f: Flyouts;
  section: S;
  field: {
    [K in keyof Flyouts[S]]: Flyouts[S][K] extends boolean ? K : never;
  }[keyof Flyouts[S]];
  label: string;
  hint?: string;
  off: boolean;
}) {
  const value = f[section][field] as boolean;
  return (
    <Row label={label} hint={hint}>
      <Toggle
        label={label}
        checked={value}
        disabled={off}
        onChange={(v) => patch((x) => ({ ...x, [section]: { ...x[section], [field]: v } }))}
      />
    </Row>
  );
}

const seconds = (ms: number) => `${Math.round(ms / 10) / 100} ${t("sn")}`;

function Duration({
  f,
  section,
  off,
  disabled,
}: {
  f: Flyouts;
  section: SectionKey;
  off: boolean;
  disabled?: boolean;
}) {
  const label = t("Ekranda kalma süresi");
  return (
    <Row label={label} hint={t("Son etkileşimden sonra kart bu kadar açık kalır.")}>
      <Slider
        label={label}
        min={500}
        max={10000}
        step={250}
        value={f[section].durationMs}
        format={seconds}
        disabled={off || disabled}
        onChange={(durationMs) =>
          patch((x) => ({ ...x, [section]: { ...x[section], durationMs } }), 120)
        }
      />
    </Row>
  );
}

function SectionCard({
  title,
  enabledLabel,
  enabledHint,
  f,
  section,
  children,
}: {
  title: string;
  enabledLabel: string;
  enabledHint: string;
  f: Flyouts;
  section: SectionKey;
  children: (off: boolean) => ReactNode;
}) {
  const on = f[section].enabled;
  const off = !f.enabled || !on;
  return (
    <>
      <h3 className={styles.subTitle}>{title}</h3>
      <Card>
        <Row label={enabledLabel} hint={enabledHint}>
          <Toggle
            label={enabledLabel}
            checked={on}
            disabled={!f.enabled}
            onChange={(v) => patch((x) => ({ ...x, [section]: { ...x[section], enabled: v } }))}
          />
        </Row>
        {children(off)}
      </Card>
    </>
  );
}

function AppFilter({ f }: { f: Flyouts }) {
  const { mode, apps } = f.appFilter;
  const [text, setText] = useState("");
  const media = useService(commands.getMedia, events.media);
  const current = media?.sourceApp;
  const suggestion = current && !apps.includes(current) ? current : null;
  const off = !f.enabled;

  const setFilter = (change: (a: Flyouts["appFilter"]) => Flyouts["appFilter"]) =>
    patch((x) => ({ ...x, appFilter: change(x.appFilter) }));
  const add = (id: string) => {
    const v = id.trim();
    if (!v || apps.some((a) => a.toLowerCase() === v.toLowerCase())) return;
    setFilter((a) => ({ ...a, apps: [...a.apps, v] }));
    setText("");
  };

  return (
    <>
      <Row
        label={t("Uygulama süzgeci")}
        hint={t(
          "Medya kartı yalnızca seçtiğin oynatıcılar için (izin ver) ya da onlar dışında (engelle) görünür.",
        )}
      >
        <Segmented<Flyouts["appFilter"]["mode"]>
          label={t("Uygulama süzgeci")}
          value={mode}
          options={[
            { value: "off", label: t("Kapalı") },
            { value: "allow", label: t("İzin ver") },
            { value: "block", label: t("Engelle") },
          ]}
          onChange={(m) => setFilter((a) => ({ ...a, mode: m }))}
        />
      </Row>
      {mode !== "off" && (
        <>
          <div className={styles.urlList}>
            {apps.length === 0 && <p className={styles.listEmpty}>{t("Liste boş.")}</p>}
            {apps.map((a) => (
              <div key={a} className={styles.urlItem}>
                <span className={styles.urlText}>{a}</span>
                <button
                  type="button"
                  className={styles.button}
                  disabled={off}
                  onClick={() => setFilter((x) => ({ ...x, apps: x.apps.filter((v) => v !== a) }))}
                >
                  {t("Kaldır")}
                </button>
              </div>
            ))}
          </div>
          <div className={styles.addRow}>
            <input
              className={styles.input}
              value={text}
              disabled={off}
              placeholder={t("Oynatıcı kimliği (ör. Spotify.exe)")}
              aria-label={t("Oynatıcı kimliği (ör. Spotify.exe)")}
              onChange={(e) => setText(e.currentTarget.value)}
              onKeyDown={(e) => e.key === "Enter" && add(text)}
            />
            <button
              type="button"
              className={styles.button}
              disabled={off || !text.trim()}
              onClick={() => add(text)}
            >
              {t("Ekle")}
            </button>
          </div>
          {suggestion && (
            <Row label={suggestion} hint={t("Şu an çalan oynatıcı")}>
              <button
                type="button"
                className={styles.button}
                disabled={off}
                onClick={() => add(suggestion)}
              >
                {t("Ekle")}
              </button>
            </Row>
          )}
        </>
      )}
    </>
  );
}

export function Flyouts({ config }: { config: Config }) {
  const f = config.flyouts;
  const master = f.enabled;
  const dis = !master;

  return (
    <Section
      title={t("Göstergeler")}
      intro={t(
        "Medya, ses, parlaklık ve tuş durumu değişince ekranda beliren kartlar. FluentFlyout gibi benzer bir uygulama aynı anda çalışıyorsa her ikisi de kart gösterir; birini kapat.",
      )}
    >
      <Card>
        <Row label={t("Göstergeleri aç")} hint={t("Kapalıyken aşağıdaki hiçbir kart görünmez.")}>
          <Toggle
            label={t("Göstergeleri aç")}
            checked={master}
            onChange={(enabled) => patch((x) => ({ ...x, enabled }))}
          />
        </Row>
        <Row label={t("Gösterilecek ekran")} hint={t("Kartların çıkacağı monitör.")}>
          <Segmented<FlyoutMonitor>
            label={t("Gösterilecek ekran")}
            value={f.monitor}
            options={[
              { value: "primary", label: t("Ana ekran") },
              { value: "cursor", label: t("İmlecin olduğu ekran") },
              { value: "focused", label: t("Odaktaki pencerenin ekranı") },
            ]}
            onChange={(monitor) => patch((x) => ({ ...x, monitor }))}
          />
        </Row>
        <Row
          label={t("Tam ekran oyunda/videoda gösterme")}
          hint={t("Tam ekran uygulama açıkken kartlar bastırılır.")}
        >
          <Toggle
            label={t("Tam ekran oyunda/videoda gösterme")}
            checked={f.hideInFullscreen}
            disabled={dis}
            onChange={(hideInFullscreen) => patch((x) => ({ ...x, hideInFullscreen }))}
          />
        </Row>
        <Row label={t("Animasyon hızı")} hint={t("Kartların açılış ve kapanış hızı.")}>
          <Segmented<string>
            label={t("Animasyon hızı")}
            value={String(f.animationSpeed)}
            options={[
              { value: "0", label: t("Kapalı") },
              { value: "0.5", label: "0.5x" },
              { value: "1", label: "1x" },
              { value: "1.5", label: "1.5x" },
              { value: "2", label: "2x" },
              { value: "3", label: "3x" },
            ]}
            onChange={(v) => patch((x) => ({ ...x, animationSpeed: Number(v) }))}
          />
        </Row>
        <Row label={t("Animasyon eğrisi")} hint={t("Hareketin hızlanıp yavaşlama biçimi.")}>
          <Segmented<FlyoutEasing>
            label={t("Animasyon eğrisi")}
            value={f.easing}
            options={[
              { value: "linear", label: t("Doğrusal") },
              { value: "sine", label: t("Sinüs") },
              { value: "quad", label: t("Karesel") },
              { value: "cubic", label: t("Kübik") },
            ]}
            onChange={(easing) => patch((x) => ({ ...x, easing }))}
          />
        </Row>
        <Row
          label={t("Albüm kapağının rengini kullan")}
          hint={t("Vurgu rengi çalan parçanın kapağından alınır.")}
        >
          <Toggle
            label={t("Albüm kapağının rengini kullan")}
            checked={f.albumAccent}
            disabled={dis}
            onChange={(albumAccent) => patch((x) => ({ ...x, albumAccent }))}
          />
        </Row>
        <Row label={t("Cam efekti")} hint={t("Kart arka planı yarı saydam, bulanık cam olur.")}>
          <Toggle
            label={t("Cam efekti")}
            checked={f.glass}
            disabled={dis}
            onChange={(glass) => patch((x) => ({ ...x, glass }))}
          />
        </Row>
        <AppFilter f={f} />
      </Card>

      <NotifyPlacement
        config={config}
        value={f.placement}
        onChange={(placement, delay) => patch((x) => ({ ...x, placement }), delay)}
        disabled={dis}
        title={t("Göstergelerin konumu")}
        card={{ w: 380, h: 120 }}
        cardLabel={t("Gösterge kartı")}
        hint={
          dis
            ? t("Göstergeleri yukarıdan açınca buradan yerini seçebilirsin.")
            : t(
                "Kartı sürükle ya da ekranda bir yere tıkla: en yakın köşeye ya da kenarın ortasına oturur. Altta olunca uzaklık dock'un üstünden ölçülür.",
              )
        }
      />

      <SectionCard
        title={t("Medya kartı")}
        enabledLabel={t("Medya kartını göster")}
        enabledHint={t("Parça değişince ya da medya tuşuna basınca çalan parçayı gösterir.")}
        f={f}
        section="media"
      >
        {(off) => (
          <>
            <Opt
              f={f}
              off={off}
              section="media"
              field="alwaysShow"
              label={t("Kapatana kadar açık kalsın")}
              hint={t("Süre dolunca kapanmaz; kartı sen kapatırsın.")}
            />
            <Duration f={f} section="media" off={off} disabled={f.media.alwaysShow} />
            <Opt
              f={f}
              off={off}
              section="media"
              field="compact"
              label={t("Kompakt görünüm")}
              hint={t("Daha küçük, tek satırlık kart.")}
            />
            <Opt f={f} off={off} section="media" field="centerText" label={t("Metni ortala")} />
            <Opt
              f={f}
              off={off}
              section="media"
              field="showPlayer"
              label={t("Oynatıcı düğmeleri")}
              hint={t("Önceki, oynat/duraklat ve sonraki.")}
            />
            <Opt f={f} off={off} section="media" field="showRepeat" label={t("Tekrar düğmesi")} />
            <Opt
              f={f}
              off={off}
              section="media"
              field="showShuffle"
              label={t("Karıştır düğmesi")}
            />
            <Opt f={f} off={off} section="media" field="showSeekbar" label={t("İlerleme çubuğu")} />
            <Row label={t("Arka plan")} hint={t("Kartın arkasındaki albüm kapağı efekti.")}>
              <Segmented<FlyoutMediaBackground>
                label={t("Arka plan")}
                value={f.media.background}
                options={[
                  { value: "none", label: t("Yok") },
                  { value: "coverGlow", label: t("Kapak parıltısı") },
                  { value: "risingGlow", label: t("Yükselen parıltı") },
                  { value: "blur", label: t("Bulanık") },
                ]}
                onChange={(background) =>
                  patch((x) => ({ ...x, media: { ...x.media, background } }))
                }
              />
            </Row>
            <Opt
              f={f}
              off={off}
              section="media"
              field="excludeVolumeKeys"
              label={t("Ses tuşlarında gösterme")}
              hint={t("Ses tuşları medya kartını açmaz.")}
            />
            <Opt
              f={f}
              off={off}
              section="media"
              field="pauseOthers"
              label={t("Yeni medya başlayınca diğerlerini duraklat")}
              hint={t("Bir oynatıcı çalmaya başlayınca diğerleri duraklatılır.")}
            />
          </>
        )}
      </SectionCard>

      <SectionCard
        title={t("Sıradaki parça")}
        enabledLabel={t("Sıradaki parçayı göster")}
        enabledHint={t("Parça bitmeden önce sıradaki parçayı haber verir.")}
        f={f}
        section="nextUp"
      >
        {(off) => <Duration f={f} section="nextUp" off={off} />}
      </SectionCard>

      <SectionCard
        title={t("Kilit tuşları")}
        enabledLabel={t("Kilit tuşlarını göster")}
        enabledHint={t("Caps Lock gibi tuşlar açılıp kapanınca durumu gösterir.")}
        f={f}
        section="lockKeys"
      >
        {(off) => (
          <>
            <Duration f={f} section="lockKeys" off={off} />
            <Opt f={f} off={off} section="lockKeys" field="caps" label="Caps Lock" />
            <Opt f={f} off={off} section="lockKeys" field="num" label="Num Lock" />
            <Opt f={f} off={off} section="lockKeys" field="scroll" label="Scroll Lock" />
            <Opt f={f} off={off} section="lockKeys" field="insert" label="Insert" />
            <Opt
              f={f}
              off={off}
              section="lockKeys"
              field="bold"
              label={t("Kalın yazı")}
              hint={t("Kart metni kalın yazılır.")}
            />
            <Opt
              f={f}
              off={off}
              section="lockKeys"
              field="animated"
              label={t("Animasyonlu simge")}
            />
          </>
        )}
      </SectionCard>

      <SectionCard
        title={t("Ses")}
        enabledLabel={t("Windows ses göstergesinin yerine geçsin")}
        enabledHint={t("Ses tuşlarında Windows'un kendi göstergesi yerine bu kart çıkar.")}
        f={f}
        section="volume"
      >
        {(off) => (
          <>
            <Duration f={f} section="volume" off={off} />
            <Opt
              f={f}
              off={off}
              section="volume"
              field="aboveMedia"
              label={t("Medya kartının üstünde")}
              hint={t("İkisi birlikte açıksa ses kartı medya kartının üstüne gelir.")}
            />
            <Opt
              f={f}
              off={off}
              section="volume"
              field="mixer"
              label={t("Uygulama ses karıştırıcısı")}
              hint={t("Uygulamaların ses düzeyleri de listelenir.")}
            />
            <Opt
              f={f}
              off={off}
              section="volume"
              field="highlightActive"
              label={t("Etkin uygulamayı vurgula")}
            />
          </>
        )}
      </SectionCard>

      <SectionCard
        title={t("Parlaklık")}
        enabledLabel={t("Parlaklık kartını göster")}
        enabledHint={t("Ekran parlaklığı değişince düzeyi gösterir.")}
        f={f}
        section="brightness"
      >
        {(off) => <Duration f={f} section="brightness" off={off} />}
      </SectionCard>
    </Section>
  );
}
