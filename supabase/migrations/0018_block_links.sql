-- Kum 0.9.88: takvim düzeltmelerinden öğrenilen tür çiftleri. Kullanıcı bir bloğu iki türün (kategori
-- ya da kategorisiz uygulama) arasından böldüyse 'split', bir türün bloğunu ötekinin üstüne uzattıysa
-- 'join' kaydı düşer (aralık düzenlenen bloğunkidir); bloklar bunlarla bölünür, öteki bilgisayarda da aynı.
-- 0017'den sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa
-- eşitleme sürer, yalnızca her bilgisayar kendi düzeltmelerinden öğrenir.
create table if not exists public.block_links (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    a                 text not null,
    b                 text not null,
    kind              text not null check (kind in ('join', 'split')),
    started_at        timestamptz not null,
    ended_at          timestamptz not null,
    created_at        timestamptz not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id),
    check (ended_at >= started_at)
);
create index if not exists block_links_user_cursor
    on public.block_links (user_id, server_updated_at);

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi); yetkiler
-- 0009'daki gibi dar.
drop trigger if exists kum_lww on public.block_links;
create trigger kum_lww before insert or update on public.block_links
    for each row execute function public.kum_lww();
alter table public.block_links enable row level security;
drop policy if exists "kendi satırları" on public.block_links;
create policy "kendi satırları" on public.block_links for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
revoke all on public.block_links from anon;
revoke truncate, references, trigger on public.block_links from authenticated;
grant select, insert, update, delete on public.block_links to authenticated;
notify pgrst, 'reload schema';
