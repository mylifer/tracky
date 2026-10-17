//! Windows'ta görüşme ve izleme sinyallerinin yorumu. Kayıt defteri, süreç ve medya sorguları
//! Windows'a özgüdür ([`crate::windows`]); burada yalnızca okunanların anlamı var, her sistemde
//! test edilir.

/// Windows'un gizlilik kaydındaki (CapabilityAccessManager) bir uygulamanın mikrofon kullanımı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentEntry {
    /// Paketsiz uygulamada exe yolu (`C:#Program Files#Zoom#bin#Zoom.exe`), paketli uygulamada
    /// paket ailesi (`MSTeams_8wekyb3d8bbwe`).
    pub key: String,
    pub packaged: bool,
    /// Son kullanımın başı ve sonu (FILETIME); kullanım sürerken sonu 0.
    pub start: u64,
    pub stop: u64,
}

impl ConsentEntry {
    /// Mikrofon şu an kullanılıyor mu: kullanım başlamış ve bitmemiş.
    pub fn in_use(&self) -> bool {
        self.start != 0 && self.stop == 0
    }
}

/// Çalışan bir süreç: exe yolu ve (paketli uygulamada) paket ailesi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub path: String,
    pub family: Option<String>,
}

/// Mikrofonu şu an kullanan uygulamaların kimlikleri (exe yolu; oturumlardaki uygulama kimliğiyle
/// aynı). Çöken uygulamanın kaydında kullanım bitmemiş kalabilir: yalnızca çalışan süreçler
/// sayılır. Paketli uygulamanın (yeni Teams) kimliği paketin içindeki süreçtir.
pub fn microphone_users(entries: &[ConsentEntry], running: &[Process]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in entries.iter().filter(|e| e.in_use()) {
        let found = if e.packaged {
            running
                .iter()
                .filter(|p| {
                    p.family
                        .as_deref()
                        .is_some_and(|f| f.eq_ignore_ascii_case(&e.key))
                })
                // Paketin kendi exe'si (WindowsApps altında), yardımcı WebView2 süreçleri değil.
                .min_by_key(|p| !p.path.to_ascii_lowercase().contains(r"\windowsapps\"))
                .map(|p| p.path.clone())
        } else {
            let path = e.key.replace('#', "\\");
            running
                .iter()
                .find(|p| p.path.eq_ignore_ascii_case(&path))
                .map(|p| p.path.clone())
        };
        if let Some(id) = found
            && !out.iter().any(|o| o.eq_ignore_ascii_case(&id))
        {
            out.push(id);
        }
    }
    out
}

/// Müzik çalarlar: çalarken ekran başında olunduğunu söylemez (dinlerken uzakta olunabilir).
const MUSIC_APPS: &[&str] = &[
    "spotify",
    "zunemusic",
    "applemusic",
    "itunes",
    "tidal",
    "deezer",
    "amazonmusic",
    "foobar2000",
    "musicbee",
    "winamp",
    "aimp",
];

/// Medya oturumunun sahibi (AppUserModelID ya da exe adı) bir müzik çalar mı?
pub fn is_music_app(source: &str) -> bool {
    let s = source.to_ascii_lowercase();
    MUSIC_APPS.iter().any(|m| s.contains(m))
}

/// Ekran dikdörtgeni: sol, üst, sağ, alt.
pub type Rect = (i32, i32, i32, i32);

/// Pencere ekranın tamamını kaplıyor mu (tam ekran video, sunum)? Ekranı kaplayan büyütülmüş
/// pencere görev çubuğunu açıkta bırakır, sayılmaz.
pub fn covers(window: Rect, monitor: Rect) -> bool {
    window.0 <= monitor.0 && window.1 <= monitor.1 && window.2 >= monitor.2 && window.3 >= monitor.3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, packaged: bool, start: u64, stop: u64) -> ConsentEntry {
        ConsentEntry {
            key: key.into(),
            packaged,
            start,
            stop,
        }
    }

    fn process(path: &str, family: Option<&str>) -> Process {
        Process {
            path: path.into(),
            family: family.map(Into::into),
        }
    }

    const ZOOM: &str = r"C:\Users\k\AppData\Roaming\Zoom\bin\Zoom.exe";
    const TEAMS: &str = r"C:\Program Files\WindowsApps\MSTeams_25198.1112.3855.1066_x64__8wekyb3d8bbwe\ms-teams.exe";

    #[test]
    fn apps_using_the_microphone_are_named_by_their_exe() {
        let running = [
            process(ZOOM, None),
            process(
                r"C:\Program Files (x86)\Microsoft\EdgeWebView\Application\msedgewebview2.exe",
                Some("MSTeams_8wekyb3d8bbwe"),
            ),
            process(TEAMS, Some("MSTeams_8wekyb3d8bbwe")),
        ];
        let entries = [
            entry(
                r"C:#Users#k#AppData#Roaming#Zoom#bin#Zoom.exe",
                false,
                10,
                0,
            ),
            entry("MSTeams_8wekyb3d8bbwe", true, 10, 0),
            // Kullanımı bitmiş: sayılmaz.
            entry("Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe", true, 10, 20),
        ];
        assert_eq!(microphone_users(&entries, &running), [ZOOM, TEAMS]);
    }

    #[test]
    fn a_crashed_app_does_not_hold_the_microphone() {
        // Kayıtta kullanım bitmemiş ama uygulama çalışmıyor.
        let entries = [
            entry(
                r"C:#Users#k#AppData#Roaming#Zoom#bin#Zoom.exe",
                false,
                10,
                0,
            ),
            entry("MSTeams_8wekyb3d8bbwe", true, 10, 0),
        ];
        assert!(microphone_users(&entries, &[]).is_empty());
        // Hiç kullanılmamış kayıt da sayılmaz.
        assert!(!entry("x", false, 0, 0).in_use());
    }

    #[test]
    fn music_players_are_not_watching() {
        assert!(is_music_app("Spotify.exe"));
        assert!(is_music_app("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"));
        assert!(is_music_app(
            "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"
        ));
        assert!(!is_music_app("Chrome"));
        assert!(!is_music_app("MSEdge"));
        assert!(!is_music_app("VLC.exe"));
    }

    #[test]
    fn only_a_window_over_the_whole_screen_is_fullscreen() {
        let monitor = (0, 0, 1920, 1080);
        assert!(covers((0, 0, 1920, 1080), monitor));
        // Büyütülmüş pencere görev çubuğunu açıkta bırakır.
        assert!(!covers((0, 0, 1920, 1040), monitor));
        // İkinci ekrandaki tam ekran.
        assert!(covers((1920, 0, 3840, 1080), (1920, 0, 3840, 1080)));
    }
}
