//! Tauri commands: the API the React UI calls via `invoke`.
use crate::config::{Account, ImportedTiming, ManualGame, Settings};
use crate::desktop::{self, DesktopInfo};
use crate::features::farming::{FarmGame, FarmStatus, FarmingManager};
use crate::features::idler::{IdleSnapshot, StartResult};
use crate::features::unlocker::UnlockStatus;
use crate::logbus::{self, LogLine};
use crate::secrets;
use crate::state::{App, GameEntry};
use crate::steam::art::GameArt;
use crate::steam::auth::SignedIn;
use crate::steam::cm::{self, CmStatus};
use crate::steam::community::{self, WebSession};
use crate::steam::local::LocalSteam;
use crate::steam::market::{self, InvItem, Listing, PriceInfo, SellOutcome};
use crate::steam::pictures;
use crate::steam::pricing;
use crate::steam::schema;
use crate::updates::{self, UpdateState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;
use tauri::{AppHandle, State};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn anyhow_err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn new_id() -> String {
    format!("{:x}", rand::random::<u64>())
}

// ------------------------------------------------------------ bootstrap --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    #[serde(flatten)]
    pub account: Account,
    pub has_token: bool,
    pub has_cookie: bool,
    pub local_active: bool,
    /// Steam refused the saved sign-in during this run: signing in again is the only way back.
    pub sign_in_refused: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub version: String,
    pub data_dir: String,
    pub settings: Settings,
    pub accounts: Vec<AccountView>,
    pub active_account: Option<String>,
    pub local_steam: LocalSteam,
    pub helper_found: bool,
    pub cm: Vec<CmStatus>,
    pub idle: Vec<IdleSnapshot>,
    pub farming: Vec<FarmStatus>,
    pub unlocker: Vec<UnlockStatus>,
    pub logs: Vec<LogLine>,
    /// Where log.txt is, when it could be opened.
    pub log_file: Option<String>,
    pub desktop: DesktopInfo,
    pub update: UpdateState,
    /// The number of the moment this was read (see `logbus::emit`): events
    /// numbered up to it are already in here.
    pub seq: u64,
}

fn account_views(app: &App, local: &LocalSteam) -> Vec<AccountView> {
    app.with_config(|c| c.accounts.clone())
        .into_iter()
        .map(|a| AccountView {
            has_token: secrets::get(&secrets::refresh_token_key(&a.steam_id)).is_some(),
            sign_in_refused: app.sign_in_refused(&a.steam_id),
            has_cookie: secrets::get(&secrets::cookie_key(&a.steam_id)).is_some(),
            local_active: local.active_steam_id.as_deref() == Some(a.steam_id.as_str()),
            account: a,
        })
        .collect()
}

#[tauri::command]
pub async fn get_bootstrap(app: State<'_, App>, handle: AppHandle) -> CmdResult<Bootstrap> {
    // Taken first: every status read below is at least as new as this number.
    let (seq, logs) = logbus::snapshot();
    let local = app.local_steam();
    let (settings, active_account) = app.with_config(|c| (c.settings.clone(), c.active_account.clone()));
    Ok(Bootstrap {
        version: crate::updates::shown(env!("CARGO_PKG_VERSION")),
        data_dir: app.data_dir.to_string_lossy().into_owned(),
        accounts: account_views(&app, &local),
        settings,
        active_account,
        helper_found: app.helpers.helper_path().exists(),
        local_steam: local,
        cm: app.cm.statuses(),
        idle: app.idler.all(),
        farming: app.farming.all(),
        unlocker: app.unlocker.all(),
        logs,
        log_file: logbus::file_path().map(|p| p.display().to_string()),
        desktop: desktop::info(&handle),
        update: updates::snapshot(&app, &handle),
        seq,
    })
}

#[tauri::command]
pub async fn get_accounts(app: State<'_, App>) -> CmdResult<Vec<AccountView>> {
    let local = app.local_steam();
    Ok(account_views(&app, &local))
}

