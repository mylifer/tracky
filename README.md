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
- [x] 7. Paketleme ve CI: her push'ta Mac/Windows paketleri, `v*` etiketiyle sürüm, macOS'ta sabit (kendinden imzalı) sertifika
- [x] 8. Otomatik güncelleme: 6 saatte bir denetim, arka planda indirme, imza doğrulama, tek tıkla kurulum
- [x] 9. Oturum düzenleme: takvimdeki bloğu kategoriye atama ya da silme, elle kayıt ekleme; takvimde sürükleyerek
      seçilen aralığı kategoriye atama, silme ya da elle kayda çevirme
- [x] 10. Kategori limitleri: günlük sınır, %80'de ve dolunca bildirim, özette limit çubukları
- [ ] 11. ~~Odak modu~~: kaldırıldı (zamanlayıcı, odak koruması, odak skoru ve süresi)
- [x] 12. Kullanım kolaylıkları: takvimde boş alana tıklayarak kayıt, klavye kısayolları (←/→, T, 1/2/3, +/−/0, /),
      süreli duraklatma, görünüm seçimi (sistem/açık/koyu), hafta/ay öne çıkanları
- [x] 13. Uygulama çizelgesi: gün ve hafta görünümünde uygulama ve pencere başlığı bazında Gantt çizelgesi
- [x] 14. Gün sonu özeti: seçilen saatte (varsayılan 18:00) süre, hedef ve en çok kategori bildirimi; yeni haftanın
      ilk çalışmasında geçen haftanın özeti (önceki haftaya göre değişim, en yoğun gün)
- [x] 15. Otomatik öneriler: pencere başlıklarından projeler (VS Code, JetBrains, Xcode, terminal, GitHub),
      tanınan uygulama/sitelerden kategoriler; onayla ya da yoksay, tamamen yerel
- [x] 16. Yakınlaştırma: gün/hafta takviminde ve uygulama çizelgesinde ⌘ + kaydırma, kıstırma, +/− ve 0
- [x] 17. Arama: başlıkta ya da uygulama adında geçen ifadeye göre süre, günlük dağılım, uygulamalar, pencereler (`/`)
- [x] 18. Eğilimler: proje ve kategorilerin son 8/12/26 haftadaki haftalık süresi, bu hafta ve haftalık ortalama
- [x] 19. Proje hedefleri: proje başına haftalık saat; hafta özeti ve eğilimlerde ilerleme, dolunca bildirim
- [x] 20. Zaman çizelgesi: projeye atanan süreden günlük iş kayıtları (başlangıç, saat, Working/Online/F2F,
      açıklama, taraf, birim), gözden geçirip onaylama ve firmanın Excel dosyasına biçimini koruyarak ekleme
- [x] 21. Outlook takvimi: yayımlanan ICS bağlantısından toplantılar (tekrarlar, saat dilimleri, iptaller); konusu
      proje kuralına uyan toplantı zaman çizelgesine Online/F2F kayıt olarak girer ve o sürede takip edilen işin
      yerini alır; diğerleri gün kartında seri olarak projeye atanır ya da yoksayılır
- [x] 22. Google Sheets: kayıtlar tabloya eklenen Apps Script web uygulamasıyla (OAuth gerekmeden) Excel'deki
      kurallarla yazılır; aynı kayıt iki kez yazılmaz
- [x] 23. Müşteriler: projeler bir müşteriye bağlanır; Müşteriler sayfası, müşteriye göre gruplanan projeler,
      raporda müşteri dökümü; zaman çizelgesi şablonundaki firma müşteri olarak eklenir
- [x] 24. Boşta geçen süre: bilgisayardan uzakta (3 dk girdi yok ya da uyku) geçen süre takvimde taralı "Boşta"
      bloğu olur; çalışma süresine sayılmaz, tıklayıp projeye/kategoriye atanınca ya da elle kayda çevrilince
      sayılır (zaman çizelgesine F2F girer). En uzun boşluk ayarlanabilir (varsayılan 3 sa; gece kaydedilmez)
