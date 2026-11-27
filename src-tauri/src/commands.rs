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
use crate::steam::cm::CmStatus;
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
use std::sync::atomic::Ordering;
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
}

fn account_views(app: &App, local: &LocalSteam) -> Vec<AccountView> {
    app.with_config(|c| c.accounts.clone())
        .into_iter()
        .map(|a| AccountView {
            has_token: secrets::get(&secrets::refresh_token_key(&a.steam_id)).is_some(),
            has_cookie: secrets::get(&secrets::cookie_key(&a.steam_id)).is_some(),
            local_active: local.active_steam_id.as_deref() == Some(a.steam_id.as_str()),
            account: a,
        })
        .collect()
}

#[tauri::command]
pub async fn get_bootstrap(app: State<'_, App>, handle: AppHandle) -> CmdResult<Bootstrap> {
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
        logs: logbus::recent(),
        log_file: logbus::file_path().map(|p| p.display().to_string()),
        desktop: desktop::info(&handle),
        update: updates::snapshot(&app, &handle),
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
            secrets::set(&secrets::refresh_token_key(&done.steam_id), &done.refresh_token)?;
            secrets::delete(&secrets::cookie_key(&done.steam_id));
            upsert_account(&app, &done.steam_id, &done.account_name, "", &mode);
            let account = app.account(&done.steam_id)?;
            app.forget_web_session(&done.steam_id).await;
            if let Some(token) = &done.access_token {
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
                        tokio::spawn(async move {
                            if let Err(e) = cm.connect(&sid).await {
                                logbus::warn("direct", format!("{}: {e:#}", logbus::label(&sid)));
                            }
                        });
                    }
                }
            } else if let Some(conn) = done.connection {
                // Local accounts only needed the token; close the extra session.
                use steam_vent::ConnectionTrait;
                let _ = conn
                    .send(steam_vent_proto_steam::steammessages_clientserver_login::CMsgClientLogOff::new())
                    .await;
            }
            logbus::info("accounts", format!("Signed in: {}", logbus::label(&done.steam_id)));
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
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    market::inventory(&ws).await.map_err(anyhow_err)
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

/// An item's price for the window, from its order book as seen: with the
/// price to ask, by the strategy in the settings, and what that leaves you.
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

/// "about 12 min left", for the progress bar.
fn time_left(items: usize, every: Duration) -> String {
    let secs = (items as f64 * every.as_secs_f64()).round() as u64;
    match secs {
        0..=59 => "Checking prices, less than a minute left".into(),
        60..=5399 => format!("Checking prices, about {} min left", secs.div_ceil(60)),
        _ => format!("Checking prices, about {:.1} h left", secs as f64 / 3600.0),
    }
}

