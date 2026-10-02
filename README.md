# Tracky

Rize / Timely benzeri, macOS ve Windows'ta pencerelerde geçirilen süreyi takip eden masaüstü uygulaması.

**Teknoloji:** Tauri + Rust (çekirdek), React + TypeScript (arayüz), SQLite (yerel), Supabase (senkronizasyon).

## Yol haritası

- [x] 1. Çekirdek: veri modeli, sync'e hazır SQLite şeması, oturum motoru (idle/uyku tespiti), domain çıkarma
- [ ] 2. Platform katmanı: macOS ve Windows'ta aktif pencere, tarayıcı URL'si, idle süresi
- [ ] 3. Tauri kabuğu: arka plan servisi, tray/menü çubuğu, otomatik başlatma
- [ ] 4. Arayüz: günlük/haftalık rapor, zaman çizelgesi (Türkçe)
- [ ] 5. Kategoriler / projeler ve kural motoru
- [ ] 6. Supabase senkronizasyonu
- [ ] 7. Paketleme, imzalama, CI

## Yapı

```
crates/tracky-core/   Platformdan bağımsız çekirdek
  engine.rs           Gözlemleri oturumlara çevirir (idle, uyku, kısa geçişler)
  store.rs            SQLite: göçler, oturum kaydı, raporlama sorguları
  platform.rs         Her OS'un uygulayacağı ActivityProvider trait'i
  url_util.rs         URL'den domain çıkarma
```

## Geliştirme

```sh
cargo test
```
