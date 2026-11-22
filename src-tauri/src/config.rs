//! Persisted app configuration: settings, accounts and per-account game lists.
//! Secrets (refresh tokens, cookies) are NOT stored here — see `secrets.rs`.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct FarmingSettings {
    /// Phase 1: games below this many hours are idled together until they reach it
    /// (Steam often holds back drops until a game has ~2h on record). 0 disables phase 1.
    pub threshold_hours: f64,
    /// Max games idled at once in phase 1 (Steam caps at 32).
    pub max_games: u32,
    /// Games idled at once in phase 2 (one-at-a-time is the classic fast-drop setup).
    pub solo_games: u32,
    /// Minutes between drop checks.
    pub check_minutes: u32,
    /// Briefly stop and restart the games at each check (helps trigger pending drops).
    pub restart_on_check: bool,
    /// Skip games bought in the last 14 days with < 2h played (still refundable).
    pub skip_refundable: bool,
    /// "mostDrops" | "leastDrops" | "name" | "playtime"
    pub order: String,
}

impl Default for FarmingSettings {
    fn default() -> Self {
        Self {
            threshold_hours: 2.0,
            max_games: 32,
            solo_games: 1,
            check_minutes: 10,
            restart_on_check: true,
            skip_refundable: true,
            order: "mostDrops".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct UnlockerSettings {
    pub min_delay_minutes: u32,
    pub max_delay_minutes: u32,
    pub schedule_enabled: bool,
    /// "HH:MM" local time window in which unlocking is allowed.
    pub schedule_start: String,
    pub schedule_end: String,
    /// "common" (highest global % first) | "rare" | "schema"
    pub order: String,
    pub idle_while_unlocking: bool,
    /// How many games run their unlock queues at the same time (max 32).
    pub parallel_games: u32,
    /// Multiplier applied to imported unlock gaps (1.0 = same pace as the other player).
    pub imported_time_scale: f64,
}

impl Default for UnlockerSettings {
    fn default() -> Self {
        Self {
            min_delay_minutes: 15,
            max_delay_minutes: 75,
            schedule_enabled: false,
            schedule_start: "18:00".into(),
            schedule_end: "23:30".into(),
            order: "common".into(),
            idle_while_unlocking: true,
            parallel_games: 1,
            imported_time_scale: 1.0,
        }
    }
}

/// How pages were left, so they come back the same after a restart. The window
/// decides what each may be and falls back to its default for anything else.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewSettings {
    /// Library: which games are shown.
    pub library_filter: String,
    /// Library: what the games are sorted by.
    pub library_sort: String,
    /// Market: "inventory" | "listings".
    pub market_tab: String,
    /// Market inventory: which kind of item is shown.
    pub market_filter: String,
    /// Logs: which lines are shown.
    pub logs_level: String,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            library_filter: "all".into(),
            library_sort: "name".into(),
            market_tab: "inventory".into(),
            market_filter: "all".into(),
            logs_level: "all".into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct MarketSettings {
    /// "undercutLowest" | "matchLowest" | "aboveHighestBuy"
    pub strategy: String,
    pub undercut_cents: u32,
    /// Buyer-pays price limits in cents; 0 = no limit.
    pub min_price_cents: u32,
    pub max_price_cents: u32,
    /// Pause between market requests to stay under Steam's rate limits.
    pub delay_ms: u64,
}

impl Default for MarketSettings {
    fn default() -> Self {
        Self {
            strategy: "undercutLowest".into(),
            undercut_cents: 1,
            min_price_cents: 3,
            max_price_cents: 0,
            delay_ms: 1500,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// UI theme id: "graphite" | "midnight" | "daylight".
    pub theme: String,
    pub accent: String,
    /// Library cards: "cover" (portrait art) | "banner" (wide art).
    pub card_style: String,
    /// Leave games Steam labels as adult-only or as frequent nudity or sexual
    /// content out of every list in the window. They can still idle and farm.
    pub hide_nsfw: bool,
    pub language: String,
    pub max_idle_games: u32,
    /// Shown as a "non-Steam game" while idling over a direct connection (empty = off).
    pub idle_status_text: String,
    pub auto_idle_on_launch: bool,
    pub farm_on_launch: bool,
    /// What to start when card farming finishes: "none" | "autoIdle" | "unlocker"
    pub after_farming: String,
    pub connect_direct_on_launch: bool,
    /// What the window's close button does: "ask" (the first time) | "tray" | "quit".
    pub close_action: String,
    /// When IdleKit is started at sign-in, stay in the tray instead of opening its window.
    pub start_in_tray: bool,
    /// Look for a newer version each time IdleKit opens.
    pub check_updates: bool,
    pub farming: FarmingSettings,
    pub unlocker: UnlockerSettings,
    pub market: MarketSettings,
    pub views: ViewSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "graphite".into(),
            accent: "amber".into(),
            card_style: "cover".into(),
            hide_nsfw: false,
            language: "english".into(),
            max_idle_games: 32,
            idle_status_text: String::new(),
            auto_idle_on_launch: false,
            farm_on_launch: false,
            after_farming: "none".into(),
            connect_direct_on_launch: true,
            close_action: "ask".into(),
            start_in_tray: true,
            check_updates: true,
            farming: FarmingSettings::default(),
            unlocker: UnlockerSettings::default(),
            market: MarketSettings::default(),
            views: ViewSettings::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManualGame {
    pub appid: u32,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportedTiming {
    pub api_name: String,
    /// Seconds after the first imported unlock.
    pub offset_secs: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Account {
    pub steam_id: String,
    pub account_name: String,
    pub persona_name: String,
    pub avatar_url: String,
    /// "local" = idles through the Steam client on this PC.
    /// "direct" = logs in to Steam's servers itself (no client needed).
    pub mode: String,
    /// 1 online, 3 away, 7 invisible, 0 offline (direct mode only)
    pub persona_state: u32,
    pub favorites: Vec<u32>,
    pub manual_games: Vec<ManualGame>,
    pub auto_idle: Vec<u32>,
    pub farm_blacklist: Vec<u32>,
    pub farm_whitelist: Vec<u32>,
    pub unlock_queue: Vec<u32>,
    /// appid -> achievement API names to never auto-unlock
    pub unlock_skip: HashMap<u32, Vec<String>>,
    /// appid -> custom unlock order (API names)
    pub unlock_order: HashMap<u32, Vec<String>>,
    /// appid -> unlock timings imported from another player's profile
    pub unlock_imported: HashMap<u32, Vec<ImportedTiming>>,
    /// appid -> playtime booster target, in minutes of idling this session
    pub booster_targets: HashMap<u32, u32>,
    pub added_at: i64,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            steam_id: String::new(),
            account_name: String::new(),
            persona_name: String::new(),
            avatar_url: String::new(),
            mode: "local".into(),
            persona_state: 1,
            favorites: vec![],
            manual_games: vec![],
            auto_idle: vec![],
            farm_blacklist: vec![],
            farm_whitelist: vec![],
            unlock_queue: vec![],
            unlock_skip: HashMap::new(),
            unlock_order: HashMap::new(),
            unlock_imported: HashMap::new(),
            booster_targets: HashMap::new(),
            added_at: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub version: u32,
    pub settings: Settings,
    pub accounts: Vec<Account>,
    pub active_account: Option<String>,
    /// market_hash_name -> item_nameid (these never change, so cache them)
    pub item_name_ids: HashMap<String, u64>,
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    pub fn load(dir: &Path) -> Config {
        let path = Self::path(dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::logbus::warn("config", format!("config.json unreadable ({e}); starting fresh"));
                let _ = std::fs::copy(&path, dir.join("config.broken.json"));
                Config::default()
            }),
            Err(_) => Config { version: 1, ..Default::default() },
        }
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join("config.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, Self::path(dir))?;
        Ok(())
    }

    pub fn account(&self, steam_id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.steam_id == steam_id)
    }

    pub fn account_mut(&mut self, steam_id: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.steam_id == steam_id)
    }
}

/// What differs between two sets of settings, one line per setting, for the
/// log: "farming.checkMinutes: 10 → 15". Free text the user typed (the custom
/// status line) is only reported as changed, not quoted.
pub fn setting_changes(before: &Settings, after: &Settings) -> Vec<String> {
    fn walk(path: &str, a: &serde_json::Value, b: &serde_json::Value, out: &mut Vec<String>) {
        use serde_json::Value;
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
                keys.sort();
                keys.dedup();
                for key in keys {
                    let next = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                    walk(&next, x.get(key).unwrap_or(&Value::Null), y.get(key).unwrap_or(&Value::Null), out);
                }
            }
            _ if a != b => {
                if path.ends_with("Text") {
                    out.push(format!("{path}: changed"));
                    return;
                }
                let show = |v: &Value| {
                    let text = v.to_string();
                    if text.chars().count() > 80 {
                        text.chars().take(80).collect::<String>() + "…"
                    } else {
                        text
                    }
                };
                out.push(format!("{path}: {} → {}", show(a), show(b)));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let (Ok(a), Ok(b)) = (serde_json::to_value(before), serde_json::to_value(after)) {
        walk("", &a, &b, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_changes_name_each_setting_that_moved() {
        let before = Settings::default();
        assert!(setting_changes(&before, &before).is_empty());
        let mut after = before.clone();
        after.close_action = "tray".into();
        after.farming.check_minutes += 5;
        after.idle_status_text = "something private".into();
        let changes = setting_changes(&before, &after);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert!(changes.contains(&format!("closeAction: \"{}\" → \"tray\"", before.close_action)));
        assert!(changes.contains(&format!("farming.checkMinutes: {} → {}", before.farming.check_minutes, after.farming.check_minutes)));
        assert!(changes.contains(&"idleStatusText: changed".to_string()));
        assert!(!changes.iter().any(|c| c.contains("private")));
    }

    #[test]
    fn how_pages_were_left_is_saved_with_the_settings() {
        // settings saved before this existed: every page starts as it always did
        let old: Settings = serde_json::from_str(r#"{"theme":"midnight","hideNsfw":true}"#).unwrap();
        assert_eq!(
            (old.views.library_filter.as_str(), old.views.library_sort.as_str(), old.views.market_tab.as_str()),
            ("all", "name", "inventory")
        );
        assert_eq!((old.views.market_filter.as_str(), old.views.logs_level.as_str()), ("all", "all"));
        assert_eq!(old.theme, "midnight");

        // some choices saved, the rest keep their defaults
        let saved: Settings = serde_json::from_str(r#"{"views":{"librarySort":"playtime","logsLevel":"problems"}}"#).unwrap();
        assert_eq!((saved.views.library_sort.as_str(), saved.views.logs_level.as_str()), ("playtime", "problems"));
        assert_eq!(saved.views.library_filter, "all");

        // written back under the same names, and a change is named in the log
        let json = serde_json::to_value(&saved).unwrap();
        assert_eq!(json["views"]["librarySort"], "playtime");
        assert_eq!(json["views"]["marketTab"], "inventory");
        let changes = setting_changes(&Settings::default(), &saved);
        assert!(changes.contains(&"views.librarySort: \"name\" → \"playtime\"".to_string()), "{changes:?}");
    }
}
