# Dockora

🇹🇷 Türkçe · [🇬🇧 English](README.en.md)

**Dockora**, Windows 11 görev çubuğunun yerine
geçen, cam görünümlü bir dock. Açık uygulamalar, saat ve takvim, müzik (Spotify
sıradakiler dahil), hava durumu, ses, pil, Wi-Fi ve Bluetooth tek çubukta.

![Dockora dock](docs/screenshots/dock.png)

<p align="center">
  <img src="docs/screenshots/weather.png" alt="Hava durumu" width="32%">
  <img src="docs/screenshots/ai-usage.png" alt="Yapay zekâ kullanımı" width="32%">
  <img src="docs/screenshots/battery.png" alt="Pil ve güç" width="32%">
</p>
<p align="center">
  <img src="docs/screenshots/calendar.png" alt="Takvim" width="58%">
  <img src="docs/screenshots/volume.png" alt="Ses" width="38%">
</p>

## Kurulum

1. [Releases](https://github.com/Bthncko/dockora/releases/latest) sayfasından
   `Akilli.Dock_…_x64-setup.exe` dosyasını indir ve çalıştır. Yönetici izni istemez.
2. Windows "Bilgisayarınız korundu" (SmartScreen) uyarısı gösterirse
   **Ek bilgi → Yine de çalıştır**'a tıkla. Uygulama ücretli bir kod imzası
   taşımadığı için bu uyarı çıkıyor.
3. Kurulum bitince dock ekranın altında açılır ve bundan sonra Windows'la birlikte
   başlar.

## İlk ayarlar

- **Windows'un kendi çubuğunu gizlemek için:** dock'un boş bir yerine sağ tıkla →
  **Windows görev çubuğunu gizle**. Dock kapanınca Windows çubuğu geri gelir.
- **Ayarlar:** aynı menüden **Ayarlar…**. Renk, cam, boyut, widget'ların yeri,
  hava durumu şehri, takvim abonelikleri ve dil burada.
- **Takvim:** Ayarlar → Saat ve takvim → Google Takvim'in "gizli iCal adresi"ni
  yapıştır. Adres gizlidir, kimseyle paylaşma.

## Diller

| Dil                 | Durum  |
| ------------------- | ------ |
| Türkçe              | ✅ Tam |
| English (İngilizce) | ✅ Tam |

Dil varsayılan olarak Windows'un görüntüleme dilini izler (Türkçe Windows'ta Türkçe,
diğerlerinde İngilizce). **Ayarlar → Genel → Dil** bölümünden istediğin zaman
değiştirebilirsin; dock, panelleri, tepsi menüsü ve bildirimler hemen yeni dile geçer.

Yeni bir dil eklemek istersen arayüz metinleri iki yerde: dock ve ayarlar için
[`src/locales/en.ts`](src/locales/en.ts) (kaynak metin Türkçe, her satır bir çeviriye
eşlenir), tepsi menüsü / sağ tık menüleri / bildirim kartları için
[`src-tauri/src/i18n.rs`](src-tauri/src/i18n.rs). Katkılar açık.

## Güncellemeler

Yeni sürüm çıkınca dock bunu birkaç saat içinde kendisi fark eder, indirir ve kurar:
küçük bir ilerleme penceresi açılır, dock birkaç saniyeliğine kapanıp yeniden açılır.
Ayarlar → Genel → Güncellemeler'den kapatılabilir ya da **Şimdi denetle** ile hemen
denetlenebilir.

## Spotify sıradakiler

Müzik karosuna tıklayınca açılan panelde Spotify'ın sıradaki şarkıları görünür.
Bunun için Spotify Premium ve bir Spotify geliştirici uygulaması gerekir; adımlar
Ayarlar → Müzik sayfasında. İki yol var:

- **Kendi uygulamanı aç** (önerilen): sayfadaki 4 adımı izle, kendi Client ID'ni yapıştır.
- **Arkadaşının uygulamasını kullan:** uygulamanın sahibi seni developer.spotify.com
  → uygulama → **User Management**'a Spotify e-postanla ekler, sen de onun Client
  ID'sini yapıştırırsın. Geliştirme modunda bir uygulamayı en çok 5 kişi kullanabilir.

