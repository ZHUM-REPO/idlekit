mod commands;
mod config;
mod desktop;
mod features;
mod logbus;
mod secrets;
mod state;
mod steam;
mod updates;

use state::App;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// Work done once at launch: reconnect direct accounts, auto-idle, auto-farm,
/// and a look for a newer version.
async fn startup(app: App, handle: AppHandle) {
    let cfg = app.config();
    let s = cfg.settings.clone();
    logbus::info("app", format!("IdleKit {} started", env!("CARGO_PKG_VERSION")));
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
    if s.connect_direct_on_launch {
        for a in cfg.accounts.iter().filter(|a| a.mode == "direct") {
            if secrets::get(&secrets::refresh_token_key(&a.steam_id)).is_none() {
                continue;
            }
            if let Err(e) = app.cm.connect(&a.steam_id).await {
                logbus::warn("direct", format!("{}: {e:#}", a.account_name));
            }
        }
    }
    if s.auto_idle_on_launch {
        for a in cfg.accounts.iter().filter(|a| !a.auto_idle.is_empty()) {
            let _ = app.load_games(&a.steam_id, false).await;
            let games = a.auto_idle.iter().map(|id| (*id, app.game_name(&a.steam_id, *id))).collect();
            if let Err(e) = app.idler.start(&app, &a.steam_id, games, "auto", None).await {
                logbus::warn("idle", format!("Auto-idle for {} failed: {e:#}", a.account_name));
            }
        }
    }
    if s.farm_on_launch {
        if let Some(active) = cfg.active_account.clone() {
            if let Err(e) = app.farming.start(app.clone(), active) {
                logbus::warn("farming", format!("{e:#}"));
            }
        }
    }
    if s.check_updates {
        // Not the first thing the app does: Steam connections come first.
        tokio::time::sleep(Duration::from_secs(8)).await;
        updates::check(&app, &handle).await;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // reqwest and the websocket client both use rustls; pick one provider explicitly.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tauri::Builder::default()
        // Starting IdleKit again while it sits in the tray brings its window back.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| desktop::show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().arg(desktop::AUTOSTART_ARG).build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_window_event(desktop::on_window_event)
        .setup(|app| {
            logbus::init(app.handle().clone());
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            secrets::init(data_dir.clone());
            let state = App::new(data_dir);
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
            commands::update_check,
            commands::update_install,
        ])
        .build(tauri::generate_context!())
        .expect("error while building IdleKit")
        .run(|handle, event| {
            if let tauri::RunEvent::Exit = event {
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
