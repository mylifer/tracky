//! Gizli bilgiler (oturum jetonları, Google bağlantısı, yapay zekâ anahtarı) işletim sisteminin
//! anahtar deposunda durur: macOS'ta Anahtar Zinciri, Windows'ta Kimlik Bilgisi Yöneticisi.
//! Veritabanında (ve yedeklerde) yalnızca "anahtar deposunda" işareti kalır.
//!
//! Anahtar deposu kullanılamazsa (izin verilmedi, desteklenmeyen sistem) eski davranışa
//! dönülür: değer veritabanında kalır ve durum günlüğe yazılır. Hiçbir şey çalışmaz hale gelmez.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tracky_core::{Store, StoreError};

/// Değeri anahtar deposunda olan ayarın veritabanındaki yer tutucusu.
const MARKER: &str = "keychain";

#[cfg(all(not(test), any(target_os = "macos", windows)))]
mod backend {
    const SERVICE: &str = "com.kum.app";

    fn entry(name: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, name).map_err(|e| e.to_string())
    }

    pub fn get(name: &str) -> Result<Option<String>, String> {
        match entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn set(name: &str, value: &str) -> Result<(), String> {
        entry(name)?.set_password(value).map_err(|e| e.to_string())
    }

    pub fn delete(name: &str) -> Result<(), String> {
        match entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Anahtar deposu olmayan sistem: değerler veritabanında kalır.
#[cfg(all(not(test), not(any(target_os = "macos", windows))))]
mod backend {
    const NONE: &str = "bu sistemde anahtar deposu yok";

    pub fn get(_: &str) -> Result<Option<String>, String> {
        Err(NONE.into())
    }

    pub fn set(_: &str, _: &str) -> Result<(), String> {
        Err(NONE.into())
    }

    pub fn delete(_: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Testlerde gerçek Anahtar Zinciri'ne dokunulmaz.
#[cfg(test)]
pub(crate) mod backend {
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Testler aynı sahte depoyu paylaşır: sırayla çalışsınlar.
    pub static SERIAL: Mutex<()> = Mutex::new(());
    pub static ITEMS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    /// `true` iken anahtar deposu kullanılamıyormuş gibi davranır.
    pub static BROKEN: Mutex<bool> = Mutex::new(false);

    fn check() -> Result<(), String> {
        if *BROKEN.lock().unwrap() {
            Err("anahtar deposu yok".into())
        } else {
            Ok(())
        }
    }

    pub fn get(name: &str) -> Result<Option<String>, String> {
        check()?;
        Ok(ITEMS
            .lock()
            .unwrap()
            .get_or_insert_default()
            .get(name)
            .cloned())
    }

    pub fn set(name: &str, value: &str) -> Result<(), String> {
        check()?;
        ITEMS
            .lock()
            .unwrap()
            .get_or_insert_default()
            .insert(name.into(), value.into());
        Ok(())
    }

    pub fn delete(name: &str) -> Result<(), String> {
        check()?;
        ITEMS.lock().unwrap().get_or_insert_default().remove(name);
        Ok(())
    }
}

fn is_marker(v: &Value) -> bool {
    v.get(MARKER).and_then(Value::as_bool) == Some(true)
}

/// Bütün değeri gizli olan ayarı okur. Veritabanında hâlâ açık metin duruyorsa (eski sürüm)
/// anahtar deposuna taşır. `null` ya da kayıt yoksa `None`: yedekten dönünce ya da çıkış
/// yapılınca veritabanındaki işaret silinir ve depodaki eski değer okunmaz.
pub fn load<T: DeserializeOwned>(store: &Store, key: &str) -> Option<T> {
    let stored: Value = store.setting(key).ok().flatten()?;
    if stored.is_null() {
        return None;
    }
    if !is_marker(&stored) {
        let value = serde_json::from_value(stored.clone()).ok()?;
        migrate(store, key, &stored);
        return Some(value);
    }
    match backend::get(key) {
        Ok(Some(text)) => serde_json::from_str(&text).ok(),
        Ok(None) => None,
        Err(e) => {
            log_error!("anahtar deposundan okunamadı ({key}): {e}");
            None
        }
    }
}

/// Açık metin değeri anahtar deposuna taşır; taşınamazsa olduğu gibi bırakır.
fn migrate(store: &Store, key: &str, value: &Value) {
    if backend::set(key, &value.to_string()).is_ok() {
        match store.save_setting(key, &json!({ MARKER: true })) {
            Ok(()) => log_info!("{key} anahtar deposuna taşındı"),
            Err(e) => log_error!("{key} taşınırken veritabanına yazılamadı: {e}"),
        }
    }
}

/// Bütün değeri gizli olan ayarı yazar (anahtar deposu yoksa veritabanına).
pub fn save<T: Serialize>(store: &Store, key: &str, value: &T) -> Result<(), StoreError> {
    let text = serde_json::to_string(value).unwrap_or_default();
    match backend::set(key, &text) {
        Ok(()) => store.save_setting(key, &json!({ MARKER: true })),
        Err(e) => {
            log_error!("anahtar deposuna yazılamadı ({key}), veritabanında kalıyor: {e}");
            store.save_setting(key, value)
        }
    }
}

/// Ayarı ve depodaki değerini siler (veritabanında `null` kalır).
pub fn clear(store: &Store, key: &str) -> Result<(), StoreError> {
    if let Err(e) = backend::delete(key) {
        log_error!("anahtar deposundan silinemedi ({key}): {e}");
    }
    store.save_setting(key, &Value::Null)
}

/// Tek bir gizli değer (ör. eşitlenen bir ayarın yerel alanı). Boş değer siler.
pub fn get_value(name: &str) -> Option<String> {
    backend::get(name).unwrap_or_else(|e| {
        log_error!("anahtar deposundan okunamadı ({name}): {e}");
        None
    })
}

/// Değeri depoya yazar; yazılamazsa `false` (çağıran veritabanında tutar).
pub fn set_value(name: &str, value: &str) -> bool {
    let result = if value.is_empty() {
        backend::delete(name)
    } else {
        backend::set(name, value)
    };
    result
        .inspect_err(|e| log_error!("anahtar deposuna yazılamadı ({name}): {e}"))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::backend::SERIAL;
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Auth {
        token: String,
    }

    fn auth(t: &str) -> Auth {
        Auth { token: t.into() }
    }

    #[test]
    fn plaintext_moves_to_the_keychain() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = Store::open_in_memory().unwrap();
        store.save_setting("t_auth", &auth("eski")).unwrap();
        assert_eq!(load::<Auth>(&store, "t_auth"), Some(auth("eski")));
        let raw: Value = store.setting("t_auth").unwrap().unwrap();
        assert!(is_marker(&raw), "veritabanında açık metin kalmamalı: {raw}");
        assert_eq!(load::<Auth>(&store, "t_auth"), Some(auth("eski")));

        save(&store, "t_auth", &auth("yeni")).unwrap();
        assert_eq!(load::<Auth>(&store, "t_auth"), Some(auth("yeni")));
        clear(&store, "t_auth").unwrap();
        assert_eq!(load::<Auth>(&store, "t_auth"), None);
        assert_eq!(backend::get("t_auth").unwrap(), None);
    }

    #[test]
    fn a_forgotten_marker_hides_the_stored_value() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = Store::open_in_memory().unwrap();
        save(&store, "t_forget", &auth("x")).unwrap();
        // Yedekten dönüş: veritabanında `null`.
        store.save_setting("t_forget", &Value::Null).unwrap();
        assert_eq!(load::<Auth>(&store, "t_forget"), None);
    }

    #[test]
    fn without_a_keychain_values_stay_in_the_database() {
        let _s = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = Store::open_in_memory().unwrap();
        *backend::BROKEN.lock().unwrap() = true;
        save(&store, "t_broken", &auth("z")).unwrap();
        let loaded = load::<Auth>(&store, "t_broken");
        assert!(!set_value("t_broken_value", "k"));
        *backend::BROKEN.lock().unwrap() = false;
        assert_eq!(loaded, Some(auth("z")));
        let raw: Value = store.setting("t_broken").unwrap().unwrap();
        assert!(!is_marker(&raw));
    }
}
