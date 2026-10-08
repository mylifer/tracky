//! Takvimde ve ayrıntılarda gösterilen uygulama simgeleri ve site simgeleri (favicon).
//!
//! Simgeler `data:` adresi olarak döner (CSP yalnızca `self` ve `data:` görsellerine izin
//! verir) ve uygulama verisinin `icons/` klasöründe saklanır: uygulama simgesi bir kez
//! çıkarılır, site simgesi bir kez indirilir. Bulunamayan simge de kısa süre hatırlanır ki
//! her takvim çiziminde yeniden aranmasın. Simgeler eşitlenmez; her cihaz kendisi bulur.

use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use tracky_core::favicon;
use url::Url;

/// Çıkarılan uygulama simgesinin kenarı (nokta); Retina'da iki katı piksel olabilir.
const APP_ICON_PX: u32 = 64;
/// Bulunan simge bu kadar sonra yenilenir (uygulama güncellenir, site simgesini değiştirir).
const FOUND_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
/// Bulunamayan uygulama simgesi ertesi gün yeniden denenir (uygulama sonradan kurulabilir).
const APP_MISS_TTL: Duration = Duration::from_secs(24 * 3600);
/// Bulunamayan site simgesi bir hafta sonra yeniden denenir (site o an kapalı olabilir).
const SITE_MISS_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// Ana sayfadan yalnızca başı okunur: bağlantılar `<head>`'dedir.
const PAGE_BYTES: u64 = 256 * 1024;
/// Bundan büyük simge alınmaz.
const ICON_BYTES: u64 = 256 * 1024;
/// Sayfanın önerdiği simgelerden en çok bu kadarı denenir.
const MAX_CANDIDATES: usize = 3;
/// Yönlendirmeler elle izlenir (her adım [`allowed`] ile denetlenir); en çok bu kadar.
const MAX_REDIRECTS: usize = 4;

type CmdResult<T> = Result<T, String>;

/// Uygulamanın simgesi; bu bilgisayarda yoksa (elle kayıt, boşta, başka cihazdan gelen
/// kayıt) `None` ve arayüz baş harfi gösterir.
#[tauri::command]
pub async fn app_icon(app: AppHandle, app_id: String) -> CmdResult<Option<String>> {
    if app_id.starts_with("kum.") {
        return Ok(None);
    }
    let dir = cache_dir(&app)?;
    let handle = app.clone();
    blocking(move || {
        cached(&dir, "app", &app_id, APP_MISS_TTL, || {
            let png = extract_app_icon(&handle, app_id.clone())?;
            Some(data_url("image/png", &png))
        })
    })
    .await
}

/// Sitenin simgesi: ana sayfanın önerdiği simge, yoksa `/favicon.ico`. İstek yalnızca
/// sitenin kendisine gider (zaten ziyaret edilmiş bir adres); üçüncü taraf bir simge
/// hizmetine alan adı gönderilmez.
#[tauri::command]
pub async fn site_icon(app: AppHandle, domain: String) -> CmdResult<Option<String>> {
    let domain = domain.trim().to_ascii_lowercase();
    if !favicon::is_fetchable_host(&domain) {
        return Ok(None);
    }
    let dir = cache_dir(&app)?;
    blocking(move || {
        cached(&dir, "site", &domain, SITE_MISS_TTL, || {
            fetch_site_icon(&domain)
        })
    })
    .await
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

fn cache_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("icons");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Önbellekteki simge; yoksa ya da eskidiyse `find` ile bulunur ve yazılır. Boş dosya
/// "bulunamadı" demektir.
fn cached(
    dir: &Path,
    kind: &str,
    key: &str,
    miss_ttl: Duration,
    find: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let hash = Sha256::digest(key.as_bytes());
    let name: String = hash[..12].iter().map(|b| format!("{b:02x}")).collect();
    let path = dir.join(format!("{kind}-{name}.txt"));
    if let Ok(meta) = std::fs::metadata(&path) {
        let age = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .unwrap_or_default();
        let found = meta.len() > 0;
        if age < if found { FOUND_TTL } else { miss_ttl } {
            return found.then(|| std::fs::read_to_string(&path).ok()).flatten();
        }
    }
    let icon = find();
    if let Err(e) = std::fs::write(&path, icon.as_deref().unwrap_or("")) {
        log_error!("simge önbelleğe yazılamadı: {e}");
    }
    icon
}

/// AppKit ana iş parçacığında çalışsın diye macOS'ta oraya gönderilir.
#[cfg(target_os = "macos")]
fn extract_app_icon(app: &AppHandle, app_id: String) -> Option<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(tracky_platform::app_icon(&app_id, APP_ICON_PX));
    })
    .ok()?;
    rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
}