- [x] 25. Web sitesi kuralları: tarayıcının adresi okunur (macOS'ta Erişilebilirlik, Windows'ta UI Automation),
      sorgu ve parça atılarak saklanır; `jira.togg.com` ya da `github.com/firma` gibi kurallar projeye/kategoriye
      bağlar. Gizli pencerelerde ve başlığı gizlenen uygulamalarda adres kaydedilmez
- [x] 26. Yedekler: haftada bir otomatik (son 8 saklanır), elle yedek ve yedekten geri yükleme (şimdiki veri
      silinmez, yedek klasörüne taşınır)
- [x] 27. Zaman çizelgesi hatırlatması: cuma (varsayılan 17:00) bu hafta aktarılmamış günleri bildirir
- [x] 28. Gözden geçir: projeye düşmeyen süre siteye ve uygulamaya göre gruplanır; grup ya da başlık tek tıkla
      projeye atanır, istenirse kural eklenir. Öncesinde ve sonrasında çalışılan proje önerilir; başlıktaki iş
      anahtarından (`LOY-214`) kural öneki çıkar. Boşta süre de buradan atanır, gruplar gizlenebilir
- [x] 29. Kural önizlemesi: kural eklenmeden son 30 günde ne kadar sürenin geçeceği, ne kadarının başka projeden
      alınacağı ve etkilenen pencereler gösterilir
- [x] 30. Geri al: takvimde atama, silme, elle kayıt; kural ve proje/kategori silme; zaman çizelgesi satırı.
      Silmede onay sorulmaz, alttaki bildirimden geri alınır
- [x] 31. Komut paleti (⌘K / Ctrl+K), ⌘1–5 sayfalar, ⌘F arama, ⌘, ayarlar; macOS'ta Git menüsü. Bildirime
      tıklayınca ilgili sayfa (gün, hafta, zaman çizelgesi) açılır; menü çubuğundan "Gözden geçir"
- [x] 32. Zaman çizelgesi açıklamaları: kaydın iş anahtarları ve süreye göre en önemli başlıkları (yerel, ücretsiz)
- [x] 33. Görünüm: kum tonlarında marka rengi, hedef halkaları, canlı kart, iskelet yükleme, sayfa geçişleri,
      okunur hata mesajları, karşılamada proje adımı (elle ya da zaman çizelgesi şablonundan)
- [x] 34. Proje seçici: projeler müşteriye göre gruplanır, son seçilen 5 proje en üstte; 12 ve daha fazla projede
      yanındaki alanla süzülür (Enter ilk eşleşeni seçer). Liste Windows'ta da sorunsuz olsun diye yerel kalır
- [x] 35. Takvimde renk merceği: gün/hafta blokları kategori ya da proje renginde; proje görünümünde atanmamış
      süre taralı "Atanmamış" bloktur, lejantta projeler toplamlarıyla
- [x] 36. Menü çubuğu ve canlı kart: şu anki iş bir projeye düşüyorsa proje adı ve bugünkü süresi
      ("Portal · 5sa 20dk"); menü çubuğundan "Zaman Çizelgesini Aç"
- [x] 37. Terimler ve erişilebilirlik: henüz atanmamış süre "Atanmamış", elle proje yok denen süre "Projesiz";
      kenar çubuğu rozetlerinin anlamı ekran okuyucuda, düğmelerde klavye odak halkası

## Yapı