/// Prices of inventory items, from Steam's order books. What was checked in
/// the last hour comes at once, from the kept list, unless it is in
/// `recheck`; the rest is asked about a few at a time, in the order given,
/// and each price goes to the window (`market://prices`) as it comes. A
/// newer request for prices, or a listing job, takes over from this one.
#[tauri::command]
pub async fn market_prices(
    app: State<'_, App>,
    steam_id: String,
    hash_names: Vec<String>,
    recheck: Option<Vec<String>>,
) -> CmdResult<PriceReport> {
    let app = app.inner().clone();
    let run = app.price_runs.fetch_add(1, Ordering::SeqCst) + 1;
    app.market_cancel.store(false, Ordering::SeqCst);
    let _job = app.market_job();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let settings = app.settings().market;
    let (mut wallet, mut wallet_from) = price_wallet(&app, &ws, &steam_id).await;

    // Each item once, in the order given.
    let mut named = HashSet::new();
    let names: Vec<String> = hash_names.into_iter().filter(|n| !n.trim().is_empty() && named.insert(n.clone())).collect();
    let again: HashSet<&str> = recheck.iter().flatten().map(String::as_str).collect();
    let now = now_ms();
    let mut prices: HashMap<String, PriceInfo> = HashMap::new();
    let mut to_ask = Vec::new();
    app.with_prices(|list| {
        for name in &names {
            // With no idea of the currency, nothing kept can be trusted to be in it.
            let kept = if wallet_from == WalletFrom::Assumed { None } else { list.get(wallet.currency, name) };
            if let Some(seen) = &kept {
                prices.insert(name.clone(), price_info(name, &wallet, seen, &settings));
            }
            if again.contains(name.as_str()) || !kept.is_some_and(|seen| seen.fresh(now)) {
                to_ask.push(name.clone());
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
    if to_ask.is_empty() {
        return Ok(report(&prices, wallet));
    }

    // One run asks Steam at a time: one under way stops once it sees this
    // one, and lets go when the answers it was waiting for are in.
    let mut pacer = app.price_pacer.lock().await;
    let cancelled = || app.market_cancel.load(Ordering::SeqCst);
    // Steam's lookup failing for item after item, from the start, is a lookup
    // that doesn't work (Steam changed something): asking about the rest
    // would only fail the same way.
    let broken = std::sync::atomic::AtomicBool::new(false);
    let stop = || cancelled() || app.price_runs.load(Ordering::SeqCst) != run || broken.load(Ordering::SeqCst);
    if stop() {
        return Ok(report(&prices, wallet));
    }
    let total = to_ask.len();
    let mut every = Duration::from_millis(settings.delay_ms.max(300));
    let pace = pricing::Pace { every, at_once: 3 };
    let started = std::time::Instant::now();
    progress("prices", 0, total, time_left(total, every));
    // Order-book numbers kept from Steam's old market: a way in when asking
    // by name fails, for the items that have one.
    let by_number: HashMap<String, u64> = if wallet_from == WalletFrom::Assumed {
        HashMap::new()
    } else {
        app.with_config(|c| to_ask.iter().filter_map(|n| Some((n.clone(), *c.item_name_ids.get(n)?))).collect())
    };
    let currency = wallet.currency;
    let ask = |name: String| {
        let (ws, number) = (&*ws, by_number.get(&name).copied());
        async move {
            let answer = |r: anyhow::Result<Option<market::Book>>| match r {
                Ok(Some(book)) => pricing::Answer::Book(book),
                Ok(None) => pricing::Answer::NotOnMarket,
                Err(e) => match community::steam_busy(&e) {
                    Some(429) => pricing::Answer::Busy,
                    Some(_) => pricing::Answer::Trouble(format!("{e}")),
                    None if community::connection_failed(&e) => {
                        pricing::Answer::Trouble(format!("no answer from Steam ({})", e.root_cause()))
                    }
                    None => pricing::Answer::Failed(format!("{e:#}")),
                },
            };
            match (answer(market::order_book_by_name(ws, &name).await), number) {
                (pricing::Answer::Failed(why), Some(id)) => match answer(market::order_book_by_id(ws, id, currency).await.map(Some)) {
                    pricing::Answer::Failed(_) => pricing::Answer::Failed(why),
                    other => other,
                },
                (other, _) => other,
            }
        }
    };
    let (mut done, mut unpriced, mut since_saved) = (0usize, 0usize, 0usize);
    // What was learned goes to disk every few answers, and while waiting
    // out a pause, so quitting halfway keeps most of it.
    let mut saved_at = std::time::Instant::now();
    let mut problems_told = 0;
    let mut told_currency = false;
    let (mut answered, mut failed_from_start) = (false, 0usize);
    let mut first_problem = String::new();
    let end = pricing::run(to_ask, pace, &mut pacer, stop, ask, |news| match news {
        pricing::News::Answered(name, answer) => {
            let info = match answer {
                pricing::Answer::Book(book) => {
                    answered = true;
                    let checked = now_ms();
                    app.with_prices(|list| list.put(book.currency, name, pricing::Seen::of(&book, checked)));
                    since_saved += 1;
                    if book.currency != wallet.currency && wallet_from != WalletFrom::Page && wallet_from != WalletFrom::Prices {
                        // The wallet was a guess, or from before: Steam's prices say which currency it is now.
                        wallet = market::Wallet::standard(book.currency, false);
                        wallet_from = WalletFrom::Prices;
                        app.with_prices(|list| list.forget_wallet(&steam_id));
                        logbus::info("market", "Steam's prices came in another currency than IdleKit had for this account's wallet; using theirs, with Steam's standard fees");
                    }
                    if book.currency == wallet.currency {
                        price_info(name, &wallet, &pricing::Seen::of(&book, checked), &settings)
                    } else {
                        unpriced += 1;
                        if !told_currency {
                            told_currency = true;
                            logbus::warn("market", "Steam gave prices in another currency than this account's wallet: those were left out");
                        }
                        no_price(name, &wallet, "Steam gave its price in another currency than your wallet's")
                    }
                }
                pricing::Answer::NotOnMarket => {
                    answered = true;
                    unpriced += 1;
                    no_price(name, &wallet, "Steam has no market for this item")
                }
                pricing::Answer::Failed(why) | pricing::Answer::Trouble(why) => {
                    unpriced += 1;
                    problems_told += 1;
                    if !answered {
                        failed_from_start += 1;
                        if first_problem.is_empty() {
                            first_problem = why.clone();
                        }
                        if failed_from_start >= 8 {
                            broken.store(true, Ordering::SeqCst);
                        }
                    }
                    if problems_told <= 5 {
                        logbus::warn("market", format!("{name}: {why}"));
                    } else {
                        logbus::debug("market", format!("{name}: {why}"));
                    }
                    no_price(name, &wallet, format!("Couldn't get a price: {why}"))
                }
                pricing::Answer::Busy => return,
            };
            emit_prices(&steam_id, &wallet, vec![info.clone()]);
            prices.insert(name.to_string(), info);
            done += 1;
            if !stop() {
                progress("prices", done, total, time_left(total - done, every));
            }
            if since_saved >= 10 || (since_saved > 0 && saved_at.elapsed() >= Duration::from_secs(30)) {
                since_saved = 0;
                saved_at = std::time::Instant::now();
                app.save_prices();
            }
        }
        pricing::News::Pausing { pause, every: slower } => {
            every = slower;
            if since_saved > 0 {
                since_saved = 0;
                saved_at = std::time::Instant::now();
                app.save_prices();
            }
            logbus::info(
                "market",
                format!(
                    "Steam asked IdleKit to slow down: price checks wait {} s, then go on one every {:.1} s",
                    pause.as_secs(),
                    slower.as_secs_f64()
                ),
            );
            progress("prices", done, total, format!("Steam asked to slow down: waiting {} s", pause.as_secs()));
        }
    })
    .await;
    // Saved before letting go, so the next run doesn't write the file at the same time.
    app.save_prices();
    drop(pacer);
    let took = started.elapsed().as_secs();
    let took = if took < 90 { format!("{took} s") } else { format!("{} min", took.div_ceil(60)) };
    match end {
        pricing::RunEnd::Done => {
            progress("prices", total, total, "Done");
            logbus::info(
                "market",
                format!(
                    "Checked {total} price(s) in {took}{}",
                    if unpriced > 0 { format!("; {unpriced} item(s) got no price") } else { String::new() }
                ),
            );
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
            if cancelled() {
                progress("prices", total, total, "Cancelled");
                logbus::info("market", format!("Stopped checking prices: {done} of {total} checked"));
            } else {
                logbus::debug("market", format!("A newer price check took over after {done} of {total}"));
            }
        }
        pricing::RunEnd::GaveUp { left } => {
            progress("prices", total, total, "Stopped");
            let why = format!(
                "Steam kept asking IdleKit to slow down, so {left} price(s) weren't checked. Try Refresh prices in a few minutes"
            );
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
            let r = market::sell(&ws, asset, seller).await.unwrap_or_else(|e| SellOutcome {
                asset_id: asset.clone(),
                ok: false,
                needs_confirmation: false,
                message: format!("{e:#}"),
            });
            if r.needs_confirmation {
                pending_confirmations += 1;
            }
            if !r.ok {
                logbus::warn("market", format!("{}: {}", item.market_hash_name, r.message));
            }
            out.push(r);
            done += 1;
            if done < total {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }
    progress("sell", total, total, "Done");
    let ok = out.iter().filter(|o| o.ok).count();
    logbus::info(
        "market",
        format!("Listed {ok} item(s){}", if pending_confirmations > 0 {
            format!("; {pending_confirmations} need confirming in the Steam mobile app")
        } else {
            String::new()
        }),
    );
    Ok(out)
}

#[tauri::command]
pub async fn market_listings(app: State<'_, App>, steam_id: String) -> CmdResult<Vec<Listing>> {
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    market::my_listings(&ws).await.map_err(anyhow_err)
}

#[tauri::command]
pub async fn market_remove(app: State<'_, App>, steam_id: String, listing_ids: Vec<String>) -> CmdResult<usize> {
    app.price_runs.fetch_add(1, Ordering::SeqCst);
    app.market_cancel.store(false, Ordering::SeqCst);
    let _job = app.market_job();
    let ws = app.web_session(&steam_id).await.map_err(anyhow_err)?;
    let delay = app.settings().market.delay_ms.max(500);
    let total = listing_ids.len();
    let mut removed = 0;
    for (i, id) in listing_ids.iter().enumerate() {
        if app.market_cancel.load(Ordering::SeqCst) {
            break;
        }
        progress("remove", i, total, format!("Removing listing {id}"));
        match market::remove_listing(&ws, id).await {
            Ok(()) => removed += 1,
            Err(e) => logbus::warn("market", format!("{e:#}")),
        }
        if i + 1 < total {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
    }
    progress("remove", total, total, "Done");
    Ok(removed)
}

#[tauri::command]
pub async fn web_check(app: State<'_, App>, steam_id: String) -> CmdResult<String> {
    let ws = app.web_session(&steam_id).await.map_err(err)?;
    ws.verify().await.map_err(anyhow_err)?;
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
