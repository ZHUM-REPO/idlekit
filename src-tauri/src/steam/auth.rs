//! Interactive sign-in: QR code (via Steam's HTTPS auth API) and
//! username/password + Steam Guard (via a CM connection, using steam-vent).
//! Both produce a long-lived refresh token, stored with `secrets`.
use super::{cm, jwt, webapi};
use crate::{logbus, secrets};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use steam_vent::auth::{
    AuthConfirmationHandler, ConfirmationAction, ConfirmationMethod, ConfirmationMethodClass, GuardDataStore,
    UserProvidedAuthConfirmationHandler,
};
use steam_vent::connection::UnAuthenticatedConnection;
use steam_vent::{Connection, ConnectionError};
use tokio::io::{AsyncWriteExt, DuplexStream};

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthEvent {
    pub session_id: String,
    /// "qr" | "scanned" | "code" | "confirm" | "working" | "done" | "error" | "expired"
    pub status: String,
    pub challenge_url: Option<String>,
    /// For "code": "email" | "device"
    pub code_type: Option<String>,
    pub hint: Option<String>,
    /// For "code": the user may also approve in the Steam mobile app.
    pub can_confirm: bool,
    pub message: Option<String>,
    pub steam_id: Option<String>,
    pub account_name: Option<String>,
}

/// A finished sign-in, handed to the app to create/update the account.
pub struct SignedIn {
    pub steam_id: String,
    pub account_name: String,
    pub refresh_token: String,
    pub access_token: Option<String>,
    /// Live CM connection from a password sign-in (reused for direct mode).
    pub connection: Option<Connection>,
}

struct Ctl {
    code_tx: Option<Arc<tokio::sync::Mutex<DuplexStream>>>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
pub struct AuthSessions {
    map: Mutex<HashMap<String, Ctl>>,
}

fn emit(ev: AuthEvent) {
    logbus::emit("auth://status", ev);
}

fn ev(session_id: &str, status: &str) -> AuthEvent {
    AuthEvent {
        session_id: session_id.to_string(),
        status: status.to_string(),
        ..Default::default()
    }
}

struct SecretGuardStore;

impl GuardDataStore for SecretGuardStore {
    type Err = std::convert::Infallible;

    async fn store(&mut self, account: &str, machine_token: String) -> Result<(), Self::Err> {
        let _ = secrets::set(&secrets::guard_key(account), &machine_token);
        Ok(())
    }

    async fn load(&mut self, account: &str) -> Result<Option<String>, Self::Err> {
        Ok(secrets::get(&secrets::guard_key(account)))
    }
}

/// Tells the UI what Steam Guard wants, then waits for a code (if one is needed).
/// Approval in the mobile app is picked up by steam-vent's polling in parallel.
struct UiConfirmationHandler {
    session_id: String,
    inner: UserProvidedAuthConfirmationHandler<DuplexStream, tokio::io::Sink>,
}

impl AuthConfirmationHandler for UiConfirmationHandler {
    async fn handle_confirmation(self, allowed: &[ConfirmationMethod]) -> Option<ConfirmationAction> {
        let code = allowed.iter().find(|m| m.token_type().is_some());
        let can_confirm = allowed
            .iter()
            .any(|m| m.class() == ConfirmationMethodClass::Confirmation);
        if let Some(method) = code {
            let device = method.confirmation_type() == "device code";
            emit(AuthEvent {
                code_type: Some(if device { "device" } else { "email" }.into()),
                hint: Some(method.confirmation_details().to_string()).filter(|s| !s.is_empty()),
                can_confirm,
                ..ev(&self.session_id, "code")
            });
            self.inner.handle_confirmation(allowed).await
        } else if can_confirm {
            emit(AuthEvent { can_confirm: true, ..ev(&self.session_id, "confirm") });
            Some(ConfirmationAction::None)
        } else {
            None
        }
    }
}

fn friendly_login_error(e: &ConnectionError) -> String {
    let text = e.to_string();
    match e {
        ConnectionError::LoginError(steam_vent::LoginError::InvalidCredentials) => {
            "Wrong username or password.".into()
        }
        ConnectionError::LoginError(steam_vent::LoginError::RateLimited) => {
            "Steam is rate-limiting sign-ins from this network. Wait a while and try again.".into()
        }
        ConnectionError::Aborted => "Sign-in cancelled.".into(),
        _ if text.contains("TwoFactorCodeMismatch") || text.contains("InvalidLoginAuthCode") => {
            "That Steam Guard code was not accepted.".into()
        }
        _ => text,
    }
}

impl AuthSessions {
    /// Whether a sign-in is under way: a QR code on screen, or a password
    /// sign-in waiting for its Steam Guard code.
    pub fn in_progress(&self) -> bool {
        !self.map.lock().unwrap().is_empty()
    }

