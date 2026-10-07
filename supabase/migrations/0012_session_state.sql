-- Kum 0.9.31: oturumun ataması (kategori, proje) ve silinmesi içerikten ayrı zamanla eşitlenir
-- (state_at): başka bilgisayarda süren oturumu takip uzatırken, bu arada yapılan atamayı ya da
-- silmeyi geri almaz. 0011'den sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak
-- zararsızdır. Çalıştırılmazsa oturumlar eskisi gibi bütün olarak eşitlenir.
alter table public.sessions add column if not exists state_at timestamptz;
notify pgrst, 'reload schema';