#[tauri::command]
pub async fn save_settings(app: State<'_, App>, settings: Settings) -> CmdResult<()> {
    let text = settings.idle_status_text.clone();
    let before = app.settings();
    let changed = before.idle_status_text != text;
    for change in crate::config::setting_changes(&before, &settings) {
        logbus::info("settings", change);
    }
    app.update_config(|c| c.settings = settings);
    if !changed {
        return Ok(());
    }
    // Apply the custom status text to connected direct accounts right away.
    let direct: Vec<String> = app.with_config(|c| c.accounts.iter().filter(|a| a.mode == "direct").map(|a| a.steam_id.clone()).collect());
    for steam_id in direct {
        if app.cm.is_online(&steam_id) {
            let _ = app.cm.set_status_text(&steam_id, &text).await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn set_active_account(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.account(&steam_id).map_err(anyhow_err)?;
    app.update_config(|c| c.active_account = Some(steam_id));
    Ok(())
}

#[tauri::command]
pub async fn detect_local_steam(app: State<'_, App>) -> CmdResult<LocalSteam> {
    Ok(app.local_steam())
}

/// Steam's upcoming sales and fests. `refresh` reads Valve's page again.
#[tauri::command]
pub async fn steam_events(app: State<'_, App>, refresh: bool) -> CmdResult<crate::steam::events::EventList> {
    Ok(app.steam_events(refresh).await)
}

#[tauri::command]
pub async fn get_logs() -> CmdResult<Vec<LogLine>> {
    Ok(logbus::recent())
}

#[tauri::command]
pub async fn clear_logs() -> CmdResult<()> {
    logbus::clear();
    Ok(())
}

/// A line from the window for the log: an action and how it ended, or a
/// fault in the interface. It is scrubbed like every other line.
#[tauri::command]
pub async fn log_event(level: String, scope: String, message: String) -> CmdResult<()> {
    logbus::from_window(&level, &scope, message);
    Ok(())
}

/// Opens the folder that holds log.txt, with the file selected.
#[tauri::command]
pub async fn open_log_file(handle: AppHandle) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let path = logbus::file_path().ok_or("There is no log file: it couldn't be created")?;
    handle.opener().reveal_item_in_dir(&path).map_err(|e| format!("Couldn't open the folder: {e}"))
}

// ------------------------------------------------------------- accounts --

fn upsert_account(app: &App, steam_id: &str, account_name: &str, persona: &str, mode: &str) {
    app.update_config(|c| {
        match c.account_mut(steam_id) {
            Some(a) => {
                if !account_name.is_empty() {
                    a.account_name = account_name.to_string();
                }
                if !persona.is_empty() && a.persona_name.is_empty() {
                    a.persona_name = persona.to_string();
                }
            }
            None => c.accounts.push(Account {
                steam_id: steam_id.to_string(),
                account_name: account_name.to_string(),
                persona_name: persona.to_string(),
                mode: mode.to_string(),
                added_at: chrono::Utc::now().timestamp(),
                ..Default::default()
            }),
        }
        if c.active_account.is_none() {
            c.active_account = Some(steam_id.to_string());
        }
    });
}

#[tauri::command]
pub async fn add_local_account(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    let local = app.local_steam();
    let user = local
        .users
        .iter()
        .find(|u| u.steam_id == steam_id)
        .ok_or("That account is not in this PC's Steam login list")?;
    upsert_account(&app, &steam_id, &user.account_name, &user.persona_name, "local");
    app.emit_accounts();
    let a = app.inner().clone();
    tokio::spawn(async move { a.fetch_profile(&steam_id).await });
    Ok(())
}

/// Build the completion callback shared by QR and password sign-ins.
fn on_signed_in(app: App, mode: String) -> impl FnOnce(SignedIn) -> futures_util::future::BoxFuture<'static, anyhow::Result<()>> {
    move |done: SignedIn| {
        Box::pin(async move {
            if done.steam_id.is_empty() || done.refresh_token.is_empty() {
                anyhow::bail!("Steam did not return a usable sign-in token");
            }
            let who = logbus::label(&done.steam_id);
            secrets::set(&secrets::refresh_token_key(&done.steam_id), &done.refresh_token)?;
            secrets::delete(&secrets::cookie_key(&done.steam_id));
            upsert_account(&app, &done.steam_id, &done.account_name, "", &mode);
            let account = app.account(&done.steam_id)?;
            app.forget_web_session(&done.steam_id).await;
            // A web session right away, kept for the next start: a QR sign-in
            // brings one; a password sign-in asks for one over its own
            // connection, which is signed in already.
            let web_token = match (&done.access_token, &done.connection) {
                (Some(token), _) => Some(token.clone()),
                (None, Some(conn)) => match cm::generate_web_token(conn, &done.steam_id, &done.refresh_token).await {
                    Ok(token) => Some(token),
                    Err(e) => {
                        logbus::warn("web", format!("{who}: no web session yet ({e}); one is made when it is needed"));
                        None
                    }
                },
                (None, None) => None,
            };
            if let Some(token) = &web_token {
                app.seed_web_session(&done.steam_id, token).await;
            }
            if account.mode == "direct" {
                let s = app.settings();
                app.cm.configure(&done.steam_id, &account.account_name, account.persona_state, &s.idle_status_text);
                match done.connection {
                    Some(conn) => app.cm.adopt(&done.steam_id, &account.account_name, conn).await,
                    None => {
                        let cm = app.cm.clone();
                        let sid = done.steam_id.clone();
                        // How it went is written down by connect itself.
                        tokio::spawn(async move {
                            let _ = cm.connect(&sid).await;
                        });
                    }
                }
            } else if let Some(conn) = done.connection {
                // Local accounts only needed the token; close the extra session.
                use steam_vent::ConnectionTrait;
                let off = steam_vent_proto_steam::steammessages_clientserver_login::CMsgClientLogOff::new();
                match tokio::time::timeout(Duration::from_secs(10), conn.send(off)).await {
                    Ok(Ok(())) => logbus::debug("accounts", format!("{who}: signed off the sign-in's own connection")),
                    _ => logbus::debug("accounts", format!("{who}: closed the sign-in's own connection")),
                }
            }
            let lasts = crate::steam::jwt::decode(&done.refresh_token)
                .map(|c| (c.exp - chrono::Utc::now().timestamp()) / 86_400)
                .filter(|days| *days > 0)
                .map(|days| format!(" (the saved sign-in is good for {days} more days)"))
                .unwrap_or_default();
            logbus::info("accounts", format!("Signed in: {who}{lasts}"));
            app.emit_accounts();
            let a = app.clone();
            let sid = done.steam_id.clone();
            tokio::spawn(async move { a.fetch_profile(&sid).await });
            Ok(())
        })
    }
}

#[tauri::command]
pub async fn auth_start_qr(app: State<'_, App>, mode: String) -> CmdResult<String> {
    let id = new_id();
    let a = app.inner().clone();
    a.auth.clone().start_qr(a.http.clone(), id.clone(), on_signed_in(a.clone(), mode));
    Ok(id)
}

#[tauri::command]
pub async fn auth_start_credentials(
    app: State<'_, App>,
    username: String,
    password: String,
    mode: String,
) -> CmdResult<String> {
    if username.trim().is_empty() || password.is_empty() {
        return Err("Enter your Steam username and password".into());
    }
    // Before anything can fail and quote it back.
    logbus::hide(&username, "the sign-in name");
    let id = new_id();
    let a = app.inner().clone();
    a.auth
        .clone()
        .start_credentials(a.cm.clone(), id.clone(), username.trim().to_string(), password, on_signed_in(a.clone(), mode));
    Ok(id)
}

#[tauri::command]
pub async fn auth_submit_code(app: State<'_, App>, session_id: String, code: String) -> CmdResult<()> {
    app.auth.submit_code(&session_id, &code).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn auth_cancel(app: State<'_, App>, session_id: String) -> CmdResult<()> {
    app.auth.cancel(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn set_web_cookie(app: State<'_, App>, steam_id: String, cookie: String) -> CmdResult<()> {
    let ws = WebSession::from_cookie(&cookie).map_err(anyhow_err)?;
    if ws.steam_id != steam_id {
        return Err(format!("This cookie belongs to {} — not this account", ws.steam_id));
    }
    ws.verify().await.map_err(anyhow_err)?;
    // Keep the cookie's own value, not everything that was pasted: a whole
    // cookie line can be longer than the Windows credential store accepts.
    let (_, value, _) = community::parse_login_secure(&cookie).map_err(anyhow_err)?;
    secrets::set(&secrets::cookie_key(&steam_id), &value).map_err(anyhow_err)?;
    app.forget_web_session(&steam_id).await;
    logbus::info("accounts", format!("Web session cookie saved for {}", logbus::label(&steam_id)));
    app.emit_accounts();
    Ok(())
}

#[tauri::command]
pub async fn remove_account(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.farming.stop(&steam_id);
    app.unlocker.stop(&steam_id);
    let _ = app.idler.stop_all(&app, &steam_id, None).await;
    app.cm.disconnect(&steam_id).await;
    app.cm.forget(&steam_id);
    secrets::delete(&secrets::refresh_token_key(&steam_id));
    secrets::delete(&secrets::cookie_key(&steam_id));
    // The Steam Guard "remember this device" token is kept under the sign-in name.
    if let Some(name) = app.account(&steam_id).ok().map(|a| a.account_name).filter(|n| !n.is_empty()) {
        secrets::delete(&secrets::guard_key(&name));
    }
    app.forget_web_session(&steam_id).await;
    let _ = std::fs::remove_file(app.data_dir.join(format!("games_{steam_id}.json")));
    app.update_config(|c| {
        c.accounts.retain(|a| a.steam_id != steam_id);
        if c.active_account.as_deref() == Some(steam_id.as_str()) {
            c.active_account = c.accounts.first().map(|a| a.steam_id.clone());
        }
    });
    app.emit_accounts();
    Ok(())
}

#[tauri::command]
pub async fn set_account_mode(app: State<'_, App>, steam_id: String, mode: String) -> CmdResult<()> {
    if mode != "local" && mode != "direct" {
        return Err("mode must be 'local' or 'direct'".into());
    }
    if mode == "direct" && secrets::get(&secrets::refresh_token_key(&steam_id)).is_none() {
        return Err("Direct mode needs a QR or password sign-in for this account first".into());
    }
    // Games idled through the old backend have to be stopped first.
    let _ = app.idler.stop_all(&app, &steam_id, None).await;
    if mode == "local" {
        app.cm.disconnect(&steam_id).await;
    }
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.mode = mode.clone();
        }
    });
    app.emit_accounts();
    Ok(())
}

#[tauri::command]
pub async fn account_connect(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    let account = app.account(&steam_id).map_err(anyhow_err)?;
    let s = app.settings();
    app.cm.configure(&steam_id, &account.account_name, account.persona_state, &s.idle_status_text);
    app.cm.connect(&steam_id).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn account_disconnect(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.farming.stop(&steam_id);
    let _ = app.idler.stop_all(&app, &steam_id, None).await;
    app.cm.disconnect(&steam_id).await;
    Ok(())
}

#[tauri::command]
pub async fn set_persona_state(app: State<'_, App>, steam_id: String, state: u32) -> CmdResult<()> {
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.persona_state = state;
        }
    });
    app.emit_accounts();
    if app.cm.is_online(&steam_id) {
        app.cm.set_persona(&steam_id, state).await.map_err(anyhow_err)?;
    }
    Ok(())
}

// -------------------------------------------------------------- library --

#[tauri::command]
pub async fn get_games(app: State<'_, App>, steam_id: String, refresh: bool) -> CmdResult<Vec<GameEntry>> {
    app.load_games(&steam_id, refresh).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn add_manual_game(app: State<'_, App>, steam_id: String, appid: u32, name: String) -> CmdResult<()> {
    if appid == 0 {
        return Err("Enter a valid AppID".into());
    }
    let name = if name.trim().is_empty() { format!("App {appid}") } else { name.trim().to_string() };
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.manual_games.retain(|g| g.appid != appid);
            a.manual_games.push(ManualGame { appid, name });
        }
    });
    app.refresh_manual(&steam_id);
    app.emit_accounts();
    Ok(())
}

#[tauri::command]
pub async fn remove_manual_game(app: State<'_, App>, steam_id: String, appid: u32) -> CmdResult<()> {
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.manual_games.retain(|g| g.appid != appid);
        }
    });
    app.refresh_manual(&steam_id);
    app.emit_accounts();
    Ok(())
}

/// Icon, banner and cover URLs for these games. Ids are missing from the
/// result only when Steam's store API couldn't be reached for them.
#[tauri::command]
pub async fn resolve_game_art(app: State<'_, App>, appids: Vec<u32>) -> CmdResult<HashMap<u32, GameArt>> {
    let language = app.settings().language;
    let found = app.art.resolve(&app.http, &language, &appids).await;
    // The lookup also says what each game is called: games listed by number
    // only get their names from it.
    let names: HashMap<u32, String> = found.iter().filter_map(|(id, art)| art.name.clone().map(|name| (*id, name))).collect();
    app.names_arrived(&names);
    Ok(found)
}

/// Which of these pictures are on disk already. Never touches the network, so
/// it is answered at once however slow the connection is.
#[tauri::command]
pub async fn pictures_cached(app: State<'_, App>, items: Vec<pictures::Ask>) -> CmdResult<HashMap<String, pictures::Cached>> {
    let mut found = HashMap::new();
    for ask in items.iter().take(2000) {
        if ask.kind == "avatar" {
            app.adopt_local_avatar(&ask.id);
        }
        if let Some(cached) = app.pictures.cached(ask) {
            found.insert(pictures::key(&ask.kind, &ask.id), cached);
        }
    }
    Ok(found)
}

/// One picture's file on disk, downloaded first when it isn't there yet. Only
/// a few downloads run at a time; the others wait.
#[tauri::command]
pub async fn picture_fetch(
    app: State<'_, App>,
    kind: String,
    id: String,
    urls: Vec<String>,
    current: Option<String>,
    certain: bool,
) -> CmdResult<Option<String>> {
    let ask = pictures::Ask { kind, id, current };
    Ok(app.pictures.fetch(&app.http, &ask, &urls, certain).await)
}

