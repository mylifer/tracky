-- Kum 0.9.75: anında eşitleme. Bir bilgisayar düzenlemesini gönderince kullanıcının tek satırını
-- günceller ("dürter"); öteki bilgisayarlar Supabase Realtime ile bunu duyup hemen eşitler.
-- 0016'dan sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa
-- eşitleme 5 dakikada bir sürer.
create table if not exists public.sync_nudges (
    user_id uuid primary key default auth.uid() references auth.users (id) on delete cascade,
    -- Dürten açılış (eşitlemedeki `writer`): kendi dürtmesini duyan bilgisayar yeniden eşitlemez.
    writer  uuid not null,
    at      timestamptz not null default now()
);
-- Dürtmenin zamanı sunucunun saatidir.
create or replace function public.sync_nudges_touch() returns trigger
    language plpgsql set search_path = '' as $$
begin
    new.at := now();
    return new;
end $$;
drop trigger if exists sync_nudges_touch on public.sync_nudges;
create trigger sync_nudges_touch before insert or update on public.sync_nudges
    for each row execute function public.sync_nudges_touch();
alter table public.sync_nudges enable row level security;
drop policy if exists "kendi satırı" on public.sync_nudges;
create policy "kendi satırı" on public.sync_nudges for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
revoke all on public.sync_nudges from anon;
revoke truncate, references, trigger on public.sync_nudges from authenticated;
grant select, insert, update on public.sync_nudges to authenticated;
-- Realtime yalnızca yayındaki tabloların değişikliklerini gönderir (satır güvenliğiyle).
do $$
begin
    alter publication supabase_realtime add table public.sync_nudges;
exception when duplicate_object then null;
end $$;
notify pgrst, 'reload schema';
