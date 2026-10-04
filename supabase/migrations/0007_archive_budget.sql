-- Kum 0.8: proje arşivi (tags.archived_at) ve sözleşme bütçesi (adam-gün; tags.budget_days,
-- clients.budget_days). 0006'dan sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak
-- zararsızdır. Çalıştırılmazsa eşitleme sürer, yalnızca arşiv ve bütçeler diğer cihazlara
-- geçmez (Kum uyarır); çalıştırınca bekleyenler kendiliğinden gönderilir. Eski Kum sürümleri bu
-- sütunları göndermez, sunucudaki değerlere de dokunmaz.
alter table public.tags add column if not exists archived_at timestamptz;
alter table public.tags add column if not exists budget_days double precision;
alter table public.clients add column if not exists budget_days double precision;

alter table public.tags drop constraint if exists tags_budget_days_check;
alter table public.tags add constraint tags_budget_days_check
    check (budget_days is null or budget_days > 0);
alter table public.clients drop constraint if exists clients_budget_days_check;
alter table public.clients add constraint clients_budget_days_check
    check (budget_days is null or budget_days > 0);
