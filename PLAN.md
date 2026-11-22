# Dockora — Uygulama Planı

Windows 11 görev çubuğunun yerine geçen, kişisel kullanım için bir dock.
Yığın: **Tauri 2 + Rust + React + TypeScript + Vite**.

> **Dürüstlük notu:** Bu plan Linux ortamında yazıldı, hiçbir kısmı Windows'ta
> çalıştırılıp denenmedi. API adları hafızamdan; **(doğrula)** işaretli yerleri
> resmi dokümandan kontrol et. Sürüm numarası bilerek yazmadım, kurulumda en
> güncel kararlı sürümleri kullan.

---

## 0. Kapsam

**Var:** dock penceresi, uygulama ikonları (sabit + açık) ve büyütme efekti,
saat, müzik, hava durumu, takvim, ses/pil/Wi-Fi/Bluetooth, ayarlar, tepsi ikonu,
açılışta başlama.

**Sonraya:** AI limitleri, bildirim rozetleri, çoklu monitör, 100+ widget.

**Yok:** Başlat menüsü ve sistem tepsisinin yeniden yazımı. Başlat için Windows'un
kendi menüsü açılır (Win tuşu simülasyonu).

---

## 1. Kurulum (Windows 11)

1. **Git**
2. **Node.js LTS**
3. **Rust** — `rustup` ile, varsayılan `stable-x86_64-pc-windows-msvc`
4. **Visual Studio Build Tools** — "Desktop development with C++" iş yükü
   (MSVC + Windows SDK). Rust'ın linker'ı için şart.
5. **WebView2 Runtime** — Windows 11'de yüklü gelir, kontrol etmen yeterli.
6. **VS Code eklentileri:** `rust-analyzer`, `Tauri`, `ESLint`, `Prettier`,
   `Even Better TOML`, `CodeLLDB` (Rust debug için).

Kontrol: `node -v`, `cargo -V`, `rustc -V`.

### Projeyi oluşturma

Repo boş değil (bu dosya var), bu yüzden geçici klasöre kurup taşı:

```powershell
git clone <repo> ; cd dockora
git checkout claude/windows-vscode-project-plan-ggrjt9
npm create tauri-app@latest app-tmp     # React + TypeScript + npm seç
# app-tmp içindeki her şeyi (gizli dosyalar dahil) repo köküne taşı, app-tmp'yi sil
npm install
npm run tauri dev
```

İlk Rust derlemesi uzun sürer (birkaç dakika), sonrakiler hızlıdır.

---

## 2. Mimari

```
┌──────────── Frontend (WebView2) ────────────┐
│ React widget'ları, animasyon, popup'lar      │
└──────▲───────────────────────┬──────────────┘
  event (emit)            command (invoke)
┌──────┴───────────────────────▼──────────────┐
│ Rust: sistem servisleri (Win32 / WinRT)      │
│ pencereler, medya, ses, pil, radyo, appbar   │
└──────────────────────────────────────────────┘
```

**Kural:** Windows'a dokunan her şey Rust'ta. Frontend sadece çizer.

**İletişim deseni (her servis için aynı):**
- `get_<şey>` komutu → açılışta ilk durumu verir.
- `<şey>://changed` olayı → durum değişince tam yeni durumu gönderir.
- Eylem komutları (`media_next`, `set_volume` ...).

Rust tarafı son durumu `Mutex` içinde önbellekte tutar; frontend hiçbir zaman
sistemden doğrudan okumaz.

### Klasör yapısı

```
src/                          # frontend
  main.tsx                    # hash route'a göre pencere seçer
  windows/
    DockWindow.tsx
    PopupWindow.tsx           # #/popup/<widgetId>
    SettingsWindow.tsx
  widgets/
    registry.ts               # tüm widget tanımları
    types.ts
    apps/  clock/  media/  weather/  calendar/  volume/  battery/  radios/
      index.ts                # WidgetDefinition
      Bar.tsx                 # dock üstündeki görünüm
      Popup.tsx               # (varsa) açılır panel
  lib/
    ipc.ts                    # tipli invoke/listen sarmalayıcıları
    useService.ts             # get + listen birleştiren hook
    store.ts                  # zustand: config + servis durumları
    magnify.ts
  styles/
    tokens.css                # renk, radius, boşluk değişkenleri
src-tauri/
  tauri.conf.json
  capabilities/default.json   # JS'in çağırabileceği izinler
  src/
    main.rs / lib.rs          # kurulum, tepsi, komut kaydı
    config.rs                 # config.json oku/yaz
    error.rs
    win/
      appbar.rs               # yer ayırma + tam ekran bildirimi
      taskbar.rs              # Windows çubuğunu gizle/geri getir
      window_style.rs         # NOACTIVATE, TOOLWINDOW, blur
    services/
      windows_list.rs         # açık pencereler
      icons.rs
      launcher.rs
      media.rs
      audio.rs
      power.rs
      radios.rs
    commands.rs
```

