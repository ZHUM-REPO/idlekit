//! Storage for refresh tokens, pasted cookies and Steam Guard machine data.
//!
//! Windows/macOS: the OS credential store (Credential Manager / Keychain).
//! Linux: a JSON file in the app data folder with owner-only permissions.
use std::path::PathBuf;
use std::sync::OnceLock;

static DIR: OnceLock<PathBuf> = OnceLock::new();
#[cfg(any(windows, target_os = "macos"))]
const SERVICE: &str = "IdleKit";

pub fn init(dir: PathBuf) {
    let _ = DIR.set(dir);
}

pub fn refresh_token_key(steam_id: &str) -> String {
    format!("refresh:{steam_id}")
}

pub fn cookie_key(steam_id: &str) -> String {
    format!("cookie:{steam_id}")
}

/// The web sign-in (an access token, good for about a day) last made for
/// an account. Kept so that a restart or an update needs nothing from Steam
/// while it is good, instead of asking for a new one at every start.
pub fn web_token_key(steam_id: &str) -> String {
    format!("web:{steam_id}")
}

pub fn guard_key(account_name: &str) -> String {
    format!("guard:{}", account_name.to_lowercase())
}

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use super::SERVICE;

    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        keyring::Entry::new(SERVICE, key)?.set_password(value)?;
        Ok(())
    }

    pub fn get(key: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, key).ok()?.get_password().ok()
    }

    pub fn delete(key: &str) {
        if let Ok(e) = keyring::Entry::new(SERVICE, key) {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::collections::HashMap;
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    fn file() -> std::path::PathBuf {
        super::DIR
            .get()
            .cloned()
            .unwrap_or_else(std::env::temp_dir)
            .join("secrets.json")
    }

    fn read() -> HashMap<String, String> {
        std::fs::read_to_string(file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn write(map: &HashMap<String, String>) -> anyhow::Result<()> {
        let path = file();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(map)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = read();
        map.insert(key.to_string(), value.to_string());
        write(&map)
    }

    pub fn get(key: &str) -> Option<String> {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        read().remove(key)
    }

    pub fn delete(key: &str) {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = read();
        if map.remove(key).is_some() {
            let _ = write(&map);
        }
    }
}

pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
    imp::set(key, value)
}

pub fn get(key: &str) -> Option<String> {
    imp::get(key).filter(|v| !v.is_empty())
}

pub fn delete(key: &str) {
    imp::delete(key)
}
