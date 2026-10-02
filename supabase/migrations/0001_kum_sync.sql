-- Kum senkronizasyon şeması.
--
-- Her satır bir kullanıcıya aittir (user_id = auth.uid(), RLS ile zorunlu).
-- Anahtar (user_id, id): varsayılan kategoriler her kullanıcıda aynı kimliğe
-- sahiptir, kullanıcılar arasında çakışmamalıdır.
-- Cihazlar satırları UUID ile upsert eder; çakışmada daha yeni updated_at
-- kazanır (lww tetikleyicisi). server_updated_at sunucuda atanır ve cihazların
-- "son çekimden beri değişenler" sorgusunun imlecidir.

create table if not exists public.sessions (
    id                uuid not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    device_id         uuid not null,
    app_id            text not null,
    app_name          text not null,
    title             text not null,
    url               text,
    domain            text,
    started_at        timestamptz not null,
    ended_at          timestamptz not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    primary key (user_id, id),
    check (ended_at >= started_at)
);

create table if not exists public.tags (
    id                uuid not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    kind              text not null check (kind in ('category', 'project')),
    name              text not null,
    color             smallint not null check (color between 1 and 8),
    position          integer not null default 0,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    primary key (user_id, id)
);

create table if not exists public.rules (
    id                uuid not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    tag_id            uuid not null,
    field             text not null check (field in ('app', 'title')),
    pattern           text not null,
    position          integer not null default 0,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    primary key (user_id, id)
);

create index if not exists sessions_user_cursor on public.sessions (user_id, server_updated_at);
create index if not exists tags_user_cursor on public.tags (user_id, server_updated_at);
create index if not exists rules_user_cursor on public.rules (user_id, server_updated_at);

-- Son yazan kazanır: daha eski bir sürüm, yeni olanın üzerine yazamaz.
create or replace function public.kum_lww() returns trigger
language plpgsql as $$
begin
    if tg_op = 'UPDATE' then
        if new.updated_at < old.updated_at then
            return old;
        end if;
        -- Satır başka bir kullanıcıya taşınamaz.
        new.user_id := old.user_id;
    end if;
    new.server_updated_at := clock_timestamp();
    return new;
end;
$$;

do $$
declare t text;
begin
    foreach t in array array['sessions', 'tags', 'rules'] loop
        execute format('drop trigger if exists kum_lww on public.%I', t);
        execute format(
            'create trigger kum_lww before insert or update on public.%I
             for each row execute function public.kum_lww()', t);
        execute format('alter table public.%I enable row level security', t);
        execute format('drop policy if exists "kendi satırları" on public.%I', t);
        execute format(
            'create policy "kendi satırları" on public.%I for all to authenticated
             using (user_id = (select auth.uid()))
             with check (user_id = (select auth.uid()))', t);
    end loop;
end;
$$;
