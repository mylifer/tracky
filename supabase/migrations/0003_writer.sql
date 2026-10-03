-- Kum 0.3.3: satırı sunucuda son yazan cihaz. Cihazlar kendi gönderdikleri
-- satırları geri çekmez. 0002'den sonra SQL Editor'da bir kez çalıştırın;
-- tekrar çalıştırmak zararsızdır. Çalıştırılmazsa eşitleme eskisi gibi sürer.
alter table public.sessions add column if not exists writer uuid;
alter table public.tags add column if not exists writer uuid;
alter table public.rules add column if not exists writer uuid;
