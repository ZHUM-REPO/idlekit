//! Shared application state and the cross-cutting operations features need:
//! config persistence, per-account web sessions, games lists, local-client checks.
use crate::config::{Account, Config};
use crate::features::{farming::FarmingManager, idler::Idler, unlocker::UnlockerManager};
use crate::net::Logged;
use crate::steam::cm::Refusal;
use crate::steam::community::{self, WebSession};
use crate::steam::events::{self, EventList};
use crate::steam::market::InvItem;
use crate::steam::pricing::{self, PriceList};
use crate::steam::webapi::{self, OwnedGame};
use crate::steam::{art::ArtResolver, auth::AuthSessions, cm::CmManager, helper::HelperManager, jwt, local, pictures::Pictures};
use crate::{logbus, secrets};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A web session this close to running out is replaced before it is used.
const WEB_MARGIN_SECS: i64 = 600;
/// How long making a new web session may take, every step together.
const WEB_RENEW_LIMIT: Duration = Duration::from_secs(75);
const REFUSED_EARLIER: &str =
    "Steam refused this account's saved sign-in earlier. Sign in to this account again on the Accounts page.";

/// After a failed try at a new web session, how long the next one waits:
/// trouble reaching Steam, and Steam limiting sign-ins.
const WEB_RETRY_AFTER_TROUBLE: Duration = Duration::from_secs(30);
const WEB_RETRY_AFTER_LIMIT: Duration = Duration::from_secs(10 * 60);

/// A failed try at a new web session (see `App::web_failures`).
struct WebFailure {
    at: Instant,
    print: u64,
    wait: Duration,
    message: String,
}

/// Tells saved sign-ins apart without keeping one: a hash that only lives
/// as long as this run.
fn fingerprint(secret: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    secret.hash(&mut h);
    h.finish()
}

/// How long a web sign-in is still good, in words: "good for 23 h more".
fn good_for(token: &str) -> String {
    let left = jwt::decode(token).map(|c| c.exp - chrono::Utc::now().timestamp()).unwrap_or(0);
    match left {
        i64::MIN..=0 => "already run out".into(),
        1..=5399 => format!("good for {} min more", (left + 30) / 60),
        _ => format!("good for {} h more", (left + 1800) / 3600),
    }
}

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

/// The shortest gap between two lookups of an account's unnamed games.
const ASK_FOR_NAMES_EVERY: Duration = Duration::from_secs(60);

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
    /// Web sessions by account. Only held to look one up or put one in: a
    /// refresh, which goes over the network, never holds it.
    web: Mutex<HashMap<String, Arc<WebSession>>>,
    /// One refresh at a time per account, so a slow one holds up no other account.
    web_refresh: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Saved sign-ins Steam refused during this run, by account (a
    /// fingerprint, never the sign-in itself). Asking again with the same
    /// one changes nothing and counts against the account at Steam; a new
    /// sign-in has a new fingerprint and is tried.
    refused_sign_ins: Mutex<HashMap<String, u64>>,
    /// The last failed try at a new web session, by account: when, for which
    /// saved sign-in (a fingerprint), how long to leave Steam alone, and why.
    /// Pages that ask meanwhile get the same answer instead of a new try each.
    web_failures: Mutex<HashMap<String, WebFailure>>,
    games: Mutex<HashMap<String, Vec<GameEntry>>>,
    /// What Steam's store calls games, by app id: for the ones that the files
    /// on this PC list by number only.
    store_names: Mutex<HashMap<u32, String>>,
    /// Accounts whose unnamed games are being asked about, and since when.
    naming: Mutex<HashMap<String, Instant>>,
    /// appids with an open Achievement Manager view (keeps their helper alive)
    pub ach_open: Mutex<HashSet<u32>>,
    /// Each game's global unlock rates, and when they were fetched.
    ach_percents: Mutex<HashMap<u32, (Instant, Arc<HashMap<String, f64>>)>>,
    pub market_cancel: AtomicBool,
    /// How many market jobs (pricing, listing, removing) are under way.
    pub market_busy: AtomicUsize,
    /// Which request for prices is the latest: one under way stops when it
    /// sees a newer one, or a listing job, start.
    pub price_runs: AtomicU64,
    /// The pace of price checks. The run that is asking Steam holds it; the
    /// next one waits its turn.
    pub price_pacer: tokio::sync::Mutex<pricing::Pacer>,
    /// The market's prices as last seen, read from disk when first needed.
    prices: Mutex<Option<PriceList>>,
    /// What each inventory item is (a card, a foil card, an emoticon…), by
    /// its market name, from the inventories loaded while IdleKit runs: the
    /// market's search is asked by kind.
    item_classes: Mutex<HashMap<String, String>>,
    /// The market's search isn't asked for prices until then, and why: its
    /// answers were in another currency than the wallet's, or not answers.
    searches_off: Mutex<Option<(Instant, String)>>,
    /// The items Steam said it has no market for, and when (milliseconds
    /// since 1970): they aren't asked about again within the hour.
    no_market: Mutex<HashMap<String, i64>>,
}

