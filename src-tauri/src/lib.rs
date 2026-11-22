mod commands;
mod config;
mod desktop;
mod features;
mod logbus;
mod secrets;
mod state;
mod steam;
mod updates;

use config::Account;
use state::App;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// How long auto-idle and farm-on-launch wait for Steam before giving up.
const STARTUP_WAIT: Duration = Duration::from_secs(5 * 60);
const STARTUP_POLL: Duration = Duration::from_secs(5);

/// The account can idle right now: its Steam client is running and signed in
/// to it, or its direct connection is online.
fn can_idle(app: &App, account: &Account) -> bool {
    if account.mode == "direct" {
        app.cm.is_online(&account.steam_id)
    } else {
        app.require_local_client(&account.steam_id).is_ok()
    }
}

/// Started at sign-in, IdleKit is usually up before the Steam client has
/// finished starting, and sometimes before the network is. Waits for the
/// account to become usable, so "start on launch" doesn't fail on the spot.
async fn wait_until_usable(app: &App, account: &Account, wait: Duration, poll: Duration) -> bool {
    if can_idle(app, account) {
        return true;
    }
    let waiting_for = if account.mode == "direct" {
        if secrets::get(&secrets::refresh_token_key(&account.steam_id)).is_none() {
            return false;
        }
        if !app.settings().connect_direct_on_launch {
            // Nothing has tried to connect yet. A failed attempt keeps retrying by itself.
            let _ = app.cm.connect(&account.steam_id).await;
        }
        "the connection to Steam"
    } else {
        if app.local_steam().install_path.is_none() {
            return false;
        }
        "the Steam client to start and sign in"
    };
    if can_idle(app, account) {
        return true;
    }
    logbus::info("app", format!("{}: waiting for {waiting_for}", logbus::label(&account.steam_id)));
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        tokio::time::sleep(poll).await;
        if can_idle(app, account) {
            return true;
        }
    }
    false
}

/// Work done once at launch: reconnect direct accounts, auto-idle, auto-farm,
/// and a look for a newer version.
async fn startup(app: App, handle: AppHandle) {
    let cfg = app.config();
    let s = cfg.settings.clone();
    logbus::info("app", format!("IdleKit {} started", updates::shown(env!("CARGO_PKG_VERSION"))));
    // With "install updates by itself" on: before anything is started, so a
    // newer version is installed while nothing is running yet. Comes back
    // unless it installed one.
    updates::update_before_starting(&app, &handle).await;
    // On its own, so waiting for Steam below doesn't hold it up. It looks a
    // few seconds from now and then every half hour for as long as IdleKit
    // runs, whenever "look for updates" is on.
    tokio::spawn(updates::keep_looking(app.clone(), handle.clone()));
    if !app.helpers.helper_path().exists() {
        logbus::warn(
            "app",
            format!(
                "steam-helper not found at {} — local Steam client features won't work",
                app.helpers.helper_path().display()
            ),
        );
    }
    for a in cfg.accounts.iter() {
        app.cm.configure(&a.steam_id, &a.account_name, a.persona_state, &s.idle_status_text);
    }
    // An account whose profile couldn't be read when it was added (no avatar
    // yet) gets another try at each start, once the first rush is over.
    for a in cfg.accounts.iter().filter(|a| a.avatar_url.is_empty()) {
        let (app, steam_id) = (app.clone(), a.steam_id.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(12)).await;
            app.fetch_profile(&steam_id).await;
        });
    }
    if s.connect_direct_on_launch {
        for a in cfg.accounts.iter().filter(|a| a.mode == "direct") {
            if secrets::get(&secrets::refresh_token_key(&a.steam_id)).is_none() {
                continue;
            }
            if let Err(e) = app.cm.connect(&a.steam_id).await {
                logbus::warn("direct", format!("{}: {e:#}", logbus::label(&a.steam_id)));
            }
        }
    }
    if s.auto_idle_on_launch {
        for a in cfg.accounts.iter().filter(|a| !a.auto_idle.is_empty()) {
            // Whether or not Steam turned up: starting says what is wrong if it didn't.
            wait_until_usable(&app, a, STARTUP_WAIT, STARTUP_POLL).await;
            let _ = app.load_games(&a.steam_id, false).await;
            let games = a.auto_idle.iter().map(|id| (*id, app.game_name(&a.steam_id, *id))).collect();
            if let Err(e) = app.idler.start(&app, &a.steam_id, games, "auto", None).await {
                logbus::warn("idle", format!("Auto-idle for {} failed: {e:#}", logbus::label(&a.steam_id)));
            }
        }
    }
    if s.farm_on_launch {
        if let Some(active) = cfg.active_account.clone() {
            if let Some(account) = cfg.account(&active) {
                wait_until_usable(&app, account, STARTUP_WAIT, STARTUP_POLL).await;
            }
            if let Err(e) = app.farming.start(app.clone(), active) {
                logbus::warn("farming", format!("{e:#}"));
            }
        }
    }
}

async fn ticker(app: App) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        app.idler.tick(&app).await;
    }
}

