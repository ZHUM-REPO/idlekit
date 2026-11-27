//! Direct connections to Steam's connection managers (no Steam client needed).
//! Used for "direct" accounts: idling up to 32 games via ClientGamesPlayed,
//! online status, owned-games lookup and access-token refresh.
use super::webapi::{eresult_name, OwnedGame};
use crate::{logbus, secrets};
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::Serialize;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use steam_vent::connection::UnAuthenticatedConnection;
use steam_vent::{
    Connection, ConnectionError, ConnectionTrait, DiscoverOptions, EResult, LoginError, NetworkError, ServerList,
};
use steam_vent_proto_steam::enums_clientserver::EMsg;
use steam_vent_proto_steam::steammessages_auth_steamclient::{
    CAuthentication_AccessToken_GenerateForApp_Request, ETokenRenewalType,
};
use steam_vent_proto_steam::steammessages_clientserver::cmsg_client_games_played::GamePlayed;
use steam_vent_proto_steam::steammessages_clientserver::CMsgClientGamesPlayed;
use steam_vent_proto_steam::steammessages_clientserver_2::CMsgClientPlayingSessionState;
use steam_vent_proto_steam::steammessages_clientserver_friends::CMsgClientChangeStatus;
use steam_vent_proto_steam::steammessages_clientserver_login::{CMsgClientLogOff, CMsgClientLoggedOff};
use steam_vent_proto_steam::steammessages_player_steamclient::CPlayer_GetOwnedGames_Request;

/// GameID of a "non-Steam game" shortcut; Steam shows `game_extra_info` as its name.
const CUSTOM_STATUS_GAME_ID: u64 = 15_190_414_816_125_648_896;
const RECONNECT_DELAYS: [u64; 7] = [5, 15, 30, 60, 120, 300, 600];
/// Pauses, in seconds, after Steam says it is limiting sign-ins. Trying
/// again sooner only keeps the limit in place.
const RATE_LIMIT_DELAYS: [u64; 3] = [15 * 60, 30 * 60, 60 * 60];
/// How long a new web sign-in may wait for a direct account's own
/// connection to come up (at startup, or while it reconnects).
const OWN_CONNECTION_WAIT: Duration = Duration::from_secs(40);
/// A message sent on a connection gives up after this long: a connection
/// that has quietly died must not hold anything up.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// How long finding Steam's servers, connecting and signing in may take all
/// together. A connection that hangs half way (Wi-Fi dropping, the PC waking
/// up) would otherwise leave an account "connecting" for good; this way the
/// attempt fails and is tried again like any other.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// A list of Steam's servers is used for this long before a fresh one is asked for.
const SERVERS_FRESH: Duration = Duration::from_secs(60 * 60);
/// Where the last list of Steam's servers is kept, in the app's data folder.
pub const SERVERS_FILE: &str = "cm-servers.json";

/// Steam's servers as kept on disk, so a start soon after the last one, or a
/// reconnect while Steam's directory doesn't answer, can do without asking.
#[derive(Serialize, Deserialize)]
struct SavedServers {
    /// When the list was fetched, in Unix seconds.
    at: i64,
    tcp: Vec<SocketAddr>,
    ws: Vec<String>,
}