fn hide_account_names(cfg: &Config) {
    for account in &cfg.accounts {
        logbus::hide(&account.account_name, &logbus::label(&account.steam_id));
    }
}

/// Held while a market job runs, so an update isn't installed in the middle of one.
pub struct MarketJob(App);

impl Drop for MarketJob {
    fn drop(&mut self) {
        self.0.market_busy.fetch_sub(1, Ordering::SeqCst);
    }
}

impl App {
    pub fn market_job(&self) -> MarketJob {
        self.market_busy.fetch_add(1, Ordering::SeqCst);
        MarketJob(self.clone())
    }

    pub fn new(data_dir: PathBuf, pictures_dir: PathBuf) -> Self {
        let config = Config::load(&data_dir);
        let art = ArtResolver::new(&data_dir);
        let cm = CmManager::new(Some(data_dir.join(crate::steam::cm::SERVERS_FILE)), webapi::http_client());
        Self(Arc::new(Inner {
            data_dir,
            art,
            pictures: Pictures::new(pictures_dir),
            updates: Default::default(),
            config: Mutex::new(config),
            http: webapi::http_client(),
            helpers: Arc::new(HelperManager::default()),
            cm: Arc::new(cm),
            auth: Arc::new(AuthSessions::default()),
            idler: Idler::default(),
            farming: FarmingManager::default(),
            unlocker: UnlockerManager::default(),
            web: Mutex::new(HashMap::new()),
            web_refresh: Mutex::new(HashMap::new()),
            refused_sign_ins: Mutex::new(HashMap::new()),
            web_failures: Mutex::new(HashMap::new()),
            games: Mutex::new(HashMap::new()),
            store_names: Mutex::new(HashMap::new()),
            naming: Mutex::new(HashMap::new()),
            ach_open: Mutex::new(HashSet::new()),
            ach_percents: Mutex::new(HashMap::new()),
            market_cancel: AtomicBool::new(false),
            market_busy: AtomicUsize::new(0),
            price_runs: AtomicU64::new(0),
            price_pacer: tokio::sync::Mutex::new(pricing::Pacer::default()),
            prices: Mutex::new(None),
            item_classes: Mutex::new(HashMap::new()),
            searches_off: Mutex::new(None),
            no_market: Mutex::new(HashMap::new()),
        }))
    }

    // ------------------------------------------------------- market prices --

    /// The prices kept from earlier checks, read from disk the first time.
    pub fn with_prices<R>(&self, f: impl FnOnce(&mut PriceList) -> R) -> R {
        let mut kept = self.prices.lock().unwrap();
        let list = kept.get_or_insert_with(|| {
            PriceList::load(&self.data_dir.join(pricing::FILE), chrono::Utc::now().timestamp_millis())
        });
        f(list)
    }

    /// Writes the kept prices to disk.
    pub fn save_prices(&self) {
        let bytes = self.with_prices(|list| list.to_bytes());
        if let Err(e) = PriceList::save(&bytes, &self.data_dir.join(pricing::FILE)) {
            logbus::warn("market", format!("Couldn't keep the prices on disk: {e:#}"));
        }
    }

    /// Keeps what each of an inventory's items is, for the market's search.
    pub fn remember_classes(&self, items: &[InvItem]) {
        let mut classes = self.item_classes.lock().unwrap();
        for item in items {
            classes.insert(item.market_hash_name.clone(), item.class.clone());
        }
    }

    /// What each of these items is, where an inventory said.
    pub fn classes_of(&self, names: &[String]) -> Vec<Option<String>> {
        let classes = self.item_classes.lock().unwrap();
        names.iter().map(|name| classes.get(name).cloned()).collect()
    }

    /// Why the market's search isn't asked for prices for now, when it isn't.
    pub fn searches_off(&self) -> Option<String> {
        let mut off = self.searches_off.lock().unwrap();
        match &*off {
            Some((until, why)) if Instant::now() < *until => Some(why.clone()),
            _ => {
                *off = None;
                None
            }
        }
    }

    /// When Steam said it has no market for this item, if that was within the hour.
    pub fn no_market_since(&self, name: &str, now: i64) -> Option<i64> {
        let at = *self.no_market.lock().unwrap().get(name)?;
        (now - at < pricing::FRESH_FOR_MS && at <= now).then_some(at)
    }

    /// Steam has no market for this item: it isn't asked about again for an hour.
    pub fn note_no_market(&self, name: &str, at: i64) {
        self.no_market.lock().unwrap().insert(name.to_string(), at);
    }

