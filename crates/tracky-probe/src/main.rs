//! Takip katmanını gerçek makinede denemek için komut satırı aracı.
//!
//! ```text
//! tracky-probe [--db DOSYA] [run]          Takibi başlatır (Ctrl+C ile durur)
//! tracky-probe [--db DOSYA] report         Bugünün uygulama özetini yazdırır
//! tracky-probe [--db DOSYA] privacy        Gizlilik ayarlarını gösterir
//! tracky-probe [--db DOSYA] exclude ID     Uygulamayı hiç kaydetme
//! tracky-probe [--db DOSYA] hide-title ID  Uygulamanın pencere başlığını kaydetme
//! tracky-probe diag                        15 sn boyunca ham gözlemleri yazdırır
//! ```

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use tracky_core::{ActivityProvider, Engine, EngineConfig, Session, Store};

const DEFAULT_DB: &str = "tracky-probe.db";
/// Devam eden oturum bu kadar gözlemde bir diske yazılır.
const FLUSH_EVERY: u32 = 5;
/// Bu kadar gözlemden sonra hâlâ oturum yoksa kullanıcı uyarılır.
const WARN_AFTER: u32 = 5;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let db = match args.iter().position(|a| a == "--db") {
        Some(i) if i + 1 < args.len() => {
            let path = args.remove(i + 1);
            args.remove(i);
            path
        }
        Some(_) => return fail("--db için dosya yolu gerekli"),
        None => DEFAULT_DB.to_string(),
    };
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{db} açılamadı: {e}")),
    };

    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["run"] => run(&store),
        ["report"] => report(&store),
        ["diag"] => diag(),
        ["privacy"] => show_privacy(&store),
        ["exclude", id] => edit_privacy(&store, |s| s.excluded_apps.push(id.to_string())),
        ["hide-title", id] => edit_privacy(&store, |s| s.hidden_title_apps.push(id.to_string())),
        _ => Err("bilinmeyen komut; kullanım için kaynak dosyanın başına bakın".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn run(store: &Store) -> Result<(), String> {
    wait_for_permissions();

    let privacy = store.privacy_settings().map_err(|e| e.to_string())?;
    let mut provider = tracky_platform::provider();
    if let Err(e @ tracky_platform::PlatformError::Unsupported) = provider.active_window() {
        return Err(e.to_string());
    }
    let mut engine = Engine::new(EngineConfig::default());

    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    ctrlc::set_handler(move || flag.store(false, Ordering::SeqCst)).map_err(|e| e.to_string())?;

    println!("Takip başladı (Ctrl+C ile durdur).");
    let mut ticks = 0u32;
    while running.load(Ordering::SeqCst) {
        let window = match provider.active_window() {
            Ok(w) => w.and_then(|w| privacy.apply(w)),
            Err(e) => {
                eprintln!("pencere okunamadı: {e}");
                None
            }
        };
        let idle = provider.idle_seconds().unwrap_or(0);
        let before = engine.current().map(|s| s.id);

        if let Some(closed) = engine.tick(Utc::now(), window, idle) {
            save(store, &closed);
        }
        match engine.current() {
            Some(s) if Some(s.id) != before => {
                println!(
                    "{}  {}  —  {}  [{}]",
                    clock(s.started_at),
                    s.app_name,
                    s.title,
                    s.app_id
                );
            }
            None if before.is_some() => println!("{}  (boşta / kayıt yok)", clock(Utc::now())),
            _ => {}
        }

        ticks += 1;
        if ticks == WARN_AFTER && engine.current().is_none() {
            println!(
                "Uyarı: {WARN_AFTER} sn'dir aktif pencere kaydedilmedi (son idle: {idle} sn).\n\
                 Sorunu bulmak için çıktısını paylaşın: tracky-probe diag"
            );
        }
        if ticks.is_multiple_of(FLUSH_EVERY)
            && let Some(current) = engine.current()
        {
            save(store, current);
        }
        thread::sleep(Duration::from_secs(1));
    }

    if let Some(last) = engine.flush(Utc::now()) {
        save(store, &last);
    }
    println!();
    report(store)
}

fn diag() -> Result<(), String> {
    println!(
        "İzin isteniyor: {:?}",
        tracky_platform::request_permissions()
    );
    println!("15 sn boyunca farklı pencerelere geçin:");
    for _ in 0..15 {
        println!("{}  {}", clock(Utc::now()), tracky_platform::diagnose());
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn wait_for_permissions() {
    if tracky_platform::request_permissions().all_granted() {
        return;
    }
    println!(
        "Erişilebilirlik izni gerekli: Sistem Ayarları > Gizlilik ve Güvenlik > Erişilebilirlik\n\
         bölümünden bu aracı çalıştıran uygulamayı (örn. Terminal) etkinleştirin. Bekleniyor..."
    );
    while !tracky_platform::permissions().all_granted() {
        thread::sleep(Duration::from_secs(2));
    }
    println!("İzin verildi.");
}

fn report(store: &Store) -> Result<(), String> {
    let start = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(Local).earliest())
        .ok_or("gün başlangıcı hesaplanamadı")?
        .with_timezone(&Utc);
    let totals = store
        .app_totals(start, Utc::now())
        .map_err(|e| e.to_string())?;
    if totals.is_empty() {
        println!("Bugün için kayıt yok.");
        return Ok(());
    }
    println!("Bugün:");
    for t in &totals {
        println!("  {:>10}  {}", duration(t.seconds), t.label);
    }
    println!(
        "  {:>10}  toplam",
        duration(totals.iter().map(|t| t.seconds).sum())
    );
    Ok(())
}

fn show_privacy(store: &Store) -> Result<(), String> {
    let s = store.privacy_settings().map_err(|e| e.to_string())?;
    println!(
        "Duraklatıldı:              {}",
        if s.paused { "evet" } else { "hayır" }
    );
    println!(
        "Gizli pencereleri gizle:   {}",
        if s.hide_private_windows {
            "evet"
        } else {
            "hayır"
        }
    );
    println!("Kaydedilmeyen uygulamalar: {:?}", s.excluded_apps);
    println!("Başlığı gizlenenler:       {:?}", s.hidden_title_apps);
    Ok(())
}

fn edit_privacy(
    store: &Store,
    edit: impl FnOnce(&mut tracky_core::PrivacySettings),
) -> Result<(), String> {
    let mut s = store.privacy_settings().map_err(|e| e.to_string())?;
    edit(&mut s);
    for list in [&mut s.excluded_apps, &mut s.hidden_title_apps] {
        list.sort_unstable();
        list.dedup();
    }
    store.save_privacy_settings(&s).map_err(|e| e.to_string())?;
    show_privacy(store)
}

fn save(store: &Store, session: &Session) {
    if let Err(e) = store.upsert_session(session) {
        eprintln!("kayıt yazılamadı: {e}");
    }
}

fn clock(t: DateTime<Utc>) -> String {
    t.with_timezone(&Local).format("%H:%M:%S").to_string()
}

fn duration(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s}sn"),
        (0, _) => format!("{m}dk {s}sn"),
        _ => format!("{h}sa {m}dk"),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("hata: {msg}");
    ExitCode::FAILURE
}