#[cfg(not(target_os = "macos"))]
fn extract_app_icon(_app: &AppHandle, app_id: String) -> Option<Vec<u8>> {
    tracky_platform::app_icon(&app_id, APP_ICON_PX)
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(6)))
            // Bazı siteler tanımadığı istemciye simge yerine hata sayfası döner.
            .user_agent("Mozilla/5.0 (Kum)")
            // Yönlendirmeler `get` içinde, hedefi denetlenerek izlenir.
            .max_redirects(0)
            .build()
            .into()
    })
}

/// Alan adının simgesi; bulunamazsa üst alan adınınki (`app.asana.com` giriş sayfasına
/// yönlenir ve simgesi yoktur, `asana.com`'un vardır).
fn fetch_site_icon(domain: &str) -> Option<String> {
    let parent = domain
        .split_once('.')
        .map(|(_, rest)| rest)
        .filter(|rest| rest.contains('.'));
    std::iter::once(domain).chain(parent).find_map(host_icon)
}

fn host_icon(host: &str) -> Option<String> {
    // Şirket içi siteler çoğu zaman yalnızca http'dir.
    for scheme in ["https", "http"] {
        let page = Url::parse(&format!("{scheme}://{host}/")).ok()?;
        // Bot korumalı siteler ana sayfayı reddedip `/favicon.ico`'yu verir.
        if let Some(html) = fetch_page(&page) {
            for link in favicon::icon_links(&html, &page)
                .iter()
                .take(MAX_CANDIDATES)
            {
                if link.starts_with("data:") {
                    if link.len() as u64 <= ICON_BYTES {
                        return Some(link.clone());
                    }
                } else if let Some(icon) = Url::parse(link).ok().and_then(|u| fetch_icon(&u, &page))
                {
                    return Some(icon);
                }
            }
        }
        if let Some(icon) = page
            .join("/favicon.ico")
            .ok()
            .and_then(|u| fetch_icon(&u, &page))
        {
            return Some(icon);
        }
    }
    None
}

/// `page` sitesinden istenebilecek adres mi? Ziyaret edilmiş ana makinenin kendisine
/// (şirket içi olsa da) gidilir. Başka bir ana makine (sayfadaki simge bağlantısı ya da
/// yönlendirme) yalnızca aynı siteden (alt ya da üst alan adı) olabilir; IP adresi,
/// `localhost` ve yerel ağa çözülen adlar reddedilir. Yoksa bir sayfa uygulamayı
/// `http://192.168.1.1/...` gibi iç ağ adreslerine istek atmaya yöneltebilirdi.
fn allowed(url: &Url, page: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let (Some(host), Some(page_host)) = (url.host_str(), page.host_str()) else {
        return false;
    };
    if host == page_host {
        return true;
    }
    if !allowed_name(url, page_host) {
        return false;
    }
    // Ad yerel ağa çözülüyorsa da gidilmez.
    let port = url.port_or_known_default().unwrap_or(443);
    match (host, port).to_socket_addrs() {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            !addrs.is_empty() && addrs.iter().all(|a| is_public(a.ip()))
        }
        Err(_) => false,
    }
}

/// [`allowed`]'ın ağa sormayan kısmı: ad IP ya da `localhost` değil ve `page_host` ile aynı
/// sitede (biri ötekinin alt alan adı; kısa olan en az iki parçalı).
fn allowed_name(url: &Url, page_host: &str) -> bool {
    let Some(url::Host::Domain(host)) = url.host() else {
        return false;
    };
    let host = host.trim_end_matches('.');
    if host == "localhost" || host.ends_with(".localhost") {
        return false;
    }
    let page_host = page_host.trim_end_matches('.');
    let (short, long) = if host.len() <= page_host.len() {
        (host, page_host)
    } else {
        (page_host, host)
    };
    short.contains('.') && (long == short || long.ends_with(&format!(".{short}")))
}

