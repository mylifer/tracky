# Kum

Rize / Timely benzeri, macOS ve Windows'ta pencerelerde geçirilen süreyi takip eden masaüstü uygulaması.

**Teknoloji:** Tauri + Rust (çekirdek), React + TypeScript (arayüz), SQLite (yerel), Supabase (senkronizasyon).

## Yol haritası

- [x] 1. Çekirdek: veri modeli, sync'e hazır SQLite şeması, oturum motoru (idle/uyku tespiti), domain çıkarma
- [x] 2. Platform katmanı: macOS ve Windows'ta aktif pencere, sekme adı, idle süresi, gizlilik ayarları (macOS gerçek makinede doğrulandı)
- [x] 3. Tauri uygulaması: arka planda takip, menü çubuğu/tray, karşılama ve izin ekranı, otomatik başlatma
- [x] 4. Arayüz: gün (zaman çizelgesi) ve hafta raporları, uygulama/başlık dökümü, ayarlar
- [x] 5. Kategoriler / projeler: uygulama ve başlık kuralları, hazır kategoriler, geçmişe dönük sınıflandırma
- [ ] 6. Supabase senkronizasyonu
- [ ] 7. Paketleme, imzalama, CI

## Yapı

```
crates/tracky-core/       Platformdan bağımsız çekirdek
  engine.rs               Gözlemleri oturumlara çevirir (idle, uyku, kısa geçişler)
  store.rs                SQLite: göçler, oturum kaydı, ayarlar, raporlama sorguları
  privacy.rs              Gizlilik: duraklatma, hariç uygulamalar, başlık gizleme
  browser.rs              Tarayıcı tanıma, sekme adı temizleme, gizli pencere tespiti
  platform.rs             Her OS'un uygulayacağı ActivityProvider trait'i
  classify.rs             Kategoriler, projeler, kurallar ve varsayılan kategoriler
  report.rs               Gün/hafta raporu (kategori, proje, uygulama, gün, zaman çizelgesi)
  tracker.rs              Gözlem → gizlilik → motor → depolama hattı
  url_util.rs             URL'den domain çıkarma (ileride eklenti için)
crates/tracky-platform/   macOS (Erişilebilirlik API) ve Windows (Win32) gözlemcileri
crates/tracky-probe/      Takibi terminalden denemek için araç
app/                      Kum masaüstü uygulaması (Tauri + React)
  src/                    Arayüz: karşılama, gün/hafta raporları, kategoriler, ayarlar
  src-tauri/              Takip iş parçacığı, menü çubuğu, komutlar
```

## Uygulamayı geliştirmek

```sh
cd app
npm install
npm run tauri dev      # geliştirme modunda aç
npm run tauri build    # .app/.dmg (macOS) ya da kurulum dosyası (Windows) üret
```

Hazır paketler: GitHub **Actions** sekmesindeki son başarılı çalıştırmada
`kum-macos-arm64` (.dmg) ve `kum-windows-x64` (kurulum .exe).

- **macOS:** Paket imzasız (ad-hoc). İlk açılışta "geliştirici doğrulanamadı" uyarısında
  uygulamaya sağ tıklayıp **Aç** deyin ya da `xattr -cr /Applications/Kum.app` çalıştırın.
  Her yeni sürümde Erişilebilirlik iznini yeniden vermek gerekebilir.
- Veriler: macOS'ta `~/Library/Application Support/com.kum.app/kum.db`,
  Windows'ta `%APPDATA%\com.kum.app\kum.db`.

## Takibi denemek (tracky-probe)

Rust kuruluysa:

```sh
cargo run --release -p tracky-probe            # takibi başlat, Ctrl+C ile durdur ve özet gör
cargo run --release -p tracky-probe -- report  # bugünün özeti
cargo run --release -p tracky-probe -- exclude com.1password.1password   # hiç kaydetme
cargo run --release -p tracky-probe -- hide-title com.apple.mail         # başlığı kaydetme
cargo run --release -p tracky-probe -- privacy                           # ayarları göster
```

Rust kurmadan: GitHub'da **Actions** sekmesindeki son başarılı çalıştırmanın altından
`tracky-probe-macos-arm64` ya da `tracky-probe-windows-x64` dosyasını indirin.

- Uygulama kimliği (`exclude` için) takip sırasında köşeli parantez içinde yazdırılır:
  macOS'ta bundle id, Windows'ta exe yolu.
- **macOS:** İlk çalıştırmada Erişilebilirlik izni istenir. İzin, aracı çalıştıran
  uygulamaya (Terminal, iTerm, VS Code...) verilir. İndirilen dosya imzasız olduğu için
  önce `xattr -d com.apple.quarantine tracky-probe && chmod +x tracky-probe` çalıştırın.
- **Windows:** İzin gerekmez. SmartScreen uyarısında "Ek bilgi > Yine de çalıştır".

## Geliştirme

```sh
cargo test
```