/// A connection that failed on the way: never a reason to give up on the
/// saved sign-in. Its text is IdleKit's own words (kind `Other`).
fn network_error(text: impl Into<String>) -> ConnectionError {
    ConnectionError::Network(NetworkError::IO(std::io::Error::other(text.into())))
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CmStatus {
    pub steam_id: String,
    /// "offline" | "connecting" | "online" | "blocked" | "reconnecting" | "error"
    pub status: String,
    pub message: String,
    pub games: Vec<u32>,
    pub persona_state: u32,
}

#[derive(Default)]
struct CmState {
    status: String,
    message: String,
    games: Vec<u32>,
    status_text: String,
    persona_state: u32,
    blocked: bool,
}

pub struct CmSession {
    steam_id: String,
    account_name: Mutex<String>,
    conn: tokio::sync::Mutex<Option<Connection>>,
    state: Mutex<CmState>,
    generation: AtomicU64,
    wanted: AtomicBool,
    /// Held while connecting: two callers at once (startup, a button, idling)
    /// make one connection, not two sign-ins.
    opening: tokio::sync::Mutex<()>,
    /// A chain of reconnect attempts is under way; a second one isn't started.
    reconnecting: AtomicBool,
    /// Held while games or the online status are worked out and sent, so two
    /// quick changes reach Steam in order and the last one is what stays.
    pushing: tokio::sync::Mutex<()>,
}

impl CmSession {
    fn snapshot(&self) -> CmStatus {
        let s = self.state.lock().unwrap();
        CmStatus {
            steam_id: self.steam_id.clone(),
            status: if s.status.is_empty() { "offline".into() } else { s.status.clone() },
            message: s.message.clone(),
            games: s.games.clone(),
            persona_state: s.persona_state,
        }
    }

    fn set_status(&self, status: &str, message: impl Into<String>) {
        {
            let mut s = self.state.lock().unwrap();
            s.status = status.to_string();
            s.message = message.into();
        }
        logbus::emit("cm://status", self.snapshot());
    }

    fn games_msg(&self) -> CMsgClientGamesPlayed {
        let s = self.state.lock().unwrap();
        let mut msg = CMsgClientGamesPlayed::new();
        if !s.status_text.is_empty() && !s.games.is_empty() {
            let mut g = GamePlayed::new();
            g.set_game_id(CUSTOM_STATUS_GAME_ID);
            g.set_game_extra_info(s.status_text.clone());
            msg.games_played.push(g);
        }
        for &appid in s.games.iter().take(32) {
            let mut g = GamePlayed::new();
            g.set_game_id(appid as u64);
            msg.games_played.push(g);
        }
        msg
    }

    /// The connection, if there is one. Taken out of the lock at once: a send
    /// that hangs must not keep everyone else waiting for the lock.
    async fn current(&self) -> Option<Connection> {
        self.conn.lock().await.clone()
    }

    async fn push_games(&self) -> Result<()> {
        let _in_order = self.pushing.lock().await;
        if self.state.lock().unwrap().blocked {
            return Ok(());
        }
        let msg = self.games_msg();
        let count = msg.games_played.len();
        let Some(conn) = self.current().await else { return Ok(()) };
        tokio::time::timeout(SEND_TIMEOUT, conn.send_with_kind(msg, EMsg::k_EMsgClientGamesPlayedWithDataBlob))
            .await
            .map_err(|_| anyhow!("could not update games: Steam's server didn't take the message within {} s", SEND_TIMEOUT.as_secs()))?
            .map_err(|e| anyhow!("could not update games: {e}"))?;
        logbus::debug("direct", format!("{}: told Steam {count} game(s) are running", logbus::label(&self.steam_id)));
        Ok(())
    }

    async fn push_persona(&self) -> Result<()> {
        let _in_order = self.pushing.lock().await;
        let state = self.state.lock().unwrap().persona_state;
        let mut msg = CMsgClientChangeStatus::new();
        msg.set_persona_state(state);
        let Some(conn) = self.current().await else { return Ok(()) };
        tokio::time::timeout(SEND_TIMEOUT, conn.send(msg))
            .await
            .map_err(|_| anyhow!("could not set status: Steam's server didn't take the message within {} s", SEND_TIMEOUT.as_secs()))?
            .map_err(|e| anyhow!("could not set status: {e}"))?;
        logbus::debug("direct", format!("{}: set online status {state}", logbus::label(&self.steam_id)));
        Ok(())
    }
}

/// What a failed sign-in at Steam's servers means for what to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Steam refused the saved sign-in itself (wrong, expired, revoked):
    /// only signing in again helps, and asking again changes nothing.
    SignIn,
    /// Steam is limiting sign-ins for now: worth trying again, but not soon.
    RateLimited,
    /// Anything else: no answer, a busy server, "try another server".
    /// Worth trying again soon.
    Trouble,
}

/// What a result code Steam sends back about a sign-in means.
pub fn refusal_of(result: EResult) -> Refusal {
    use EResult as R;
    match result {
        R::InvalidPassword
        | R::AccessDenied
        | R::Revoked
        | R::Expired
        | R::AccountLogonDenied
        | R::AccountLogonDeniedNoMail
        | R::AccountLogonDeniedVerifiedEmailRequired
        | R::AccountLoginDeniedNeedTwoFactor
        | R::InvalidLoginAuthCode
        | R::ExpiredLoginAuthCode
        | R::TwoFactorCodeMismatch
        | R::RequirePasswordReEntry
        | R::PasswordRequiredToKickSession
        | R::AccountDisabled
        | R::AccountLockedDown
        | R::AccountNotFound
        | R::AccountHasBeenDeleted
        | R::InvalidSteamID
        | R::Banned
        | R::Suspended
        | R::ParentalControlRestricted
        | R::RestrictedDevice => Refusal::SignIn,
        R::RateLimitExceeded
        | R::AccountLoginDeniedThrottle
        | R::LimitExceeded
        | R::AccountLimitExceeded
        | R::AccountActivityLimitExceeded
        | R::IPBanned => Refusal::RateLimited,
        _ => Refusal::Trouble,
    }
}

/// What a failed connection or sign-in means for what to do next. Before
/// 2.3 every refusal but a rate limit counted as "sign in again", including
/// Steam's "try another server" and "busy" answers, which only need another try.
pub fn refusal(e: &ConnectionError) -> Refusal {
    match e {
        ConnectionError::AccessToken(_) => Refusal::SignIn,
        ConnectionError::LoginError(err) => match err {
            LoginError::RateLimited => Refusal::RateLimited,
            LoginError::InvalidCredentials
            | LoginError::SteamGuardRequired
            | LoginError::InvalidSteamId
            | LoginError::UnavailableAccount => Refusal::SignIn,
            LoginError::Unknown(result) => refusal_of(*result),
            _ => Refusal::Trouble,
        },
        _ => Refusal::Trouble,
    }
}

