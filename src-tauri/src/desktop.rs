//! Desktop integration: the tray icon, what the window's close button does,
//! and starting IdleKit when the user signs in.
use crate::state::App;
use crate::{badge, logbus, updates};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Window, WindowEvent};
use tauri_plugin_autostart::AutoLaunchManager;

/// Passed to IdleKit when the system starts it at sign-in.
pub const AUTOSTART_ARG: &str = "--autostart";
const MAIN_WINDOW: &str = "main";
const TRAY_ID: &str = "main";
/// Left in the data folder before an update is installed, so the restarted app
/// opens its window even when it was originally started in the tray.
const SHOW_AFTER_UPDATE: &str = ".show-after-update";
/// The other way round, for an update installed by itself while IdleKit was in
/// the tray: the restarted app stays there.
const HIDDEN_AFTER_UPDATE: &str = ".hidden-after-update";

/// A tray icon exists. Without one the window is never hidden, because there
/// would be nothing to click to bring it back.
static TRAY_READY: AtomicBool = AtomicBool::new(false);
/// The "keep running or quit?" question is on screen.
static CLOSE_PROMPT_OPEN: AtomicBool = AtomicBool::new(false);
static LAST_TOOLTIP: Mutex<String> = Mutex::new(String::new());
/// Whether the tray icon shows its warning badge right now.
static TRAY_WARNS: AtomicBool = AtomicBool::new(false);
/// The tray icon as built, and the same with the warning badge.
static TRAY_ICONS: OnceLock<Option<(Image<'static>, Image<'static>)>> = OnceLock::new();

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DesktopInfo {
    /// "windows" | "macos" | "linux"
    pub platform: &'static str,
    /// A tray icon exists, so the window can be closed without quitting.
    pub tray: bool,
    /// IdleKit is registered to start at sign-in.
    pub autostart: bool,
}

pub fn info(handle: &AppHandle) -> DesktopInfo {
    DesktopInfo {
        platform: std::env::consts::OS,
        tray: tray_ready(),
        autostart: autostart_enabled(handle),
    }
}

fn tray_ready() -> bool {
    TRAY_READY.load(Ordering::SeqCst)
}

// ---------------------------------------------------------------- window --

pub fn show_main(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        // What was closed while the window was away (achievement lists) opens again.
        let _ = handle.emit("window://shown", ());
    }
}

fn hide_main(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
    // An open achievements list keeps its game running in Steam: not while
    // the window is in the tray, where nobody sees it.
    if let Some(app) = handle.try_state::<App>() {
        let app = app.inner().clone();
        tauri::async_runtime::spawn(async move { app.close_all_achievements().await });
    }
}

/// Whether this launch should stay in the tray: started at sign-in, with the
/// setting on. A launch right after an update always shows the window.
pub fn should_start_hidden(app: &App) -> bool {
    let after_update = std::fs::remove_file(app.data_dir.join(SHOW_AFTER_UPDATE)).is_ok();
    let hidden_after_update = std::fs::remove_file(app.data_dir.join(HIDDEN_AFTER_UPDATE)).is_ok();
    let at_sign_in = std::env::args().any(|arg| arg == AUTOSTART_ARG);
    let hidden = start_hidden(at_sign_in, app.settings().start_in_tray, tray_ready(), after_update, hidden_after_update);
    STARTED_HIDDEN.store(hidden, std::sync::atomic::Ordering::SeqCst);
    hidden
}

/// Whether this run began in the tray, with no window shown.
static STARTED_HIDDEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn started_hidden() -> bool {
    STARTED_HIDDEN.load(std::sync::atomic::Ordering::SeqCst)
}

fn start_hidden(at_sign_in: bool, start_in_tray: bool, tray: bool, after_update: bool, hidden_after_update: bool) -> bool {
    tray && !after_update && (hidden_after_update || (at_sign_in && start_in_tray))
}

pub fn mark_show_after_update(app: &App) {
    let _ = std::fs::remove_file(app.data_dir.join(HIDDEN_AFTER_UPDATE));
    let _ = std::fs::write(app.data_dir.join(SHOW_AFTER_UPDATE), b"");
}

pub fn mark_hidden_after_update(app: &App) {
    let _ = std::fs::remove_file(app.data_dir.join(SHOW_AFTER_UPDATE));
    let _ = std::fs::write(app.data_dir.join(HIDDEN_AFTER_UPDATE), b"");
}

pub fn clear_after_update(app: &App) {
    let _ = std::fs::remove_file(app.data_dir.join(SHOW_AFTER_UPDATE));
    let _ = std::fs::remove_file(app.data_dir.join(HIDDEN_AFTER_UPDATE));
}

