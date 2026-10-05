-- Kum'u başka bir uygulamanın Supabase projesinde, ayrı "kum" şemasında kurar (ücretsiz plandaki
-- proje sınırına takılmamak için). 0001–0007 göçlerinin "kum" şemasına uyarlanmış hâlidir;
-- SQL Editor'da bir kez çalıştırın, tekrar çalıştırmak zararsızdır. Ardından Project Settings →
-- Data API → Exposed schemas listesine "kum" ekleyin ve Kum'da Ayarlar → Senkronizasyon →
-- Şema alanına "kum" yazın. Diğer uygulamanın tablolarına ve ayarlarına dokunmaz.
create schema if not exists kum;
grant usage on schema kum to authenticated, service_role;

-- ===== 0001_kum_sync.sql =====
-- Kum senkronizasyon şeması.
--
-- Her satır bir kullanıcıya aittir (user_id = auth.uid(), RLS ile zorunlu).
-- Anahtar (user_id, id): varsayılan kategoriler her kullanıcıda aynı kimliğe
-- sahiptir, kullanıcılar arasında çakışmamalıdır.
-- Cihazlar satırları UUID ile upsert eder; çakışmada daha yeni updated_at
-- kazanır (lww tetikleyicisi). server_updated_at sunucuda atanır ve cihazların
-- "son çekimden beri değişenler" sorgusunun imlecidir.

create table if not exists kum.sessions (
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

create table if not exists kum.tags (
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

create table if not exists kum.rules (
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

create index if not exists sessions_user_cursor on kum.sessions (user_id, server_updated_at);
create index if not exists tags_user_cursor on kum.tags (user_id, server_updated_at);
create index if not exists rules_user_cursor on kum.rules (user_id, server_updated_at);

-- Son yazan kazanır: daha eski bir sürüm, yeni olanın üzerine yazamaz.
create or replace function kum.kum_lww() returns trigger
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
        execute format('drop trigger if exists kum_lww on kum.%I', t);
        execute format(
            'create trigger kum_lww before insert or update on kum.%I
             for each row execute function kum.kum_lww()', t);
        execute format('alter table kum.%I enable row level security', t);
        execute format('drop policy if exists "kendi satırları" on kum.%I', t);
        execute format(
            'create policy "kendi satırları" on kum.%I for all to authenticated
             using (user_id = (select auth.uid()))
             with check (user_id = (select auth.uid()))', t);
    end loop;
end;
$$;

-- ===== 0002_session_category.sql =====
-- Kum 0.2: oturuma elle verilen kategori (takvimde bloğu atama, manuel kayıt).
-- 0001'den sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır.
alter table kum.sessions add column if not exists category_id uuid;

-- ===== 0003_writer.sql =====
-- Kum 0.3.3: satırı sunucuda son yazan cihaz. Cihazlar kendi gönderdikleri
-- satırları geri çekmez. 0002'den sonra SQL Editor'da bir kez çalıştırın;
-- tekrar çalıştırmak zararsızdır. Çalıştırılmazsa eşitleme eskisi gibi sürer.
alter table kum.sessions add column if not exists writer uuid;
alter table kum.tags add column if not exists writer uuid;
alter table kum.rules add column if not exists writer uuid;

-- ===== 0004_session_project.sql =====
-- Kum 0.4: oturuma elle verilen proje (takvimde bloğu ya da aralığı projeye atama,
-- elle kayıt). 0003'ten sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır.
alter table kum.sessions add column if not exists project_id uuid;

-- ===== 0005_clients.sql =====
-- Kum 0.6: müşteriler; projeler bir müşteriye bağlanır (tags.client_id). 0004'ten sonra
-- SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Kum 0.6'dan itibaren
-- gereklidir: çalıştırılmazsa eşitleme müşteriler tablosunu bulamadığını bildirir.
create table if not exists kum.clients (
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
create index if not exists clients_user_cursor on kum.clients (user_id, server_updated_at);

alter table kum.tags add column if not exists client_id uuid;

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi).
drop trigger if exists kum_lww on kum.clients;
create trigger kum_lww before insert or update on kum.clients
    for each row execute function kum.kum_lww();
alter table kum.clients enable row level security;
drop policy if exists "kendi satırları" on kum.clients;
create policy "kendi satırları" on kum.clients for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));

-- ===== 0006_domain_rules.sql =====
-- Kurallar tarayıcı adresine de bakabilir (rules.field = 'domain').
alter table kum.rules drop constraint if exists rules_field_check;
alter table kum.rules add constraint rules_field_check
    check (field in ('app', 'title', 'domain'));

-- ===== 0007_archive_budget.sql =====
-- Proje arşivi (tags.archived_at) ve sözleşme bütçesi (adam-gün; tags/clients.budget_days).
alter table kum.tags add column if not exists archived_at timestamptz;
alter table kum.tags add column if not exists budget_days double precision;
alter table kum.clients add column if not exists budget_days double precision;

alter table kum.tags drop constraint if exists tags_budget_days_check;
alter table kum.tags add constraint tags_budget_days_check
    check (budget_days is null or budget_days > 0);
alter table kum.clients drop constraint if exists clients_budget_days_check;
alter table kum.clients add constraint clients_budget_days_check
    check (budget_days is null or budget_days > 0);

-- ===== 0008_settings.sql =====
-- Cihazdan bağımsız ayarlar (id: ayarın anahtarı, value: JSON metni).
create table if not exists kum.settings (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    value             text not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id)
);
create index if not exists settings_user_cursor on kum.settings (user_id, server_updated_at);

drop trigger if exists kum_lww on kum.settings;
create trigger kum_lww before insert or update on kum.settings
    for each row execute function kum.kum_lww();
alter table kum.settings enable row level security;
drop policy if exists "kendi satırları" on kum.settings;
create policy "kendi satırları" on kum.settings for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));

-- Erişim: oturum açmış kullanıcılar (satır güvenliğiyle yalnız kendi satırları) ve sunucu rolü.
grant all on all tables in schema kum to authenticated, service_role;
alter default privileges in schema kum grant all on tables to authenticated, service_role;
