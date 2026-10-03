# Kum

Rize / Timely benzeri, macOS ve Windows'ta pencerelerde geçirilen süreyi takip eden masaüstü uygulaması.

**Teknoloji:** Tauri + Rust (çekirdek), React + TypeScript (arayüz), SQLite (yerel), Supabase (senkronizasyon).

## Yol haritası

- [x] 1. Çekirdek: veri modeli, sync'e hazır SQLite şeması, oturum motoru (idle/uyku tespiti), domain çıkarma
- [x] 2. Platform katmanı: macOS ve Windows'ta aktif pencere, sekme adı, idle süresi, gizlilik ayarları (macOS gerçek makinede doğrulandı)
- [x] 3. Tauri uygulaması: arka planda takip, menü çubuğu/tray, karşılama ve izin ekranı, otomatik başlatma
- [x] 4. Arayüz: gün (zaman çizelgesi) ve hafta raporları, uygulama/başlık dökümü, ayarlar
- [x] 5. Kategoriler / projeler: uygulama ve başlık kuralları, hazır kategoriler, geçmişe dönük sınıflandırma
- [x] 6. Supabase senkronizasyonu: e-posta/şifre ile giriş, 5 dakikada bir eşitleme, son yazan kazanır
- [x] 7. Paketleme ve CI: her push'ta Mac/Windows paketleri, `v*` etiketiyle sürüm (Apple imzası henüz yok)
- [x] 8. Otomatik güncelleme: 6 saatte bir denetim, arka planda indirme, imza doğrulama, tek tıkla kurulum
- [x] 9. Oturum düzenleme: takvimdeki bloğu kategoriye atama ya da silme, elle kayıt ekleme

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
crates/tracky-sync/       Supabase istemcisi (Auth + PostgREST)
supabase/migrations/      Sunucu şeması (tablolar, RLS, çakışma kuralı)
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

## Cihazlar arası senkronizasyon (Supabase)

Veriler varsayılan olarak yalnızca bilgisayarda kalır. Mac ve Windows'ta birleşik rapor için:

1. [supabase.com](https://supabase.com) üzerinde ücretsiz bir proje oluştur.
2. **SQL Editor**'da `supabase/migrations/` altındaki dosyaları sırayla (`0001_…`, `0002_…`) çalıştır.
   Önceki bir sürümden geliyorsan yalnızca yeni dosyaları çalıştırman yeterli.
3. **Project Settings → API** sayfasından **Project URL** ve **anon / publishable** anahtarını kopyala.
4. Kum'da **Ayarlar → Senkronizasyon** bölümüne bu ikisini gir, sonra e-posta ve şifreyle
   **Hesap oluştur** (ya da **Giriş yap**). E-posta doğrulaması açıksa önce gelen bağlantıya tıkla.
5. Diğer bilgisayarda aynı proje bilgileri ve aynı hesapla giriş yap.

Nasıl çalışır: her satırın kimliği UUID'dir; değişen satırlar gönderilir, sunucuda son çekimden
beri değişenler alınır. Aynı satır iki cihazda değiştiyse daha yeni olan kazanır. Silmeler
yumuşaktır, o yüzden silinenler de eşitlenir. Satır güvenliği (RLS) sayesinde her kullanıcı yalnız
kendi verisini görür. Bağlantı bilgileri ve oturum yalnızca o cihazda saklanır.

## Sürüm çıkarmak

**Bir kerelik kurulum:** güncelleme imzalama anahtarını GitHub'da
*Settings → Secrets and variables → Actions → New repository secret* ile
`TAURI_SIGNING_PRIVATE_KEY` adıyla ekle (anahtar dosyasının içeriği). Anahtarın şifresi
yoksa `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` gerekmez. Anahtarı yedekle: kaybolursa
kurulu uygulamalar yeni sürümleri kabul etmez (açık anahtar `tauri.conf.json` içinde).

1. `app/src-tauri/tauri.conf.json`, `app/package.json` ve `app/src-tauri/Cargo.toml` içindeki sürümü artır.
2. Commit'le, sonra etiketle ve push'la: `git tag v0.3.0 && git push origin v0.3.0`
3. İş akışı Mac/Windows paketlerini imzalar ve `latest.json` ile birlikte yayınlanmış bir
   Release'e yükler.

### Otomatik güncelleme

Kum açılıştan 30 sn sonra ve sonra 6 saatte bir
`releases/latest/download/latest.json` dosyasını denetler. Yeni sürüm varsa arka planda
indirir ve imzasını doğrular; kurulum kullanıcı onayıyla yapılır (menü çubuğunda
**Güncellemeyi Yükle**, kenar çubuğundaki bildirim ya da *Ayarlar → Güncellemeler*).
Kurulumdan sonra uygulama yeniden başlar.

macOS paketi ad-hoc imzalıdır. Apple Developer hesabıyla (yıllık ücretli) imzalanıp
notarize edilene kadar ilk açılışta "geliştirici doğrulanamadı" uyarısı çıkar ve her yeni sürümde
Erişilebilirlik izni yeniden verilmelidir.
