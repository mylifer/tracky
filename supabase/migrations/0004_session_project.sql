-- Kum 0.4: oturuma elle verilen proje (takvimde bloğu ya da aralığı projeye atama,
-- elle kayıt). 0003'ten sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır.
alter table public.sessions add column if not exists project_id uuid;