/// Whether the window is on screen (not in the tray, not minimized).
pub fn main_window_visible(handle: &AppHandle) -> bool {
    handle
        .get_webview_window(MAIN_WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

/// What a click on the close button does, given the saved choice.
#[derive(Debug, PartialEq)]
enum CloseOutcome {
    Hide,
    Quit,
    Ask,
}

fn close_outcome(action: &str, tray: bool, already_asking: bool) -> CloseOutcome {
    match action {
        _ if !tray => CloseOutcome::Quit,
        "tray" => CloseOutcome::Hide,
        "quit" => CloseOutcome::Quit,
        // Closing again while the question is on screen means "just close".
        _ if already_asking => CloseOutcome::Quit,
        _ => CloseOutcome::Ask,
    }
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else { return };
    if window.label() != MAIN_WINDOW {
        return;
    }
    let handle = window.app_handle();
    let action = handle
        .try_state::<App>()
        .map(|app| app.settings().close_action)
        .unwrap_or_else(|| "quit".into());
    match close_outcome(&action, tray_ready(), CLOSE_PROMPT_OPEN.load(Ordering::SeqCst)) {
        CloseOutcome::Hide => {
            api.prevent_close();
            logbus::info("window", "Close button: kept running in the tray");
            hide_main(handle);
        }
        // Letting the last window close ends the app, which stops everything cleanly.
        CloseOutcome::Quit => logbus::info("window", "Close button: quitting"),
        CloseOutcome::Ask => {
            api.prevent_close();
            logbus::info("window", "Close button: asking whether to keep running");
            CLOSE_PROMPT_OPEN.store(true, Ordering::SeqCst);
            let _ = handle.emit("window://close-requested", ());
        }
    }
}

/// The answer to the close question: "tray", "quit", or anything else to stay open.
pub fn close_choice(handle: &AppHandle, action: &str) {
    CLOSE_PROMPT_OPEN.store(false, Ordering::SeqCst);
    match action {
        "tray" => hide_main(handle),
        "quit" => handle.exit(0),
        _ => {}
    }
}

// ------------------------------------------------------------------ tray --

pub fn setup_tray(handle: &AppHandle) {
    match build_tray(handle) {
        Ok(()) => TRAY_READY.store(true, Ordering::SeqCst),
        Err(e) => logbus::warn("app", format!("No tray icon ({e}), so closing the window quits IdleKit")),
    }
}

fn build_tray(handle: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(handle, "open", "Open IdleKit", true, None::<&str>)?;
    let update = MenuItem::with_id(handle, "update", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "Quit IdleKit", true, None::<&str>)?;
    let first_rule = PredefinedMenuItem::separator(handle)?;
    let second_rule = PredefinedMenuItem::separator(handle)?;
    let menu = Menu::with_items(handle, &[&open, &first_rule, &update, &second_rule, &quit])?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("IdleKit")
        .menu(&menu)
        // Left click opens the window; the menu is on right click.
        .show_menu_on_left_click(false)
        .on_menu_event(|handle, event| match event.id().as_ref() {
            "open" => {
                logbus::info("tray", "Menu: Open IdleKit");
                show_main(handle);
            }
            "update" => {
                logbus::info("tray", "Menu: Check for updates");
                show_main(handle);
                let _ = handle.emit("app://navigate", "settings");
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let app = handle.state::<App>().inner().clone();
                    updates::check(&app, &handle).await;
                });
            }
            "quit" => {
                logbus::info("tray", "Menu: Quit IdleKit");
                handle.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                logbus::info("tray", "Icon clicked: opening the window");
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = handle.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(handle)?;
    Ok(())
}

fn tray_text(games: usize, farming: bool, unlocking: bool, not_signed_in: bool) -> String {
    let mut doing = Vec::new();
    if games > 0 {
        doing.push(format!("idling {games} {}", if games == 1 { "game" } else { "games" }));
    }
    if farming {
        doing.push("farming cards".to_string());
    }
    if unlocking {
        doing.push("unlocking achievements".to_string());
    }
    let mut text = if doing.is_empty() { "IdleKit".to_string() } else { format!("IdleKit: {}", doing.join(", ")) };
    if not_signed_in {
        // What the badge on the icon means.
        text.push_str(if doing.is_empty() { ": " } else { "; " });
        text.push_str("Steam client not signed in");
    }
    text
}

/// Whether the account selected in the window goes through the Steam client,
/// and the client isn't signed in to it: the top bar then says "steam client
/// not signed in", and the tray icon wears its warning badge.
fn steam_not_signed_in(app: &App) -> bool {
    let selected = app.with_config(|c| {
        let account = c.active_account.as_deref().and_then(|id| c.account(id))?;
        (account.mode != "direct").then(|| account.steam_id.clone())
    });
    let Some(steam_id) = selected else { return false };
    app.local_steam().active_steam_id.as_deref() != Some(steam_id.as_str())
}

/// The tray icon as built, and the same with the warning badge, made once.
fn tray_icons(handle: &AppHandle) -> Option<&'static (Image<'static>, Image<'static>)> {
    TRAY_ICONS
        .get_or_init(|| {
            let icon = handle.default_window_icon()?;
            let (width, height) = (icon.width(), icon.height());
            let plain = Image::new_owned(icon.rgba().to_vec(), width, height);
            let warning = Image::new_owned(badge::with_warning_badge(icon.rgba(), width, height), width, height);
            Some((plain, warning))
        })
        .as_ref()
}

/// Keep the tray icon's hover text in step with what is running, and its
/// warning badge with whether the Steam client is signed in.
pub fn refresh_tray(handle: &AppHandle, app: &App) {
    if !tray_ready() {
        return;
    }
    let games = app.idler.all().iter().map(|account| account.games.len()).sum();
    let farming = app.farming.all().iter().any(|f| f.running);
    let unlocking = app.unlocker.all().iter().any(|u| u.running);
    let not_signed_in = steam_not_signed_in(app);
    let text = tray_text(games, farming, unlocking, not_signed_in);
    let Some(tray) = handle.tray_by_id(TRAY_ID) else { return };
    {
        let mut last = LAST_TOOLTIP.lock().unwrap();
        if *last != text {
            let _ = tray.set_tooltip(Some(&text));
            *last = text;
        }
    }
    if TRAY_WARNS.load(Ordering::SeqCst) != not_signed_in {
        if let Some((plain, warning)) = tray_icons(handle) {
            let icon = if not_signed_in { warning } else { plain };
            if tray.set_icon(Some(icon.clone())).is_ok() {
                TRAY_WARNS.store(not_signed_in, Ordering::SeqCst);
                logbus::debug(
                    "tray",
                    if not_signed_in {
                        "The Steam client isn't signed in to the selected account: the tray icon shows a warning"
                    } else {
                        "The Steam client is signed in to the selected account: the tray icon's warning is gone"
                    },
                );
            }
        }
    }
}

// ------------------------------------------------------- start at sign-in --

pub fn autostart_enabled(handle: &AppHandle) -> bool {
    handle
        .try_state::<AutoLaunchManager>()
        .and_then(|manager| manager.is_enabled().ok())
        .unwrap_or(false)
}

/// Register or unregister IdleKit to start at sign-in. Returns the state afterwards.
pub fn set_autostart(handle: &AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = handle
        .try_state::<AutoLaunchManager>()
        .ok_or("Starting at sign-in isn't available on this system")?;
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(|e| format!("Couldn't change the startup setting: {e}"))?;
    logbus::info(
        "app",
        if enabled { "IdleKit will start when you sign in" } else { "IdleKit will no longer start when you sign in" },
    );
    Ok(autostart_enabled(handle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_button_follows_the_saved_choice() {
        assert_eq!(close_outcome("tray", true, false), CloseOutcome::Hide);
        assert_eq!(close_outcome("quit", true, false), CloseOutcome::Quit);
        assert_eq!(close_outcome("ask", true, false), CloseOutcome::Ask);
        // an unknown value is treated as "ask" rather than hiding or quitting unasked
        assert_eq!(close_outcome("", true, false), CloseOutcome::Ask);
        // closing again while the question is showing just closes
        assert_eq!(close_outcome("ask", true, true), CloseOutcome::Quit);
        // with no tray icon the window is never hidden
        assert_eq!(close_outcome("tray", false, false), CloseOutcome::Quit);
        assert_eq!(close_outcome("ask", false, false), CloseOutcome::Quit);
    }

    #[test]
    fn stays_in_the_tray_only_when_started_at_sign_in() {
        assert!(start_hidden(true, true, true, false, false));
        assert!(!start_hidden(false, true, true, false, false), "opened by the user");
        assert!(!start_hidden(true, false, true, false, false), "setting off");
        assert!(!start_hidden(true, true, false, false, false), "no tray to come back from");
        assert!(!start_hidden(true, true, true, true, false), "just updated from the window");
        // updated by itself while in the tray: back in the tray, however it was started
        assert!(start_hidden(false, false, true, false, true));
        assert!(!start_hidden(false, false, false, false, true), "no tray to come back from");
        assert!(!start_hidden(true, true, true, true, true), "asked for from the window wins");
    }

    #[test]
    fn tray_text_says_what_is_running() {
        assert_eq!(tray_text(0, false, false, false), "IdleKit");
        assert_eq!(tray_text(1, false, false, false), "IdleKit: idling 1 game");
        assert_eq!(tray_text(3, true, false, false), "IdleKit: idling 3 games, farming cards");
        // With the warning badge on the icon, the hover text says what it means.
        assert_eq!(tray_text(0, false, false, true), "IdleKit: Steam client not signed in");
        assert_eq!(tray_text(2, false, false, true), "IdleKit: idling 2 games; Steam client not signed in");
        assert_eq!(tray_text(0, false, true, false), "IdleKit: unlocking achievements");
    }
}
