-- Kum 0.9.45: görüşmeler. Bir görüşme uygulamasının (Teams, Zoom, tarayıcıda Meet…) mikrofonu
-- kullandığı aralıklar; toplantıya katılım ve gerçek süre bunlardan çıkar ve öteki bilgisayarda
-- da aynı yargılanır. Ses kaydedilmez. 0013'ten sonra SQL Editor'da bir kez çalıştırın; tekrar
-- çalıştırmak zararsızdır. Çalıştırılmazsa eşitleme sürer, yalnızca görüşmeler taşınmaz.
create table if not exists public.calls (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    device_id         text not null,
    app_id            text not null,
    started_at        timestamptz not null,
    ended_at          timestamptz not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id),
    check (ended_at >= started_at)
);
create index if not exists calls_user_cursor on public.calls (user_id, server_updated_at);

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi); yetkiler
-- 0009'daki gibi dar.
drop trigger if exists kum_lww on public.calls;
create trigger kum_lww before insert or update on public.calls
    for each row execute function public.kum_lww();
alter table public.calls enable row level security;
drop policy if exists "kendi satırları" on public.calls;
create policy "kendi satırları" on public.calls for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
revoke all on public.calls from anon;
revoke truncate, references, trigger on public.calls from authenticated;
grant select, insert, update, delete on public.calls to authenticated;
notify pgrst, 'reload schema';
