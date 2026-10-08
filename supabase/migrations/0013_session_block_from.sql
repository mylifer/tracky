-- Kum 0.9.39: takvim bloğunun elle bölündüğü an (block_from). Blok kısaltılınca kesilen kısım
-- silinmez, bu anda başlayan oturumdan ayrı blok olur; atama gibi state_at ile eşitlenir.
-- 0012'den sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır.
-- Çalıştırılmazsa oturumlar bölmesiz eşitlenir.
alter table public.sessions add column if not exists block_from timestamptz;
notify pgrst, 'reload schema';
