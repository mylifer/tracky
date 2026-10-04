-- Kum 0.7: kurallar tarayıcı adresine de bakabilir (rules.field = 'domain'). 0005'ten sonra
-- SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa web
-- sitesi kuralı eklenen cihazda eşitleme "rules_field_check" hatası verir.
alter table public.rules drop constraint if exists rules_field_check;
alter table public.rules add constraint rules_field_check
    check (field in ('app', 'title', 'domain'));