    /// The market's search gave nothing to go by: it isn't asked again for a while.
    pub fn turn_searches_off(&self, how_long: Duration, why: impl Into<String>) {
        *self.searches_off.lock().unwrap() = Some((Instant::now() + how_long, why.into()));
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

    // ------------------------------------------------------ achievements --

    /// Closes a game's achievements view. Its helper stops, unless the game
    /// is idling (the unlocker counts as idling too).
    pub async fn close_achievements(&self, appid: u32) {
        self.ach_open.lock().unwrap().remove(&appid);
        if !self.idler.anyone_idling(appid) {
            self.helpers.stop(appid).await;
        }
    }

    /// Closes every open achievements view, as the window goes to the tray:
    /// an open view keeps its game running in Steam, with nobody to see it.
    /// The window is told which ones, and opens them again when it comes back.
    pub async fn close_all_achievements(&self) {
        let open: Vec<u32> = self.ach_open.lock().unwrap().drain().collect();
        if open.is_empty() {
            return;
        }
        for &appid in &open {
            if !self.idler.anyone_idling(appid) {
                self.helpers.stop(appid).await;
            }
        }
        logbus::info("achievements", format!("Closed {} open achievement list(s) while IdleKit is in the tray", open.len()));
        logbus::emit("ach://closed", open);
    }

    /// How many players have each of a game's achievements. Kept for an hour,
    /// so opening the same game again doesn't ask Steam again; a failed
    /// download is not kept, so the next open tries again.
    pub async fn achievement_percents(&self, appid: u32) -> Arc<HashMap<String, f64>> {
        const FRESH: Duration = Duration::from_secs(60 * 60);
        if let Some((at, kept)) = self.ach_percents.lock().unwrap().get(&appid) {
            if at.elapsed() < FRESH {
                return kept.clone();
            }
        }
        match webapi::global_achievement_percentages(&self.http, appid).await {
            Ok(percents) => {
                let percents = Arc::new(percents);
                self.ach_percents.lock().unwrap().insert(appid, (Instant::now(), percents.clone()));
                percents
            }
            Err(_) => Arc::default(),
        }
    }

    // ---------------------------------------------------------- steam events --

    /// Steam's upcoming sales and fests: what is there already, or (with
    /// `refresh`) Valve's page read again. See `events::list`.
    pub async fn steam_events(&self, refresh: bool) -> EventList {
        events::list(&self.data_dir.join(events::FILE), refresh, || events::fetch(&self.http)).await
    }

    // ------------------------------------------------------- web session --

    /// Puts in a web sign-in that just arrived with a sign-in, and keeps it
    /// for the next start.
    pub async fn seed_web_session(&self, steam_id: &str, access_token: &str) {
        match WebSession::from_access_token(steam_id, access_token) {
            Ok(ws) => {
                self.web.lock().unwrap().insert(steam_id.to_string(), Arc::new(ws));
                self.keep_web_token(steam_id, access_token);
                logbus::debug("web", format!("{}: web session ready ({})", logbus::label(steam_id), good_for(access_token)));
            }
            Err(e) => logbus::warn("web", format!("{}: the new web sign-in couldn't be used: {e:#}", logbus::label(steam_id))),
        }
    }

    /// Drops the account's web session, in memory and the one kept for the
    /// next start: a new sign-in replaces it, or Steam no longer takes it.
    pub async fn forget_web_session(&self, steam_id: &str) {
        let held = self.web.lock().unwrap().remove(steam_id).is_some();
        let key = secrets::web_token_key(steam_id);
        let kept = secrets::get(&key).is_some();
        secrets::delete(&key);
        if held || kept {
            logbus::debug("web", format!("{}: dropped the web session", logbus::label(steam_id)));
        }
    }

    /// Drops the account's web session only if it is still the one that
    /// failed: another caller may have made a new one in the meantime.
    fn forget_failed_web_session(&self, steam_id: &str, failed: &WebSession) {
        let mut web = self.web.lock().unwrap();
        if web.get(steam_id).is_some_and(|ws| ws.access_token == failed.access_token) {
            web.remove(steam_id);
        }
        drop(web);
        let key = secrets::web_token_key(steam_id);
        if !failed.access_token.is_empty() && secrets::get(&key).as_deref() == Some(failed.access_token.as_str()) {
            secrets::delete(&key);
        }
    }

    /// Keeps a web sign-in for the next start, in the system's credential store.
    fn keep_web_token(&self, steam_id: &str, token: &str) {
        if let Err(e) = secrets::set(&secrets::web_token_key(steam_id), token) {
            logbus::warn(
                "web",
                format!(
                    "{}: the web session couldn't be kept for the next start ({e:#}); a new one is made then",
                    logbus::label(steam_id)
                ),
            );
        }
    }

    /// The session kept for this account, unless its access token is about to run out.
    fn kept_web_session(&self, steam_id: &str) -> Option<Arc<WebSession>> {
        let web = self.web.lock().unwrap();
        web.get(steam_id)
            .filter(|ws| ws.access_token.is_empty() || !jwt::expires_soon(&ws.access_token, WEB_MARGIN_SECS))
            .cloned()
    }

    /// The web sign-in kept from an earlier start (a restart or an update),
    /// if it is this account's and still good for a while.
    fn stored_web_session(&self, steam_id: &str) -> Option<WebSession> {
        let key = secrets::web_token_key(steam_id);
        let token = secrets::get(&key)?;
        let who = logbus::label(steam_id);
        let mine = jwt::decode(&token).is_some_and(|c| c.sub == steam_id);
        if !mine || jwt::expires_soon(&token, WEB_MARGIN_SECS) {
            let why = if mine { "has run out" } else { "isn't this account's" };
            logbus::debug("web", format!("{who}: the web session kept from before {why}; a new one is needed"));
            secrets::delete(&key);
            return None;
        }
        match WebSession::from_access_token(steam_id, &token) {
            Ok(ws) => {
                logbus::info("web", format!("{who}: using the web session kept from before ({})", good_for(&token)));
                Some(ws)
            }
            Err(e) => {
                logbus::debug("web", format!("{who}: the web session kept from before couldn't be used ({e:#})"));
                secrets::delete(&key);
                None
            }
        }
    }

    /// A signed-in community/store session for this account.
    ///
    /// In order: the one in memory; the one kept from an earlier start; a new
    /// one made from the saved sign-in over Steam's own servers; a pasted
    /// cookie. A new one is never asked for over HTTPS: for a Steam-client
    /// sign-in Steam refuses that and then throws the saved sign-in away,
    /// which is what broke the web session after every update before 2.3.
    pub async fn web_session(&self, steam_id: &str) -> Result<Arc<WebSession>> {
        if let Some(ws) = self.kept_web_session(steam_id) {
            return Ok(ws);
        }
        let gate = self.web_refresh.lock().unwrap().entry(steam_id.to_string()).or_default().clone();
        let _refreshing = gate.lock().await;
        // Another caller may have made it while this one waited.
        if let Some(ws) = self.kept_web_session(steam_id) {
            return Ok(ws);
        }
        let ws = if let Some(ws) = self.stored_web_session(steam_id) {
            ws
        } else if let Some(refresh) = secrets::get(&secrets::refresh_token_key(steam_id)) {
            self.new_web_session(steam_id, &refresh).await?
        } else if let Some(cookie) = secrets::get(&secrets::cookie_key(steam_id)) {
            let ws = WebSession::from_cookie(&cookie)?;
            if !ws.access_token.is_empty() && jwt::expires_soon(&ws.access_token, 0) {
                bail!("The saved steamLoginSecure cookie has expired — paste a fresh one in Accounts");
            }
            logbus::debug("web", format!("{}: using the pasted web session cookie", logbus::label(steam_id)));
            ws
        } else {
            bail!("This account has no web session. In Accounts, sign in with QR/password or paste a steamLoginSecure cookie.");
        };
        let ws = Arc::new(ws);
        self.web.lock().unwrap().insert(steam_id.to_string(), ws.clone());
        Ok(ws)
    }

    /// A new web session made from the saved sign-in over Steam's servers.
    async fn new_web_session(&self, steam_id: &str, refresh: &str) -> Result<WebSession> {
        let who = logbus::label(steam_id);
        let print = fingerprint(refresh);
        if self.refused_sign_ins.lock().unwrap().get(steam_id) == Some(&print) {
            logbus::debug("web", format!("{who}: not asking Steam again: it refused this saved sign-in earlier"));
            bail!("{REFUSED_EARLIER}");
        }
        if let Some(f) = self.web_failures.lock().unwrap().get(steam_id).filter(|f| f.print == print && f.at.elapsed() < f.wait) {
            let ago = f.at.elapsed().as_secs();
            logbus::debug("web", format!("{who}: not asking Steam again yet: the last try failed {ago} s ago"));
            bail!("{}", f.message);
        }
        let account = self.account(steam_id)?;
        logbus::info("web", format!("{who}: making a new web session over Steam's servers"));
        let started = Instant::now();
        let minted = tokio::time::timeout(WEB_RENEW_LIMIT, self.cm.mint_web_token(steam_id, &account.account_name, refresh)).await;
        let ms = started.elapsed().as_millis();
        let failed = |wait: Duration, message: String| {
            self.web_failures
                .lock()
                .unwrap()
                .insert(steam_id.to_string(), WebFailure { at: Instant::now(), print, wait, message: message.clone() });
            anyhow!(message)
        };
        let token = match minted {
            Ok(Ok(token)) => token,
            Ok(Err(e)) if e.refusal == Refusal::SignIn => {
                self.refused_sign_ins.lock().unwrap().insert(steam_id.to_string(), print);
                // The Accounts page says so at once.
                self.emit_accounts();
                logbus::error("web", format!("{who}: Steam refused the saved sign-in ({e}, after {ms} ms); sign in to this account again"));
                bail!("Steam refused this account's saved sign-in ({e}). Sign in to this account again on the Accounts page.");
            }
            Ok(Err(e)) if e.refusal == Refusal::RateLimited => {
                logbus::warn("web", format!("{who}: couldn't make a new web session ({e}, after {ms} ms); trying again in 10 min at the soonest"));
                return Err(failed(
                    WEB_RETRY_AFTER_LIMIT,
                    format!("Steam is limiting sign-ins right now ({e}). IdleKit asks again in about 10 minutes."),
                ));
            }
            Ok(Err(e)) => {
                logbus::warn("web", format!("{who}: couldn't make a new web session ({e}, after {ms} ms)"));
                return Err(failed(
                    WEB_RETRY_AFTER_TROUBLE,
                    format!("Couldn't get a web session from Steam right now ({e}). IdleKit tries again the next time one is needed."),
                ));
            }
            Err(_) => {
                let secs = WEB_RENEW_LIMIT.as_secs();
                logbus::warn("web", format!("{who}: Steam didn't answer within {secs} s for a new web session"));
                return Err(failed(
                    WEB_RETRY_AFTER_TROUBLE,
                    format!("Steam didn't answer within {secs} seconds for a new web session. IdleKit tries again the next time one is needed."),
                ));
            }
        };
        self.web_failures.lock().unwrap().remove(steam_id);
        if jwt::decode(&token).is_none_or(|c| c.sub != steam_id) {
            logbus::warn("web", format!("{who}: Steam's new web session is for a different account"));
            bail!("Steam answered with a web session for a different account");
        }
        logbus::info("web", format!("{who}: new web session ready in {ms} ms ({})", good_for(&token)));
        self.keep_web_token(steam_id, &token);
        WebSession::from_access_token(steam_id, &token)
    }

    /// Whether Steam refused the account's saved sign-in during this run (and
    /// no new one has been saved since).
    pub fn sign_in_refused(&self, steam_id: &str) -> bool {
        let Some(print) = self.refused_sign_ins.lock().unwrap().get(steam_id).copied() else { return false };
        secrets::get(&secrets::refresh_token_key(steam_id)).is_some_and(|t| fingerprint(&t) == print)
    }

    /// Runs `work` with the account's web session. When Steam says the
    /// session has ended (its page was shown to nobody signed in), the
    /// session is dropped and `work` runs once more with a new one: a kept
    /// session Steam stopped taking costs one extra page, not an error.
    /// Only for work that is safe to repeat.
    pub async fn with_web<T, F, Fut>(&self, steam_id: &str, work: F) -> Result<T>
    where
        F: Fn(Arc<WebSession>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let ws = self.web_session(steam_id).await?;
        match work(ws.clone()).await {
            Err(e) if community::session_ended(&e) => {
                logbus::info(
                    "web",
                    format!("{}: Steam no longer takes this web session; making a new one and trying again", logbus::label(steam_id)),
                );
                self.forget_failed_web_session(steam_id, &ws);
                let ws = self.web_session(steam_id).await?;
                work(ws).await
            }
            other => other,
        }
    }

    /// After write work (listing, removing) failed because Steam says the
    /// session ended: the next try gets a new one. Not repeated by itself.
    pub fn web_failed(&self, steam_id: &str, ws: &WebSession, e: &anyhow::Error) {
        if community::session_ended(e) {
            logbus::info("web", format!("{}: Steam no longer takes this web session; the next try makes a new one", logbus::label(steam_id)));
            self.forget_failed_web_session(steam_id, ws);
        }
    }

    // ------------------------------------------------------------- games --

    fn games_file(&self, steam_id: &str) -> PathBuf {
        self.data_dir.join(format!("games_{steam_id}.json"))
    }

    /// What a game is shown as until its name is known.
    fn placeholder(appid: u32) -> String {
        format!("App {appid}")
    }

    /// True for a game that has no name of its own yet.
    fn unnamed(name: &str, appid: u32) -> bool {
        let name = name.trim();
        name.is_empty() || name == Self::placeholder(appid)
    }

    /// The name to show: the game's own, else the store's, else "App <id>".
    fn name_for(name: String, appid: u32, store: &HashMap<u32, String>) -> String {
        if !Self::unnamed(&name, appid) {
            return name;
        }
        store.get(&appid).cloned().unwrap_or_else(|| Self::placeholder(appid))
    }

    fn sort_by_name(list: &mut [GameEntry]) {
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }

    fn to_entries(owned: Vec<OwnedGame>, account: &Account, store: &HashMap<u32, String>) -> Vec<GameEntry> {
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
                        name: Self::name_for(g.name, g.appid, store),
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
                name: Self::name_for(m.name.clone(), m.appid, store),
                manual: true,
                ..Default::default()
            });
        }
        let mut list: Vec<GameEntry> = map.into_values().collect();
        Self::sort_by_name(&mut list);
        list
    }

    fn entries(&self, owned: Vec<OwnedGame>, account: &Account) -> Vec<GameEntry> {
        Self::to_entries(owned, account, &self.store_names.lock().unwrap())
    }

    /// Takes in names from Steam's store. Every loaded games list that has one
    /// of these games without a name gets it, and the saved list too, so the
    /// name is there next time without asking. Answers which accounts' lists
    /// changed.
    pub fn adopt_names(&self, found: &HashMap<u32, String>) -> Vec<String> {
        if found.is_empty() {
            return Vec::new();
        }
        self.store_names.lock().unwrap().extend(found.iter().map(|(id, name)| (*id, name.clone())));
        let mut changed = Vec::new();
        {
            let mut games = self.games.lock().unwrap();
            for (steam_id, list) in games.iter_mut() {
                let mut any = false;
                for g in list.iter_mut().filter(|g| Self::unnamed(&g.name, g.appid)) {
                    if let Some(name) = found.get(&g.appid) {
                        g.name = name.clone();
                        any = true;
                    }
                }
                if any {
                    Self::sort_by_name(list);
                    changed.push(steam_id.clone());
                }
            }
        }
        for steam_id in &changed {
            let file = self.games_file(steam_id);
            let saved: Option<Vec<OwnedGame>> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok());
            if let Some(mut owned) = saved {
                for g in owned.iter_mut().filter(|g| Self::unnamed(&g.name, g.appid)) {
                    if let Some(name) = found.get(&g.appid) {
                        g.name = name.clone();
                    }
                }
                if let Ok(text) = serde_json::to_string(&owned) {
                    let _ = std::fs::write(&file, text);
                }
            }
        }
        changed
    }

    /// Takes in the names that came with a store lookup and tells the window
    /// about every games list that changed.
    pub fn names_arrived(&self, found: &HashMap<u32, String>) {
        for steam_id in self.adopt_names(found) {
            logbus::emit("games://updated", steam_id);
        }
    }

    /// Games that Steam's files on this PC list by number only get their names
    /// from Steam's store: at once where a lookup has been made before, and
    /// otherwise by a lookup that runs on after the list has been handed over.
    async fn name_the_unnamed(&self, steam_id: &str, entries: Vec<GameEntry>) -> Vec<GameEntry> {
        let ids: Vec<u32> = entries.iter().filter(|g| Self::unnamed(&g.name, g.appid)).map(|g| g.appid).collect();
        if ids.is_empty() {
            return entries;
        }
        let known = self.art.names(&ids).await;
        let entries = if known.is_empty() {
            entries
        } else {
            // Without a notice to the window: this list is on its way there.
            self.adopt_names(&known);
            self.games.lock().unwrap().get(steam_id).cloned().unwrap_or(entries)
        };
        let ask: Vec<u32> = ids.into_iter().filter(|id| !known.contains_key(id)).collect();
        if ask.is_empty() {
            return entries;
        }
        {
            // One lookup per account at a time, and not one after another
            // while Steam can't be reached.
            let mut naming = self.naming.lock().unwrap();
            if naming.get(steam_id).is_some_and(|since| since.elapsed() < ASK_FOR_NAMES_EVERY) {
                return entries;
            }
            naming.insert(steam_id.to_string(), Instant::now());
        }
        let app = self.clone();
        tokio::spawn(async move {
            let language = app.settings().language;
            let found = app.art.resolve(&app.http, &language, &ask).await;
            let names: HashMap<u32, String> = found.into_iter().filter_map(|(id, art)| art.name.map(|name| (id, name))).collect();
            if !names.is_empty() {
                logbus::info("library", format!("Steam's store named {} of {} game(s) that had no name", names.len(), ask.len()));
            }
            app.names_arrived(&names);
        });
        entries
    }

    pub fn cached_games(&self, steam_id: &str) -> Option<Vec<GameEntry>> {
        if let Some(g) = self.games.lock().unwrap().get(steam_id) {
            return Some(g.clone());
        }
        let text = std::fs::read_to_string(self.games_file(steam_id)).ok()?;
        let owned: Vec<OwnedGame> = serde_json::from_str(&text).ok()?;
        let account = self.account(steam_id).ok()?;
        let entries = self.entries(owned, &account);
        self.games.lock().unwrap().insert(steam_id.to_string(), entries.clone());
        Some(entries)
    }

    pub fn game_name(&self, steam_id: &str, appid: u32) -> String {
        self.games
            .lock()
            .unwrap()
            .get(steam_id)
            .and_then(|l| l.iter().find(|g| g.appid == appid).map(|g| g.name.clone()))
            .or_else(|| self.store_names.lock().unwrap().get(&appid).cloned())
            .unwrap_or_else(|| Self::placeholder(appid))
    }

    /// Fetch the owned-games list from the best available source.
    pub async fn load_games(&self, steam_id: &str, force: bool) -> Result<Vec<GameEntry>> {
        if !force {
            if let Some(g) = self.cached_games(steam_id) {
                return Ok(self.name_the_unnamed(steam_id, g).await);
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
        // A fresh list from the files on this PC has no names for games that
        // aren't installed: keep the ones the store gave earlier.
        let mut owned = owned;
        {
            let store = self.store_names.lock().unwrap();
            for g in owned.iter_mut().filter(|g| Self::unnamed(&g.name, g.appid)) {
                if let Some(name) = store.get(&g.appid) {
                    g.name = name.clone();
                }
            }
        }
        if let Ok(text) = serde_json::to_string(&owned) {
            let _ = std::fs::write(self.games_file(steam_id), text);
        }
        let entries = self.entries(owned, &account);
        self.games.lock().unwrap().insert(steam_id.to_string(), entries.clone());
        Ok(self.name_the_unnamed(steam_id, entries).await)
    }

    /// Re-merge manual games after the user edits that list.
    pub fn refresh_manual(&self, steam_id: &str) {
        let Ok(account) = self.account(steam_id) else { return };
        let owned: Vec<OwnedGame> = std::fs::read_to_string(self.games_file(steam_id))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let entries = self.entries(owned, &account);
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
        let Ok(resp) = self.http.get(&url).logged().await else { return };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ManualGame;

    #[test]
    fn unnamed_games_take_the_stores_name() {
        let owned = vec![
            // installed: Steam's files on this PC give the name
            OwnedGame { appid: 10, name: "Installed Game".into(), ..Default::default() },
            // played but not installed: listed by number only
            OwnedGame { appid: 1030300, playtime_minutes: 5490, ..Default::default() },
            OwnedGame { appid: 1044720, name: "App 1044720".into(), ..Default::default() },
            // one the store has no name for either
            OwnedGame { appid: 77, ..Default::default() },
        ];
        let account = Account {
            manual_games: vec![
                ManualGame { appid: 500, name: "App 500".into() },
                ManualGame { appid: 600, name: "My own name".into() },
            ],
            ..Default::default()
        };
        let store = HashMap::from([
            (10, "The Store's Name".to_string()),
            (1030300, "Hollow Knight: Silksong".to_string()),
            (1044720, "Farthest Frontier".to_string()),
            (500, "Added By Number".to_string()),
            (600, "Something Else".to_string()),
        ]);
        let named = App::to_entries(owned.clone(), &account, &store);
        let name = |appid: u32| named.iter().find(|g| g.appid == appid).unwrap().name.clone();
        assert_eq!(name(1030300), "Hollow Knight: Silksong");
        assert_eq!(name(1044720), "Farthest Frontier");
        assert_eq!(name(500), "Added By Number");
        // a name the game already has is never replaced
        assert_eq!(name(10), "Installed Game");
        assert_eq!(name(600), "My own name");
        // no name anywhere: the number, as before
        assert_eq!(name(77), "App 77");
        // in order of the names shown
        let order: Vec<u32> = named.iter().map(|g| g.appid).collect();
        assert_eq!(order, vec![500, 77, 1044720, 1030300, 10, 600]);
        assert_eq!(named.iter().find(|g| g.appid == 1030300).unwrap().playtime_minutes, 5490);

        // with nothing from the store, everything is as it was
        let bare = App::to_entries(owned, &account, &HashMap::new());
        let name = |appid: u32| bare.iter().find(|g| g.appid == appid).unwrap().name.clone();
        assert_eq!([name(1030300), name(1044720), name(500)], ["App 1030300", "App 1044720", "App 500"]);
        assert!(App::unnamed("", 5) && App::unnamed(" App 5 ", 5) && !App::unnamed("App 6", 5) && !App::unnamed("Portal", 5));
    }

    // ------------------------------------------------- web session order --

    use base64::Engine;

    /// A token shaped like Steam's, for this account, good for `secs` more.
    fn token(steam_id: &str, secs: i64, tag: &str) -> String {
        let payload = format!(
            r#"{{"iss":"r:{tag}","sub":"{steam_id}","aud":["web","renew","derive"],"exp":{}}}"#,
            chrono::Utc::now().timestamp() + secs
        );
        format!("eyJhbGciOiJFZERTQSJ9.{}.sig", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload))
    }

    /// An app on a folder of its own with one account on this PC, whose
    /// connections to Steam go nowhere (a server list naming a closed port).
    fn app_with(tag: &str, steam_id: &str) -> (App, PathBuf) {
        let dir = std::env::temp_dir().join(format!("idlekit-web-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pictures")).unwrap();
        let servers = serde_json::json!({ "at": chrono::Utc::now().timestamp(), "tcp": ["127.0.0.1:9"], "ws": ["127.0.0.1:9"] });
        std::fs::write(dir.join(crate::steam::cm::SERVERS_FILE), servers.to_string()).unwrap();
        let app = App::new(dir.clone(), dir.join("pictures"));
        app.update_config(|c| {
            c.accounts.push(Account { steam_id: steam_id.into(), account_name: "someone".into(), mode: "local".into(), ..Default::default() })
        });
        (app, dir)
    }

    fn clean(steam_id: &str, dir: PathBuf) {
        for key in [secrets::refresh_token_key(steam_id), secrets::web_token_key(steam_id), secrets::cookie_key(steam_id)] {
            secrets::delete(&key);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_web_session_carries_over_a_restart_without_asking_steam() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let sid = "76561197960265001";
        let (app, dir) = app_with("restart", sid);
        secrets::set(&secrets::refresh_token_key(sid), &token(sid, 200 * 86_400, "refresh")).unwrap();
        // Signed in: the web sign-in that came with it is kept for the next start.
        let web = token(sid, 20 * 3600, "web");
        app.seed_web_session(sid, &web).await;
        assert_eq!(secrets::get(&secrets::web_token_key(sid)).as_deref(), Some(web.as_str()));

        // An update or a restart: a new app on the same folder. It uses the
        // kept session at once; asking Steam could only have failed here.
        drop(app);
        let restarted = App::new(dir.clone(), dir.join("pictures"));
        let started = Instant::now();
        let ws = restarted.web_session(sid).await.expect("the kept session");
        assert_eq!(ws.access_token, web);
        assert!(started.elapsed() < Duration::from_millis(500), "took {:?}", started.elapsed());
        // And from memory after that.
        assert!(Arc::ptr_eq(&ws, &restarted.web_session(sid).await.unwrap()));
        clean(sid, dir);
    }

    #[tokio::test]
    async fn a_kept_session_about_to_run_out_is_dropped_and_a_new_one_asked_for_over_steams_servers() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let sid = "76561197960265002";
        let (app, dir) = app_with("expiring", sid);
        secrets::set(&secrets::refresh_token_key(sid), &token(sid, 200 * 86_400, "refresh")).unwrap();
        secrets::set(&secrets::web_token_key(sid), &token(sid, 300, "web")).unwrap();
        let started = Instant::now();
        let e = app.web_session(sid).await.err().expect("an error").to_string();
        // Steam's servers can't be reached here: trouble, to be tried again,
        // never "sign in again", and never by HTTPS.
        assert!(e.contains("Couldn't get a web session from Steam right now"), "{e}");
        assert!(!e.contains("Sign in"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
        assert_eq!(secrets::get(&secrets::web_token_key(sid)), None, "the run-out session is gone");
        assert!(app.refused_sign_ins.lock().unwrap().is_empty(), "trouble is not a refusal");
        // Asked again at once (another page): the same answer, without a new try.
        let tried_at = app.web_failures.lock().unwrap().get(sid).map(|f| f.at).expect("the failure is kept");
        let again = app.web_session(sid).await.err().expect("an error").to_string();
        assert_eq!(again, e);
        assert_eq!(app.web_failures.lock().unwrap().get(sid).map(|f| f.at), Some(tried_at), "Steam was asked again");
        // A session for another account is never used either.
        secrets::set(&secrets::web_token_key(sid), &token("76561197960265999", 20 * 3600, "web")).unwrap();
        assert!(app.web_session(sid).await.is_err());
        assert_eq!(secrets::get(&secrets::web_token_key(sid)), None);
        clean(sid, dir);
    }

    #[tokio::test]
    async fn a_sign_in_steam_refused_is_not_asked_about_again_until_a_new_one() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let sid = "76561197960265003";
        let (app, dir) = app_with("refused", sid);
        let refresh = token(sid, 200 * 86_400, "refresh");
        secrets::set(&secrets::refresh_token_key(sid), &refresh).unwrap();
        // As if Steam had refused this saved sign-in a moment ago.
        app.refused_sign_ins.lock().unwrap().insert(sid.into(), fingerprint(&refresh));
        let started = Instant::now();
        let e = app.web_session(sid).await.err().expect("an error").to_string();
        assert!(e.contains("refused this account's saved sign-in earlier"), "{e}");
        assert!(started.elapsed() < Duration::from_millis(200), "Steam was asked again: {:?}", started.elapsed());
        // A new sign-in is another one, and is tried.
        secrets::set(&secrets::refresh_token_key(sid), &token(sid, 200 * 86_400, "refresh-2")).unwrap();
        let e = app.web_session(sid).await.err().expect("an error").to_string();
        assert!(e.contains("Couldn't get a web session from Steam right now"), "{e}");
        clean(sid, dir);
    }

    #[tokio::test]
    async fn work_steam_says_needs_a_new_session_runs_once_more_with_one() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let sid = "76561197960265004";
        let (app, dir) = app_with("ended", sid);
        // A pasted cookie: a new session can be made from it with no network.
        let cookie = format!("{sid}%7C%7C{}", token(sid, 20 * 3600, "cookie"));
        secrets::set(&secrets::cookie_key(sid), &cookie).unwrap();
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let first = app.web_session(sid).await.unwrap();
        let got = app
            .with_web(sid, |ws| {
                let n = tries.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n == 0 {
                        Err(anyhow::Error::new(community::SessionEnded))
                    } else {
                        Ok(ws)
                    }
                }
            })
            .await
            .expect("the second try works");
        assert_eq!(tries.load(Ordering::SeqCst), 2);
        assert!(!Arc::ptr_eq(&first, &got), "a new session was made");
        // Anything else is not repeated.
        let once = std::sync::atomic::AtomicUsize::new(0);
        let e = app
            .with_web(sid, |_| {
                once.fetch_add(1, Ordering::SeqCst);
                async { Err::<(), _>(anyhow!("HTTP 500")) }
            })
            .await
            .unwrap_err();
        assert_eq!(once.load(Ordering::SeqCst), 1, "{e}");
        clean(sid, dir);
    }

    #[test]
    fn time_left_reads_as_words() {
        assert_eq!(good_for(&token("1", 23 * 3600 + 100, "a")), "good for 23 h more");
        assert_eq!(good_for(&token("1", 1800, "a")), "good for 30 min more");
        assert_eq!(good_for(&token("1", -5, "a")), "already run out");
        assert_ne!(fingerprint("a"), fingerprint("b"));
    }
}