/// Toggle membership of an appid in one of the per-account lists.
#[tauri::command]
pub async fn set_in_list(app: State<'_, App>, steam_id: String, list: String, appid: u32, on: bool) -> CmdResult<()> {
    change_list(&app, &steam_id, &list, &[appid], on).map(|_| ())
}

/// The same for many games at once: one save and one notice to the window,
/// however many games there are. Answers how many games were given.
#[tauri::command]
pub async fn set_in_list_many(app: State<'_, App>, steam_id: String, list: String, appids: Vec<u32>, on: bool) -> CmdResult<usize> {
    change_list(&app, &steam_id, &list, &appids, on)
}

fn change_list(app: &App, steam_id: &str, list: &str, appids: &[u32], on: bool) -> CmdResult<usize> {
    let ok = app.update_config(|c| {
        let Some(a) = c.account_mut(steam_id) else { return false };
        let target = match list {
            "favorites" => &mut a.favorites,
            "autoIdle" => &mut a.auto_idle,
            "farmBlacklist" => &mut a.farm_blacklist,
            "farmWhitelist" => &mut a.farm_whitelist,
            "unlockQueue" => &mut a.unlock_queue,
            _ => return false,
        };
        apply_to_list(target, appids, on);
        true
    });
    if !ok {
        return Err(format!("unknown list {list}"));
    }
    app.emit_accounts();
    Ok(appids.len())
}

/// Takes `appids` out of the list and, when adding, puts them at its end in
/// the order given, each once. One pass over the list, whatever its length.
fn apply_to_list(target: &mut Vec<u32>, appids: &[u32], on: bool) {
    let changing: HashSet<u32> = appids.iter().copied().collect();
    target.retain(|x| !changing.contains(x));
    if on {
        let mut added = HashSet::new();
        target.extend(appids.iter().copied().filter(|a| added.insert(*a)));
    }
}

// ---------------------------------------------------------------- idling --

#[derive(Deserialize)]
pub struct GameRef {
    pub appid: u32,
    pub name: String,
}

