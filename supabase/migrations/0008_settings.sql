-- Kum 0.9.7: cihazdan bağımsız ayarlar (zaman çizelgeleri, takvim, gizlilik, hedefler,
-- görünüm, yapay zekâ) cihazlar arasında taşınır; yeni cihazda giriş yapınca gelir. id ayarın
-- anahtarı, value JSON metni. 0007'den sonra SQL Editor'da bir kez çalıştırın; tekrar
-- çalıştırmak zararsızdır. Çalıştırılmazsa eşitleme sürer, yalnızca ayarlar taşınmaz.
create table if not exists public.settings (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    value             text not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id)
);
create index if not exists settings_user_cursor on public.settings (user_id, server_updated_at);

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi).
drop trigger if exists kum_lww on public.settings;
create trigger kum_lww before insert or update on public.settings
    for each row execute function public.kum_lww();
alter table public.settings enable row level security;
drop policy if exists "kendi satırları" on public.settings;
create policy "kendi satırları" on public.settings for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