```
crates/tracky-core/       Platformdan bağımsız çekirdek
  engine.rs               Gözlemleri oturumlara çevirir (idle, uyku, kısa geçişler)
  store/                  SQLite: göçler, oturum kaydı, ayarlar, raporlama sorguları; taxonomy.rs
                          (etiketler, kurallar, müşteriler, öneriler), timesheet.rs (zaman çizelgesi),
                          edits.rs (geri alma, atanmamış süre)
  inbox.rs                Atanmamış süreyi gruplama, muhtemel proje, kural önizlemesi
  blocks.rs               Takvimdeki çalışma blokları, molalar, bağlam değişimi
  privacy.rs              Gizlilik: duraklatma, hariç uygulamalar, başlık gizleme, adres temizleme
  browser.rs              Tarayıcı tanıma, sekme adı temizleme, gizli pencere tespiti
  platform.rs             Her OS'un uygulayacağı ActivityProvider trait'i
  classify.rs             Kategoriler, projeler, kurallar ve varsayılan kategoriler
  calendar.rs             iCalendar (.ics) ayrıştırma: tekrar kuralları, VTIMEZONE, istisnalar
  timesheet.rs            Oturum ve toplantılardan günlük iş kaydı önerileri
  report.rs               Gün/hafta raporu (kategori, proje, uygulama, gün, zaman çizelgesi)
  tracker.rs              Gözlem → gizlilik → motor → depolama hattı
  url_util.rs             URL'den domain çıkarma, adres temizleme, web sitesi kuralı eşleştirme
crates/tracky-platform/   macOS (Erişilebilirlik API) ve Windows (Win32, UI Automation) gözlemcileri
crates/tracky-probe/      Takibi terminalden denemek için araç
crates/tracky-sync/       Supabase istemcisi (Auth + PostgREST)
crates/tracky-xlsx/       Zaman çizelgesini Excel dosyasına ya da (Apps Script ile) Google Sheets'e ekleme
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

- **macOS:** Paket Kum'un kendinden imzalı sertifikasıyla imzalanır (Apple onaylı değil).
  İlk açılışta "geliştirici doğrulanamadı" uyarısında uygulamaya sağ tıklayıp **Aç** deyin,
  *Sistem Ayarları → Gizlilik ve Güvenlik → Yine de Aç* seçin ya da
  `xattr -cr /Applications/Kum.app` çalıştırın. İmza kimliği sabit olduğu için Erişilebilirlik
  izni güncellemelerde korunur. Ad-hoc imzalı eski bir sürümden (0.2.0 ve öncesi) geçerken
  izni bir kez daha vermek gerekir.
- Veriler: macOS'ta `~/Library/Application Support/com.kum.app/kum.db`,
  Windows'ta `%APPDATA%\com.kum.app\kum.db`. Yedekler aynı klasördeki `backups/` altında
  (*Ayarlar → Veriler*).

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

CI'daki denetimlerin aynısı:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p tracky-core --no-default-features   # çekirdek depo olmadan da derlenmeli
cargo test --workspace

cd app
npm run build          # tsc + vite
npm run format:check   # Prettier (satır genişliği 120, .prettierrc)
npm test               # Vitest; saat dilimi Europe/Berlin'e sabit (yaz saati testleri)
```

Windows platform kodu macOS'ta da denetlenebilir:
`rustup target add x86_64-pc-windows-msvc && cargo clippy -p tracky-platform --target x86_64-pc-windows-msvc`.

## Zaman çizelgesi: Outlook takvimi ve Google Sheets

**Outlook takvimi:** Outlook web'de *Ayarlar → Takvim → Paylaşılan takvimler → Takvim yayımla*
ile takvimi "Tüm ayrıntılar" (en az "Başlıklar ve konumlar") düzeyinde yayımla, **ICS**
bağlantısını Kum'da *Zaman çizelgesi → Ayarlar → Outlook takvimi* alanına yapıştır. Kum takvimi
15 dakikada bir okur. Konusu bir projenin başlık kuralına uyan toplantı o projeye yazılır; diğerleri
gün kartında listelenir, seçilen proje serinin tüm tekrarlarına uygulanır. Bağlantıyı bilen herkes
takvimi görebilir; şirket yayımlamayı kapattıysa bu seçenek Outlook'ta görünmez.