/// Keeps the tray icon's hover text in step with what is running.
async fn tray_ticker(app: App, handle: AppHandle) {
    loop {
        desktop::refresh_tray(&handle, &app);
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// One file from the pictures folder, and only a file the app itself wrote there.
fn serve_picture(handle: &AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    use steam::pictures;
    let file = path.trim_start_matches('/');
    let bytes = pictures::parse_name(file)
        .and_then(|_| handle.path().app_cache_dir().ok())
        .and_then(|dir| std::fs::read(dir.join(pictures::FOLDER).join(file)).ok());
    let reply = tauri::http::Response::builder().header("Access-Control-Allow-Origin", "*");
    match bytes {
        Some(bytes) => reply
            .header("Content-Type", pictures::content_type(file))
            // A changed picture gets a new file name, so a file never changes.
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .body(bytes),
        None => reply.status(404).body(Vec::new()),
    }
    .unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logbus::log_panics();
    // reqwest and the websocket client both use rustls; pick one provider explicitly.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tauri::Builder::default()
        // Starting IdleKit again while it sits in the tray brings its window back.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| desktop::show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().arg(desktop::AUTOSTART_ARG).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_window_event(desktop::on_window_event)
        // Pictures kept on disk reach the window through this address:
        // pic://localhost/<file> (http://pic.localhost/<file> on Windows).
        .register_uri_scheme_protocol("pic", |ctx, request| serve_picture(ctx.app_handle(), request.uri().path()))
        .setup(|app| {
            logbus::init(app.handle().clone());
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            secrets::init(data_dir.clone());
            // Pictures are bulky and can always be fetched again, so they go
            // in the cache folder (%LOCALAPPDATA% on Windows), not with the settings.
            let pictures_dir = app.path().app_cache_dir()?.join(steam::pictures::FOLDER);
            let state = App::new(data_dir.clone(), pictures_dir);
            // Names first, so nothing written to the file can carry one.
            state.hide_names();
            let home = app.path().home_dir().ok();
            let folder = home.as_ref().and_then(|h| h.file_name()).map(|n| n.to_string_lossy().into_owned());
            for name in [folder, std::env::var("USERNAME").ok(), std::env::var("USER").ok()].into_iter().flatten() {
                logbus::hide_user(&name);
            }
            logbus::open_file(
                &data_dir,
                &format!(
                    "IdleKit {} started on {} {}, web view {}, launched at sign-in: {}",
                    updates::shown(env!("CARGO_PKG_VERSION")),
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    tauri::webview_version().unwrap_or_else(|_| "unknown".into()),
                    if std::env::args().any(|arg| arg == desktop::AUTOSTART_ARG) { "yes" } else { "no" },
                ),
            );
            if let Ok(res) = app.path().resource_dir() {
                state.helpers.add_lib_dir(res.join("resources"));
                state.helpers.add_lib_dir(res);
            }
            let hook_app = state.clone();
            state.helpers.set_on_exit(Arc::new(move |appid, unexpected| {
                if unexpected {
                    hook_app.idler.on_helper_exit(appid);
                }
            }));
            app.manage(state.clone());
            desktop::setup_tray(app.handle());
            // The window is created hidden so a start at sign-in can stay in the tray.
            if !desktop::should_start_hidden(&state) {
                desktop::show_main(app.handle());
            }
            tauri::async_runtime::spawn(startup(state.clone(), app.handle().clone()));
            tauri::async_runtime::spawn(tray_ticker(state.clone(), app.handle().clone()));
            tauri::async_runtime::spawn(ticker(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::get_accounts,
            commands::save_settings,
            commands::set_active_account,
            commands::detect_local_steam,
            commands::get_logs,
            commands::clear_logs,
            commands::add_local_account,
            commands::auth_start_qr,
            commands::auth_start_credentials,
            commands::auth_submit_code,
            commands::auth_cancel,
            commands::set_web_cookie,
            commands::remove_account,
            commands::set_account_mode,
            commands::account_connect,
            commands::account_disconnect,
            commands::set_persona_state,
            commands::get_games,
            commands::add_manual_game,
            commands::remove_manual_game,
            commands::set_in_list,
            commands::set_in_list_many,
            commands::resolve_game_art,
            commands::idle_start,
            commands::idle_stop,
            commands::idle_stop_all,
            commands::auto_idle_start,
            commands::booster_start,
            commands::farm_scan,
            commands::farm_start,
            commands::farm_stop,
            commands::farm_status,
            commands::ach_open,
            commands::ach_close,
            commands::ach_apply,
            commands::ach_set_stats,
            commands::ach_reset_all,
            commands::ach_set_prefs,
            commands::ach_import_timings,
            commands::ach_clear_imported,
            commands::unlock_start,
            commands::unlock_stop,
            commands::market_inventory,
            commands::market_cancel,
            commands::market_prices,
            commands::market_sell,
            commands::market_listings,
            commands::market_remove,
            commands::web_check,
            commands::set_autostart,
            commands::window_close_choice,
            commands::window_control,
            commands::update_check,
            commands::update_install,
            commands::log_event,
            commands::pictures_cached,
            commands::picture_fetch,
            commands::open_log_file,
        ])
        .build(tauri::generate_context!())
        .expect("error while building IdleKit")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
                logbus::info("app", "IdleKit is closing");
                let app = handle.state::<App>().inner().clone();
                tauri::async_runtime::block_on(async move {
                    app.farming.stop_everything();
                    app.unlocker.stop_everything();
                    let _ = tokio::time::timeout(Duration::from_secs(4), async {
                        app.helpers.stop_all().await;
                        app.cm.disconnect_all().await;
                    })
                    .await;
                });
            }
        });
}