## Sorun olursa

- **Görev çubuğu kayboldu, geri gelmiyor:** Görev Yöneticisi'nde (Ctrl+Shift+Esc)
  "Dockora"yı kapat; çubuk geri gelir. Gelmezse
  [restore-taskbar.ps1](scripts/restore-taskbar.ps1) dosyasını indirip sağ tıkla →
  **PowerShell ile çalıştır**.
- **Kaldırmak:** Windows Ayarlar → Uygulamalar → Yüklü uygulamalar → Dockora →
  Kaldır.

## Gizlilik

Dock hiçbir kullanım verisi ya da istatistik toplamaz, kendi sunucusu yoktur.
Ayarlar ve girişler yalnızca bu bilgisayarda, `%APPDATA%\com.proxd.akillidock`
klasöründe durur. Dışarıya yalnızca şu bağlantılar yapılır:

| Ne için                     | Nereye                                    | Ne gider                                     |
| --------------------------- | ----------------------------------------- | -------------------------------------------- |
| Hava durumu                 | `api.open-meteo.com`                      | Seçilen şehrin koordinatları                 |
| Şehir arama                 | `geocoding-api.open-meteo.com`            | Yazılan şehir adı                            |
| Takvim                      | Yapıştırdığın iCal adresleri              | Yalnızca adresin kendisi (indirme)           |
| Spotify (isteğe bağlı)      | `accounts.spotify.com`, `api.spotify.com` | Kendi Client ID'n ve Spotify girişin         |
| AI kullanımı (isteğe bağlı) | `api.anthropic.com`                       | Claude Code'un oturum token'ı                |
| Güncellemeler               | `github.com` (bu depo)                    | Hiçbir şey; yalnızca sürüm dosyası indirilir |

- **AI kullanımı widget'ı:** Claude Code kuruluysa `~/.claude/.credentials.json`
  dosyasındaki oturum token'ını **yalnızca okur** ve sadece Anthropic'e, limitlerini
  sormak için gönderir; token kaydedilmez, değiştirilmez, yenilenmez. Bilgisayardaki
  Claude Code kayıtlarından yalnızca sayılar (istek ve token adedi, model) çıkarılır;
  mesaj metinleri saklanmaz ve hiçbir yere gönderilmez. Widget'ı kaldırınca hiçbiri
  yapılmaz.
- **Bildirimler:** "Bildirimler dock tarzında" açıkken Windows bildirimleri dock'un
  kendi kartlarında gösterilir; içerikleri bilgisayardan çıkmaz.
- **Güncellemeler imzalıdır:** dock yalnızca bu deponun imza anahtarıyla imzalanmış
  sürümleri kurar.

Bir güvenlik açığı bulursan lütfen herkese açık bir issue yerine GitHub'daki
**Security → Report a vulnerability** ile özel olarak bildir.

## Lisans

[MIT](LICENSE)

---

## Geliştirme

Tauri 2 (Rust) + React + TypeScript + Vite. Plan: [PLAN.md](PLAN.md),
[PLAN-ASAMA-7.md](PLAN-ASAMA-7.md), [PLAN-ASAMA-8.md](PLAN-ASAMA-8.md).

```sh
npm install
npm run tauri dev      # geliştirme
npm run lint && npm run format:check
cd src-tauri && cargo clippy && cargo test
```

### Yeni sürüm çıkarmak

```sh
npm run release 0.3.0
```

Sürüm numarasını her yerde değiştirir, commit'ler, `v0.3.0` etiketini gönderir.
GitHub Actions ([release.yml](.github/workflows/release.yml)) kurulum dosyasını
derleyip imzalar ve Releases'e koyar (~15 dk); kurulu dock'lar `latest.json`
üzerinden kendini günceller.

Güncelleme imzası için depo sırları gerekir (Settings → Secrets and variables →
Actions): `TAURI_SIGNING_PRIVATE_KEY` ve `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
Anahtar `%USERPROFILE%\.tauri\akilli-dock.key` dosyasında; **kaybolursa kurulu
dock'lara bir daha güncelleme gönderilemez**, yedeğini sakla.
