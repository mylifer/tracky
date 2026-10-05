-- Kum 0.9.19: yetkileri daraltır. Supabase varsayılanı anon ve authenticated rollerine tablolarda
-- her yetkiyi (TRUNCATE dahil; TRUNCATE satır güvenliğini atlar) verir. Kum tablolara yalnızca
-- giriş yapmış kullanıcının jetonuyla erişir: anon'un hiçbir yetkisi gerekmez, authenticated'a
-- yalnızca okuma ve yazma yeter. Tetikleyici işlevinin arama yolu sabitlenir. SQL Editor'da bir
-- kez çalıştırın; tekrar çalıştırmak zararsızdır.
do $$
declare t text;
begin
    foreach t in array array['sessions', 'tags', 'rules', 'clients', 'settings'] loop
        if to_regclass(format('public.%I', t)) is not null then
            execute format('revoke all on public.%I from anon', t);
            execute format(
                'revoke truncate, references, trigger on public.%I from authenticated', t);
        end if;
    end loop;
end;
$$;

do $$
begin
    if to_regprocedure('public.kum_lww()') is not null then
        alter function public.kum_lww() set search_path = '';
    end if;
end;
$$;