/// İnternetteki bir adres mi (yerel ağ, geri döngü, bağlantı-yerel vb. değil)?
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                // 100.64.0.0/10: operatör NAT'ı.
                || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // fc00::/7 benzersiz yerel, fe80::/10 bağlantı-yerel.
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// `url`'ye GET; yönlendirmeler her adımda [`allowed`] ile denetlenerek izlenir.
fn get(url: &Url, page: &Url) -> Option<ureq::http::Response<ureq::Body>> {
    let mut url = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        if !allowed(&url, page) {
            return None;
        }
        let resp = agent().get(url.as_str()).call().ok()?;
        if !resp.status().is_redirection() {
            return Some(resp);
        }
        let location = resp.headers().get("location")?.to_str().ok()?;
        url = url.join(location).ok()?;
    }
    None
}

fn fetch_page(url: &Url) -> Option<String> {
    let mut resp = get(url, url)?;
    let mut bytes = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(PAGE_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn fetch_icon(url: &Url, page: &Url) -> Option<String> {
    let mut resp = get(url, page)?;
    let declared = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });
    let bytes = resp
        .body_mut()
        .with_config()
        .limit(ICON_BYTES)
        .read_to_vec()
        .ok()?;
    // Sunucunun dediğine değil içeriğe bakılır: "200 OK" ile dönen hata sayfaları da var.
    let mime =
        sniff(&bytes).or(declared.filter(|m| m.starts_with("image/") && m != "image/svg+xml"))?;
    Some(data_url(&mime, &bytes))
}

fn sniff(b: &[u8]) -> Option<String> {
    let m = if b.starts_with(b"\x89PNG") {
        "image/png"
    } else if b.starts_with(&[0, 0, 1, 0]) {
        "image/x-icon"
    } else if b.starts_with(b"GIF8") {
        "image/gif"
    } else if b.starts_with(&[0xFF, 0xD8]) {
        "image/jpeg"
    } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if is_svg(b) {
        "image/svg+xml"
    } else {
        return None;
    };
    Some(m.into())
}

fn is_svg(b: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&b[..b.len().min(1024)]).to_ascii_lowercase();
    let head = head.trim_start_matches('\u{feff}').trim_start();
    (head.starts_with("<?xml") || head.starts_with("<svg") || head.starts_with("<!--"))
        && head.contains("<svg")
}

fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_images_not_html() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n").as_deref(), Some("image/png"));
        assert_eq!(sniff(&[0, 0, 1, 0, 1]).as_deref(), Some("image/x-icon"));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?>\n<svg xmlns=\"\"/>").as_deref(),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"<!DOCTYPE html><html>"), None);
    }

    #[test]
    fn icons_are_fetched_only_from_the_same_site() {
        let page = Url::parse("https://app.asana.com/").unwrap();
        let ok = |u: &str| allowed_name(&Url::parse(u).unwrap(), page.host_str().unwrap());
        assert!(ok("https://asana.com/favicon.ico"));
        assert!(ok("https://static.app.asana.com/i.png"));
        assert!(!ok("https://cdn.com/i.png"));
        assert!(!ok("https://evilasana.com/i.png"));
        assert!(!ok("http://192.168.1.1/i.png"));
        assert!(!ok("http://[::1]/i.png"));
        assert!(!ok("http://localhost:8080/i.png"));
        assert!(!ok("https://com/i.png"));
        // Ziyaret edilen ana makinenin kendisi (şirket içi de olsa) istenebilir.
        let intranet = Url::parse("http://10.0.0.5:8080/").unwrap();
        assert!(allowed(
            &Url::parse("http://10.0.0.5:8080/favicon.ico").unwrap(),
            &intranet
        ));
        assert!(!allowed(
            &Url::parse("http://10.0.0.6/favicon.ico").unwrap(),
            &intranet
        ));
        assert!(!allowed(
            &Url::parse("file:///etc/passwd").unwrap(),
            &intranet
        ));
    }

    #[test]
    fn private_addresses_are_not_public() {
        for ip in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_public("140.82.112.3".parse().unwrap()));
        assert!(is_public("2606:4700::1111".parse().unwrap()));
    }

    #[test]
    fn cache_remembers_hits_and_misses() {
        let dir = std::env::temp_dir().join(format!("kum-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let hit = cached(&dir, "t", "a", APP_MISS_TTL, || Some("data:x".into()));
        assert_eq!(hit.as_deref(), Some("data:x"));
        let again = cached(&dir, "t", "a", APP_MISS_TTL, || {
            panic!("önbellekten gelmeliydi")
        });
        assert_eq!(again.as_deref(), Some("data:x"));
        assert_eq!(cached(&dir, "t", "b", APP_MISS_TTL, || None), None);
        assert_eq!(
            cached(&dir, "t", "b", APP_MISS_TTL, || panic!(
                "bulunamadı hatırlanmalı"
            )),
            None
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
