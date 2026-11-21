//! Direct connections to Steam's connection managers (no Steam client needed).
//! Used for "direct" accounts: idling up to 32 games via ClientGamesPlayed,
//! online status, owned-games lookup and access-token refresh.
use super::webapi::{eresult_name, OwnedGame};
use crate::{logbus, secrets};
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steam_vent::connection::UnAuthenticatedConnection;
use steam_vent::{Connection, ConnectionError, ConnectionTrait, ServerList};
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

#[derive(Default)]
pub struct CmManager {
    sessions: Mutex<HashMap<String, Arc<CmSession>>>,
}

fn is_auth_error(e: &ConnectionError) -> bool {
    matches!(e, ConnectionError::LoginError(err) if !matches!(err, steam_vent::LoginError::RateLimited))
        || matches!(e, ConnectionError::AccessToken(_))
}

impl CmManager {
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

    async fn open(account_name: &str, refresh_token: &str) -> Result<Connection, ConnectionError> {
        let servers = ServerList::discover()
            .await
            .map_err(|e| ConnectionError::Network(steam_vent::NetworkError::IO(std::io::Error::other(e.to_string()))))?;
        let unauth = UnAuthenticatedConnection::connect(&servers).await?;
        unauth.access(account_name, refresh_token).await
    }

    /// Connect (or reconnect) a direct account using its saved refresh token.
    pub async fn connect(self: &Arc<Self>, steam_id: &str) -> Result<()> {
        let s = self.session(steam_id);
        s.wanted.store(true, Ordering::SeqCst);
        if s.conn.lock().await.is_some() {
            return Ok(());
        }
        let token = secrets::get(&secrets::refresh_token_key(steam_id))
            .ok_or_else(|| anyhow!("No saved sign-in for this account — sign in again"))?;
        let account_name = s.account_name.lock().unwrap().clone();
        s.set_status("connecting", "");
        match Self::open(&account_name, &token).await {
            Ok(conn) => {
                self.install(s, conn).await;
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
        let me = self.clone();
        tokio::spawn(async move {
            let delay = RECONNECT_DELAYS[attempt.min(RECONNECT_DELAYS.len() - 1)];
            tokio::time::sleep(Duration::from_secs(delay)).await;
            if !s.wanted.load(Ordering::SeqCst) || s.generation.load(Ordering::SeqCst) != gen {
                return;
            }
            if s.conn.lock().await.is_some() {
                return;
            }
            let Some(token) = secrets::get(&secrets::refresh_token_key(&s.steam_id)) else {
                s.set_status("error", "No saved sign-in for this account — sign in again");
                return;
            };
            let account_name = s.account_name.lock().unwrap().clone();
            s.set_status("reconnecting", format!("Reconnecting (attempt {})…", attempt + 1));
            match Self::open(&account_name, &token).await {
                Ok(conn) => me.install(s, conn).await,
                Err(e) if is_auth_error(&e) => {
                    s.wanted.store(false, Ordering::SeqCst);
                    s.set_status("error", format!("Steam rejected the saved sign-in ({e}). Sign in again."));
                }
                Err(e) => {
                    logbus::warn("direct", format!("{}: reconnect failed: {e}", logbus::label(&s.steam_id)));
                    me.schedule_reconnect(s, gen, attempt + 1);
                }
            }
        });
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