---

## 3. Pencere modeli

### 3.1 Dock penceresi

`tauri.conf.json` içinde (anahtar adlarını **doğrula**):

```json
{
  "label": "dock",
  "decorations": false,
  "transparent": true,
  "alwaysOnTop": true,
  "skipTaskbar": true,
  "resizable": false,
  "shadow": false,
  "visible": false
}
```

Açılışta Rust tarafında:
1. Birincil monitörün boyutunu al → pencereyi alt kenara tam genişlikte yerleştir.
2. Genişletilmiş stiller: `WS_EX_TOOLWINDOW` (Alt+Tab'da görünmesin),
   `WS_EX_NOACTIVATE` (tıklayınca odağı çalmasın — bkz. 5.4).
   Tauri'nin yeni sürümlerinde `focusable: false` ayarı olabilir **(doğrula)**;
   yoksa `SetWindowLongPtrW(GWL_EXSTYLE)`.
3. AppBar kaydı (bölüm 4).
4. Pencereyi göster.

**Ölçüler (mantıksal piksel, başlangıç değerleri):**
bar yüksekliği 56, ikon 40, büyütülmüş ikon 72, büyütme için üst pay 40.

### 3.2 Büyütme taşması sorunu (önemli tasarım kararı)

Büyüyen ikonlar çubuğun üstüne taşar. Şeffaf pencere yüksek olursa, görünmeyen
kısım alttaki uygulamaya giden tıklamaları yutar.

- **Plan A (önerilen, basit):** Pencere normalde bar yüksekliğinde. Fare dock'a
  girince pencereyi yukarı doğru büyüt (bar + pay), çıkınca ~150 ms sonra küçült.
  Risk: yeniden boyutlandırmada titreme olabilir.
- **Plan B (titreme olursa):** Pencere hep yüksek. Rust'ta ~30 ms'de bir
  `GetCursorPos` ile imleç konumuna bak; imleç görünür bölgenin dışındaysa
  `set_ignore_cursor_events(true)`, içindeyse `false`. Görünür dikdörtgenleri
  frontend Rust'a bildirir.

Önce A'yı dene.

### 3.3 Arka plan

CSS `backdrop-filter` pencerenin **arkasındaki** masaüstünü bulanıklaştırmaz.
Gerçek cam efekti için `window-vibrancy` crate'i (`apply_acrylic` / `apply_mica`)
**(doğrula: Tauri 2 uyumu)**. İlk aşamada yarı saydam düz renk yeterli.

### 3.4 Popup pencereleri

Takvim, hava durumu gibi paneller **ayrı Tauri pencereleri**:
- Aynı frontend paketi, farklı route: `index.html#/popup/calendar`.
- Açılışta gizli olarak önceden oluştur (anında açılsın).
- Konum: tıklanan widget'ın ekran koordinatı → üstüne ortala, ekran kenarına sıkıştır.
- Odak kaybedince (`blur`) gizle. `Esc` ile gizle.
- Bunlar odak alabilir (NOACTIVATE yok).
- Aynı anda tek popup açık.

---

## 4. Ekranda yer ayırma ve Windows görev çubuğu

### 4.1 AppBar (`win/appbar.rs`)

Amaç: büyütülmüş pencereler dock'un altına girmesin.

1. `SHAppBarMessage(ABM_NEW)` — `APPBARDATA.hWnd` = dock HWND,
   `uCallbackMessage` = kendi `WM_APP+1` mesajın.
2. `ABM_QUERYPOS` → `ABM_SETPOS`, `uEdge = ABE_BOTTOM`, yükseklik = bar yüksekliği.
3. Geri bildirim mesajlarını almak için pencereyi alt sınıfla (`SetWindowSubclass`):
   - `ABN_POSCHANGED` → konumu yeniden hesapla.
   - `ABN_FULLSCREENAPP` → tam ekran uygulama/oyunda dock'u gizle, çıkınca göster.
4. Çıkışta **mutlaka** `ABM_REMOVE`. Çökme durumunda ayrılan alan takılı
   kalabilir **(doğrula)**; çözüm: Explorer'ı yeniden başlatmak.

Koordinatlar **fiziksel piksel**; DPI ölçeğiyle çarpmayı unutma.

### 4.2 Windows çubuğunu gizleme (`win/taskbar.rs`)

Resmi "kaldır" yolu yok. Yöntem:

1. `SHAppBarMessage(ABM_SETSTATE, ABS_AUTOHIDE)` ile otomatik gizlemeyi aç →
   çubuk ekranda yer ayırmayı bırakır.
2. `FindWindowW("Shell_TrayWnd")` (+ ikinci monitörler için
   `Shell_SecondaryTrayWnd`) → `ShowWindow(SW_HIDE)`.
3. Explorer çubuğu geri gösterebilir → `EVENT_OBJECT_SHOW` olayında veya
   birkaç saniyede bir kontrol edip yeniden gizle.
4. Çıkışta: `SW_SHOW` + eski otomatik gizleme durumunu geri yükle.

**Güvenlik ağı:** Eski durumu diske yaz. Uygulama çökerse bir sonraki açılışta
veya ayrı bir `restore-taskbar.ps1` betiğiyle geri getir. Bu betiği 3. aşamadan
önce yaz ve dene.

**Geliştirme sırasında** bu özelliği kapalı tut (`config.hideWindowsTaskbar = false`)
ve dock'u ekranın **üstüne** koy; iki çubuk çakışmasın.

Bilinmeyenler **(doğrula)**: çubuk gizliyken Win tuşu / Başlat menüsü ve
bildirim merkezi normal çalışıyor mu; Windows güncellemeleri davranışı
değiştirebilir.

---

## 5. Uygulamalar widget'ı (en zor parça)

### 5.1 Açık pencereleri listeleme (`services/windows_list.rs`)

`EnumWindows` ile gez, şu filtrelerden geçenleri al:
- `IsWindowVisible` doğru
- Sahibi yok: `GetWindow(hwnd, GW_OWNER)` boş
- `WS_EX_TOOLWINDOW` yok (veya `WS_EX_APPWINDOW` var)
- Gizlenmemiş: `DwmGetWindowAttribute(DWMWA_CLOAKED) == 0`
  (UWP hayalet pencereleri ve diğer sanal masaüstleri bununla elenir)
- Başlığı boş değil
- Kendi pencerelerimiz değil

Her pencere için:
- `GetWindowThreadProcessId` → `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`
  → `QueryFullProcessImageNameW` → exe yolu
- AppUserModelID: `SHGetPropertyStoreForWindow` → `PKEY_AppUserModel_ID`
  (yoksa exe yolu kimlik olur)
- UWP uygulamaları `ApplicationFrameHost.exe` altında görünür; gerçek kimlik
  AUMID'den gelir.

**Gruplama anahtarı:** AUMID varsa o, yoksa küçük harfli exe yolu.

```ts
type AppItem = {
  id: string;            // gruplama anahtarı
  name: string;
  icon: string;          // data URL
  pinned: boolean;
  windows: { hwnd: number; title: string }[];
  focused: boolean;
};
```

### 5.2 Değişiklikleri izleme

- **v1:** 750 ms'de bir yeniden listele, öncekiyle karşılaştır, fark varsa
  `apps://changed` gönder. Basit ve yeterli.
- **v2:** `SetWinEventHook` (`WINEVENT_OUTOFCONTEXT`) ile
  `EVENT_SYSTEM_FOREGROUND`, `EVENT_OBJECT_SHOW/HIDE/DESTROY`,
  `EVENT_OBJECT_CLOAKED/UNCLOAKED`, `EVENT_OBJECT_NAMECHANGE`. Hook'u kuran
  iş parçacığında mesaj döngüsü gerekir. Olayları ~100 ms biriktirip tek
  seferde yeniden listele.

### 5.3 İkonlar (`services/icons.rs`)

- Tek yol (exe ve UWP için ortak olduğuna inanıyorum, **doğrula**):
  `SHCreateItemFromParsingName` → exe yolu veya `shell:AppsFolder\<AUMID>` →
  `IShellItemImageFactory::GetImage(256x256)` → `HBITMAP`.
- `GetDIBits` ile BGRA piksel al → RGBA'ya çevir → `image` crate'i ile PNG →
  base64 data URL.
- Kimliğe göre bellekte önbellekle; aynı ikonu iki kez çıkarma.
- Yedek yol: `SHGetFileInfoW` (küçük ikon verir).

### 5.4 Tıklama davranışı

| Durum | Sol tık |
|---|---|
| Uygulama kapalı (sabit) | Başlat |
| Açık, odakta değil | Öne getir (simge durumundaysa `SW_RESTORE`) + `SetForegroundWindow` |
| Açık ve odakta | Simge durumuna küçült |
| Birden çok pencere | Sırayla dolaş (v2: küçük önizleme listesi) |

- **Odak tuzağı:** Dock tıklamayla odağı alırsa "odakta mı?" kontrolü hep
  yanlış çıkar. İki önlem: (1) dock `WS_EX_NOACTIVATE`, (2) Rust tarafında
  "dock olmayan son ön plan penceresi"ni ayrıca tut.
- `SetForegroundWindow` bazen reddedilir (Windows kısıtı). Yedek:
  `AttachThreadInput` hilesi **(doğrula)**.
- Orta tık: yeni pencere. Sağ tık: menü (Sabitle/Kaldır, Pencereyi kapat →
  `PostMessageW(WM_CLOSE)`).

### 5.5 Başlatma ve sabitleme (`services/launcher.rs`)

- Exe: `ShellExecuteW` ("open"). UWP: `explorer.exe shell:AppsFolder\<AUMID>`.
- Sabitleme: açık uygulamaya sağ tık → "Sabitle" (kimlik + yol config'e yazılır).
- v2: ayarlar penceresine `.lnk`/`.exe` sürükle-bırak (`.lnk` çözümü: `IShellLinkW`).
- Başlat düğmesi: Win tuşu gönder (`SendInput`, `VK_LWIN`).

### 5.6 Büyütme efekti (`lib/magnify.ts`)

- `mousemove`'da her ikonun merkezine uzaklığı hesapla:
  `ölçek = 1 + (maks − 1) · cos²(π/2 · min(|d| / menzil, 1))`
  (maks ≈ 1.8, menzil ≈ 3 ikon genişliği).
- **Genişliği de** değiştir (sadece `transform` değil) ki komşular itilsin;
  `transform-origin: bottom`.
- React state kullanma: `requestAnimationFrame` içinde `ref` üzerinden doğrudan
  `style` yaz. Her fare hareketinde yeniden render performansı öldürür.
- Fare çıkınca CSS geçişiyle 1'e dön.
- Açık uygulamanın altında nokta, odaktakinde daha parlak nokta.

### 5.7 Rozetler (ertelendi)

Görev çubuğu rozetlerini (`ITaskbarList3::SetOverlayIcon`) başka bir uygulamadan
okumanın açık bir API'sini bilmiyorum. Olası yol `UserNotificationListener`
(WinRT), ama paket kimliği/izin gerektirebilir **(doğrula)**. Şimdilik yok.

---

## 6. Diğer widget'lar

### 6.1 Saat
Tamamen frontend. `Intl.DateTimeFormat('tr-TR')`. Zamanlayıcıyı dakika başına
hizala (her saniye render etme). Tıklayınca takvim popup'ı.

### 6.2 Müzik (`services/media.rs`)
WinRT `Windows.Media.Control`:
- `GlobalSystemMediaTransportControlsSessionManager::RequestAsync()`
- `GetCurrentSession()` → `TryGetMediaPropertiesAsync()` (başlık, sanatçı,
  küçük resim), `GetPlaybackInfo()` (çalıyor/duraklı),
  `GetTimelineProperties()` (konum/süre)
- Olaylar: `CurrentSessionChanged`, `MediaPropertiesChanged`, `PlaybackInfoChanged`
- Kontroller: `TryTogglePlayPauseAsync`, `TrySkipNextAsync`, `TrySkipPreviousAsync`

Spotify, tarayıcı, çoğu oynatıcı bu arayüzle görünür. Küçük resim akışını
bayta çevirip base64 gönder. Kart rengi: kapak resminin ortalama rengini
frontend'de `canvas` ile çıkar. Oturum yoksa widget gizlenir.

```ts
type MediaState = {
  title: string; artist: string; artwork?: string;
  playing: boolean; positionMs?: number; durationMs?: number;
  sourceApp: string;
} | null;
```

### 6.3 Hava durumu
**Open-Meteo** — ticari olmayan kullanımda ücretsiz ve anahtarsız olduğuna
inanıyorum **(doğrula: open-meteo.com koşulları)**.

- Tahmin: `https://api.open-meteo.com/v1/forecast?latitude=..&longitude=..&current=temperature_2m,weather_code&hourly=temperature_2m,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto`
- Şehir arama: `https://geocoding-api.open-meteo.com/v1/search?name=Istanbul`

Parametre adlarını **doğrula**. Ayrıntılar:
- Şehir ayarlardan seçilir, enlem/boylam config'e yazılır.
- 20 dakikada bir yenile; son yanıtı önbellekle (çevrimdışıyken eskiyi göster).
- WMO hava kodu → ikon + Türkçe metin eşleme tablosu.
- İstek frontend `fetch` ile; CORS/CSP sorun çıkarırsa Rust `reqwest`'e taşı.
- Bar: ikon + sıcaklık. Popup: saatlik şerit + 7 günlük liste.

### 6.4 Takvim
- Ay ızgarası: `date-fns`, hafta Pazartesi başlar, bugün vurgulu.
- **Etkinlikler v1 — ICS aboneliği (OAuth yok):** Google Takvim'in
  "iCal biçiminde gizli adres"ini ayarlara yapıştır → `ical.js` ile ayrıştır,
  15 dakikada bir yenile. Salt okunur. Bu adres gizlidir; yalnızca yerel
  config'te dursun, repoya girmesin.
- **v2:** Google Calendar API + OAuth (masaüstü, loopback + PKCE). Google Cloud
  projesi gerektirir; ancak ICS yetmezse.
- Tekrarlayan etkinlikler (RRULE) ICS'te zahmetlidir; kütüphanenin desteğini
  **doğrula**.

### 6.5 Ses (`services/audio.rs`)
Core Audio (COM):
- `IMMDeviceEnumerator::GetDefaultAudioEndpoint(eRender, eMultimedia)` →
  `Activate::<IAudioEndpointVolume>`
- `GetMasterVolumeLevelScalar` / `SetMasterVolumeLevelScalar`, `GetMute` / `SetMute`
- Aygıt adı: özellik deposundan `PKEY_Device_FriendlyName`
- Değişim: v1 500 ms yoklama; v2 `IAudioEndpointVolumeCallback` +
  `IMMNotificationClient` (varsayılan aygıt değişimi)

Bar: kaydırıcı + yüzde; tekerlekle ±2; ikona tık sessize alır.

### 6.6 Pil (`services/power.rs`)
`GetSystemPowerStatus` → yüzde, şarjda mı. 30 sn yoklama.
`BatteryFlag` "pil yok" (128) ise widget gizlenir (masaüstü PC).

### 6.7 Wi-Fi / Bluetooth (`services/radios.rs`)
- WinRT `Windows.Devices.Radios`: `Radio::RequestAccessAsync`,
  `Radio::GetRadiosAsync`, `Kind`, `State`, `SetStateAsync`.
  Paketsiz masaüstü uygulamada çalıştığına inanıyorum ama emin değilim **(doğrula)**.
- Ağ adı (SSID): `WlanQueryInterface` veya `netsh wlan show interfaces` çıktısı.
  Yeni Windows 11 sürümlerinde konum izni isteyebilir **(doğrula)**.
- Çalışmazsa yedek: tıklayınca `ms-settings:network-wifi` /
  `ms-settings:bluetooth` aç.

### 6.8 AI limitleri (en son, belirsiz)
Bildiğim kadarıyla Claude/ChatGPT **abonelik** limitleri için resmi API yok.
Seçenekler:
1. **Claude Code yerel kayıtları:** `~/.claude/projects` altındaki JSONL
   dosyalarından token kullanımı tahmini. Resmi limit yüzdesi değil, tahmindir.
   Dosya biçimi değişebilir **(doğrula)**.
2. **API kullanımı:** Anthropic ve OpenAI'ın yönetici kullanım/maliyet uç
   noktaları — yalnızca API faturalı hesaplar için, abonelik için değil **(doğrula)**.
3. Oturum çereziyle resmi olmayan web uç noktaları — **önermiyorum**: kırılgan,
   kullanım koşullarını ihlal edebilir.

Karar: diğer her şey bitince 1 numarayı dene.

---

## 7. Widget sistemi

```ts
// widgets/types.ts
export interface WidgetDefinition<C = unknown> {
  id: string;                      // "media"
  name: string;                    // "Müzik"
  Bar: React.FC<{ config: C }>;
  Popup?: React.FC<{ config: C }>;
  popupSize?: { width: number; height: number };
  defaultConfig: C;
  Settings?: React.FC<{ config: C; onChange(c: C): void }>;
}
```

`registry.ts` hepsini tek nesnede toplar. Yeni widget = yeni klasör + kayıt satırı.

### Config (`%APPDATA%\<uygulama>\config.json`)

```json
{
  "version": 1,
  "position": "bottom",
  "hideWindowsTaskbar": false,
  "autostart": false,
  "theme": { "accent": "#6aa8ff", "opacity": 0.6, "radius": 14 },
  "layout": {
    "left":   [{ "id": "apps" }],
    "center": [{ "id": "media" }, { "id": "weather" }, { "id": "clock" }],
    "right":  [{ "id": "radios" }, { "id": "volume" }, { "id": "battery" }]
  },
  "widgets": {
    "apps":     { "pinned": [], "magnify": 1.8 },
    "weather":  { "lat": 41.01, "lon": 28.98, "city": "İstanbul", "unit": "C" },
    "calendar": { "icsUrls": [] }
  }
}
```

- Okuma/yazma Rust'ta (`serde`), atomik yazım (geçici dosya + yeniden adlandır).
- Değişince `config://changed` olayı → tüm pencereler güncellenir.
- Bozuk dosyada varsayılana dön, eskisini `.bak` olarak sakla.
- `version` alanı ileride göç (migration) için.

---

## 8. Komut ve olay tablosu

| Servis | Komutlar | Olay |
|---|---|---|
| config | `get_config`, `set_config` | `config://changed` |
| apps | `get_apps`, `activate_app(id)`, `launch_app(id)`, `close_window(hwnd)`, `pin_app(id)`, `unpin_app(id)`, `open_start` | `apps://changed` |
| media | `get_media`, `media_toggle`, `media_next`, `media_prev` | `media://changed` |
| audio | `get_audio`, `set_volume(v)`, `set_mute(b)` | `audio://changed` |
| power | `get_power` | `power://changed` |
| radios | `get_radios`, `set_radio(kind, on)` | `radios://changed` |
| pencere | `show_popup(widgetId, rect)`, `hide_popup`, `set_dock_expanded(b)`, `open_settings` | `fullscreen://changed` |

Frontend'de tek hook:

```ts
function useService<T>(getCmd: string, event: string): T | undefined
// açılışta invoke(getCmd), sonra listen(event) ile güncelle
```

Tauri 2'de JS'ten çağrılan her yerleşik API için `capabilities/default.json`
içine izin eklemek gerekir **(doğrula)**. "not allowed" hatası alırsan ilk
bakacağın yer orası.

---

## 9. Rust tarafı kuralları

- **Bağımlılıklar:** `tauri`, `serde`, `serde_json`, `windows`, `thiserror`,
  `image`, `base64`, `window-vibrancy`; eklentiler: `tauri-plugin-autostart`,
  `tauri-plugin-single-instance`.
- **`windows` crate özellikleri (yaklaşık liste, doğrula):**
  `Win32_Foundation`, `Win32_UI_WindowsAndMessaging`, `Win32_UI_Shell`,
  `Win32_UI_Shell_PropertiesSystem`, `Win32_UI_Accessibility`,
  `Win32_UI_Input_KeyboardAndMouse`, `Win32_Graphics_Dwm`, `Win32_Graphics_Gdi`,
  `Win32_System_Threading`, `Win32_System_Com`, `Win32_System_Power`,
  `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`,
  `Win32_Devices_FunctionDiscovery`, `Media_Control`, `Devices_Radios`,
  `Storage_Streams`, `Foundation`.
  Eksik olanı derleyici hatası söyler; doküman her fonksiyonun özelliğini yazar.
- **İş parçacıkları:** Her servis kendi iş parçacığında. COM kullanan her iş
  parçacığı başta `CoInitializeEx` çağırır. Ana (UI) iş parçacığını asla bloklama.
- **Kaynak sızıntısı:** `HANDLE`, `HBITMAP`, `HICON` mutlaka kapatılır
  (`CloseHandle`, `DeleteObject`, `DestroyIcon`). Sürekli açık uygulamada küçük
  sızıntı günler içinde büyür.
- **Hata:** `thiserror` ile tek `AppError`; komutlar `Result<T, String>` döner.
  Bir servis çökerse dock ayakta kalır, o widget "kullanılamıyor" gösterir.
- **`unsafe`:** Win32 çağrıları kaçınılmaz olarak `unsafe`. Her modülde güvenli
  sarmalayıcı fonksiyonlar yaz, `unsafe`'i dışarı sızdırma.

---

## 10. Frontend kuralları

- **Durum:** `zustand` (config + servis durumları).
- **Stil:** CSS değişkenleri (`tokens.css`) + CSS Modules. Tema config'ten
  `:root` değişkenlerine yazılır.
- **Yönlendirme:** Kütüphane yok; `location.hash`'e bakan basit seçim.
- **Performans:** Animasyonda yalnızca `transform`/`opacity`/`width`;
  fare dock'ta değilken rAF döngüsü durur; listelerde `React.memo`.
- **Erişilebilirlik:** Düğmeler gerçek `<button>`, `title` ipuçları.
- Metin seçimi ve sağ tık tarayıcı menüsü kapalı (`user-select: none`,
  `contextmenu` engeli).

---

## 11. Kabuk özellikleri

- **Tepsi ikonu:** Ayarlar · Yeniden başlat · Windows çubuğunu göster/gizle · Çıkış.
- **Tek örnek:** `tauri-plugin-single-instance`.
- **Açılışta başlat:** `tauri-plugin-autostart`.
- **Temiz çıkış:** AppBar kaldır → Windows çubuğunu geri getir → çık.
  Panik kancasında (`panic::set_hook`) da aynısı.
- **Ekran değişimi:** Çözünürlük/DPI/monitör değişince yeniden konumlan ve
  AppBar'ı güncelle.
- **Çoklu monitör:** v1 yalnızca birincil ekran.

---

## 12. Aşamalar

Her aşama sonunda çalışan bir uygulama olmalı. Sırayı bozma.

### Aşama 0 — Kurulum
- [ ] Araçlar kurulu, `npm run tauri dev` şablon pencereyi açıyor
- [ ] Klasör yapısı (bölüm 2) oluşturuldu, ESLint + Prettier ayarlı

### Aşama 1 — İskelet dock
- [ ] Çerçevesiz, şeffaf, üstte duran tam genişlik pencere (ekranın **üstünde**)
- [ ] Alt+Tab'da ve görev çubuğunda görünmüyor
- [ ] Sabit kodlu 6–8 sahte ikon + büyütme efekti (60 fps, takılmasız)
- [ ] Taşma çözümü (3.2 Plan A)
- [ ] Saat widget'ı
- [ ] Tepsi ikonu + Çıkış

**Bitti ölçütü:** Görsel olarak dock'a benziyor, sistemle bağı yok.

### Aşama 2 — Gerçek uygulamalar
- [ ] `get_apps` açık pencereleri doğru listeliyor (Chrome, VS Code, Dosya
      Gezgini, Ayarlar [UWP], Spotify ile dene)
- [ ] Gerçek ikonlar
- [ ] Tıkla: öne getir / küçült / dolaş
- [ ] Yoklama ile canlı güncelleme
- [ ] Sabitleme + başlatma, config'e kayıt
- [ ] Sağ tık menüsü
- [ ] Başlat düğmesi

**Bitti ölçütü:** Bir gün boyunca uygulama geçişini yalnızca dock'tan yapabiliyorsun.

### Aşama 3 — Görev çubuğunun yerini alma
- [ ] `restore-taskbar.ps1` yazıldı ve denendi
- [ ] AppBar yer ayırıyor; büyütülmüş pencereler dock'un altına girmiyor
- [ ] Tam ekranda dock gizleniyor
- [ ] Windows çubuğu gizleniyor, çıkışta geri geliyor
- [ ] Dock alta alındı
- [ ] Açılışta başlat + tek örnek

**Bitti ölçütü:** Yeniden başlatmadan sonra yalnızca dock var; uygulamayı
öldürünce (Görev Yöneticisi) sistem kurtarılabiliyor.

### Aşama 4 — Sistem widget'ları
- [ ] Config dosyası + `useService` deseni + widget kayıt sistemi
- [ ] Müzik
- [ ] Ses
- [ ] Pil
- [ ] Wi-Fi / Bluetooth
- [ ] Hava durumu (yalnızca bar görünümü)

### Aşama 5 — Popup'lar
- [ ] Popup pencere altyapısı (konumlama, odak kaybında kapanma)
- [ ] Hava durumu paneli
- [ ] Takvim ay görünümü
- [ ] ICS etkinlikleri

### Aşama 6 — Ayarlar
- [ ] Ayarlar penceresi: widget aç/kapat, sürükleyerek sırala
- [ ] Tema (vurgu rengi, saydamlık, köşe)
- [ ] Şehir arama, ICS adresi, sabitlenenleri düzenleme

### Aşama 7 — Cila
- [ ] Yoklamaları olaylara çevir (WinEventHook, ses geri çağrıları)
- [ ] Akrilik/mika arka plan
- [ ] Açılış/kapanış animasyonları, popup geçişleri
- [ ] Boşta CPU ölçümü (hedef: ~%0), bellek sızıntısı kontrolü (24 saat açık bırak)
- [ ] `npm run tauri build` ile kurulum dosyası

### Aşama 8 — İsteğe bağlı
- [ ] AI limitleri (6.8)
- [ ] Çoklu monitör
- [ ] Pencere önizlemeleri (DWM thumbnail API)
- [ ] Rozetler

---

## 13. Riskler ve doğrulanacaklar

| # | Konu | Risk | Önlem |
|---|---|---|---|
| 1 | Windows çubuğunu gizleme | Güncellemeyle bozulabilir, Explorer geri getirebilir | Ayarla aç/kapa, kurtarma betiği |
| 2 | `SetForegroundWindow` | Windows reddedebilir | NOACTIVATE + `AttachThreadInput` yedeği |
| 3 | UWP ikon/kimlik | `shell:AppsFolder` yolu varsayım | Aşama 2'nin ilk işi olarak küçük deneme |
| 4 | Şeffaf pencere + yeniden boyutlandırma | Titreme | Plan B (imleç yoklama) |
| 5 | Radios API | Paketsiz uygulamada çalışmayabilir | `ms-settings:` bağlantısı |
| 6 | SSID okuma | Konum izni | Yalnızca açık/kapalı göster |
| 7 | Bellek | WebView2 tabanı yüksek; birkaç yüz MB olabilir (tahmin, ölç) | Popup'ları tek pencerede birleştir |
| 8 | AI limitleri | Resmi API yok | En sona, yalnızca yerel tahmin |
| 9 | Çökme | Ayrılmış alan/gizli çubuk takılı kalır | Panik kancası + açılışta onarım |

---

## 14. Eve gidince ilk oturum (≈2 saat)

1. Bölüm 1'deki araçları kur, projeyi oluştur, şablon pencereyi gör.
2. `tauri.conf.json`'da pencereyi bölüm 3.1'e göre ayarla.
3. `DockWindow.tsx`: yarı saydam yuvarlak çubuk + 8 renkli kare.
4. `magnify.ts`: büyütme formülü, rAF ile.
5. Rust `setup` içinde: pencereyi ekranın üstüne tam genişlikte yerleştir.
6. Commit.

Sonraki oturum: 3.2 taşma çözümü + tepsi ikonu, ardından Aşama 2.