    pub fn cancel(&self, session_id: &str) {
        if let Some(ctl) = self.map.lock().unwrap().remove(session_id) {
            ctl.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub async fn submit_code(&self, session_id: &str, code: &str) -> Result<()> {
        let tx = self
            .map
            .lock()
            .unwrap()
            .get(session_id)
            .and_then(|c| c.code_tx.clone())
            .ok_or_else(|| anyhow::anyhow!("This sign-in is no longer waiting for a code"))?;
        let mut w = tx.lock().await;
        w.write_all(format!("{}\n", code.trim()).as_bytes()).await?;
        w.flush().await?;
        emit(ev(session_id, "working"));
        Ok(())
    }

    /// QR sign-in. `on_done` runs when the user approves in the Steam app.
    pub fn start_qr<F>(self: &Arc<Self>, http: reqwest::Client, session_id: String, on_done: F)
    where
        F: FnOnce(SignedIn) -> futures_util::future::BoxFuture<'static, Result<()>> + Send + 'static,
    {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.map.lock().unwrap().insert(
            session_id.clone(),
            Ctl { code_tx: None, cancelled: cancelled.clone() },
        );
        let me = self.clone();
        tokio::spawn(async move {
            let result = qr_flow(&http, &session_id, &cancelled).await;
            me.map.lock().unwrap().remove(&session_id);
            match result {
                Ok(Some(done)) => {
                    let sid = done.steam_id.clone();
                    let name = done.account_name.clone();
                    match on_done(done).await {
                        Ok(()) => emit(AuthEvent {
                            steam_id: Some(sid),
                            account_name: Some(name),
                            ..ev(&session_id, "done")
                        }),
                        Err(e) => emit(AuthEvent { message: Some(format!("{e:#}")), ..ev(&session_id, "error") }),
                    }
                }
                Ok(None) => {}
                Err(e) => emit(AuthEvent { message: Some(format!("{e:#}")), ..ev(&session_id, "error") }),
            }
        });
    }

    /// Username + password sign-in with Steam Guard handled through the UI.
    pub fn start_credentials<F>(
        self: &Arc<Self>,
        cm: Arc<cm::CmManager>,
        session_id: String,
        username: String,
        password: String,
        on_done: F,
    ) where
        F: FnOnce(SignedIn) -> futures_util::future::BoxFuture<'static, Result<()>> + Send + 'static,
    {
        let (code_tx, code_rx) = tokio::io::duplex(256);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.map.lock().unwrap().insert(
            session_id.clone(),
            Ctl {
                code_tx: Some(Arc::new(tokio::sync::Mutex::new(code_tx))),
                cancelled: cancelled.clone(),
            },
        );
        let me = self.clone();
        tokio::spawn(async move {
            emit(ev(&session_id, "working"));
            let handler = UiConfirmationHandler {
                session_id: session_id.clone(),
                inner: UserProvidedAuthConfirmationHandler::new(code_rx, tokio::io::sink()),
            };
            let attempt = async {
                // Reaching Steam gets its own time limit: the 10 minutes below
                // are for finding a Steam Guard code, not for a connection that hangs.
                let reach = async {
                    let servers = cm.servers().await?;
                    UnAuthenticatedConnection::connect(&servers).await
                };
                let unauth = tokio::time::timeout(cm::OPEN_TIMEOUT, reach)
                    .await
                    .map_err(|_| anyhow::anyhow!("Could not reach Steam: no answer within {} seconds", cm::OPEN_TIMEOUT.as_secs()))?
                    .map_err(|e| anyhow::anyhow!("Could not reach Steam: {e}"))?;
                unauth
                    .login(&username, &password, SecretGuardStore, handler)
                    .await
                    .map_err(|e| anyhow::anyhow!(friendly_login_error(&e)))
            };
            // Give the user up to 10 minutes to find their code.
            let result = tokio::time::timeout(Duration::from_secs(600), attempt).await;
            me.map.lock().unwrap().remove(&session_id);
            if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let conn = match result {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => {
                    emit(AuthEvent { message: Some(e.to_string()), ..ev(&session_id, "error") });
                    return;
                }
                Err(_) => {
                    emit(AuthEvent { message: Some("Sign-in timed out.".into()), ..ev(&session_id, "error") });
                    return;
                }
            };
            let steam_id = u64::from(conn.steam_id()).to_string();
            // steam-vent logs on with the refresh token, so this is the refresh token.
            let refresh_token = conn.access_token().unwrap_or_default().to_string();
            let done = SignedIn {
                steam_id: steam_id.clone(),
                account_name: username.clone(),
                refresh_token,
                access_token: None,
                connection: Some(conn),
            };
            match on_done(done).await {
                Ok(()) => emit(AuthEvent {
                    steam_id: Some(steam_id),
                    account_name: Some(username),
                    ..ev(&session_id, "done")
                }),
                Err(e) => emit(AuthEvent { message: Some(format!("{e:#}")), ..ev(&session_id, "error") }),
            }
        });
    }
}

async fn qr_flow(
    http: &reqwest::Client,
    session_id: &str,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<Option<SignedIn>> {
    use std::sync::atomic::Ordering;
    let start = webapi::begin_qr(http).await?;
    let mut client_id = start.client_id;
    let request_id = start.request_id;
    let interval = Duration::from_secs_f32(start.interval.clamp(1.0, 10.0));
    emit(AuthEvent { challenge_url: Some(start.challenge_url), ..ev(session_id, "qr") });
    let mut scanned = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        tokio::time::sleep(interval).await;
        if cancelled.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if tokio::time::Instant::now() > deadline {
            emit(ev(session_id, "expired"));
            return Ok(None);
        }
        let poll = match webapi::poll_auth(http, client_id, &request_id).await {
            Ok(p) => p,
            Err(e) => {
                if let Some(api) = e.downcast_ref::<webapi::SteamApiError>() {
                    // Expired / FileNotFound: the QR session is gone.
                    if matches!(api.eresult, 9 | 27 | 29) {
                        emit(ev(session_id, "expired"));
                        return Ok(None);
                    }
                }
                logbus::debug("auth", format!("QR poll failed: {e:#}"));
                continue;
            }
        };
        if let Some(new_id) = poll.new_client_id {
            client_id = new_id;
        }
        if let Some(url) = poll.new_challenge_url {
            emit(AuthEvent { challenge_url: Some(url), ..ev(session_id, "qr") });
        }
        if poll.had_remote_interaction && !scanned {
            scanned = true;
            emit(ev(session_id, "scanned"));
        }
        if let Some(refresh_token) = poll.refresh_token {
            let claims = jwt::decode(&refresh_token).unwrap_or_default();
            if let (Some(guard), Some(name)) = (&poll.new_guard_data, &poll.account_name) {
                let _ = secrets::set(&secrets::guard_key(name), guard);
            }
            return Ok(Some(SignedIn {
                steam_id: claims.sub,
                account_name: poll.account_name.unwrap_or_default(),
                refresh_token,
                access_token: poll.access_token,
                connection: None,
            }));
        }
    }
}