/// A failed connection or sign-in in words, with Steam's own result name.
pub fn describe(e: &ConnectionError) -> String {
    match e {
        ConnectionError::LoginError(LoginError::Unknown(result)) => format!("Steam answered {result:?}"),
        ConnectionError::LoginError(LoginError::RateLimited) => "Steam is limiting sign-ins right now".into(),
        ConnectionError::LoginError(LoginError::InvalidCredentials) => {
            "Steam answered that the saved sign-in isn't valid".into()
        }
        // IdleKit's own words, from network_error.
        ConnectionError::Network(NetworkError::IO(io)) if io.kind() == std::io::ErrorKind::Other => io.to_string(),
        ConnectionError::Network(NetworkError::IO(io)) => format!("couldn't reach Steam's servers ({io})"),
        ConnectionError::Network(NetworkError::Ws(ws)) => format!("couldn't reach Steam's servers ({ws})"),
        ConnectionError::Network(NetworkError::Timeout) => "Steam's server didn't answer in time".into(),
        ConnectionError::Network(NetworkError::EOF) => "Steam's server closed the connection".into(),
        ConnectionError::Network(other) => format!("trouble talking to Steam's server ({other})"),
        other => other.to_string(),
    }
}

/// Why a new web sign-in couldn't be made.
#[derive(Debug, Clone)]
pub struct MintError {
    pub refusal: Refusal,
    pub message: String,
}

impl MintError {
    fn trouble(message: impl Into<String>) -> Self {
        Self { refusal: Refusal::Trouble, message: message.into() }
    }
}

impl std::fmt::Display for MintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MintError {}

/// Asks Steam for a web sign-in (an access token) over a connection that is
/// signed in with this account's saved sign-in. This is the only way Steam
/// still makes one for a Steam-client sign-in: asked over HTTPS instead, it
/// answers AccountLogonDenied (63), then AccessDenied (15), and the saved
/// sign-in stops working everywhere.
pub async fn generate_web_token(conn: &Connection, steam_id: &str, refresh_token: &str) -> Result<String, MintError> {
    let sid: u64 = steam_id.parse().map_err(|_| MintError::trouble("not a Steam ID"))?;
    let mut req = CAuthentication_AccessToken_GenerateForApp_Request::new();
    req.set_refresh_token(refresh_token.to_string());
    req.set_steamid(sid);
    req.set_renewal_type(ETokenRenewalType::k_ETokenRenewalType_None);
    match conn.service_method(req).await {
        Ok(resp) if !resp.access_token().is_empty() => Ok(resp.access_token().to_string()),
        Ok(_) => Err(MintError::trouble("Steam answered without a web sign-in")),
        Err(NetworkError::ApiError(result)) => {
            Err(MintError { refusal: refusal_of(result), message: format!("Steam answered {result:?}") })
        }
        Err(NetworkError::Timeout) => Err(MintError::trouble("Steam's server didn't answer in time")),
        Err(e) => Err(MintError::trouble(e.to_string())),
    }
}

/// What to do after a reconnect attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// Connected, or there is nothing to do.
    Stop,
    /// Try again after the usual pause.
    Soon,
    /// Steam is limiting sign-ins: try again after a long pause.
    Later,
}

pub struct CmManager {
    sessions: Mutex<HashMap<String, Arc<CmSession>>>,
    /// Steam's servers as last asked for, and when (`None`: read from disk).
    /// The same list is handed out every time, so that each new attempt goes
    /// to the next server in it and a server that is down is not tried twice in a row.
    servers: Mutex<Option<(ServerList, Option<Instant>)>>,
    /// The file the list is kept in between starts; none in tests.
    servers_file: Option<PathBuf>,
    /// What asks for the list: a client with a time limit.
    http: reqwest::Client,
}

impl Default for CmManager {
    fn default() -> Self {
        Self::new(None, reqwest::Client::new())
    }
}

/// A pause in words: "5 s", "2 min", "1 h".
fn wait_words(secs: u64) -> String {
    match secs {
        0..=89 => format!("{secs} s"),
        90..=5399 => format!("{} min", (secs + 30) / 60),
        _ => format!("{} h", (secs + 1800) / 3600),
    }
}

impl CmManager {
    pub fn new(servers_file: Option<PathBuf>, http: reqwest::Client) -> Self {
        Self { sessions: Mutex::default(), servers: Mutex::default(), servers_file, http }
    }

