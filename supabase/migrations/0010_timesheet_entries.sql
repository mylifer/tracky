-- Kum 0.9.22: zaman çizelgesi satırları cihazlar arasında taşınır. Kaydedilen, birleştirilen,
-- gönderilen ve silinen satırlar diğer bilgisayarda da aynı görünür; aktarılmış satır orada bir
-- daha gönderilmez. 0009'dan sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak
-- zararsızdır. Çalıştırılmazsa eşitleme sürer, yalnızca satırlar taşınmaz.
--
-- Tarih (YYYY-MM-DD) ve başlangıç (HH:MM) cihazın yerel saatindedir; metin olarak saklanır.
-- coverage: satırın kapsadığı takip aralıkları (JSON, unix ms).
create table if not exists public.timesheet_entries (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    date              text not null,
    start             text not null,
    hours             double precision not null check (hours > 0),
    kind              text not null check (kind in ('Working', 'Online', 'F2F')),
    details           text not null,
    party             text not null,
    project_id        text not null,
    division          text not null,
    actual_hours      double precision,
    coverage          text,
    timesheet_id      text,
    exported_at       timestamptz,
    dismissed_at      timestamptz,
    created_at        timestamptz not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id)
);
create index if not exists timesheet_entries_user_cursor
    on public.timesheet_entries (user_id, server_updated_at);

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi); yetkiler
-- 0009'daki gibi dar.
drop trigger if exists kum_lww on public.timesheet_entries;
create trigger kum_lww before insert or update on public.timesheet_entries
    for each row execute function public.kum_lww();
alter table public.timesheet_entries enable row level security;
drop policy if exists "kendi satırları" on public.timesheet_entries;
create policy "kendi satırları" on public.timesheet_entries for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
revoke all on public.timesheet_entries from anon;
revoke truncate, references, trigger on public.timesheet_entries from authenticated;
grant select, insert, update, delete on public.timesheet_entries to authenticated;
