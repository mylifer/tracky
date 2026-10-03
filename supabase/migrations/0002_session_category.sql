-- Kum 0.2: oturuma elle verilen kategori (takvimde bloğu atama, manuel kayıt).
-- 0001'den sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır.
alter table public.sessions add column if not exists category_id uuid;