    /// Steam's servers: the list asked for within the hour (in memory, or
    /// on disk from a start shortly before), else a fresh one. When Steam's
    /// directory doesn't answer, the last list known, however old: Steam's
    /// servers change little, and one that has gone is skipped at the next try.
    pub async fn servers(&self) -> Result<ServerList, ConnectionError> {
        // Taken out first: the lock must be let go before the file is read
        // and the list put back.
        let kept = self.servers.lock().unwrap().clone();
        let known = kept.or_else(|| {
            let (list, age) = self.load_servers()?;
            let fetched = Instant::now().checked_sub(age).filter(|_| age < SERVERS_FRESH);
            *self.servers.lock().unwrap() = Some((list.clone(), fetched));
            Some((list, fetched))
        });
        if let Some((list, Some(at))) = &known {
            if at.elapsed() < SERVERS_FRESH {
                logbus::debug(
                    "steam",
                    format!(
                        "Using Steam's server list from {} min ago ({} server(s))",
                        at.elapsed().as_secs() / 60,
                        list.ws_servers().len()
                    ),
                );
                return Ok(list.clone());
            }
        }
        let options = DiscoverOptions::default().with_web_client(self.http.clone());
        let asked = Instant::now();
        match ServerList::discover_with(options).await {
            Ok(list) => {
                logbus::debug(
                    "steam",
                    format!(
                        "Asked Steam for its server list: {} server(s) in {} ms",
                        list.ws_servers().len(),
                        asked.elapsed().as_millis()
                    ),
                );
                self.save_servers(&list);
                *self.servers.lock().unwrap() = Some((list.clone(), Some(Instant::now())));
                Ok(list)
            }
            Err(e) => match known {
                Some((list, _)) => {
                    logbus::info(
                        "steam",
                        format!(
                            "Steam's server list couldn't be fetched ({e}, after {} ms); using the one from before",
                            asked.elapsed().as_millis()
                        ),
                    );
                    Ok(list)
                }
                None => {
                    logbus::warn(
                        "steam",
                        format!("Steam's server list couldn't be fetched ({e}, after {} ms)", asked.elapsed().as_millis()),
                    );
                    Err(network_error(format!("Couldn't get Steam's server list: {e}")))
                }
            },
        }
    }

    /// The list kept on disk, and how old it is.
    fn load_servers(&self) -> Option<(ServerList, Duration)> {
        let text = std::fs::read(self.servers_file.as_ref()?).ok()?;
        let saved: SavedServers = serde_json::from_slice(&text).ok()?;
        let age = (chrono::Utc::now().timestamp() - saved.at).max(0) as u64;
        Some((ServerList::new(saved.tcp, saved.ws).ok()?, Duration::from_secs(age)))
    }