#[tauri::command]
pub async fn idle_start(app: State<'_, App>, steam_id: String, games: Vec<GameRef>) -> CmdResult<Vec<StartResult>> {
    let list = games.into_iter().map(|g| (g.appid, g.name)).collect();
    app.idler.start(&app, &steam_id, list, "manual", None).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn idle_stop(app: State<'_, App>, steam_id: String, appids: Vec<u32>) -> CmdResult<()> {
    app.idler.stop(&app, &steam_id, &appids, None).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn idle_stop_all(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.idler.stop_all(&app, &steam_id, None).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn auto_idle_start(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<StartResult>> {
    let account = app.account(&steam_id).map_err(anyhow_err)?;
    if account.auto_idle.is_empty() {
        return Err("The auto-idle list is empty".into());
    }
    let games = account.auto_idle.iter().map(|a| (*a, app.game_name(&steam_id, *a))).collect();
    app.idler.start(&app, &steam_id, games, "auto", None).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn booster_start(
    app: State<'_, App>,
    steam_id: String,
    appid: u32,
    name: String,
    minutes: u32,
) -> CmdResult<Vec<StartResult>> {
    if minutes == 0 {
        return Err("Set a target of at least 1 minute".into());
    }
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.booster_targets.insert(appid, minutes);
        }
    });
    app.idler
        .start(&app, &steam_id, vec![(appid, name)], "booster", Some(minutes))
        .await
        .map_err(anyhow_err)
}

// --------------------------------------------------------------- farming --

#[tauri::command]
pub async fn farm_scan(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<FarmGame>> {
    FarmingManager::scan(&app, &steam_id).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn farm_start(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.farming.start(app.inner().clone(), steam_id).map_err(anyhow_err)
}

#[tauri::command]
pub async fn farm_stop(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.farming.stop(&steam_id);
    Ok(())
}

#[tauri::command]
pub async fn farm_status(app: State<'_, App>, steam_id: String) -> CmdResult<FarmStatus> {
    Ok(app.farming.status(&steam_id))
}

// ---------------------------------------------------------- achievements --

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AchItem {
    pub api_name: String,
    pub name: String,
    pub description: String,
    pub icon_url: String,
    pub icon_gray_url: String,
    pub hidden: bool,
    pub achieved: bool,
    pub unlock_time: u64,
    pub percent: Option<f64>,
    pub protected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatItem {
    pub name: String,
    pub display_name: String,
    pub kind: String,
    pub value: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub increment_only: bool,
    pub protected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AchView {
    pub appid: u32,
    pub achievements: Vec<AchItem>,
    pub stats: Vec<StatItem>,
    pub schema_found: bool,
    pub warning: Option<String>,
}

#[tauri::command]
pub async fn ach_open(app: State<'_, App>, steam_id: String, appid: u32) -> CmdResult<AchView> {
    app.require_local_client(&steam_id).map_err(anyhow_err)?;
    app.ach_open.lock().unwrap().insert(appid);
    let result = async {
        // Two slow parts at the same time: the game starting in Steam, and its
        // unlock rates coming from the web.
        let from_steam = async {
            let helper = app.helpers.ensure(appid).await?;
            let data = helper.request("achievements", Value::Null, Duration::from_secs(40)).await?;
            anyhow::Ok((helper, data))
        };
        let (from_steam, percents) = tokio::join!(from_steam, app.achievement_percents(appid));
        let (helper, data) = from_steam?;
        let lang = app.settings().language;
        let schema = app.steam_path().ok().and_then(|p| schema::load(&p, appid, &lang).ok());
        let by_name: HashMap<String, &schema::SchemaAchievement> = schema
            .as_ref()
            .map(|s| s.achievements.iter().map(|a| (a.api_name.clone(), a)).collect())
            .unwrap_or_default();
        let achievements: Vec<AchItem> = data
            .get("achievements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| {
                let api = a.get("apiName")?.as_str()?.to_string();
                let def = by_name.get(&api);
                let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let name = Some(s("name")).filter(|n| !n.is_empty()).or_else(|| def.map(|d| d.name.clone()));
                Some(AchItem {
                    name: name.unwrap_or_else(|| api.clone()),
                    description: Some(s("description"))
                        .filter(|n| !n.is_empty())
                        .or_else(|| def.map(|d| d.description.clone()))
                        .unwrap_or_default(),
                    icon_url: def.map(|d| schema::icon_url(appid, &d.icon)).unwrap_or_default(),
                    icon_gray_url: def.map(|d| schema::icon_url(appid, &d.icon_gray)).unwrap_or_default(),
                    hidden: a.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                    achieved: a.get("achieved").and_then(Value::as_bool).unwrap_or(false),
                    unlock_time: a.get("unlockTime").and_then(Value::as_u64).unwrap_or(0),
                    percent: percents.get(&api).copied(),
                    protected: def.map(|d| d.permission & 3 != 0).unwrap_or(false),
                    api_name: api,
                })
            })
            .collect();
        let mut stats = Vec::new();
        if let Some(sc) = &schema {
            if !sc.stats.is_empty() {
                let req: Vec<Value> = sc
                    .stats
                    .iter()
                    .map(|s| json!({ "name": s.name, "type": if s.kind == "int" { "int" } else { "float" } }))
                    .collect();
                let values = helper
                    .request("get_stats", json!({ "stats": req }), Duration::from_secs(20))
                    .await
                    .ok()
                    .and_then(|v| v.get("values").cloned())
                    .unwrap_or(Value::Null);
                for s in &sc.stats {
                    stats.push(StatItem {
                        name: s.name.clone(),
                        display_name: s.display_name.clone(),
                        kind: s.kind.clone(),
                        value: values.get(&s.name).and_then(Value::as_f64),
                        min: s.min,
                        max: s.max,
                        increment_only: s.increment_only,
                        protected: s.permission & 3 != 0,
                    });
                }
            }
        }
        let still_loading = achievements.is_empty() && schema.as_ref().is_some_and(|s| !s.achievements.is_empty());
        let warning = if schema.is_none() {
            Some("Steam's local stats file for this game wasn't found, so protected flags and stats can't be shown.".to_string())
        } else if still_loading {
            Some("Steam hasn't loaded this game's achievements yet. Close the game here and open it again in a moment.".to_string())
        } else {
            None
        };
        anyhow::Ok(AchView { appid, achievements, stats, schema_found: schema.is_some(), warning })
    }
    .await;
    if result.is_err() {
        app.close_achievements(appid).await;
    }
    result.map_err(anyhow_err)
}

#[tauri::command]
pub async fn ach_close(app: State<'_, App>, appid: u32) -> CmdResult<()> {
    app.close_achievements(appid).await;
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AchChange {
    pub api_name: String,
    pub achieved: bool,
}

#[tauri::command]
pub async fn ach_apply(app: State<'_, App>, steam_id: String, appid: u32, changes: Vec<AchChange>) -> CmdResult<Value> {
    app.require_local_client(&steam_id).map_err(anyhow_err)?;
    let helper = app.helpers.ensure(appid).await.map_err(anyhow_err)?;
    let n = changes.len();
    let v = helper
        .request("set_achievements", json!({ "changes": changes }), Duration::from_secs(40))
        .await
        .map_err(anyhow_err)?;
    logbus::info("achievements", format!("Applied {n} achievement change(s) for app {appid}"));
    Ok(v)
}

#[derive(Deserialize, Serialize)]
pub struct StatChange {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub value: f64,
}

#[tauri::command]
pub async fn ach_set_stats(app: State<'_, App>, steam_id: String, appid: u32, stats: Vec<StatChange>) -> CmdResult<Value> {
    app.require_local_client(&steam_id).map_err(anyhow_err)?;
    let helper = app.helpers.ensure(appid).await.map_err(anyhow_err)?;
    let stats: Vec<StatChange> = stats
        .into_iter()
        .map(|s| StatChange { kind: if s.kind == "int" { "int".into() } else { "float".into() }, ..s })
        .collect();
    helper
        .request("set_stats", json!({ "stats": stats }), Duration::from_secs(40))
        .await
        .map_err(anyhow_err)
}

#[tauri::command]
pub async fn ach_reset_all(app: State<'_, App>, steam_id: String, appid: u32, achievements: bool) -> CmdResult<Value> {
    app.require_local_client(&steam_id).map_err(anyhow_err)?;
    let helper = app.helpers.ensure(appid).await.map_err(anyhow_err)?;
    helper
        .request("reset_all", json!({ "achievements": achievements }), Duration::from_secs(40))
        .await
        .map_err(anyhow_err)
}

#[tauri::command]
pub async fn ach_set_prefs(
    app: State<'_, App>,
    steam_id: String,
    appid: u32,
    order: Option<Vec<String>>,
    skip: Option<Vec<String>>,
) -> CmdResult<()> {
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            if let Some(o) = order {
                if o.is_empty() {
                    a.unlock_order.remove(&appid);
                } else {
                    a.unlock_order.insert(appid, o);
                }
            }
            if let Some(s) = skip {
                if s.is_empty() {
                    a.unlock_skip.remove(&appid);
                } else {
                    a.unlock_skip.insert(appid, s);
                }
            }
        }
    });
    app.emit_accounts();
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameRef {
    pub api_name: String,
    pub name: String,
}

/// Import another player's unlock order/timing. `names` maps API names to the
/// display names shown on profiles so the two can be matched.
#[tauri::command]
pub async fn ach_import_timings(
    app: State<'_, App>,
    steam_id: String,
    appid: u32,
    profile: String,
    names: Vec<NameRef>,
) -> CmdResult<usize> {
    let rows = community::profile_unlock_times(&app.http, &profile, appid)
        .await
        .map_err(anyhow_err)?;
    let norm = |s: &str| s.trim().to_lowercase();
    let lookup: HashMap<String, String> = names.iter().map(|n| (norm(&n.name), n.api_name.clone())).collect();
    let timings: Vec<ImportedTiming> = rows
        .iter()
        .filter_map(|(display, offset)| {
            lookup
                .get(&norm(display))
                .map(|api| ImportedTiming { api_name: api.clone(), offset_secs: *offset })
        })
        .collect();
    if timings.is_empty() {
        return Err("None of that player's achievements matched this game's names".into());
    }
    let n = timings.len();
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.unlock_imported.insert(appid, timings);
        }
    });
    logbus::info("unlocker", format!("Imported {n} unlock time(s) for app {appid}"));
    app.emit_accounts();
    Ok(n)
}

#[tauri::command]
pub async fn ach_clear_imported(app: State<'_, App>, steam_id: String, appid: u32) -> CmdResult<()> {
    app.update_config(|c| {
        if let Some(a) = c.account_mut(&steam_id) {
            a.unlock_imported.remove(&appid);
        }
    });
    app.emit_accounts();
    Ok(())
}

#[tauri::command]
pub async fn unlock_start(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.unlocker.start(app.inner().clone(), steam_id).map_err(anyhow_err)
}

#[tauri::command]
pub async fn unlock_stop(app: State<'_, App>, steam_id: String) -> CmdResult<()> {
    app.unlocker.stop(&steam_id);
    Ok(())
}

// ---------------------------------------------------------------- market --

#[tauri::command]
pub async fn market_inventory(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<InvItem>> {
    let started = std::time::Instant::now();
    let items = app.with_web(&steam_id, |ws| async move { market::inventory(&ws).await }).await.map_err(anyhow_err)?;
    // What each item is, for the market's search to be asked by kind.
    app.remember_classes(&items);
    logbus::debug(
        "market",
        format!("Inventory of {}: {} item(s) in {} ms", logbus::label(&steam_id), items.len(), started.elapsed().as_millis()),
    );
    Ok(items)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MarketProgress {
    task: String,
    done: usize,
    total: usize,
    message: String,
}

fn progress(task: &str, done: usize, total: usize, message: impl Into<String>) {
    logbus::emit(
        "market://progress",
        MarketProgress { task: task.into(), done, total, message: message.into() },
    );
}

#[tauri::command]
pub async fn market_cancel(app: State<'_, App>) -> CmdResult<()> {
    app.market_cancel.store(true, Ordering::SeqCst);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceReport {
    /// Steam wallet currency id (1 = USD, 3 = EUR, 31 = SAR, …)
    pub currency: u32,
    /// The wallet's fee numbers, so the page computes "you receive" the same way.
    pub wallet: market::Wallet,
    pub items: Vec<PriceInfo>,
}

/// Prices for the window as they come, a few or one at a time, while a
/// price check goes on.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PricesUpdate {
    steam_id: String,
    wallet: market::Wallet,
    items: Vec<PriceInfo>,
}

fn emit_prices(steam_id: &str, wallet: &market::Wallet, items: Vec<PriceInfo>) {
    logbus::emit("market://prices", PricesUpdate { steam_id: steam_id.into(), wallet: *wallet, items });
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// An item's price for the window, as seen: with the price to ask, by the
/// strategy in the settings, and what that leaves you.
fn price_info(name: &str, wallet: &market::Wallet, seen: &pricing::Seen, s: &crate::config::MarketSettings) -> PriceInfo {
    let suggested = market::suggest_price(
        wallet,
        seen.lowest_sell,
        seen.highest_buy,
        &s.strategy,
        s.undercut_cents,
        s.min_price_cents,
        s.max_price_cents,
    );
    PriceInfo {
        market_hash_name: name.to_string(),
        currency: wallet.currency,
        lowest_sell: seen.lowest_sell,
        highest_buy: seen.highest_buy,
        sell_listings: seen.sell_listings,
        suggested,
        you_receive: suggested.map(|price| wallet.seller_receives(price)),
        checked_at: seen.checked_at,
        offers_at: seen.offers_at,
        problem: None,
    }
}

/// An item Steam gave no price for, and why.
fn no_price(name: &str, wallet: &market::Wallet, problem: impl Into<String>) -> PriceInfo {
    PriceInfo {
        market_hash_name: name.to_string(),
        currency: wallet.currency,
        checked_at: now_ms(),
        problem: Some(problem.into()),
        ..Default::default()
    }
}

/// Where the wallet for a price check came from.
#[derive(Clone, Copy, PartialEq, Debug)]
enum WalletFrom {
    /// Read from the market page just now.
    Page,
    /// As read from the market page before.
    Kept,
    /// Not known: a US-dollar wallet, until Steam's prices say which currency.
    Assumed,
    /// Taken from the currency Steam's prices came in.
    Prices,
}

/// The account's wallet as Steam's market page describes it, kept for the
/// next time. When it can't be read: as last read; and failing that, a
/// US-dollar wallet, with a warning in the log.
async fn market_wallet(app: &App, ws: &WebSession, steam_id: &str) -> (market::Wallet, WalletFrom) {
    match market::wallet(ws).await {
        Ok(wallet) => {
            app.with_prices(|list| list.put_wallet(steam_id, wallet, now_ms()));
            (wallet, WalletFrom::Page)
        }
        Err(e) => match app.with_prices(|list| list.wallet(steam_id)) {
            Some(kept) => {
                logbus::debug("market", format!("{e:#}; using the wallet read before"));
                (kept.wallet, WalletFrom::Kept)
            }
            None => {
                logbus::warn("market", format!("{e:#}. Using US-dollar fees: check \"you receive\" against Steam before listing"));
                (market::Wallet::default(), WalletFrom::Assumed)
            }
        },
    }
}

/// The wallet for a price check: as read in the last day, without asking
/// Steam again; else from the market page.
async fn price_wallet(app: &App, ws: &WebSession, steam_id: &str) -> (market::Wallet, WalletFrom) {
    match app.with_prices(|list| list.wallet(steam_id)) {
        Some(kept) if kept.fresh(now_ms()) => (kept.wallet, WalletFrom::Kept),
        _ => market_wallet(app, ws, steam_id).await,
    }
}

/// "Checking prices, about 12 min left", for the progress bar.
fn time_left(doing: &str, requests: usize, every: Duration) -> String {
    let secs = (requests as f64 * every.as_secs_f64()).round() as u64;
    match secs {
        0..=59 => format!("{doing}, less than a minute left"),
        60..=5399 => format!("{doing}, about {} min left", secs.div_ceil(60)),
        _ => format!("{doing}, about {:.1} h left", secs as f64 / 3600.0),
    }
}

/// Why an item has no price when Steam doesn't trade it.
const NO_MARKET: &str = "Steam has no market for this item";

/// What the progress bar says while items are still without a price, and
/// while all that is missing is their buy orders.
const CHECKING: &str = "Checking prices";
const READING_OFFERS: &str = "Prices are in. Reading buy orders";

/// The market's search isn't asked for prices again for this long after its
/// prices came in another currency than the wallet's, and after it didn't
/// answer as a search does.
const SEARCHES_OFF_OTHER_CURRENCY: Duration = Duration::from_secs(6 * 3600);
const SEARCHES_OFF_NO_GOOD: Duration = Duration::from_secs(3600);

/// What an error from one of the market's lookups means to a run.
fn refusal<T>(e: &anyhow::Error) -> pricing::Answer<T> {
    match community::steam_busy(e) {
        Some(429) => pricing::Answer::Busy,
        Some(_) => pricing::Answer::Trouble(format!("{e}")),
        None if community::connection_failed(e) => pricing::Answer::Trouble(format!("no answer from Steam ({})", e.root_cause())),
        None => pricing::Answer::Failed(format!("{e:#}")),
    }
}

/// A run waits: says so in the log and on the progress bar.
fn say_pause(pause: Duration, slower: Duration, cause: pricing::Cause, done: usize, total: usize) {
    let (why, short) = match cause {
        pricing::Cause::TooMany => ("Steam asked IdleKit to slow down", "Steam asked to slow down"),
        pricing::Cause::Trouble => ("Steam had trouble answering several times in a row", "Steam is having trouble"),
    };
    logbus::info(
        "market",
        format!(
            "{why}: price checks wait {} s, then go on one every {:.1} s ({done} of {total} done)",
            pause.as_secs(),
            slower.as_secs_f64()
        ),
    );
    progress("prices", done, total, format!("{short}: waiting {} s", pause.as_secs()));
}

/// "41 s" or "17 min".
fn took_words(secs: f64) -> String {
    if secs < 90.0 {
        format!("{secs:.0} s")
    } else {
        format!("{} min", (secs / 60.0).ceil())
    }
}

/// The first round of a price check: the market's search, which answers for
/// a hundred items at a time with each one's cheapest listing. What it
/// works with, and what came of it so far.
struct Searching<'a> {
    app: &'a App,
    steam_id: &'a str,
    settings: &'a crate::config::MarketSettings,
    wallet: market::Wallet,
    plan: &'a pricing::Plan,
    /// The items to price.
    wanted: &'a HashSet<&'a str>,
    stopped: &'a (dyn Fn() -> bool + Sync),
    /// Set for the run to stop: the prices are in another currency than the
    /// wallet's (which one), or the answers are no search's (`why_no_good`).
    other_currency: &'a AtomicU32,
    no_good: &'a AtomicBool,
    why_no_good: String,
    prices: &'a mut HashMap<String, PriceInfo>,
    /// The items the searches have priced.
    priced: HashSet<String>,
    /// Requests sent, and whether any was answered as a search is.
    requests: usize,
    answered: bool,
    /// Where each search's further answers start (`<search>:<start>`).
    further: Vec<String>,
    every: Duration,
    paused_for: Duration,
}

/// A search's request as a run knows it: which search, and from where.
fn search_job(job: &str) -> (usize, usize) {
    let (search, start) = job.split_once(':').unwrap_or((job, "0"));
    (search.parse().unwrap_or(0), start.parse().unwrap_or(0))
}

impl Searching<'_> {
    fn hear(&mut self, news: pricing::News<'_, market::SearchPage>) {
        match news {
            pricing::News::Answered(job, pricing::Answer::Got(page)) => {
                self.requests += 1;
                self.answered = true;
                let (search, start) = search_job(job);
                let what = &self.plan.searches[search];
                if let Some(other) = page.found.iter().map(|f| f.currency).find(|&c| c != self.wallet.currency) {
                    self.other_currency.store(other, Ordering::SeqCst);
                    return;
                }
                if start == 0 {
                    match pricing::further_starts(what, page.total, page.results) {
                        Some(starts) => self.further.extend(starts.into_iter().map(|from| format!("{search}:{from}"))),
                        None => {
                            self.why_no_good =
                                format!("a search for the {} of {} game(s) came back with {} items", what.kind.words(), what.games.len(), page.total);
                            self.no_good.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                }
                let now = now_ms();
                let mut new = Vec::new();
                for found in &page.found {
                    let Some(lowest) = found.lowest_sell else { continue };
                    if !self.wanted.contains(found.hash_name.as_str()) || self.priced.contains(&found.hash_name) {
                        continue;
                    }
                    let seen = self.app.with_prices(|list| {
                        let before = list.get(found.currency, &found.hash_name);
                        let seen = pricing::Seen::of_search(lowest, found.sell_listings, now, before.as_ref());
                        list.put(found.currency, &found.hash_name, seen);
                        seen
                    });
                    let info = price_info(&found.hash_name, &self.wallet, &seen, self.settings);
                    self.prices.insert(found.hash_name.clone(), info.clone());
                    self.priced.insert(found.hash_name.clone());
                    new.push(info);
                }
                logbus::debug(
                    "market",
                    format!(
                        "Market search, {} of {} game(s), from {start}: {} of its {} item(s), {} of them asked for",
                        what.kind.words(),
                        what.games.len(),
                        page.results,
                        page.total,
                        new.len()
                    ),
                );
                if !new.is_empty() {
                    emit_prices(self.steam_id, &self.wallet, new);
                }
                // The bar stays for the buy orders: it isn't shown as full here.
                if !(self.stopped)() && self.priced.len() < self.wanted.len() {
                    let left = self.plan.requests().saturating_sub(self.requests).max(1);
                    progress("prices", self.priced.len(), self.wanted.len(), time_left(CHECKING, left, self.every));
                }
            }
            pricing::News::Answered(job, pricing::Answer::Failed(why)) => {
                self.requests += 1;
                if self.answered {
                    // One answer among good ones: its items are priced one by one.
                    logbus::warn("market", format!("Market search (request {job}): {why}"));
                } else if !self.no_good.swap(true, Ordering::SeqCst) {
                    self.why_no_good = why;
                }
            }
            // A search has no other last words.
            pricing::News::Answered(..) => {}
            pricing::News::Again(job, why, next_try) => {
                logbus::debug("market", format!("Market search (request {job}): {why}; asking again later (try {next_try} of 3)"));
            }
            pricing::News::Pausing { pause, every, cause } => {
                self.every = every;
                self.paused_for += pause;
                self.app.save_prices();
                say_pause(pause, every, cause, self.priced.len(), self.wanted.len());
            }
        }
    }
}

/// The second round of a price check: each item's order book, one request
/// an item, which tells its buy orders too. What it works with, and what
/// came of it so far.
struct Reading<'a> {
    app: &'a App,
    steam_id: &'a str,
    settings: &'a crate::config::MarketSettings,
    stopped: &'a (dyn Fn() -> bool + Sync),
    /// Set for the run to stop: Steam's lookup doesn't work.
    broken: &'a AtomicBool,
    wallet: market::Wallet,
    wallet_from: WalletFrom,
    prices: HashMap<String, PriceInfo>,
    /// What the progress bar says is going on, and how many answers it
    /// takes until every item has a price: from then on, it is buy orders.
    doing: &'static str,
    prices_until: usize,
    total: usize,
    every: Duration,
    done: usize,
    /// Items that got no price, and items that kept the search's price
    /// because their order book couldn't be read.
    unpriced: usize,
    unread: usize,
    /// What was learned goes to disk every few answers, and while waiting
    /// out a pause, so quitting halfway keeps most of it.
    since_saved: usize,
    saved_at: std::time::Instant,
    problems_told: usize,
    told_currency: bool,
    answered: bool,
    failed_from_start: usize,
    first_problem: String,
    not_on_market: Vec<String>,
    paused_for: Duration,
}

impl Reading<'_> {
    /// The item's price from the market's search, when it has one of this hour.
    fn searched(&self, name: &str) -> Option<pricing::Seen> {
        let now = now_ms();
        self.app
            .with_prices(|list| list.get(self.wallet.currency, name))
            .filter(|seen| seen.lowest_sell.is_some() && seen.fresh(now))
    }

    fn hear(&mut self, news: pricing::News<'_>) {
        match news {
            pricing::News::Again(name, why, next_try) => {
                logbus::debug("market", format!("{name}: {why}; asking again later (try {next_try} of 3)"));
            }
            pricing::News::Answered(name, answer) => {
                let info = match answer {
                    pricing::Answer::Got(book) => {
                        self.answered = true;
                        let checked = now_ms();
                        self.app.with_prices(|list| list.put(book.currency, name, pricing::Seen::of(&book, checked)));
                        self.since_saved += 1;
                        if book.currency != self.wallet.currency
                            && self.wallet_from != WalletFrom::Page
                            && self.wallet_from != WalletFrom::Prices
                        {
                            // The wallet was a guess, or from before: Steam's prices say which currency it is now.
                            self.wallet = market::Wallet::standard(book.currency, false);
                            self.wallet_from = WalletFrom::Prices;
                            self.app.with_prices(|list| list.forget_wallet(self.steam_id));
                            logbus::info("market", "Steam's prices came in another currency than IdleKit had for this account's wallet; using theirs, with Steam's standard fees");
                        }
                        if book.currency == self.wallet.currency {
                            Some(price_info(name, &self.wallet, &pricing::Seen::of(&book, checked), self.settings))
                        } else {
                            self.unpriced += 1;
                            if !self.told_currency {
                                self.told_currency = true;
                                logbus::warn("market", "Steam gave prices in another currency than this account's wallet: those were left out");
                            }
                            Some(no_price(name, &self.wallet, "Steam gave its price in another currency than your wallet's"))
                        }
                    }
                    pricing::Answer::NotOnMarket => {
                        self.answered = true;
                        match self.searched(name) {
                            // The search has it for sale: nobody is buying, then.
                            Some(mut seen) => {
                                (seen.highest_buy, seen.offers_at) = (None, now_ms());
                                self.app.with_prices(|list| list.put(self.wallet.currency, name, seen));
                                self.since_saved += 1;
                                logbus::debug("market", format!("{name}: Steam has no order book for this item; the search's price stays"));
                                Some(price_info(name, &self.wallet, &seen, self.settings))
                            }
                            None => {
                                self.unpriced += 1;
                                self.not_on_market.push(name.to_string());
                                self.app.note_no_market(name, now_ms());
                                logbus::debug("market", format!("{name}: {NO_MARKET}"));
                                Some(no_price(name, &self.wallet, NO_MARKET))
                            }
                        }
                    }
                    pricing::Answer::Failed(why) | pricing::Answer::Trouble(why) => {
                        self.problems_told += 1;
                        if !self.answered {
                            self.failed_from_start += 1;
                            if self.first_problem.is_empty() {
                                self.first_problem = why.clone();
                            }
                            if self.failed_from_start >= 8 {
                                self.broken.store(true, Ordering::SeqCst);
                            }
                        }
                        if self.problems_told <= 5 {
                            logbus::warn("market", format!("{name}: {why}"));
                        } else {
                            logbus::debug("market", format!("{name}: {why}"));
                        }
                        if self.searched(name).is_some() {
                            // The search's price stays; only the buy orders are missing.
                            self.unread += 1;
                            None
                        } else {
                            self.unpriced += 1;
                            Some(no_price(name, &self.wallet, format!("Couldn't get a price: {why}")))
                        }
                    }
                    pricing::Answer::Busy => return,
                };
                if let Some(info) = info {
                    emit_prices(self.steam_id, &self.wallet, vec![info.clone()]);
                    self.prices.insert(name.to_string(), info);
                }
                self.done += 1;
                if self.done >= self.prices_until {
                    self.doing = READING_OFFERS;
                }
                if !(self.stopped)() {
                    progress("prices", self.done, self.total, time_left(self.doing, self.total - self.done, self.every));
                }
                if self.since_saved >= 10 || (self.since_saved > 0 && self.saved_at.elapsed() >= Duration::from_secs(30)) {
                    self.save();
                }
            }
            pricing::News::Pausing { pause, every, cause } => {
                self.every = every;
                self.paused_for += pause;
                if self.since_saved > 0 {
                    self.save();
                }
                say_pause(pause, every, cause, self.done, self.total);
            }
        }
    }

    fn save(&mut self) {
        self.since_saved = 0;
        self.saved_at = std::time::Instant::now();
        self.app.save_prices();
    }
}

/// Prices of inventory items. What was checked in the last hour comes at
/// once, from the kept list, unless it is in `recheck`. The rest comes in
/// two rounds: the market's search, a hundred items an answer, gives each
/// item's cheapest listing within a minute or so (when there are enough
/// items for it, and its prices are in the wallet's currency); then each
/// item's order book, a few at a time, gives its buy orders too: the items
/// in `first` (the ones ticked) before any other, then those the search had
/// no price for, then the rest in the order given. Each price goes to the
/// window (`market://prices`) as it comes. A newer request for prices, or a
/// listing job, takes over from this one.
#[tauri::command]
pub async fn market_prices(
    app: State<'_, App>,
    steam_id: String,
    hash_names: Vec<String>,
    recheck: Option<Vec<String>>,
    first: Option<Vec<String>>,
) -> CmdResult<PriceReport> {
    let app = app.inner().clone();
    let run = app.price_runs.fetch_add(1, Ordering::SeqCst) + 1;
    app.market_cancel.store(false, Ordering::SeqCst);
    let _job = app.market_job();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let settings = app.settings().market;
    let (wallet, wallet_from) = price_wallet(&app, &ws, &steam_id).await;

    // Each item once, in the order given.
    let mut named = HashSet::new();
    let names: Vec<String> = hash_names.into_iter().filter(|n| !n.trim().is_empty() && named.insert(n.clone())).collect();
    let again: HashSet<&str> = recheck.iter().flatten().map(String::as_str).collect();
    let now = now_ms();
    let mut prices: HashMap<String, PriceInfo> = HashMap::new();
    // The items whose cheapest listing is due, and those whose buy orders
    // are: the first are among the second.
    let (mut to_price, mut to_read) = (Vec::new(), Vec::new());
    app.with_prices(|list| {
        for name in &names {
            // With no idea of the currency, nothing kept can be trusted to be in it.
            let kept = if wallet_from == WalletFrom::Assumed { None } else { list.get(wallet.currency, name) };
            if let Some(seen) = &kept {
                prices.insert(name.clone(), price_info(name, &wallet, seen, &settings));
            }
            let asked_again = again.contains(name.as_str());
            if let (false, Some(at)) = (asked_again, app.no_market_since(name, now)) {
                // Steam said so within the hour: it isn't asked again.
                prices.insert(name.clone(), PriceInfo { checked_at: at, ..no_price(name, &wallet, NO_MARKET) });
                continue;
            }
            if asked_again || !kept.is_some_and(|seen| seen.fresh(now)) {
                to_price.push(name.clone());
            }
            if asked_again || !kept.is_some_and(|seen| seen.offers_fresh(now)) {
                to_read.push(name.clone());
            }
        }
    });
    let report = |prices: &HashMap<String, PriceInfo>, wallet: market::Wallet| PriceReport {
        currency: wallet.currency,
        wallet,
        items: names.iter().filter_map(|n| prices.get(n).cloned()).collect(),
    };
    if !prices.is_empty() {
        emit_prices(&steam_id, &wallet, names.iter().filter_map(|n| prices.get(n).cloned()).collect());
    }
    if to_read.is_empty() {
        return Ok(report(&prices, wallet));
    }

    // One run asks Steam at a time: one under way stops once it sees this
    // one, and lets go when the answers it was waiting for are in.
    let mut pacer = app.price_pacer.lock().await;
    let cancelled = || app.market_cancel.load(Ordering::SeqCst);
    // Steam's lookup failing for item after item, from the start, is a lookup
    // that doesn't work (Steam changed something): asking about the rest
    // would only fail the same way.
    let broken = AtomicBool::new(false);
    let stop = || cancelled() || app.price_runs.load(Ordering::SeqCst) != run || broken.load(Ordering::SeqCst);
    if stop() {
        return Ok(report(&prices, wallet));
    }
    let total = to_read.len();
    let every = Duration::from_millis(settings.delay_ms.max(300));
    let pace = |every: Duration| pricing::Pace { every, at_once: 3, patience: pricing::PAUSES_BEFORE_GIVING_UP };
    let started = std::time::Instant::now();
    // Order-book numbers kept from Steam's old market: a way in when asking
    // by name fails, for the items that have one.
    let by_number: HashMap<String, u64> = if wallet_from == WalletFrom::Assumed {
        HashMap::new()
    } else {
        app.with_config(|c| to_read.iter().filter_map(|n| Some((n.clone(), *c.item_name_ids.get(n)?))).collect())
    };
    let currency = wallet.currency;
    let ask = |name: String| {
        let (ws, number) = (&*ws, by_number.get(&name).copied());
        async move {
            let answer = |r: anyhow::Result<Option<market::Book>>| match r {
                Ok(Some(book)) => pricing::Answer::Got(book),
                Ok(None) => pricing::Answer::NotOnMarket,
                Err(e) => refusal(&e),
            };
            match (answer(market::order_book_by_name(ws, &name).await), number) {
                (pricing::Answer::Failed(why), Some(id)) => {
                    logbus::debug("market", format!("{name}: the lookup by name failed ({why}); trying Steam's older lookup by number"));
                    match answer(market::order_book_by_id(ws, id, currency).await.map(Some)) {
                        pricing::Answer::Failed(_) => pricing::Answer::Failed(why),
                        other => other,
                    }
                }
                (other, _) => other,
            }
        }
    };
    let mut reading = Reading {
        app: &app,
        steam_id: &steam_id,
        settings: &settings,
        stopped: &stop,
        broken: &broken,
        wallet,
        wallet_from,
        prices,
        doing: CHECKING,
        prices_until: total,
        total,
        every,
        done: 0,
        unpriced: 0,
        unread: 0,
        since_saved: 0,
        saved_at: std::time::Instant::now(),
        problems_told: 0,
        told_currency: false,
        answered: false,
        failed_from_start: 0,
        first_problem: String::new(),
        not_on_market: Vec::new(),
        paused_for: Duration::ZERO,
    };

    // The searches that cover the items to price, and whether they are the
    // faster way: they are from a dozen items up.
    let classes = app.classes_of(&to_price);
    let plan = pricing::Plan::of(to_price.iter().zip(&classes).map(|(name, class)| (name.as_str(), class.as_deref())));
    let mut search = plan.worth_it();
    if search {
        if let Some(why) = app.searches_off() {
            logbus::debug("market", format!("Steam's market search isn't asked for prices for now ({why}): they are checked one item at a time"));
            search = false;
        }
    }
    let searches_ahead = if search { plan.requests() } else { 0 };
    progress("prices", 0, total, time_left(CHECKING, if search { searches_ahead } else { total }, every));

    let mut end = pricing::RunEnd::Done;
    let mut read_first = 0;
    // What the searches came to: how many items they priced, of how many.
    let mut searched: Option<(usize, usize)> = None;
    if search && reading.wallet_from == WalletFrom::Assumed {
        // The market page told no wallet: one order book says which currency
        // this account's prices come in, before the search is held to it.
        end = pricing::run(vec![to_read[0].clone()], pace(every), &mut pacer, &stop, &ask, |news| reading.hear(news)).await;
        read_first = 1;
        if reading.wallet_from == WalletFrom::Assumed {
            logbus::debug("market", "Which currency this account's prices come in isn't known: the market's search isn't asked");
            search = false;
        }
    }
    // The items still without a price of this hour, after the search.
    let mut unpriced: HashSet<&str> = to_price.iter().map(String::as_str).collect();
    if read_first == 1 {
        unpriced.remove(to_read[0].as_str());
    }
    if search && end == pricing::RunEnd::Done {
        let wanted = unpriced.clone();
        let other_currency = AtomicU32::new(0);
        let no_good = AtomicBool::new(false);
        let halt = || stop() || other_currency.load(Ordering::SeqCst) != 0 || no_good.load(Ordering::SeqCst);
        let wallet = reading.wallet;
        logbus::debug(
            "market",
            format!(
                "Quick prices: asking Steam's market search about {} item(s) of {} game(s), in about {} request(s)",
                plan.covered,
                plan.games(),
                plan.requests()
            ),
        );
        let ask_search = |job: String| {
            let (search, start) = search_job(&job);
            let (ws, what) = (&*ws, &plan.searches[search]);
            async move {
                match market::search(ws, what, wallet.currency, start).await {
                    Ok(page) => pricing::Answer::Got(page),
                    Err(e) => refusal(&e),
                }
            }
        };
        let mut searching = Searching {
            app: &app,
            steam_id: &steam_id,
            settings: &settings,
            wallet,
            plan: &plan,
            wanted: &wanted,
            stopped: &stop,
            other_currency: &other_currency,
            no_good: &no_good,
            why_no_good: String::new(),
            prices: &mut reading.prices,
            priced: HashSet::new(),
            requests: 0,
            answered: false,
            further: Vec::new(),
            every: reading.every,
            paused_for: Duration::ZERO,
        };
        // Refused twice in a row, the searches are given up on: the order
        // books, which Steam is known to answer at this pace, take over.
        let search_pace = |every: Duration| pricing::Pace { patience: pricing::PAUSES_BEFORE_NO_SEARCH, ..pace(every) };
        // Each search's first answer says how many more it has.
        let first: Vec<String> = (0..plan.searches.len()).map(|k| format!("{k}:0")).collect();
        end = pricing::run(first, search_pace(searching.every), &mut pacer, &halt, &ask_search, |news| searching.hear(news)).await;
        if end == pricing::RunEnd::Done && !halt() && !searching.further.is_empty() {
            let further = std::mem::take(&mut searching.further);
            end = pricing::run(further, search_pace(searching.every), &mut pacer, &halt, &ask_search, |news| searching.hear(news)).await;
        }
        // A slower pace Steam asked of the searches stays theirs: the order
        // books have their own, and are slowed when Steam asks it of them.
        let Searching { priced, requests, why_no_good, paused_for, .. } = searching;
        app.save_prices();
        reading.paused_for += paused_for;
        for name in &priced {
            unpriced.remove(name.as_str());
        }
        let other = other_currency.load(Ordering::SeqCst);
        if other != 0 {
            // Nothing was stopped but the searches.
            end = pricing::RunEnd::Done;
            app.turn_searches_off(SEARCHES_OFF_OTHER_CURRENCY, "its prices came in another currency than the wallet's");
            logbus::info(
                "market",
                format!(
                    "Steam's market search gives its prices in {}, not in this account's wallet currency ({}), so they can't be used: prices are checked one item at a time",
                    market::currency_code(other),
                    market::currency_code(wallet.currency)
                ),
            );
        } else if no_good.load(Ordering::SeqCst) {
            end = pricing::RunEnd::Done;
            app.turn_searches_off(SEARCHES_OFF_NO_GOOD, "it didn't answer as a search does");
            logbus::warn(
                "market",
                format!("Steam's market search didn't answer as expected ({why_no_good}): prices are checked one item at a time"),
            );
        } else {
            searched = Some((priced.len(), wanted.len()));
            if let pricing::RunEnd::GaveUp { cause, .. } = end {
                end = pricing::RunEnd::Done;
                app.turn_searches_off(SEARCHES_OFF_NO_GOOD, "Steam kept refusing it");
                let kept = match cause {
                    pricing::Cause::TooMany => "Steam kept asking IdleKit to slow down",
                    pricing::Cause::Trouble => "Steam kept having trouble answering",
                };
                logbus::info(
                    "market",
                    format!(
                        "{kept} at the market's search, after {} of {} item(s) were priced by it: the rest are checked one item at a time",
                        priced.len(),
                        wanted.len()
                    ),
                );
            } else if end == pricing::RunEnd::Done {
                let rest = unpriced.len();
                logbus::info(
                    "market",
                    format!(
                        "Quick prices: {} of {} item(s) priced from {requests} request(s) to Steam's market search in {}{}; buy orders are read next, one item at a time",
                        priced.len(),
                        wanted.len(),
                        took_words(started.elapsed().as_secs_f64()),
                        if rest > 0 { format!("; {rest} not in its answers, priced one by one") } else { String::new() }
                    ),
                );
            }
        }
    }
    if end == pricing::RunEnd::Done && stop() {
        // Stopped between two rounds.
        end = pricing::RunEnd::Stopped;
    }
    if end == pricing::RunEnd::Done {
        // The order books: the ticked items before any other, then those
        // still without a price, then the rest, each in the order given.
        let ticked: HashSet<&str> = first.iter().flatten().map(String::as_str).collect();
        let mut names: Vec<String> = to_read[read_first..].to_vec();
        names.sort_by_key(|name| match (ticked.contains(name.as_str()), unpriced.contains(name.as_str())) {
            (true, _) => 0,
            (false, true) => 1,
            (false, false) => 2,
        });
        // Every item has a price once the last one without is answered.
        let without_price = names.iter().filter(|name| unpriced.contains(name.as_str())).count();
        reading.prices_until = reading.done + names.iter().rposition(|name| unpriced.contains(name.as_str())).map_or(0, |at| at + 1);
        if without_price == 0 {
            reading.doing = READING_OFFERS;
        }
        logbus::debug(
            "market",
            format!(
                "Reading the order books of {} item(s), one every {:.1} s ({} ticked ones first, {without_price} without a price yet)",
                names.len(),
                reading.every.as_secs_f64(),
                names.iter().filter(|name| ticked.contains(name.as_str())).count()
            ),
        );
        progress("prices", reading.done, total, time_left(reading.doing, names.len(), reading.every));
        end = pricing::run(names, pace(reading.every), &mut pacer, &stop, &ask, |news| reading.hear(news)).await;
    }
    // Saved before letting go, so the next run doesn't write the file at the same time.
    app.save_prices();
    drop(pacer);
    let Reading { wallet, prices, doing, done, unpriced: without_price, unread, failed_from_start, first_problem, not_on_market, paused_for, .. } = reading;
    let took_secs = started.elapsed().as_secs_f64();
    let took = took_words(took_secs);
    if !not_on_market.is_empty() {
        let shown: Vec<&str> = not_on_market.iter().take(10).map(String::as_str).collect();
        let more = not_on_market.len().saturating_sub(shown.len());
        logbus::info(
            "market",
            format!(
                "No market on Steam for {} item(s): {}{}",
                not_on_market.len(),
                shown.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() }
            ),
        );
    }
    let only_offers = doing == READING_OFFERS;
    match end {
        pricing::RunEnd::Done => {
            progress("prices", total, total, "Done");
            let pauses = if paused_for.is_zero() { String::new() } else { format!(", {} s of it in pauses", paused_for.as_secs()) };
            let each = if done > 0 { format!(", {:.1} s per item", took_secs / done as f64) } else { String::new() };
            let missed = [
                (without_price > 0).then(|| format!("{without_price} item(s) got no price")),
                (unread > 0).then(|| format!("the buy orders of {unread} item(s) couldn't be read")),
            ];
            let missed: Vec<String> = missed.into_iter().flatten().collect();
            let missed = if missed.is_empty() { String::new() } else { format!("; {}", missed.join(", ")) };
            // With the prices from the search, or from before, the order books were for the buy orders.
            let what = if searched.is_some() || to_price.is_empty() {
                format!("Read the buy orders of {total} item(s)")
            } else {
                format!("Checked {total} price(s)")
            };
            logbus::info("market", format!("{what} in {took}{each}{pauses}{missed}"));
        }
        pricing::RunEnd::Stopped if broken.load(Ordering::SeqCst) => {
            progress("prices", total, total, "Stopped");
            let why = format!(
                "Steam's price lookup didn't work for {failed_from_start} items in a row ({first_problem}), so IdleKit stopped asking. Try Refresh prices later"
            );
            logbus::warn("market", &why);
            return Err(why);
        }
        pricing::RunEnd::Stopped => {
            let so_far = match searched {
                Some((priced, wanted)) => {
                    format!("{priced} of {wanted} item(s) priced by the market's search, {done} of {total} order book(s) read")
                }
                None => format!("{done} of {total} checked"),
            };
            if cancelled() {
                progress("prices", total, total, "Cancelled");
                logbus::info("market", format!("Stopped checking prices: {so_far}"));
            } else {
                logbus::debug("market", format!("A newer price check took over ({so_far})"));
            }
        }
        pricing::RunEnd::GaveUp { left, cause } => {
            progress("prices", total, total, "Stopped");
            let kept = match cause {
                pricing::Cause::TooMany => "Steam kept asking IdleKit to slow down",
                pricing::Cause::Trouble => "Steam kept having trouble answering",
            };
            let missing = if only_offers {
                format!("the buy orders of {left} item(s) weren't read")
            } else {
                format!("{left} price(s) weren't checked")
            };
            let why = format!("{kept}, so {missing} after {took}. Try Refresh prices in a few minutes");
            logbus::warn("market", &why);
            return Err(why);
        }
    }
    Ok(report(&prices, wallet))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SellRequest {
    pub market_hash_name: String,
    pub asset_ids: Vec<String>,
    /// What the buyer pays, in minor units.
    pub buyer_cents: u32,
}

#[tauri::command]
pub async fn market_sell(app: State<'_, App>, steam_id: String, items: Vec<SellRequest>) -> CmdResult<Vec<SellOutcome>> {
    // A price check under way stops: listing asks Steam enough on its own.
    app.price_runs.fetch_add(1, Ordering::SeqCst);
    app.market_cancel.store(false, Ordering::SeqCst);
    let _job = app.market_job();
    let started = std::time::Instant::now();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let delay = app.settings().market.delay_ms.max(500);
    let (wallet, _) = market_wallet(&app, &ws, &steam_id).await;
    let total: usize = items.iter().map(|i| i.asset_ids.len()).sum();
    let mut done = 0;
    let mut out = Vec::new();
    let mut pending_confirmations = 0;
    'outer: for item in items {
        let seller = wallet.seller_receives(item.buyer_cents);
        if seller == 0 {
            logbus::warn("market", format!("{}: price too low to list", item.market_hash_name));
            continue;
        }
        for asset in &item.asset_ids {
            if app.market_cancel.load(Ordering::SeqCst) {
                break 'outer;
            }
            progress("sell", done, total, format!("Listing {}", item.market_hash_name));
            let mut ended = false;
            let r = market::sell(&ws, asset, seller).await.unwrap_or_else(|e| {
                ended = community::session_ended(&e);
                app.web_failed(&steam_id, &ws, &e);
                SellOutcome { asset_id: asset.clone(), ok: false, needs_confirmation: false, message: format!("{e:#}") }
            });
            if r.needs_confirmation {
                pending_confirmations += 1;
            }
            if r.ok {
                logbus::debug("market", format!("{}: listed{}", item.market_hash_name, if r.needs_confirmation { ", to confirm in the Steam mobile app" } else { "" }));
            } else {
                logbus::warn("market", format!("{}: {}", item.market_hash_name, r.message));
            }
            out.push(r);
            done += 1;
            if ended {
                // Every listing after this one would fail the same way.
                logbus::warn("market", "Stopped listing: Steam no longer takes this web session. Try again: a new one is made");
                break 'outer;
            }
            if done < total {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
    progress("sell", total, total, "Done");
    let ok = out.iter().filter(|o| o.ok).count();
    logbus::info(
        "market",
        format!("Listed {ok} of {total} item(s) in {} s{}", started.elapsed().as_secs(), if pending_confirmations > 0 {
            format!("; {pending_confirmations} need confirming in the Steam mobile app")
        } else {
            String::new()
        }),
    );
    Ok(out)
}

#[tauri::command]
pub async fn market_listings(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<Listing>> {
    let started = std::time::Instant::now();
    let listings = app.with_web(&steam_id, |ws| async move { market::my_listings(&ws).await }).await.map_err(anyhow_err)?;
    logbus::debug(
        "market",
        format!("Listings of {}: {} in {} ms", logbus::label(&steam_id), listings.len(), started.elapsed().as_millis()),
    );
    Ok(listings)
}

#[tauri::command]
pub async fn market_remove(app: State<'_, App>, steam_id: String, listing_ids: Vec<String>) -> CmdResult<usize> {
    app.price_runs.fetch_add(1, Ordering::SeqCst);
    app.market_cancel.store(false, Ordering::SeqCst);
    let _job = app.market_job();
    let started = std::time::Instant::now();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let delay = app.settings().market.delay_ms.max(500);
    let total = listing_ids.len();
    let mut removed = 0;
    for (i, id) in listing_ids.iter().enumerate() {
        if app.market_cancel.load(Ordering::SeqCst) {
            logbus::info("market", format!("Removing listings cancelled after {i} of {total}"));
            break;
        }
        progress("remove", i, total, format!("Removing listing {id}"));
        match market::remove_listing(&ws, id).await {
            Ok(()) => {
                removed += 1;
                logbus::debug("market", format!("Removed listing {id}"));
            }
            Err(e) => {
                logbus::warn("market", format!("{e:#}"));
                if community::session_ended(&e) {
                    app.web_failed(&steam_id, &ws, &e);
                    logbus::warn("market", "Stopped removing: Steam no longer takes this web session. Try again: a new one is made");
                    break;
                }
            }
        }
        if i + 1 < total {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
    progress("remove", total, total, "Done");
    logbus::info("market", format!("Removed {removed} of {total} listing(s) in {} s", started.elapsed().as_secs()));
    Ok(removed)
}

#[tauri::command]
pub async fn web_check(app: State<'_, App>, steam_id: String) -> CmdResult<String> {
    app.with_web(&steam_id, |ws| async move { ws.verify().await }).await.map_err(anyhow_err)?;
    logbus::info("web", format!("{}: Steam accepts the web session", logbus::label(&steam_id)));
    Ok("Web session OK".into())
}

// -------------------------------------------------- desktop and updates --

/// Turn starting at sign-in on or off. Returns the state afterwards.
#[tauri::command]
pub async fn set_autostart(handle: AppHandle, enabled: bool) -> CmdResult<bool> {
    desktop::set_autostart(&handle, enabled)
}

/// The answer to "keep running or quit?": "tray", "quit" or "cancel".
#[tauri::command]
pub async fn window_close_choice(handle: AppHandle, action: String) -> CmdResult<()> {
    desktop::close_choice(&handle, &action);
    Ok(())
}

/// The window has no title bar of Windows' own: the app draws the bar and its
/// buttons, and they end up here. Gives back whether the window is maximized.
#[tauri::command]
pub async fn window_control(window: tauri::WebviewWindow, action: String) -> CmdResult<bool> {
    match action.as_str() {
        "minimize" => window.minimize(),
        "maximize" => match window.is_maximized() {
            Ok(true) => window.unmaximize(),
            _ => window.maximize(),
        },
        // Goes through the same question as closing any other way (tray, quit or ask).
        "close" => window.close(),
        // Called while the mouse button is down on the bar: the system takes
        // over and moves the window, snapping included.
        "drag" => window.start_dragging(),
        "state" => Ok(()),
        other => return Err(format!("Unknown window action: {other}")),
    }
    .map_err(err)?;
    Ok(window.is_maximized().unwrap_or(false))
}

#[tauri::command]
pub async fn update_check(app: State<'_, App>, handle: AppHandle) -> CmdResult<UpdateState> {
    Ok(updates::check(&app, &handle).await)
}

/// Download and install the version the last check found. When it works the
/// app is replaced and restarted, so this only ever returns an error.
#[tauri::command]
pub async fn update_install(app: State<'_, App>, handle: AppHandle) -> CmdResult<()> {
    updates::install(&app, &handle).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_games_change_a_list_in_one_go() {
        let mut list = vec![10, 20, 30, 40];
        // adding moves a game already there to the end, and repeats count once
        apply_to_list(&mut list, &[50, 20, 50, 60], true);
        assert_eq!(list, vec![10, 30, 40, 50, 20, 60]);
        apply_to_list(&mut list, &[10, 60, 999], false);
        assert_eq!(list, vec![30, 40, 50, 20]);
        apply_to_list(&mut list, &[], true);
        assert_eq!(list, vec![30, 40, 50, 20]);
        // one game, as before
        apply_to_list(&mut list, &[40], true);
        assert_eq!(list, vec![30, 50, 20, 40]);
    }
}
