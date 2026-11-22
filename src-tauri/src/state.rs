//! Shared application state and the cross-cutting operations features need:
//! config persistence, per-account web sessions, games lists, local-client checks.
use crate::config::{Account, Config};
use crate::features::{farming::FarmingManager, idler::Idler, unlocker::UnlockerManager};
use crate::steam::community::WebSession;
use crate::steam::webapi::{self, OwnedGame};
use crate::steam::{art::ArtResolver, auth::AuthSessions, cm::CmManager, helper::HelperManager, jwt, local, pictures::Pictures};
use crate::{logbus, secrets};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameEntry {
    pub appid: u32,
    pub name: String,
    pub playtime_minutes: u32,
    pub icon_url: String,
    pub has_stats: bool,
    pub last_played: u32,
    pub manual: bool,
}

#[derive(Clone)]
pub struct App(Arc<Inner>);

impl Deref for App {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

pub struct Inner {
    pub data_dir: PathBuf,
    pub config: Mutex<Config>,
    pub http: reqwest::Client,
    pub helpers: Arc<HelperManager>,
    pub cm: Arc<CmManager>,
    pub auth: Arc<AuthSessions>,
    pub idler: Idler,
    pub farming: FarmingManager,
    pub unlocker: UnlockerManager,
    /// Icon, banner and cover lookups, cached on disk.
    pub art: ArtResolver,
    /// Covers, banners, icons and avatars kept on disk, fetched a few at a time.
    pub pictures: Pictures,
    /// What is known about newer versions of IdleKit.
    pub updates: crate::updates::Updates,
    web: tokio::sync::Mutex<HashMap<String, Arc<WebSession>>>,
    games: Mutex<HashMap<String, Vec<GameEntry>>>,
    /// appids with an open Achievement Manager view (keeps their helper alive)
    pub ach_open: Mutex<HashSet<u32>>,
    pub market_cancel: AtomicBool,
}

fn hide_account_names(cfg: &Config) {
    for account in &cfg.accounts {
        logbus::hide(&account.account_name, &logbus::label(&account.steam_id));
    }
}

impl App {
    pub fn new(data_dir: PathBuf, pictures_dir: PathBuf) -> Self {
        let config = Config::load(&data_dir);
        let art = ArtResolver::new(&data_dir);
        Self(Arc::new(Inner {
            data_dir,
            art,
            pictures: Pictures::new(pictures_dir),
            updates: Default::default(),
            config: Mutex::new(config),
            http: webapi::http_client(),
            helpers: Arc::new(HelperManager::default()),
            cm: Arc::new(CmManager::default()),
            auth: Arc::new(AuthSessions::default()),
            idler: Idler::default(),
            farming: FarmingManager::default(),
            unlocker: UnlockerManager::default(),
            web: tokio::sync::Mutex::new(HashMap::new()),
            games: Mutex::new(HashMap::new()),
            ach_open: Mutex::new(HashSet::new()),
            market_cancel: AtomicBool::new(false),
        }))
    }

    // ------------------------------------------------------------ config --

    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    pub fn settings(&self) -> crate::config::Settings {
        self.config.lock().unwrap().settings.clone()
    }

    /// Reads from the settings without copying all of them: every account,
    /// every list and every remembered market item. Keep `f` short, since
    /// nothing else can read or change the settings while it runs.
    pub fn with_config<R>(&self, f: impl FnOnce(&Config) -> R) -> R {
        f(&self.config.lock().unwrap())
    }

    /// Keeps every account's sign-in name out of the log: accounts are named
    /// there by the end of their Steam ID.
    pub fn hide_names(&self) {
        hide_account_names(&self.config.lock().unwrap());
    }

    /// Mutate the config and persist it.
    pub fn update_config<R>(&self, f: impl FnOnce(&mut Config) -> R) -> R {
        let mut cfg = self.config.lock().unwrap();
        let r = f(&mut cfg);
        hide_account_names(&cfg);
        if let Err(e) = cfg.save(&self.data_dir) {
            logbus::error("config", format!("could not save settings: {e:#}"));
        }
        r
    }

    pub fn account(&self, steam_id: &str) -> Result<Account> {
        self.config
            .lock()
            .unwrap()
            .account(steam_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown account {steam_id}"))
    }

    pub fn emit_accounts(&self) {
        logbus::emit("state://accounts", self.with_config(|c| c.accounts.clone()));
    }

    // ------------------------------------------------------ local client --

    pub fn local_steam(&self) -> local::LocalSteam {
        local::detect()
    }

    pub fn steam_path(&self) -> Result<PathBuf> {
        local::detect()
            .install_path
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("Steam is not installed on this PC (or IdleKit could not find it)"))
    }

