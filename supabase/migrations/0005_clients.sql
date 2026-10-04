-- Kum 0.6: müşteriler; projeler bir müşteriye bağlanır (tags.client_id). 0004'ten sonra
-- SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Kum 0.6'dan itibaren
-- gereklidir: çalıştırılmazsa eşitleme müşteriler tablosunu bulamadığını bildirir.
create table if not exists public.clients (
    id                uuid not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    name              text not null,
    position          integer not null default 0,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id)
);
create index if not exists clients_user_cursor on public.clients (user_id, server_updated_at);

alter table public.tags add column if not exists client_id uuid;

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi).
drop trigger if exists kum_lww on public.clients;
create trigger kum_lww before insert or update on public.clients
    for each row execute function public.kum_lww();
alter table public.clients enable row level security;
drop policy if exists "kendi satırları" on public.clients;
create policy "kendi satırları" on public.clients for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
