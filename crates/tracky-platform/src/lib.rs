//! İşletim sistemine özel aktif pencere ve boşta kalma tespiti.
//!
//! - macOS: Erişilebilirlik API'si (odaktaki uygulama ve pencere başlığı) ve
//!   CoreGraphics (son girdi). Erişilebilirlik izni gerektirir.
//! - Windows: Win32 (ön plandaki pencere, süreç yolu, son girdi). İzin gerekmez.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "macos", windows)))]
use unsupported as imp;
#[cfg(windows)]
use windows as imp;

pub use imp::SystemProvider;

#[cfg(any(target_os = "macos", windows))]
mod address;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("Erişilebilirlik izni verilmedi")]
    PermissionDenied,
    #[error("bu işletim sistemi desteklenmiyor")]
    Unsupported,
}

/// Takip için gereken izinlerin durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    /// macOS Erişilebilirlik izni (Windows'ta her zaman `true`).
    pub accessibility: bool,
}

impl Permissions {
    pub fn all_granted(self) -> bool {
        self.accessibility
    }
}

/// İzinleri kullanıcıya sormadan kontrol eder.
pub fn permissions() -> Permissions {
    imp::permissions()
}

/// Eksik izinleri ister: macOS'ta sistem iletişim kutusunu gösterir ve
/// Sistem Ayarları > Gizlilik ve Güvenlik > Erişilebilirlik'e yönlendirir.
pub fn request_permissions() -> Permissions {
    imp::request_permissions()
}

/// Uygulamanın simgesi PNG olarak (kenarı en az `px` piksel). `app_id` gözlemdeki
/// kimliktir (macOS'ta bundle id ya da yol, Windows'ta exe yolu); uygulama bu
/// bilgisayarda yoksa (örn. başka cihazdan eşitlenen kayıt) `None`.
pub fn app_icon(app_id: &str, px: u32) -> Option<Vec<u8>> {
    imp::app_icon(app_id, px)
}

/// Bu bilgisayarın görseli PNG olarak: macOS'ta "Bu Mac Hakkında"daki model görseli,
/// boş kenarları kırpılmış kare. Diğer sistemlerde `None`.
pub fn computer_icon(px: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        macos::computer_icon(px).and_then(|png| trim_png(&png))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = px;
        None
    }
}

/// Saydam kenarları atar; görsel ortada kalacak şekilde kare kırpar. Yalnızca macOS'ta
/// çağrılır; testleri her sistemde çalışır.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn trim_png(data: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return Some(data.to_vec());
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let alpha = |x: usize, y: usize| buf[(y * w + x) * 4 + 3];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha(x, y) > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return None;
    }
    let side = (x1 - x0 + 1).max(y1 - y0 + 1);
    let (ox, oy) = (
        (x0 + x1 + 1) as isize / 2 - side as isize / 2,
        (y0 + y1 + 1) as isize / 2 - side as isize / 2,
    );
    let mut out = vec![0u8; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let (sx, sy) = (ox + x as isize, oy + y as isize);
            if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                let from = (sy as usize * w + sx as usize) * 4;
                out[(y * side + x) * 4..][..4].copy_from_slice(&buf[from..from + 4]);
            }
        }
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, side as u32, side as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&out).ok()?;
    }
    Some(png)
}

/// Her gözlem adımının ham sonucunu tek satırda verir (sorun giderme için).
pub fn diagnose() -> String {
    imp::diagnose()
}

// macOS'ta birim struct, Windows'ta önbellekli; ortak kurucu `default`.
#[allow(clippy::default_constructed_unit_structs)]
pub fn provider() -> SystemProvider {
    SystemProvider::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(rgba)
            .unwrap();
        out
    }

    #[test]
    fn trimming_keeps_the_opaque_part_centred_in_a_square() {
        // 10x10 saydam; (2..6, 3..5) opak: 4 geniş, 2 yüksek.
        let mut rgba = vec![0u8; 10 * 10 * 4];
        for y in 3..5 {
            for x in 2..6 {
                rgba[(y * 10 + x) * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let trimmed = trim_png(&png_rgba(10, 10, &rgba)).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(trimmed))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (4, 4));
        let alpha = |x: usize, y: usize| buf[(y * 4 + x) * 4 + 3];
        assert_eq!(
            [alpha(0, 0), alpha(0, 1), alpha(0, 2), alpha(0, 3)],
            [0, 255, 255, 0]
        );
    }

    #[test]
    fn a_fully_transparent_image_has_no_icon() {
        assert!(trim_png(&png_rgba(4, 4, &[0; 64])).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn this_mac_has_an_icon() {
        let icon = computer_icon(96).expect("bu Mac'in görseli");
        assert!(icon.len() > 500);
    }
}
