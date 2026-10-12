//! Eşitlemede gizli bilgilerin uçtan uca şifrelenmesi (takvim bağlantısı, Sheets anahtarı).
//!
//! Anahtar, her bilgisayarda bir kez girilen eşitleme parolasından Argon2id ile türetilir; tuz
//! hesabın kimliğidir (aynı hesap ve parola her bilgisayarda aynı anahtarı verir). Parola
//! saklanmaz; türetilen anahtar uygulamanın anahtar deposunda durur. Değer XChaCha20-Poly1305
//! ile şifrelenir; ek veri (AAD) olarak ayarın adı bağlanır, şifreli değer başka bir ayarın
//! yerine konamaz. Sunucu yalnızca `kum-enc:v1:` önekli şifreli metni görür.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

const PREFIX: &str = "kum-enc:v1:";
const NONCE_LEN: usize = 24;
/// Parolanın doğruluğunu sınamak için eşitlenen bilinen metin ([`check_value`]).
const CHECK_TEXT: &str = "kum";
const CHECK_AAD: &str = "sync_key_check";

/// Eşitleme parolasından türetilmiş 256 bitlik anahtar.
#[derive(Clone, PartialEq, Eq)]
pub struct SyncKey([u8; 32]);

impl std::fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncKey(…)")
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("eşitleme parolası en az 8 karakter olmalı")]
    TooShort,
    #[error("anahtar türetilemedi: {0}")]
    Derive(String),
    #[error("kayıtlı anahtar bozuk")]
    BadKey,
}

impl SyncKey {
    /// `passphrase` ve hesabın kimliğinden (`user_id`) anahtar türetir. Bilerek yavaştır
    /// (Argon2id, 64 MiB): sunucudaki şifreli metne karşı parola denemesini pahalılaştırır.
    pub fn derive(passphrase: &str, user_id: &str) -> Result<Self, VaultError> {
        if passphrase.chars().count() < 8 {
            return Err(VaultError::TooShort);
        }
        let params = Params::new(64 * 1024, 3, 1, Some(32))
            .map_err(|e| VaultError::Derive(e.to_string()))?;
        let salt = format!("kum-sync-v1:{user_id}");
        let mut key = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), salt.as_bytes(), &mut key)
            .map_err(|e| VaultError::Derive(e.to_string()))?;
        Ok(Self(key))
    }

    /// Anahtar deposunda saklamak için metin hali.
    pub fn export(&self) -> String {
        B64.encode(self.0)
    }

    pub fn import(text: &str) -> Result<Self, VaultError> {
        let bytes = B64.decode(text.trim()).map_err(|_| VaultError::BadKey)?;
        Ok(Self(bytes.try_into().map_err(|_| VaultError::BadKey)?))
    }

    /// `plaintext`'i `aad`'a (ayarın adı) bağlı olarak şifreler.
    pub fn seal(&self, aad: &str, plaintext: &str) -> String {
        let cipher = XChaCha20Poly1305::new((&self.0).into());
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .expect("XChaCha20-Poly1305 şifrelemesi bellek içinde başarısız olmaz");
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        format!("{PREFIX}{}", B64.encode(out))
    }

    /// [`Self::seal`] ile şifrelenmiş metni açar; başka anahtar, başka `aad` ya da bozuk metinde
    /// `None`.
    pub fn open(&self, aad: &str, sealed: &str) -> Option<String> {
        let bytes = B64.decode(sealed.strip_prefix(PREFIX)?).ok()?;
        if bytes.len() < NONCE_LEN {
            return None;
        }
        let (nonce, body) = bytes.split_at(NONCE_LEN);
        let plain = XChaCha20Poly1305::new((&self.0).into())
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: body,
                    aad: aad.as_bytes(),
                },
            )
            .ok()?;
        String::from_utf8(plain).ok()
    }

    /// Parolayı diğer bilgisayarlarda sınamak için eşitlenen değer.
    pub fn check_value(&self) -> String {
        self.seal(CHECK_AAD, CHECK_TEXT)
    }

    /// `check` ([`Self::check_value`]) bu anahtarla mı üretilmiş: parola aynı mı.
    pub fn matches(&self, check: &str) -> bool {
        self.open(CHECK_AAD, check).as_deref() == Some(CHECK_TEXT)
    }
}

/// Değer şifreli bir metin mi.
pub fn is_sealed(value: &str) -> bool {
    value.starts_with(PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(pass: &str, user: &str) -> SyncKey {
        SyncKey::derive(pass, user).unwrap()
    }

    #[test]
    fn same_passphrase_and_account_give_the_same_key() {
        let a = key("uzun bir parola", "kullanici-1");
        assert_eq!(a, key("uzun bir parola", "kullanici-1"));
        assert_ne!(a, key("uzun bir parola", "kullanici-2"));
        assert_ne!(a, key("başka bir parola", "kullanici-1"));
        assert_eq!(SyncKey::import(&a.export()).unwrap(), a);
        assert_eq!(SyncKey::derive("kısa", "u"), Err(VaultError::TooShort));
        assert_eq!(SyncKey::import("bozuk"), Err(VaultError::BadKey));
    }

    #[test]
    fn sealed_values_open_only_with_the_same_key_and_name() {
        let a = key("uzun bir parola", "u");
        let sealed = a.seal(
            "calendar_url",
            "https://outlook.office365.com/x/calendar.ics",
        );
        assert!(is_sealed(&sealed));
        assert!(!sealed.contains("outlook"));
        // Her şifreleme farklı (rastgele nonce).
        assert_ne!(
            sealed,
            a.seal(
                "calendar_url",
                "https://outlook.office365.com/x/calendar.ics"
            )
        );
        assert_eq!(
            a.open("calendar_url", &sealed).as_deref(),
            Some("https://outlook.office365.com/x/calendar.ics")
        );
        assert_eq!(
            a.open("timesheet.sheetToken", &sealed),
            None,
            "başka ayarın yerine konamaz"
        );
        assert_eq!(
            key("yanlış parola!", "u").open("calendar_url", &sealed),
            None
        );
        assert_eq!(a.open("calendar_url", "kum-enc:v1:bozuk"), None);
        assert_eq!(a.open("calendar_url", "https://düz.metin"), None);
        assert!(a.matches(&a.check_value()));
        assert!(!key("yanlış parola!", "u").matches(&a.check_value()));
    }
}
