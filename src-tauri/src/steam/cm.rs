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
use steam_vent::{Connection, ConnectionError, ConnectionTrait, DiscoverOptions, NetworkError, ServerList};
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

/// A connection that failed on the way: never a reason to give up on the saved sign-in.
fn network_error(text: impl Into<String>) -> ConnectionError {
    ConnectionError::Network(NetworkError::IO(std::io::Error::new(std::io::ErrorKind::TimedOut, text.into())))
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

    async fn push_games(&self) -> Result<()> {
        if self.state.lock().unwrap().blocked {
            return Ok(());
        }
        let msg = self.games_msg();
        let guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            conn.send_with_kind(msg, EMsg::k_EMsgClientGamesPlayedWithDataBlob)
                .await
                .map_err(|e| anyhow!("could not update games: {e}"))?;
        }
        Ok(())
    }

    async fn push_persona(&self) -> Result<()> {
        let state = self.state.lock().unwrap().persona_state;
        let mut msg = CMsgClientChangeStatus::new();
        msg.set_persona_state(state);
        let guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            conn.send(msg).await.map_err(|e| anyhow!("could not set status: {e}"))?;
        }
        Ok(())
    }
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

fn is_auth_error(e: &ConnectionError) -> bool {
    matches!(e, ConnectionError::LoginError(err) if !matches!(err, steam_vent::LoginError::RateLimited))
        || matches!(e, ConnectionError::AccessToken(_))
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
                return Ok(list.clone());
            }
        }
        let options = DiscoverOptions::default().with_web_client(self.http.clone());
        match ServerList::discover_with(options).await {
            Ok(list) => {
                self.save_servers(&list);
                *self.servers.lock().unwrap() = Some((list.clone(), Some(Instant::now())));
                Ok(list)
            }
            Err(e) => match known {
                Some((list, _)) => {
                    logbus::debug("direct", format!("Couldn't get Steam's server list ({e}); using the one from before"));
                    Ok(list)
                }
                None => Err(network_error(format!("Couldn't get Steam's server list: {e}"))),
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
    async fn open(&self, account_name: &str, refresh_token: &str) -> Result<Connection, ConnectionError> {
        self.open_within(OPEN_TIMEOUT, account_name, refresh_token).await
    }

    async fn open_within(&self, limit: Duration, account_name: &str, refresh_token: &str) -> Result<Connection, ConnectionError> {
        let attempt = async {
            let servers = self.servers().await?;
            let unauth = UnAuthenticatedConnection::connect(&servers).await?;
            unauth.access(account_name, refresh_token).await
        };
        tokio::time::timeout(limit, attempt)
            .await
            .unwrap_or_else(|_| Err(network_error(format!("Steam didn't answer within {} seconds", limit.as_secs()))))
    }

    /// Connect (or reconnect) a direct account using its saved refresh token.
    pub async fn connect(self: &Arc<Self>, steam_id: &str) -> Result<()> {
        let s = self.session(steam_id);
        s.wanted.store(true, Ordering::SeqCst);
        if s.conn.lock().await.is_some() {
            return Ok(());
        }
        let _opening = s.opening.lock().await;
        // Someone else may have connected while this call waited.
        if s.conn.lock().await.is_some() {
            return Ok(());
        }
        let token = secrets::get(&secrets::refresh_token_key(steam_id))
            .ok_or_else(|| anyhow!("No saved sign-in for this account — sign in again"))?;
        let account_name = s.account_name.lock().unwrap().clone();
        s.set_status("connecting", "");
        match self.open(&account_name, &token).await {
            Ok(conn) => {
                self.install(s.clone(), conn).await;
                Ok(())
            }
            Err(e) => {
                let msg = format!("{e}");
                if is_auth_error(&e) {
                    s.wanted.store(false, Ordering::SeqCst);
                    s.set_status("error", format!("Steam rejected the saved sign-in ({msg}). Sign in again."));
                } else {
                    s.set_status("error", msg.clone());
                    self.schedule_reconnect(s.clone(), s.generation.load(Ordering::SeqCst), 0);
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
        self.install(s, conn).await;
    }

    async fn install(self: &Arc<Self>, s: Arc<CmSession>, conn: Connection) {
        let gen = s.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let logged_off = conn.one::<CMsgClientLoggedOff>();
        let mut playing = conn.on::<CMsgClientPlayingSessionState>();
        *s.conn.lock().await = Some(conn);
        s.state.lock().unwrap().blocked = false;
        s.set_status("online", "");
        logbus::info("direct", format!("{} connected to Steam", logbus::label(&s.steam_id)));
        if let Err(e) = s.push_persona().await {
            logbus::warn("direct", e.to_string());
        }
        if let Err(e) = s.push_games().await {
            logbus::warn("direct", e.to_string());
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
                        logbus::warn("direct", format!("{}: {reason}", logbus::label(&s.steam_id)));
                        if s.wanted.load(Ordering::SeqCst) {
                            s.set_status("reconnecting", reason);
                            me.schedule_reconnect(s.clone(), gen, 0);
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
                            s.set_status("blocked", "Paused: this account is playing a game elsewhere");
                        } else if was {
                            s.set_status("online", "");
                            let _ = s.push_games().await;
                        }
                    }
                }
            }
        });
    }

    fn schedule_reconnect(self: &Arc<Self>, s: Arc<CmSession>, gen: u64, attempt: usize) {
        // One chain of attempts at a time: a failed connect while one runs adds none.
        if attempt == 0 && s.reconnecting.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            let delay = RECONNECT_DELAYS[attempt.min(RECONNECT_DELAYS.len() - 1)];
            tokio::time::sleep(Duration::from_secs(delay)).await;
            let again = me.reconnect_once(&s, gen, attempt).await;
            if again {
                me.schedule_reconnect(s, gen, attempt + 1);
            } else {
                s.reconnecting.store(false, Ordering::SeqCst);
            }
        });
    }

    /// One reconnect attempt; whether another one should follow.
    async fn reconnect_once(self: &Arc<Self>, s: &Arc<CmSession>, gen: u64, attempt: usize) -> bool {
        if !s.wanted.load(Ordering::SeqCst) || s.generation.load(Ordering::SeqCst) != gen {
            return false;
        }
        let _opening = s.opening.lock().await;
        if s.conn.lock().await.is_some() {
            return false;
        }
        let Some(token) = secrets::get(&secrets::refresh_token_key(&s.steam_id)) else {
            s.set_status("error", "No saved sign-in for this account — sign in again");
            return false;
        };
        let account_name = s.account_name.lock().unwrap().clone();
        s.set_status("reconnecting", format!("Reconnecting (attempt {})…", attempt + 1));
        match self.open(&account_name, &token).await {
            Ok(conn) => {
                self.install(s.clone(), conn).await;
                false
            }
            Err(e) if is_auth_error(&e) => {
                s.wanted.store(false, Ordering::SeqCst);
                s.set_status("error", format!("Steam rejected the saved sign-in ({e}). Sign in again."));
                false
            }
            Err(e) => {
                logbus::warn("direct", format!("{}: reconnect failed: {e}", logbus::label(&s.steam_id)));
                true
            }
        }
    }

    pub async fn disconnect(&self, steam_id: &str) {
        let Some(s) = self.sessions.lock().unwrap().get(steam_id).cloned() else { return };
        s.wanted.store(false, Ordering::SeqCst);
        s.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(conn) = s.conn.lock().await.take() {
            let _ = conn.send(CMsgClientLogOff::new()).await;
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

    pub async fn access_token(&self, steam_id: &str, refresh_token: &str) -> Result<String> {
        let conn = self
            .connection(steam_id)
            .await
            .ok_or_else(|| anyhow!("not connected"))?;
        let mut req = CAuthentication_AccessToken_GenerateForApp_Request::new();
        req.set_refresh_token(refresh_token.to_string());
        req.set_steamid(steam_id.parse()?);
        req.set_renewal_type(ETokenRenewalType::k_ETokenRenewalType_None);
        let resp = conn
            .service_method(req)
            .await
            .map_err(|e| anyhow!("GenerateAccessTokenForApp failed: {e}"))?;
        let token = resp.access_token().to_string();
        if token.is_empty() {
            return Err(anyhow!("Steam returned an empty access token"));
        }
        Ok(token)
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
        assert!(!is_auth_error(&e));

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
        let e = cm.open_within(Duration::from_millis(800), "someone", "token").await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5), "gave up after {:?}", started.elapsed());
        assert!(!is_auth_error(&e), "a connection that hangs is never a rejected sign-in: {e}");
        assert!(e.to_string().contains("didn't answer"), "{e}");
        let _ = std::fs::remove_file(path);
    }
}