**Google Sheets:** *Zaman çizelgesi → Google Sheets'e bağla* betiği gösterir. Tabloda
*Uzantılar → Apps Script*'e yapıştır, *Dağıt → Yeni dağıtım → Web uygulaması* ("Ben" olarak yürüt,
erişim "Herkes") ile dağıt ve `…/exec` adresini Kum'a gir. Betik yalnızca Kum'un anahtarını taşıyan
istekleri kabul eder, kayıtları ilk sayfaya Excel aktarımıyla aynı kurallarla ekler ve aynı kaydı
iki kez yazmaz. Betik `crates/tracky-xlsx/src/apps_script.gs` dosyasındadır.

## Cihazlar arası senkronizasyon (Supabase)

Veriler varsayılan olarak yalnızca bilgisayarda kalır. Mac ve Windows'ta birleşik rapor için:

1. [supabase.com](https://supabase.com) üzerinde ücretsiz bir proje oluştur.
2. **SQL Editor**'da `supabase/migrations/` altındaki dosyaları sırayla (`0001_…` … `0006_…`) çalıştır.
   Önceki bir sürümden geliyorsan yalnızca yeni dosyaları çalıştırman yeterli. `0003_writer.sql`
   cihazların kendi gönderdiklerini geri indirmesini önler; çalıştırılmazsa eşitleme eskisi gibi sürer.
   `0004_session_project.sql` elle verilen projeleri eşitler; Kum 0.4'ten itibaren gereklidir.
   `0005_clients.sql` müşterileri ve projelerin müşterisini eşitler; Kum 0.6'dan itibaren gereklidir.
   `0006_domain_rules.sql` web sitesi kurallarını eşitler; çalıştırılmazsa web sitesi kuralı eklenen
   cihazda eşitleme `rules_field_check` hatası verir.
3. **Project Settings → API** sayfasından **Project URL** ve **anon / publishable** anahtarını kopyala.
4. Kum'da **Ayarlar → Senkronizasyon** bölümüne bu ikisini gir, sonra e-posta ve şifreyle
   **Hesap oluştur** (ya da **Giriş yap**). E-posta doğrulaması açıksa önce gelen bağlantıya tıkla.
5. Diğer bilgisayarda aynı proje bilgileri ve aynı hesapla giriş yap.

**Ücretsiz plandaki proje sınırı dolduysa** Kum, başka bir uygulamanın projesinde ayrı bir şemada
çalışabilir: 2. adımda göçler yerine `supabase/kum_schema.sql` dosyasını çalıştır, *Project Settings →
Data API → Exposed schemas* listesine `kum` ekle ve Kum'da **Şema** alanına `kum` yaz. Diğer
uygulamanın tablolarına dokunulmaz; ancak kullanıcı listesi ve kota o projeyle ortaktır.

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

**macOS imza sertifikası (bir kerelik):** `bash scripts/macos-sign-cert.sh` kendinden imzalı
bir kod imzalama sertifikası üretir (`~/.kum-signing`). İki secret ekle:
`MACOS_CERTIFICATE` = `kum-codesign.p12.base64` dosyasının içeriği,
`MACOS_CERTIFICATE_PASSWORD` = `kum-codesign.password` dosyasının içeriği. CI ve sürüm iş
akışları sertifikayı geçici bir anahtar zincirine aktarıp paketi onunla imzalar; secret yoksa
ad-hoc imzaya döner. Klasörü yedekle: sertifika değişirse kullanıcılar izni bir kez daha verir.

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

macOS paketi kendinden imzalı sabit bir sertifikayla imzalanır; Erişilebilirlik izni güncellemelerde
korunur. Apple Developer hesabıyla (yıllık ücretli) imzalanıp notarize edilene kadar ilk açılışta
"geliştirici doğrulanamadı" uyarısı çıkar.
