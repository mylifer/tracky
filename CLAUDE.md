# Kum (tracky) — çalışma notları

## Supabase şema değişiklikleri

Eşitlenen bir tablo ya da sütun eklenince Supabase tarafı da güncellenmeli; bunu kullanıcıya bırakma:

1. `supabase/migrations/00NN_<ad>.sql` ekle (bir sonraki numara, ad `^[0-9]{4}_[a-z0-9_]+\.sql$`).
   Dosya tekrar çalıştırılabilir olmalı (`create … if not exists`, `drop … if exists`), `public` şemasında.
2. Aynı değişikliği `kum` şemasına uyarlayıp `supabase/kum_schema.sql`'in sonuna (erişim bölümünden önce)
   `-- ===== 00NN_<ad>.sql =====` başlığıyla ekle; başlıktaki "0001–00NN" aralığını güncelle.
3. Yerel şema (`crates/tracky-core/src/store/schema.rs`) ve eşitleme tablosu (`crates/tracky-core/src/sync.rs`:
   `TABLES`, yeni tabloysa `LATE_TABLES` ve `AFTER_LEGACY_TABLES`) ile uyumlu tut; README'deki göç listesine
   bir satır ekle.

Göçleri `.github/workflows/supabase.yml` uygular: değişiklik varsayılan branch'e girince, sürüm etiketinde ve
elle. `SUPABASE_DB_URL` deposu sırrı tanımlı değilse iş atlanır; uygulandıysa iş özetinde kaç göç
uygulandığı yazar. Göçü yerelde denemek için sandbox'taki PostgreSQL'de Supabase'in `auth` şemasını ve
`anon`/`authenticated`/`service_role` rollerini taklit eden bir veritabanı kurup betiği çalıştırabilirsin.
