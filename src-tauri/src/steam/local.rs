//! Detects the Steam client installed on this PC and the accounts it knows.
use super::vdf;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const STEAMID64_BASE: u64 = 76_561_197_960_265_728;

pub fn account_id_to_steam64(account_id: u32) -> u64 {
    STEAMID64_BASE + account_id as u64
}

pub fn steam64_to_account_id(steam_id: u64) -> u32 {
    steam_id.saturating_sub(STEAMID64_BASE) as u32
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalUser {
    pub steam_id: String,
    pub account_name: String,
    pub persona_name: String,
    pub most_recent: bool,
    /// Currently signed in to the running Steam client.
    pub active: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalSteam {
    pub install_path: Option<String>,
    pub running: bool,
    pub active_steam_id: Option<String>,
    pub users: Vec<LocalUser>,
}

#[cfg(windows)]
fn registry_info() -> (Option<PathBuf>, Option<u32>, bool) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut path: Option<PathBuf> = hkcu
        .open_subkey("Software\\Valve\\Steam")
        .ok()
        .and_then(|k| k.get_value::<String, _>("SteamPath").ok())
        .map(|p| PathBuf::from(p.replace('/', "\\")));
    if path.as_ref().map(|p| !p.exists()).unwrap_or(true) {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        path = hklm
            .open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
            .or_else(|_| hklm.open_subkey("SOFTWARE\\Valve\\Steam"))
            .ok()
            .and_then(|k| k.get_value::<String, _>("InstallPath").ok())
            .map(PathBuf::from);
    }
    let active = hkcu
        .open_subkey("Software\\Valve\\Steam\\ActiveProcess")
        .ok();
    let active_user = active
        .as_ref()
        .and_then(|k| k.get_value::<u32, _>("ActiveUser").ok())
        .filter(|&u| u != 0);
    let pid = active
        .as_ref()
        .and_then(|k| k.get_value::<u32, _>("pid").ok())
        .unwrap_or(0);
    (path, active_user, pid != 0)
}

#[cfg(not(windows))]
fn registry_info() -> (Option<PathBuf>, Option<u32>, bool) {
    let home = dirs::home_dir().unwrap_or_default();
    let candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![home.join("Library/Application Support/Steam")]
    } else {
        vec![
            home.join(".steam/steam"),
            home.join(".local/share/Steam"),
            home.join(".steam/root"),
            home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            home.join("snap/steam/common/.local/share/Steam"),
        ]
    };
    let path = candidates
        .into_iter()
        .find(|p| p.join("config").is_dir() || p.join("steamapps").is_dir())
        .map(|p| p.canonicalize().unwrap_or(p));

    // registry.vdf lives at ~/.steam/registry.vdf (Linux) or in the Steam folder (macOS).
    let mut reg_files = vec![home.join(".steam/registry.vdf")];
    if let Some(p) = &path {
        reg_files.push(p.join("registry.vdf"));
    }
    let mut active_user = None;
    let mut pid = 0u32;
    for f in reg_files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let Ok(doc) = vdf::parse(&text) else { continue };
        let ap = doc.path(&["Registry", "HKCU", "Software", "Valve", "Steam", "ActiveProcess"]);
        active_user = ap
            .and_then(|n| n.get("ActiveUser"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&u| u != 0);
        pid = ap
            .and_then(|n| n.get("pid"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        break;
    }
    // A stale pid can survive a crash; check the process really exists on Linux.
    let running = pid != 0 && (cfg!(target_os = "macos") || Path::new(&format!("/proc/{pid}")).exists());
    (path, active_user, running)
}

pub fn parse_login_users(text: &str, active_account: Option<u32>) -> Vec<LocalUser> {
    let Ok(doc) = vdf::parse(text) else { return vec![] };
    let Some(users) = doc.get("users") else { return vec![] };
    users
        .entries()
        .iter()
        .filter_map(|(id, node)| {
            let sid: u64 = id.parse().ok()?;
            let field = |k: &str| node.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            Some(LocalUser {
                steam_id: id.clone(),
                account_name: field("AccountName"),
                persona_name: field("PersonaName"),
                most_recent: field("MostRecent") == "1",
                active: active_account.map(|a| account_id_to_steam64(a) == sid).unwrap_or(false),
            })
        })
        .collect()
}

pub fn detect() -> LocalSteam {
    let (path, active_user, running) = registry_info();
    let users = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p.join("config").join("loginusers.vdf")).ok())
        .map(|t| parse_login_users(&t, if running { active_user } else { None }))
        .unwrap_or_default();
    LocalSteam {
        install_path: path.map(|p| p.to_string_lossy().into_owned()),
        running,
        active_steam_id: if running {
            active_user.map(|a| account_id_to_steam64(a).to_string())
        } else {
            None
        },
        users,
    }
}

