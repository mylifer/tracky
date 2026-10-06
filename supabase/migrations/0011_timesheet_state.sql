-- Kum 0.9.25: zaman çizelgesi satırının durumu (aktarıldı, gizlendi, silindi) içerikten ayrı
-- zamanla eşitlenir (state_at): eşitlenmemiş başka bilgisayardaki eski kopyanın düzenlenmesi
-- aktarımı ya da silmeyi geri almaz. consultant: aktarılan satırın dosyaya yazıldığı danışman
-- adı; ayarlarda ad değişse de satır dosyada bulunur. 0010'dan sonra SQL Editor'da bir kez
-- çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa satırlar eskisi gibi bütün
-- olarak eşitlenir.
alter table public.timesheet_entries add column if not exists state_at timestamptz;
alter table public.timesheet_entries add column if not exists consultant text;
notify pgrst, 'reload schema';