    /// The local Steam client must be running and signed in to this account.
    pub fn require_local_client(&self, steam_id: &str) -> Result<()> {
        let ls = local::detect();
        if ls.install_path.is_none() {
            bail!("Steam is not installed on this PC");
        }
        if !ls.running {
            bail!("Start the Steam client and sign in to use this feature");
        }
        match ls.active_steam_id {
            Some(active) if active == steam_id => Ok(()),
            Some(_) => bail!("The Steam client on this PC is signed in to a different account"),
            None => bail!("The Steam client is running but not signed in"),
        }
    }

    // ------------------------------------------------------- web session --

    pub async fn seed_web_session(&self, steam_id: &str, access_token: &str) {
        if let Ok(ws) = WebSession::from_access_token(steam_id, access_token) {
            self.web.lock().await.insert(steam_id.to_string(), Arc::new(ws));
        }
    }

    pub async fn forget_web_session(&self, steam_id: &str) {
        self.web.lock().await.remove(steam_id);
    }

    /// A signed-in community/store session for this account, refreshing the
    /// access token from the saved refresh token when it is close to expiry.
    pub async fn web_session(&self, steam_id: &str) -> Result<Arc<WebSession>> {
        let mut cache = self.web.lock().await;
        if let Some(ws) = cache.get(steam_id) {
            if ws.access_token.is_empty() || !jwt::expires_soon(&ws.access_token, 600) {
                return Ok(ws.clone());
            }
        }
        let ws = if let Some(refresh) = secrets::get(&secrets::refresh_token_key(steam_id)) {
            let sid: u64 = steam_id.parse()?;
            let token = if self.cm.is_online(steam_id) {
                match self.cm.access_token(steam_id, &refresh).await {
                    Ok(t) => t,
                    Err(_) => webapi::generate_access_token(&self.http, &refresh, sid).await?,
                }
            } else {
                match webapi::generate_access_token(&self.http, &refresh, sid).await {
                    Ok(t) => t,
                    Err(web_err) => {
                        // Fall back to a short CM session, which can always mint one.
                        logbus::debug("web", format!("HTTPS token refresh failed ({web_err:#}); trying a direct connection"));
                        let acct = self.account(steam_id)?;
                        let mode = acct.mode.clone();
                        self.cm.configure(
                            steam_id,
                            &acct.account_name,
                            acct.persona_state,
                            &self.settings().idle_status_text,
                        );
                        self.cm.connect(steam_id).await.map_err(|e| {
                            anyhow!("Could not refresh the web session ({web_err:#}; {e:#}). Sign in to this account again.")
                        })?;
                        let token = self.cm.access_token(steam_id, &refresh).await;
                        if mode != "direct" {
                            self.cm.disconnect(steam_id).await;
                        }
                        token?
                    }
                }
            };
            WebSession::from_access_token(steam_id, &token)?
        } else if let Some(cookie) = secrets::get(&secrets::cookie_key(steam_id)) {
            let ws = WebSession::from_cookie(&cookie)?;
            if !ws.access_token.is_empty() && jwt::expires_soon(&ws.access_token, 0) {
                bail!("The saved steamLoginSecure cookie has expired — paste a fresh one in Accounts");
            }
            ws
        } else {
            bail!("This account has no web session. In Accounts, sign in with QR/password or paste a steamLoginSecure cookie.");
        };
        let ws = Arc::new(ws);
        cache.insert(steam_id.to_string(), ws.clone());
        Ok(ws)
    }

    // ------------------------------------------------------------- games --

    fn games_file(&self, steam_id: &str) -> PathBuf {
        self.data_dir.join(format!("games_{steam_id}.json"))
    }

    fn to_entries(owned: Vec<OwnedGame>, account: &Account) -> Vec<GameEntry> {
        let mut map: HashMap<u32, GameEntry> = owned
            .into_iter()
            .map(|g| {
                let icon_url = if g.icon_hash.is_empty() {
                    String::new()
                } else {
                    format!(
                        "https://media.steampowered.com/steamcommunity/public/images/apps/{}/{}.jpg",
                        g.appid, g.icon_hash
                    )
                };
                (
                    g.appid,
                    GameEntry {
                        appid: g.appid,
                        name: if g.name.is_empty() { format!("App {}", g.appid) } else { g.name },
                        playtime_minutes: g.playtime_minutes,
                        icon_url,
                        has_stats: g.has_stats,
                        last_played: g.last_played,
                        manual: false,
                    },
                )
            })
            .collect();
        for m in &account.manual_games {
            map.entry(m.appid).or_insert_with(|| GameEntry {
                appid: m.appid,
                name: m.name.clone(),
                manual: true,
                ..Default::default()
            });
        }
        let mut list: Vec<GameEntry> = map.into_values().collect();
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        list
    }

    pub fn cached_games(&self, steam_id: &str) -> Option<Vec<GameEntry>> {
        if let Some(g) = self.games.lock().unwrap().get(steam_id) {
            return Some(g.clone());
        }
        let text = std::fs::read_to_string(self.games_file(steam_id)).ok()?;
        let owned: Vec<OwnedGame> = serde_json::from_str(&text).ok()?;
        let account = self.account(steam_id).ok()?;
        let entries = Self::to_entries(owned, &account);
        self.games.lock().unwrap().insert(steam_id.to_string(), entries.clone());
        Some(entries)
    }

