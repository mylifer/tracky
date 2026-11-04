-- Kum 0.9.101: gün planı blokları (kullanıcının onayladığı zaman blokları). Bir bilgisayarda
-- onaylanan, adı verilen ya da boyutu değiştirilen blok öteki bilgisayarda da görünür.
-- 0017'den sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa
-- eşitleme sürer, yalnızca plan blokları taşınmaz (her bilgisayar kendi planını görür).
create table if not exists public.plan_blocks (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    started_at        timestamptz not null,
    ended_at          timestamptz not null,
    project_id        text,
    note              text not null default '',
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id),
    check (ended_at > started_at)
);
create index if not exists plan_blocks_user_cursor
    on public.plan_blocks (user_id, server_updated_at);

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi); yetkiler
-- 0009'daki gibi dar.
drop trigger if exists kum_lww on public.plan_blocks;
create trigger kum_lww before insert or update on public.plan_blocks
    for each row execute function public.kum_lww();
alter table public.plan_blocks enable row level security;
drop policy if exists "kendi satırları" on public.plan_blocks;
create policy "kendi satırları" on public.plan_blocks for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
revoke all on public.plan_blocks from anon;
revoke truncate, references, trigger on public.plan_blocks from authenticated;
grant select, insert, update, delete on public.plan_blocks to authenticated;
notify pgrst, 'reload schema';