    fn save_servers(&self, list: &ServerList) {
        let Some(path) = &self.servers_file else { return };
        let saved = SavedServers { at: chrono::Utc::now().timestamp(), tcp: list.tcp_servers(), ws: list.ws_servers() };
        if let Ok(text) = serde_json::to_vec(&saved) {
            let tmp = path.with_extension("json.tmp");
            let _ = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path));
        }
    }

    fn session(&self, steam_id: &str) -> Arc<CmSession> {
        self.sessions
            .lock()
            .unwrap()
            .entry(steam_id.to_string())
            .or_insert_with(|| {
                Arc::new(CmSession {
                    steam_id: steam_id.to_string(),
                    account_name: Mutex::new(String::new()),
                    conn: tokio::sync::Mutex::new(None),
                    state: Mutex::new(CmState { persona_state: 1, ..Default::default() }),
                    generation: AtomicU64::new(0),
                    wanted: AtomicBool::new(false),
                    opening: tokio::sync::Mutex::new(()),
                    reconnecting: AtomicBool::new(false),
                    pushing: tokio::sync::Mutex::new(()),
                })
            })
            .clone()
    }

    pub fn status(&self, steam_id: &str) -> CmStatus {
        self.sessions
            .lock()
            .unwrap()
            .get(steam_id)
            .map(|s| s.snapshot())
            .unwrap_or_else(|| CmStatus {
                steam_id: steam_id.to_string(),
                status: "offline".into(),
                persona_state: 1,
                ..Default::default()
            })
    }

    pub fn statuses(&self) -> Vec<CmStatus> {
        self.sessions.lock().unwrap().values().map(|s| s.snapshot()).collect()
    }

    pub fn is_online(&self, steam_id: &str) -> bool {
        matches!(self.status(steam_id).status.as_str(), "online" | "blocked")
    }

    pub async fn connection(&self, steam_id: &str) -> Option<Connection> {
        let s = self.sessions.lock().unwrap().get(steam_id).cloned()?;
        let guard = s.conn.lock().await;
        guard.clone()
    }

    pub fn configure(&self, steam_id: &str, account_name: &str, persona_state: u32, status_text: &str) {
        let s = self.session(steam_id);
        *s.account_name.lock().unwrap() = account_name.to_string();
        let mut st = s.state.lock().unwrap();
        st.persona_state = persona_state;
        st.status_text = status_text.to_string();
    }

    /// Connects and signs in with a saved sign-in, within [`OPEN_TIMEOUT`].
    async fn open(&self, who: &str, account_name: &str, refresh_token: &str) -> Result<Connection, ConnectionError> {
        self.open_within(OPEN_TIMEOUT, who, account_name, refresh_token).await
    }

    /// Each step is written down with how long it took: which one hangs or
    /// fails is the first thing to know about a connection problem.
    async fn open_within(
        &self,
        limit: Duration,
        who: &str,
        account_name: &str,
        refresh_token: &str,
    ) -> Result<Connection, ConnectionError> {
        let started = Instant::now();
        let attempt = async {
            let servers = self.servers().await?;
            let unauth = UnAuthenticatedConnection::connect(&servers).await.inspect_err(|e| {
                logbus::debug(
                    "steam",
                    format!("{who}: couldn't reach a Steam server ({e}, after {} ms)", started.elapsed().as_millis()),
                )
            })?;
            logbus::debug("steam", format!("{who}: reached a Steam server in {} ms; signing in", started.elapsed().as_millis()));
            unauth.access(account_name, refresh_token).await
        };
        let result = tokio::time::timeout(limit, attempt)
            .await
            .unwrap_or_else(|_| Err(network_error(format!("Steam didn't answer within {} seconds", limit.as_secs()))));
        let ms = started.elapsed().as_millis();
        match &result {
            Ok(_) => logbus::debug("steam", format!("{who}: signed in to Steam in {ms} ms")),
            Err(e) => logbus::debug("steam", format!("{who}: signing in to Steam failed after {ms} ms: {}", describe(e))),
        }
        result
    }

    /// Connect (or reconnect) a direct account using its saved refresh token.
    pub async fn connect(self: &Arc<Self>, steam_id: &str) -> Result<()> {
        let s = self.session(steam_id);
        let who = logbus::label(steam_id);
        s.wanted.store(true, Ordering::SeqCst);
        if s.conn.lock().await.is_some() {
            logbus::debug("direct", format!("{who}: already connected to Steam"));
            return Ok(());
        }
        let _opening = s.opening.lock().await;
        // Someone else may have connected while this call waited.
        if s.conn.lock().await.is_some() {
            logbus::debug("direct", format!("{who}: connected to Steam while this call waited"));
            return Ok(());
        }
        let Some(token) = secrets::get(&secrets::refresh_token_key(steam_id)) else {
            logbus::warn("direct", format!("{who}: no saved sign-in to connect with; sign in again"));
            return Err(anyhow!("No saved sign-in for this account — sign in again"));
        };
        let account_name = s.account_name.lock().unwrap().clone();
        s.set_status("connecting", "");
        logbus::info("direct", format!("{who}: connecting to Steam"));
        let started = Instant::now();
        match self.open(&who, &account_name, &token).await {
            Ok(conn) => {
                self.install(s.clone(), conn, Some(started.elapsed())).await;
                Ok(())
            }
            Err(e) => {
                let msg = describe(&e);
                let gen = s.generation.load(Ordering::SeqCst);
                match refusal(&e) {
                    Refusal::SignIn => {
                        s.wanted.store(false, Ordering::SeqCst);
                        logbus::warn("direct", format!("{who}: Steam refused the saved sign-in ({msg}); sign in again"));
                        s.set_status("error", format!("Steam refused the saved sign-in ({msg}). Sign in again."));
                    }
                    Refusal::RateLimited => {
                        logbus::warn("direct", format!("{who}: couldn't connect: {msg}"));
                        s.set_status("reconnecting", format!("{msg}. IdleKit tries again later."));
                        self.schedule_reconnect(s.clone(), gen, 0, Next::Later);
                    }
                    Refusal::Trouble => {
                        logbus::warn("direct", format!("{who}: couldn't connect: {msg}"));
                        s.set_status("error", msg.clone());
                        self.schedule_reconnect(s.clone(), gen, 0, Next::Soon);
                    }
                }
                Err(anyhow!(msg))
            }
        }
    }

    /// Take over a connection that was just created by a password sign-in.
    pub async fn adopt(self: &Arc<Self>, steam_id: &str, account_name: &str, conn: Connection) {
        let s = self.session(steam_id);
        *s.account_name.lock().unwrap() = account_name.to_string();
        s.wanted.store(true, Ordering::SeqCst);
        self.install(s, conn, None).await;
    }

    async fn install(self: &Arc<Self>, s: Arc<CmSession>, conn: Connection, took: Option<Duration>) {
        let gen = s.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let logged_off = conn.one::<CMsgClientLoggedOff>();
        let mut playing = conn.on::<CMsgClientPlayingSessionState>();
        *s.conn.lock().await = Some(conn);
        s.state.lock().unwrap().blocked = false;
        s.set_status("online", "");
        let who = logbus::label(&s.steam_id);
        match took {
            Some(t) => logbus::info("direct", format!("{who} connected to Steam in {} ms", t.as_millis())),
            None => logbus::info("direct", format!("{who} connected to Steam")),
        }
        if let Err(e) = s.push_persona().await {
            logbus::warn("direct", format!("{who}: {e}"));
        }
        if let Err(e) = s.push_games().await {
            logbus::warn("direct", format!("{who}: {e}"));
        }

        let me = self.clone();
        tokio::spawn(async move {
            tokio::pin!(logged_off);
            loop {
                tokio::select! {
                    res = &mut logged_off => {
                        if s.generation.load(Ordering::SeqCst) != gen {
                            return;
                        }
                        *s.conn.lock().await = None;
                        let reason = match res {
                            Ok(m) => format!("Steam logged this session off ({})", eresult_name(m.eresult())),
                            Err(_) => "Connection to Steam lost".to_string(),
                        };
                        logbus::warn("direct", format!("{who}: {reason}"));
                        if s.wanted.load(Ordering::SeqCst) {
                            s.set_status("reconnecting", reason);
                            me.schedule_reconnect(s.clone(), gen, 0, Next::Soon);
                        } else {
                            s.set_status("offline", "");
                        }
                        return;
                    }
                    Some(Ok(state)) = playing.next() => {
                        if s.generation.load(Ordering::SeqCst) != gen {
                            return;
                        }
                        let blocked = state.playing_blocked();
                        let was = std::mem::replace(&mut s.state.lock().unwrap().blocked, blocked);
                        if blocked {
                            if !was {
                                logbus::info("direct", format!("{who}: paused, the account is playing a game elsewhere"));
                            }
                            s.set_status("blocked", "Paused: this account is playing a game elsewhere");
                        } else if was {
                            logbus::info("direct", format!("{who}: the game elsewhere ended; idling goes on"));
                            s.set_status("online", "");
                            if let Err(e) = s.push_games().await {
                                logbus::warn("direct", format!("{who}: {e}"));
                            }
                        }
                    }
                }
            }
        });
    }

    fn schedule_reconnect(self: &Arc<Self>, s: Arc<CmSession>, gen: u64, attempt: usize, next: Next) {
        // One chain of attempts at a time: a failed connect while one runs adds none.
        if attempt == 0 && s.reconnecting.swap(true, Ordering::SeqCst) {
            logbus::debug("direct", format!("{}: a reconnect is already planned", logbus::label(&s.steam_id)));
            return;
        }
        let delay = match next {
            Next::Stop => {
                s.reconnecting.store(false, Ordering::SeqCst);
                return;
            }
            Next::Soon => RECONNECT_DELAYS[attempt.min(RECONNECT_DELAYS.len() - 1)],
            Next::Later => RATE_LIMIT_DELAYS[attempt.min(RATE_LIMIT_DELAYS.len() - 1)],
        };
        logbus::info(
            "direct",
            format!("{}: trying to reconnect in {} (attempt {})", logbus::label(&s.steam_id), wait_words(delay), attempt + 1),
        );
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay)).await;
            let next = me.reconnect_once(&s, gen, attempt).await;
            if next == Next::Stop {
                s.reconnecting.store(false, Ordering::SeqCst);
            } else {
                // The chain goes on: the flag stays set for the next attempt.
                me.schedule_reconnect(s, gen, attempt + 1, next);
            }
        });
    }

    /// One reconnect attempt, and what should follow it.
    async fn reconnect_once(self: &Arc<Self>, s: &Arc<CmSession>, gen: u64, attempt: usize) -> Next {
        let who = logbus::label(&s.steam_id);
        if !s.wanted.load(Ordering::SeqCst) || s.generation.load(Ordering::SeqCst) != gen {
            logbus::debug("direct", format!("{who}: reconnect no longer needed"));
            return Next::Stop;
        }
        let _opening = s.opening.lock().await;
        if s.conn.lock().await.is_some() {
            logbus::debug("direct", format!("{who}: already connected again"));
            return Next::Stop;
        }
        let Some(token) = secrets::get(&secrets::refresh_token_key(&s.steam_id)) else {
            logbus::warn("direct", format!("{who}: no saved sign-in to reconnect with; sign in again"));
            s.set_status("error", "No saved sign-in for this account — sign in again");
            return Next::Stop;
        };
        let account_name = s.account_name.lock().unwrap().clone();
        s.set_status("reconnecting", format!("Reconnecting (attempt {})…", attempt + 1));
        logbus::info("direct", format!("{who}: reconnecting to Steam (attempt {})", attempt + 1));
        let started = Instant::now();
        match self.open(&who, &account_name, &token).await {
            Ok(conn) => {
                self.install(s.clone(), conn, Some(started.elapsed())).await;
                Next::Stop
            }
            Err(e) => {
                let msg = describe(&e);
                match refusal(&e) {
                    Refusal::SignIn => {
                        s.wanted.store(false, Ordering::SeqCst);
                        logbus::warn("direct", format!("{who}: Steam refused the saved sign-in ({msg}); sign in again"));
                        s.set_status("error", format!("Steam refused the saved sign-in ({msg}). Sign in again."));
                        Next::Stop
                    }
                    Refusal::RateLimited => {
                        logbus::warn("direct", format!("{who}: reconnect failed: {msg}"));
                        s.set_status("reconnecting", format!("{msg}. IdleKit tries again later."));
                        Next::Later
                    }
                    Refusal::Trouble => {
                        logbus::warn("direct", format!("{who}: reconnect failed: {msg}"));
                        Next::Soon
                    }
                }
            }
        }
    }

    pub async fn disconnect(&self, steam_id: &str) {
        let Some(s) = self.sessions.lock().unwrap().get(steam_id).cloned() else { return };
        s.wanted.store(false, Ordering::SeqCst);
        s.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(conn) = s.conn.lock().await.take() {
            let signed_off = tokio::time::timeout(SEND_TIMEOUT, conn.send(CMsgClientLogOff::new())).await;
            let who = logbus::label(steam_id);
            match signed_off {
                Ok(Ok(())) => logbus::info("direct", format!("{who}: signed off from Steam")),
                Ok(Err(e)) => logbus::info("direct", format!("{who}: closed the connection to Steam (signing off failed: {e})")),
                Err(_) => logbus::info("direct", format!("{who}: closed the connection to Steam (signing off took too long)")),
            }
        }
        s.set_status("offline", "");
    }

    pub async fn disconnect_all(&self) {
        let ids: Vec<String> = self.sessions.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.disconnect(&id).await;
        }
    }

    pub fn forget(&self, steam_id: &str) {
        self.sessions.lock().unwrap().remove(steam_id);
    }

    pub async fn set_games(&self, steam_id: &str, games: Vec<u32>) -> Result<()> {
        let s = self.session(steam_id);
        s.state.lock().unwrap().games = games;
        logbus::emit("cm://status", s.snapshot());
        s.push_games().await
    }

    pub async fn set_persona(&self, steam_id: &str, persona_state: u32) -> Result<()> {
        let s = self.session(steam_id);
        s.state.lock().unwrap().persona_state = persona_state;
        logbus::emit("cm://status", s.snapshot());
        s.push_persona().await
    }

    pub async fn set_status_text(&self, steam_id: &str, text: &str) -> Result<()> {
        let s = self.session(steam_id);
        s.state.lock().unwrap().status_text = text.to_string();
        s.push_games().await
    }

    pub async fn owned_games(&self, steam_id: &str) -> Result<Vec<OwnedGame>> {
        let conn = self
            .connection(steam_id)
            .await
            .ok_or_else(|| anyhow!("not connected"))?;
        let mut req = CPlayer_GetOwnedGames_Request::new();
        req.set_steamid(steam_id.parse()?);
        req.set_include_appinfo(true);
        req.set_include_played_free_games(true);
        let resp = conn
            .service_method(req)
            .await
            .map_err(|e| anyhow!("GetOwnedGames failed: {e}"))?;
        Ok(resp
            .games
            .iter()
            .map(|g| OwnedGame {
                appid: g.appid() as u32,
                name: g.name().to_string(),
                playtime_minutes: g.playtime_forever().max(0) as u32,
                icon_hash: g.img_icon_url().to_string(),
                has_stats: g.has_community_visible_stats(),
                last_played: g.rtime_last_played(),
            })
            .collect())
    }

    /// A new web sign-in (access token) for an account, made from its saved
    /// sign-in over a connection to Steam that is signed in with it: the
    /// only way Steam still makes one (see [`generate_web_token`]).
    ///
    /// A direct account's own connection is used, waited for while it comes
    /// up: two connections of IdleKit's with the same account push each other
    /// off. Any other account gets a short connection of its own, signed off
    /// as soon as the answer is in; it touches nothing else (no status, no
    /// reconnects), and the Steam client on this PC keeps its own session.
    pub async fn mint_web_token(&self, steam_id: &str, account_name: &str, refresh_token: &str) -> Result<String, MintError> {
        let who = logbus::label(steam_id);
        let own = self.sessions.lock().unwrap().get(steam_id).cloned().filter(|s| s.wanted.load(Ordering::SeqCst));
        if let Some(s) = own {
            let waited = Instant::now();
            loop {
                if let Some(conn) = s.current().await {
                    if waited.elapsed() >= Duration::from_millis(500) {
                        logbus::debug("web", format!("{who}: the account's connection came up after {} ms", waited.elapsed().as_millis()));
                    }
                    logbus::debug("web", format!("{who}: asking for a web sign-in over the account's own connection"));
                    return generate_web_token(&conn, steam_id, refresh_token).await;
                }
                if !s.wanted.load(Ordering::SeqCst) {
                    let status = s.snapshot();
                    return Err(MintError::trouble(format!(
                        "this account's connection to Steam stopped ({})",
                        if status.message.is_empty() { status.status } else { status.message }
                    )));
                }
                if waited.elapsed() >= OWN_CONNECTION_WAIT {
                    let status = s.snapshot();
                    return Err(MintError::trouble(format!(
                        "this account's connection to Steam didn't come up within {} s ({})",
                        OWN_CONNECTION_WAIT.as_secs(),
                        if status.message.is_empty() { status.status } else { status.message }
                    )));
                }
                if waited.elapsed() < Duration::from_millis(300) {
                    logbus::debug("web", format!("{who}: waiting for the account's connection to Steam to come up"));
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        logbus::debug("web", format!("{who}: opening a short connection to Steam for a new web sign-in"));
        let conn = self
            .open(&who, account_name, refresh_token)
            .await
            .map_err(|e| MintError { refusal: refusal(&e), message: describe(&e) })?;
        let token = generate_web_token(&conn, steam_id, refresh_token).await;
        // Signed off whatever the answer: a session left open counts against the account.
        match tokio::time::timeout(SEND_TIMEOUT, conn.send(CMsgClientLogOff::new())).await {
            Ok(Ok(())) => logbus::debug("web", format!("{who}: signed off the short connection")),
            Ok(Err(e)) => logbus::debug("web", format!("{who}: closed the short connection (signing off failed: {e})")),
            Err(_) => logbus::debug("web", format!("{who}: closed the short connection (signing off took too long)")),
        }
        token
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A client for which every request fails at once: no network at all.
    fn no_network() -> reqwest::Client {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap()).build().unwrap()
    }

    fn file(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("idlekit-cm-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn save(path: &Path, age_secs: i64, ws: &str) {
        let saved = SavedServers {
            at: chrono::Utc::now().timestamp() - age_secs,
            tcp: vec!["127.0.0.1:27017".parse().unwrap()],
            ws: vec![ws.to_string()],
        };
        std::fs::write(path, serde_json::to_vec(&saved).unwrap()).unwrap();
    }

    fn fetched_at(cm: &CmManager) -> Option<Instant> {
        cm.servers.lock().unwrap().as_ref().and_then(|(_, at)| *at)
    }

    #[tokio::test]
    async fn steams_server_list_is_kept_and_used_when_the_directory_is_down() {
        // Nothing kept and no network: says why, as a failure that is tried again later.
        let cm = CmManager::new(Some(file("none")), no_network());
        let e = cm.servers().await.unwrap_err();
        assert!(e.to_string().contains("Couldn't get Steam's server list"), "{e}");
        assert_eq!(refusal(&e), Refusal::Trouble);

        // Kept ten minutes ago: used as it is, without asking.
        let fresh = file("fresh");
        save(&fresh, 600, "127.0.0.1:27018");
        let cm = CmManager::new(Some(fresh.clone()), no_network());
        assert_eq!(cm.servers().await.unwrap().ws_servers(), vec!["127.0.0.1:27018".to_string()]);
        assert!(fetched_at(&cm).is_some_and(|at| at.elapsed() >= Duration::from_secs(590)), "counted as fresh");

        // Kept two days ago: Steam's directory is asked first, and when it
        // doesn't answer, the old list still connects.
        let old = file("old");
        save(&old, 2 * 86_400, "127.0.0.1:27019");
        let cm = CmManager::new(Some(old.clone()), no_network());
        assert_eq!(cm.servers().await.unwrap().ws_servers(), vec!["127.0.0.1:27019".to_string()]);
        assert_eq!(fetched_at(&cm), None, "not counted as fresh");
        for path in [fresh, old] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn a_connection_that_never_answers_gives_up_and_is_tried_again_later() {
        // A server that takes the connection and then says nothing at all.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        let path = file("silent");
        save(&path, 0, &addr.to_string());
        let cm = CmManager::new(Some(path.clone()), no_network());
        let started = Instant::now();
        let e = cm.open_within(Duration::from_millis(800), "account …0001", "someone", "token").await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5), "gave up after {:?}", started.elapsed());
        assert_eq!(refusal(&e), Refusal::Trouble, "a connection that hangs is never a refused sign-in: {e}");
        assert!(e.to_string().contains("didn't answer"), "{e}");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn only_a_refused_sign_in_asks_for_signing_in_again() {
        let login = |r: EResult| ConnectionError::LoginError(LoginError::Unknown(r));
        // What Steam answers for a saved sign-in it threw away, as in the log
        // that showed the 2.2 bug: "Login failed: unknown error AccessDenied".
        for r in [EResult::AccessDenied, EResult::Revoked, EResult::Expired, EResult::AccountLogonDenied] {
            assert_eq!(refusal(&login(r)), Refusal::SignIn, "{r:?}");
        }
        assert_eq!(refusal(&ConnectionError::LoginError(LoginError::InvalidCredentials)), Refusal::SignIn);
        // Steam's "busy" and "try another server" answers only need another try.
        for r in [EResult::TryAnotherCM, EResult::ServiceUnavailable, EResult::Busy, EResult::Timeout, EResult::Fail] {
            assert_eq!(refusal(&login(r)), Refusal::Trouble, "{r:?}");
        }
        // Limits on sign-ins: try again, but much later.
        assert_eq!(refusal(&ConnectionError::LoginError(LoginError::RateLimited)), Refusal::RateLimited);
        assert_eq!(refusal(&login(EResult::AccountLoginDeniedThrottle)), Refusal::RateLimited);
        // No answer at all is never the sign-in's fault.
        assert_eq!(refusal(&network_error("reset")), Refusal::Trouble);
        // The words name Steam's own answer, or say plainly what went wrong.
        assert_eq!(describe(&login(EResult::AccessDenied)), "Steam answered AccessDenied");
        let refused = ConnectionError::Network(NetworkError::IO(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)));
        assert!(describe(&refused).starts_with("couldn't reach Steam's servers ("), "{}", describe(&refused));
        assert_eq!(describe(&network_error("Steam didn't answer within 30 seconds")), "Steam didn't answer within 30 seconds");
    }

    #[test]
    fn waits_read_as_words() {
        assert_eq!(wait_words(5), "5 s");
        assert_eq!(wait_words(120), "2 min");
        assert_eq!(wait_words(15 * 60), "15 min");
        assert_eq!(wait_words(3600), "60 min");
        assert_eq!(wait_words(2 * 3600), "2 h");
    }

    #[tokio::test]
    async fn a_web_sign_in_for_an_account_without_a_connection_never_waits_for_one() {
        // No direct connection is wanted for this account, so a short
        // connection of its own is tried at once; with no network that
        // fails quickly, as trouble rather than as a refused sign-in.
        let path = file("mint");
        let cm = CmManager::new(Some(path.clone()), no_network());
        let started = Instant::now();
        let e = cm.mint_web_token("76561197960265729", "someone", "token").await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
        assert_eq!(e.refusal, Refusal::Trouble, "{e}");
        let _ = std::fs::remove_file(path);
    }
}
