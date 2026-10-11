-- Kum 0.9.67: girdisiz izleme (passive). Ekran bir video için uyanıkken girdi olmadan geçen süre;
-- iki bilgisayar aynı anda kayıt tutunca girdiyle geçen süreye yenilir (öteki bilgisayarda oyun
-- oynanırken kendi kendine oynayan video süreyi almasın). 0014'ten sonra SQL Editor'da bir kez
-- çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa oturumlar işaretsiz eşitlenir.
alter table public.sessions add column if not exists passive boolean not null default false;
notify pgrst, 'reload schema';
