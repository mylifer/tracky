# Tracky

Rize / Timely benzeri, macOS ve Windows'ta pencerelerde geçirilen süreyi takip eden masaüstü uygulaması.

**Teknoloji:** Tauri + Rust (çekirdek), React + TypeScript (arayüz), SQLite (yerel), Supabase (senkronizasyon).

## Yol haritası

- [x] 1. Çekirdek: veri modeli, sync'e hazır SQLite şeması, oturum motoru (idle/uyku tespiti), domain çıkarma
- [x] 2. Platform katmanı: macOS ve Windows'ta aktif pencere, sekme adı, idle süresi, gizlilik ayarları (macOS gerçek makinede doğrulandı)
- [ ] 3. Tauri kabuğu: arka plan servisi, tray/menü çubuğu, otomatik başlatma
- [ ] 4. Arayüz: günlük/haftalık rapor, zaman çizelgesi (Türkçe)
- [ ] 5. Kategoriler / projeler ve kural motoru
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
  url_util.rs             URL'den domain çıkarma (ileride eklenti için)
crates/tracky-platform/   macOS (Erişilebilirlik API) ve Windows (Win32) gözlemcileri
crates/tracky-probe/      Takibi terminalden denemek için araç
```

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