/// Games known to the local Steam client: installed games (app manifests) plus
/// anything with recorded playtime in this user's localconfig.vdf. Used when an
/// account has no web session to fetch its full library.
pub fn local_games(steam_path: &Path, account_id: u32) -> Vec<super::webapi::OwnedGame> {
    use super::webapi::OwnedGame;
    use std::collections::BTreeMap;
    let mut games: BTreeMap<u32, OwnedGame> = BTreeMap::new();

    let mut libraries = vec![steam_path.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(steam_path.join("steamapps").join("libraryfolders.vdf")) {
        if let Ok(doc) = vdf::parse(&text) {
            if let Some(folders) = doc.get("libraryfolders") {
                for (_, f) in folders.entries() {
                    let p = f.get("path").and_then(|v| v.as_str()).or_else(|| f.as_str());
                    if let Some(p) = p {
                        let pb = PathBuf::from(p.replace("\\\\", "\\"));
                        if !libraries.contains(&pb) {
                            libraries.push(pb);
                        }
                    }
                }
            }
        }
    }
    for lib in libraries {
        let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else { continue };
            let Ok(doc) = vdf::parse(&text) else { continue };
            let Some(st) = doc.get("AppState") else { continue };
            let Some(appid) = st.get("appid").and_then(|v| v.as_str()).and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            // 228980 = Steamworks Common Redistributables; not a game.
            if appid == 228980 {
                continue;
            }
            games.insert(
                appid,
                OwnedGame {
                    appid,
                    name: st.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    ..Default::default()
                },
            );
        }
    }

    let cfg = steam_path
        .join("userdata")
        .join(account_id.to_string())
        .join("config")
        .join("localconfig.vdf");
    if let Ok(text) = std::fs::read_to_string(cfg) {
        if let Ok(doc) = vdf::parse(&text) {
            let apps = doc.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"]);
            for (id, node) in apps.map(|a| a.entries()).unwrap_or(&[]) {
                let Ok(appid) = id.parse::<u32>() else { continue };
                let minutes = node
                    .get("Playtime")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                let last = node
                    .get("LastPlayed")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                if minutes == 0 && last == 0 {
                    continue;
                }
                let g = games.entry(appid).or_insert_with(|| OwnedGame { appid, ..Default::default() });
                g.playtime_minutes = g.playtime_minutes.max(minutes);
                g.last_played = g.last_played.max(last);
            }
        }
    }
    games.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_games_from_files() {
        let dir = std::env::temp_dir().join(format!("idlekit-test-{}", std::process::id()));
        let apps = dir.join("steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(apps.join("libraryfolders.vdf"), "\"libraryfolders\" { \"0\" { \"path\" \"/nonexistent\" } }").unwrap();
        std::fs::write(apps.join("appmanifest_440.acf"), "\"AppState\" { \"appid\" \"440\" \"name\" \"Team Fortress 2\" }").unwrap();
        std::fs::write(apps.join("appmanifest_228980.acf"), "\"AppState\" { \"appid\" \"228980\" \"name\" \"Redist\" }").unwrap();
        let cfg = dir.join("userdata/5/config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(
            cfg.join("localconfig.vdf"),
            "\"UserLocalConfigStore\" { \"Software\" { \"Valve\" { \"Steam\" { \"apps\" { \"440\" { \"Playtime\" \"90\" } \"730\" { \"LastPlayed\" \"1700000000\" } \"10\" { } } } } } }",
        )
        .unwrap();
        let games = local_games(&dir, 5);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].appid, 440);
        assert_eq!(games[0].name, "Team Fortress 2");
        assert_eq!(games[0].playtime_minutes, 90);
        assert_eq!(games[1].appid, 730);
        assert_eq!(games[1].last_played, 1700000000);
    }

    #[test]
    fn steamid_conversion() {
        assert_eq!(account_id_to_steam64(1), 76561197960265729);
        assert_eq!(steam64_to_account_id(76561197960265729), 1);
    }

    #[test]
    fn login_users_marks_active() {
        let text = r#""users" { "76561197960265729" { "AccountName" "a" "PersonaName" "A" "MostRecent" "1" } "76561197960265730" { "AccountName" "b" } }"#;
        let users = parse_login_users(text, Some(2));
        assert_eq!(users.len(), 2);
        assert!(!users[0].active && users[1].active);
        assert!(users[0].most_recent);
        assert_eq!(users[1].account_name, "b");
    }
}
