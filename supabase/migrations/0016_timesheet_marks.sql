-- Kum 0.9.72: zaman çizelgesine gönderilen takvim blokları. Yalnızca gönderilen blokların işi
-- zaman çizelgesinde satır olur; öteki bilgisayarda da aynı bloklar gönderilmiş sayılır. Kimliği
-- "since:" ile başlayan boş aralık bir bilgisayarın blokları seçerek göndermeye geçtiği andır.
-- 0015'ten sonra SQL Editor'da bir kez çalıştırın; tekrar çalıştırmak zararsızdır. Çalıştırılmazsa
-- eşitleme sürer, yalnızca gönderilen bloklar taşınmaz.
create table if not exists public.timesheet_marks (
    id                text not null,
    user_id           uuid not null default auth.uid() references auth.users (id) on delete cascade,
    kind              text not null check (kind in ('sent', 'unsent', 'since')),
    started_at        timestamptz not null,
    ended_at          timestamptz not null,
    updated_at        timestamptz not null,
    deleted_at        timestamptz,
    server_updated_at timestamptz not null default now(),
    writer            uuid,
    primary key (user_id, id),
    check (ended_at >= started_at)
);
create index if not exists timesheet_marks_user_cursor
    on public.timesheet_marks (user_id, server_updated_at);

-- Son yazan kazanır ve her kullanıcı yalnız kendi satırlarını görür (0001'deki gibi); yetkiler
-- 0009'daki gibi dar.
drop trigger if exists kum_lww on public.timesheet_marks;
create trigger kum_lww before insert or update on public.timesheet_marks
    for each row execute function public.kum_lww();
alter table public.timesheet_marks enable row level security;
drop policy if exists "kendi satırları" on public.timesheet_marks;
create policy "kendi satırları" on public.timesheet_marks for all to authenticated
    using (user_id = (select auth.uid()))
    with check (user_id = (select auth.uid()));
revoke all on public.timesheet_marks from anon;
revoke truncate, references, trigger on public.timesheet_marks from authenticated;
grant select, insert, update, delete on public.timesheet_marks to authenticated;
notify pgrst, 'reload schema';