    pub fn game_name(&self, steam_id: &str, appid: u32) -> String {
        self.games
            .lock()
            .unwrap()
            .get(steam_id)
            .and_then(|l| l.iter().find(|g| g.appid == appid).map(|g| g.name.clone()))
            .unwrap_or_else(|| format!("App {appid}"))
    }

    /// Fetch the owned-games list from the best available source.
    pub async fn load_games(&self, steam_id: &str, force: bool) -> Result<Vec<GameEntry>> {
        if !force {
            if let Some(g) = self.cached_games(steam_id) {
                return Ok(g);
            }
        }
        let account = self.account(steam_id)?;
        let mut errors = Vec::new();
        let mut owned: Option<Vec<OwnedGame>> = None;

        if self.cm.is_online(steam_id) {
            match self.cm.owned_games(steam_id).await {
                Ok(g) => owned = Some(g),
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        if owned.is_none() && secrets::get(&secrets::refresh_token_key(steam_id)).is_some() {
            match self.web_session(steam_id).await {
                Ok(ws) => match webapi::owned_games_web(&self.http, &ws.access_token, steam_id).await {
                    Ok(g) if !g.is_empty() => owned = Some(g),
                    Ok(_) => errors.push("Steam returned an empty library".into()),
                    Err(e) => errors.push(format!("{e:#}")),
                },
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        if owned.is_none() && secrets::get(&secrets::cookie_key(steam_id)).is_some() {
            match self.web_session(steam_id).await {
                Ok(ws) => match ws.games_from_profile().await {
                    Ok(g) => owned = Some(g),
                    Err(e) => errors.push(format!("{e:#}")),
                },
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        if owned.is_none() {
            if let Ok(path) = self.steam_path() {
                let sid: u64 = steam_id.parse().unwrap_or(0);
                let g = local::local_games(&path, local::steam64_to_account_id(sid));
                if !g.is_empty() {
                    if !errors.is_empty() {
                        logbus::warn("library", format!("Using installed/played games only: {}", errors.join("; ")));
                    }
                    owned = Some(g);
                }
            }
        }
        let owned = match owned {
            Some(o) => o,
            None if !account.manual_games.is_empty() => vec![],
            None => {
                if errors.is_empty() {
                    bail!("No game list available. Connect a web session in Accounts or add games by AppID.");
                }
                bail!("Could not load your games: {}", errors.join("; "));
            }
        };
        if let Ok(text) = serde_json::to_string(&owned) {
            let _ = std::fs::write(self.games_file(steam_id), text);
        }
        let entries = Self::to_entries(owned, &account);
        self.games.lock().unwrap().insert(steam_id.to_string(), entries.clone());
        Ok(entries)
    }

    /// Re-merge manual games after the user edits that list.
    pub fn refresh_manual(&self, steam_id: &str) {
        let Ok(account) = self.account(steam_id) else { return };
        let owned: Vec<OwnedGame> = std::fs::read_to_string(self.games_file(steam_id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let entries = Self::to_entries(owned, &account);
        self.games.lock().unwrap().insert(steam_id.to_string(), entries);
    }

    /// Steam keeps the avatar of each account that has signed in on this PC.
    /// Taking it from there shows a profile picture with no network at all.
    pub fn adopt_local_avatar(&self, steam_id: &str) {
        if let Ok(steam) = self.steam_path() {
            let source = steam.join("config").join("avatarcache").join(format!("{steam_id}.png"));
            self.pictures.adopt_local("avatar", steam_id, &source);
        }
    }

    /// Persona name + avatar from the public profile XML (best effort).
    pub async fn fetch_profile(&self, steam_id: &str) {
        let url = format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1");
        let Ok(resp) = self.http.get(&url).send().await else { return };
        let Ok(text) = resp.text().await else { return };
        // One pattern for the tags wanted, built once.
        static TAGS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let tags = TAGS.get_or_init(|| {
            regex::Regex::new(r"<(steamID|avatarFull|avatarMedium)>\s*(?:<!\[CDATA\[)?(.*?)(?:\]\]>)?\s*</(?:steamID|avatarFull|avatarMedium)>").expect("valid regex")
        });
        let grab = |tag: &str| -> Option<String> {
            tags.captures_iter(&text).find(|c| &c[1] == tag).map(|c| c[2].trim().to_string()).filter(|s| !s.is_empty())
        };
        let name = grab("steamID");
        let avatar = grab("avatarFull").or_else(|| grab("avatarMedium"));
        if name.is_none() && avatar.is_none() {
            return;
        }
        self.update_config(|c| {
            if let Some(a) = c.account_mut(steam_id) {
                if let Some(n) = name {
                    a.persona_name = n;
                }
                if let Some(av) = avatar {
                    a.avatar_url = av;
                }
            }
        });
        self.emit_accounts();
    }
}
