//! Self-update. A build made by the release workflow knows where its releases
//! are published. It looks there for a newer version, downloads the installer,
//! checks its signature against the public key in the app's configuration, and
//! hands over to it. Builds made any other way have nowhere to look and say so.
use crate::state::App;
use crate::{desktop, logbus};
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

const EVENT: &str = "update://state";
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The look made before anything is started: short, so that a PC with no
/// connection yet isn't kept waiting for it.
const STARTUP_LOOK_TIMEOUT: Duration = Duration::from_secs(8);
/// A look that fails is tried once more after this pause before it counts as failed.
const SECOND_TRY_AFTER: Duration = Duration::from_secs(4);
/// The installer is a few megabytes; this only guards against a stalled connection.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// The first look after starting waits this long: Steam connections come first.
const FIRST_LOOK: Duration = Duration::from_secs(8);
/// IdleKit can run for weeks in the tray, so it keeps looking while it runs.
/// One look is a file of two kilobytes.
const LOOK_EVERY: Duration = Duration::from_secs(30 * 60);
/// After a look that failed (no connection, a slow one) it looks again soon,
/// then at growing gaps, and from the fourth failure on every ten minutes.
const LOOK_AGAIN: [Duration; 3] = [Duration::from_secs(60), Duration::from_secs(2 * 60), Duration::from_secs(5 * 60)];
const LOOK_AGAIN_LATER: Duration = Duration::from_secs(10 * 60);
const CHECK_FAILED: &str = "Couldn't check for updates";
/// With "install updates by itself" on: how often to see whether a version
/// that was found can be installed now.
const INSTALL_CHECK_EVERY: Duration = Duration::from_secs(60);
/// How often one run tries to install a version by itself before leaving it to the user.
const AUTO_TRIES: u32 = 2;
/// Written before an automatic install. If IdleKit comes back still older than
/// that version, it doesn't install it by itself again for this long: an
/// install that doesn't take must not turn into restart after restart.
const ATTEMPT_FILE: &str = "update-attempt.json";
const ATTEMPT_REST_SECS: i64 = 12 * 60 * 60;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    pub notes: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    /// False when this build has nowhere to look for updates.
    pub configured: bool,
    pub current: String,
    pub phase: Phase,
    pub available: Option<Available>,
    /// Unix time in milliseconds of the last look that got an answer.
    pub checked_at: Option<i64>,
    pub error: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    /// Where releases are listed, for "what's new".
    pub releases_url: Option<String>,
}

#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Downloading,
    Installing,
}

#[derive(Default)]
pub struct Updates {
    state: Mutex<UpdateState>,
    /// The release the last check found, kept so installing doesn't look again.
    found: Mutex<Option<Update>>,
    /// How many times this run has tried to install a version by itself.
    auto_tries: Mutex<u32>,
    /// The last thing said in the log about why an automatic install is waiting.
    waiting_said: Mutex<String>,
}

/// An automatic install that was started: which version, and when.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
struct Attempt {
    version: String,
    at: i64,
}

/// What is going on that an install would cut short.
#[derive(Default, Debug, Clone, Copy)]
pub struct Activity {
    pub idling: usize,
    pub farming: bool,
    pub unlocking: bool,
    pub market: bool,
    pub achievements: bool,
}

/// The first thing that stands in the way of installing right now, if any.
fn holds_up(a: &Activity) -> Option<&'static str> {
    if a.idling > 0 {
        Some("games are idling")
    } else if a.farming {
        Some("card farming is running")
    } else if a.unlocking {
        Some("the achievement unlocker is running")
    } else if a.market {
        Some("market work is under way")
    } else if a.achievements {
        Some("a game's achievements are open")
    } else {
        None
    }
}

fn activity(app: &App) -> Activity {
    Activity {
        idling: app.idler.all().iter().map(|account| account.games.len()).sum(),
        farming: app.farming.all().iter().any(|f| f.running),
        unlocking: app.unlocker.all().iter().any(|u| u.running),
        market: app.market_busy.load(std::sync::atomic::Ordering::SeqCst) > 0,
        achievements: !app.ach_open.lock().unwrap().is_empty(),
    }
}

/// Whether an automatic install of `version` was started not long ago. Being
/// here again to ask means it didn't take.
fn tried_lately(attempt: Option<&Attempt>, version: &str, now: i64) -> bool {
    attempt.is_some_and(|a| a.version == version && a.at <= now + 60 && now - a.at < ATTEMPT_REST_SECS)
}

fn read_attempt(app: &App) -> Option<Attempt> {
    let text = std::fs::read_to_string(app.data_dir.join(ATTEMPT_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// A reqwest error says "error sending request" and keeps the reason (timed
/// out, name not found, connection reset) one level down: say that too.
fn with_causes(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        let more = c.to_string();
        if !more.is_empty() && !text.contains(&more) {
            text.push_str(": ");
            text.push_str(&more);
        }
        cause = c.source();
    }
    text
}

/// Where this build looks for updates. Empty for builds not made by the release workflow.
fn endpoints(handle: &AppHandle) -> Vec<Url> {
    // For trying the updater against a local server while developing.
    #[cfg(debug_assertions)]
    if let Some(url) = std::env::var("IDLEKIT_UPDATE_ENDPOINT").ok().and_then(|v| Url::parse(v.trim()).ok()) {
        return vec![url];
    }
    handle
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("endpoints"))
        .and_then(|list| list.as_array())
        .map(|list| list.iter().filter_map(|v| v.as_str()).filter_map(|s| Url::parse(s).ok()).collect())
        .unwrap_or_default()
}

/// The page listing releases, when the update address is a GitHub repository's.
fn releases_page(endpoint: &Url) -> Option<String> {
    if endpoint.host_str() != Some("github.com") {
        return None;
    }
    let mut parts = endpoint.path_segments()?;
    let (owner, repo) = (parts.next()?, parts.next()?);
    (parts.next() == Some("releases")).then(|| format!("https://github.com/{owner}/{repo}/releases/latest"))
}

pub fn snapshot(app: &App, handle: &AppHandle) -> UpdateState {
    let endpoints = endpoints(handle);
    let mut state = app.updates.state.lock().unwrap().clone();
    state.configured = !endpoints.is_empty();
    state.current = shown(&handle.package_info().version.to_string());
    state.releases_url = endpoints.first().and_then(releases_page);
    state
}

fn publish(app: &App, handle: &AppHandle) {
    logbus::emit(EVENT, snapshot(app, handle));
}

fn explain(e: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match e {
        Error::EmptyEndpoints => "this build doesn't know where to look for updates".into(),
        Error::ReleaseNotFound => "no published release was found".into(),
        Error::Minisign(_) | Error::SignedVersionMismatch { .. } | Error::MissingSignedVersion => {
            "the download didn't match the release signature, so it was not installed".into()
        }
        other => with_causes(other),
    }
}

/// How long to wait before looking again, given how many looks in a row have failed.
fn next_look(failures_in_a_row: u32) -> Duration {
    match failures_in_a_row {
        0 => LOOK_EVERY,
        n => LOOK_AGAIN.get(n as usize - 1).copied().unwrap_or(LOOK_AGAIN_LATER),
    }
}

/// Says once why an automatic install is waiting, not at every look.
fn say_waiting(app: &App, text: String, warn: bool) {
    let mut said = app.updates.waiting_said.lock().unwrap();
    if *said == text {
        return;
    }
    if warn {
        logbus::warn("update", text.clone());
    } else {
        logbus::info("update", text.clone());
    }
    *said = text;
}

/// With "install updates by itself" on: installs the version the last look
/// found, if nothing is running that an install would cut short. Comes back
/// only when it didn't install. `starting` is for the look made before
/// anything is started, when the window may not be up yet.
async fn install_by_itself(app: &App, handle: &AppHandle, starting: bool) {
    let settings = app.settings();
    if !settings.check_updates || !settings.auto_install_updates {
        return;
    }
    let version = {
        let state = app.updates.state.lock().unwrap();
        if state.phase != Phase::Idle {
            return;
        }
        match &state.available {
            Some(a) => a.version.clone(),
            None => return,
        }
    };
    if *app.updates.auto_tries.lock().unwrap() >= AUTO_TRIES {
        return;
    }
    if tried_lately(read_attempt(app).as_ref(), &version, chrono::Utc::now().timestamp()) {
        say_waiting(
            app,
            format!("IdleKit {version} was installed by itself not long ago, but this is still an older version. Not trying again by itself: install it from Settings"),
            true,
        );
        return;
    }
    if let Some(what) = holds_up(&activity(app)) {
        say_waiting(app, format!("IdleKit {version} will be installed by itself once nothing is running ({what})"), false);
        return;
    }
    *app.updates.auto_tries.lock().unwrap() += 1;
    logbus::info("update", format!("Installing IdleKit {version} by itself: nothing is running"));
    // The window comes back the way it was: open if it is open now, in the
    // tray if not. While starting, what counts is how this run was started.
    let shown_now = if starting { !desktop::started_hidden() } else { desktop::main_window_visible(handle) };
    let _ = install_with(app, handle, Some(shown_now)).await;
}

/// Before anything is started: with "install updates by itself" on, a short
/// look, and an install right away if there is a newer version. Nothing is
/// running yet, so nothing is cut short, and what starts at launch starts in
/// the new version.
pub async fn update_before_starting(app: &App, handle: &AppHandle) {
    let settings = app.settings();
    if endpoints(handle).is_empty() || !settings.check_updates || !settings.auto_install_updates {
        return;
    }
    look(app, handle, true, STARTUP_LOOK_TIMEOUT).await;
    install_by_itself(app, handle, true).await;
}

/// Clears the note of an automatic install once IdleKit is at that version.
fn settle_last_attempt(app: &App, handle: &AppHandle) {
    let Some(attempt) = read_attempt(app) else { return };
    let current = shown(&handle.package_info().version.to_string());
    if attempt.version == current {
        logbus::info("update", format!("IdleKit updated itself to {current}"));
        let _ = std::fs::remove_file(app.data_dir.join(ATTEMPT_FILE));
    }
}

/// Looks for a newer version shortly after starting and then for as long as
/// IdleKit runs, while "look for updates" is on in Settings. One look at
/// startup is not enough: a copy left running never learned of a release, and
/// one failed look (a busy connection at startup) was never repeated.
pub async fn keep_looking(app: App, handle: AppHandle) {
    if endpoints(&handle).is_empty() {
        return;
    }
    let every = |normal: Duration| -> Duration {
        // For trying this out while developing: every look this many seconds apart.
        #[cfg(debug_assertions)]
        if let Some(secs) = std::env::var("IDLEKIT_UPDATE_EVERY_SECS").ok().and_then(|v| v.trim().parse().ok()) {
            return Duration::from_secs(secs);
        }
        normal
    };
    settle_last_attempt(&app, &handle);
    tokio::time::sleep(FIRST_LOOK).await;
    let mut failures = 0u32;
    let mut first = true;
    loop {
        if app.settings().check_updates {
            // The first look reports like one asked for by hand; the ones
            // after it stay out of the way unless they find something.
            let failed = look(&app, &handle, !first, CHECK_TIMEOUT).await.1;
            failures = if failed { failures + 1 } else { 0 };
            first = false;
        } else {
            failures = 0;
        }
        // Until the next look: now and once a minute, see whether a version
        // that was found can be installed by itself.
        let mut left = every(next_look(failures));
        loop {
            install_by_itself(&app, &handle, false).await;
            if left.is_zero() {
                break;
            }
            let step = left.min(INSTALL_CHECK_EVERY);
            tokio::time::sleep(step).await;
            left -= step;
        }
    }
}

/// Look for a newer version. Does nothing while a check or an install is under way.
pub async fn check(app: &App, handle: &AppHandle) -> UpdateState {
    look(app, handle, false, CHECK_TIMEOUT).await.0
}

/// One look for a newer version; also says whether it failed. A quiet look is
/// one nobody asked for: when it fails, the failure goes to the log and is not
/// put in front of the user, and whatever was on screen stays.
async fn look(app: &App, handle: &AppHandle, quiet: bool, timeout: Duration) -> (UpdateState, bool) {
    let endpoints = endpoints(handle);
    if endpoints.is_empty() {
        return (snapshot(app, handle), false);
    }
    {
        let mut state = app.updates.state.lock().unwrap();
        if state.phase != Phase::Idle {
            drop(state);
            return (snapshot(app, handle), false);
        }
        state.phase = Phase::Checking;
        if !quiet {
            state.error = None;
        }
    }
    publish(app, handle);

    let ask = || async {
        let updater = handle.updater_builder().endpoints(endpoints.clone())?.timeout(timeout).build()?;
        updater.check().await
    };
    let mut found = ask().await;
    // A connection that hiccups once is common: one more try before calling it failed.
    if found.is_err() && timeout >= CHECK_TIMEOUT {
        tokio::time::sleep(SECOND_TRY_AFTER).await;
        found = ask().await;
    }

    let failed = found.is_err();
    {
        let mut state = app.updates.state.lock().unwrap();
        state.phase = Phase::Idle;
        let current = shown(&handle.package_info().version.to_string());
        // A look that worked answers an earlier "couldn't check".
        if !failed && state.error.as_deref().is_some_and(|e| e.starts_with(CHECK_FAILED)) {
            state.error = None;
        }
        match found {
            Ok(Some(update)) => {
                state.checked_at = Some(chrono::Utc::now().timestamp_millis());
                let version = shown(&update.version);
                // Said once per version, not at every look that finds it again.
                if state.available.as_ref().map(|a| a.version.as_str()) != Some(version.as_str()) {
                    logbus::info("update", format!("IdleKit {version} is available; this is {current}"));
                }
                state.available = Some(Available { version, notes: update.body.clone().unwrap_or_default() });
                *app.updates.found.lock().unwrap() = Some(update);
            }
            Ok(None) => {
                state.checked_at = Some(chrono::Utc::now().timestamp_millis());
                let line = format!("Looked for a newer version: {current} is the newest");
                if quiet {
                    logbus::debug("update", line);
                } else {
                    logbus::info("update", line);
                }
                state.available = None;
                *app.updates.found.lock().unwrap() = None;
            }
            Err(e) => {
                let why = explain(&e);
                if quiet {
                    logbus::debug("update", format!("{CHECK_FAILED}: {why}. Looking again later"));
                } else {
                    logbus::warn("update", format!("{CHECK_FAILED}: {why}"));
                    state.error = Some(format!("{CHECK_FAILED}: {why}"));
                }
            }
        }
    }
    publish(app, handle);
    (snapshot(app, handle), failed)
}

/// Download the release the last check found and install it. On Windows this
/// ends the process once the installer has started; the installer starts the
/// new version when it is done.
pub async fn install(app: &App, handle: &AppHandle) -> Result<(), String> {
    install_with(app, handle, None).await
}

/// `by_itself` is set for an automatic install, and says whether the window
/// was open: the restarted app shows its window only if it was.
async fn install_with(app: &App, handle: &AppHandle, by_itself: Option<bool>) -> Result<(), String> {
    let Some(mut update) = app.updates.found.lock().unwrap().clone() else {
        return Err("Check for updates first".into());
    };
    {
        let mut state = app.updates.state.lock().unwrap();
        if state.phase != Phase::Idle {
            return Err("An update is already being installed".into());
        }
        state.phase = Phase::Downloading;
        state.downloaded = 0;
        state.total = None;
        state.error = None;
    }
    publish(app, handle);

    update.timeout = Some(DOWNLOAD_TIMEOUT);
    let failure = match download_and_hand_over(app, handle, &update, by_itself).await {
        Ok(never) => match never {},
        Err(e) => {
            let mut text = format!("Couldn't install the update: {}", explain(&e));
            // Getting as far as the install step means everything was stopped to free the files.
            if app.updates.state.lock().unwrap().phase == Phase::Installing {
                text.push_str(". Idling, card farming and unlocking were stopped for it and need starting again");
            }
            text
        }
    };
    desktop::clear_after_update(app);
    let _ = std::fs::remove_file(app.data_dir.join(ATTEMPT_FILE));
    logbus::error("update", failure.clone());
    {
        let mut state = app.updates.state.lock().unwrap();
        state.phase = Phase::Idle;
        state.error = Some(failure.clone());
    }
    publish(app, handle);
    Err(failure)
}

/// Returns only on failure: success ends in the installer or a restart.
async fn download_and_hand_over(
    app: &App,
    handle: &AppHandle,
    update: &Update,
    by_itself: Option<bool>,
) -> Result<std::convert::Infallible, tauri_plugin_updater::Error> {
    let mut last_published = Instant::now();
    let bytes = update
        .download(
            |chunk, total| {
                {
                    let mut state = app.updates.state.lock().unwrap();
                    state.downloaded += chunk as u64;
                    state.total = total;
                }
                if last_published.elapsed() >= PROGRESS_EVERY {
                    last_published = Instant::now();
                    publish(app, handle);
                }
            },
            || {},
        )
        .await?;

    app.updates.state.lock().unwrap().phase = Phase::Installing;
    publish(app, handle);
    logbus::info("update", format!("Installing IdleKit {}", shown(&update.version)));

    // The installer replaces the helper and Steam's library, so nothing may be using them.
    app.farming.stop_everything();
    app.unlocker.stop_everything();
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        app.helpers.stop_all().await;
        app.cm.disconnect_all().await;
    })
    .await;

    match by_itself {
        // Asked for from the window: come back with the window open.
        None | Some(true) => desktop::mark_show_after_update(app),
        // Installed from the tray: come back in the tray.
        Some(false) => desktop::mark_hidden_after_update(app),
    }
    if by_itself.is_some() {
        let attempt = Attempt { version: shown(&update.version), at: chrono::Utc::now().timestamp() };
        if let Ok(text) = serde_json::to_string(&attempt) {
            let _ = std::fs::write(app.data_dir.join(ATTEMPT_FILE), text);
        }
    }
    // Windows: starts the installer and ends this process right here.
    update.install(&bytes)?;
    // Elsewhere the new version is now in place, so start it.
    handle.restart()
}

/// How a version is written for people: "0.5" for 0.5.0. Installers and the
/// updater need three numbers; releases are named with the first two.
pub fn shown(version: &str) -> String {
    match version.strip_suffix(".0") {
        Some(short) if short.contains('.') => short.to_string(),
        _ => version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_shown_with_two_numbers() {
        assert_eq!(shown("0.5.0"), "0.5");
        assert_eq!(shown("1.0.0"), "1.0");
        assert_eq!(shown("0.2.2"), "0.2.2");
        assert_eq!(shown("10.0"), "10.0");
    }

    #[test]
    fn looks_again_sooner_after_a_failure() {
        // every half hour while all is well
        assert_eq!(next_look(0), Duration::from_secs(30 * 60));
        // after a look that failed: in one minute, then two, then five
        assert_eq!(
            [next_look(1), next_look(2), next_look(3)],
            [Duration::from_secs(60), Duration::from_secs(120), Duration::from_secs(300)]
        );
        // and every ten minutes from then on, however long the PC is offline
        assert_eq!(next_look(4), Duration::from_secs(600));
        assert_eq!(next_look(500), Duration::from_secs(600));
        assert!(LOOK_AGAIN_LATER < LOOK_EVERY);
    }

    #[test]
    fn installs_by_itself_only_when_nothing_is_running() {
        assert_eq!(holds_up(&Activity::default()), None);
        assert_eq!(holds_up(&Activity { idling: 1, ..Default::default() }), Some("games are idling"));
        assert_eq!(holds_up(&Activity { farming: true, ..Default::default() }), Some("card farming is running"));
        assert_eq!(holds_up(&Activity { unlocking: true, ..Default::default() }), Some("the achievement unlocker is running"));
        assert_eq!(holds_up(&Activity { market: true, ..Default::default() }), Some("market work is under way"));
        assert_eq!(holds_up(&Activity { achievements: true, ..Default::default() }), Some("a game's achievements are open"));
        // several things at once: the first is named
        assert_eq!(holds_up(&Activity { idling: 3, farming: true, market: true, ..Default::default() }), Some("games are idling"));
    }

    #[test]
    fn an_install_that_did_not_take_is_not_repeated_at_once() {
        let now = 1_800_000_000;
        let attempt = |version: &str, ago: i64| Attempt { version: version.into(), at: now - ago };
        assert!(!tried_lately(None, "1.6", now));
        // tried a minute ago and here again, still older: not again by itself
        assert!(tried_lately(Some(&attempt("1.6", 60)), "1.6", now));
        assert!(tried_lately(Some(&attempt("1.6", ATTEMPT_REST_SECS - 1)), "1.6", now));
        // half a day later it may try once more
        assert!(!tried_lately(Some(&attempt("1.6", ATTEMPT_REST_SECS + 1)), "1.6", now));
        // a different version is a different matter
        assert!(!tried_lately(Some(&attempt("1.5", 60)), "1.6", now));
        // a note from the future (the clock was changed) doesn't hold anything up
        assert!(!tried_lately(Some(&attempt("1.6", -3600)), "1.6", now));
        // what is written is what is read back
        let text = serde_json::to_string(&attempt("1.6", 60)).unwrap();
        assert_eq!(serde_json::from_str::<Attempt>(&text).unwrap(), attempt("1.6", 60));
    }

    #[test]
    fn a_failed_request_says_why() {
        #[derive(Debug)]
        struct Layer(&'static str, Option<Box<Layer>>);
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for Layer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.1.as_deref().map(|l| l as &(dyn std::error::Error + 'static))
            }
        }
        let e = Layer("error sending request", Some(Box::new(Layer("client error (Connect)", Some(Box::new(Layer("operation timed out", None)))))));
        assert_eq!(with_causes(&e), "error sending request: client error (Connect): operation timed out");
        // a reason the first line already gives isn't said twice
        let e = Layer("failed: operation timed out", Some(Box::new(Layer("operation timed out", None))));
        assert_eq!(with_causes(&e), "failed: operation timed out");
    }

    #[test]
    fn finds_the_releases_page_of_a_github_update_address() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert_eq!(
            releases_page(&url("https://github.com/someone/idlekit/releases/latest/download/latest.json")).as_deref(),
            Some("https://github.com/someone/idlekit/releases/latest")
        );
        assert_eq!(releases_page(&url("https://github.com/someone/idlekit")), None);
        assert_eq!(releases_page(&url("https://example.com/someone/idlekit/releases/latest.json")), None);
        assert_eq!(releases_page(&url("http://127.0.0.1:4600/latest.json")), None);
    }

    #[test]
    fn phases_are_sent_in_lower_camel_case() {
        let state = UpdateState { phase: Phase::Downloading, ..Default::default() };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["phase"], "downloading");
        assert_eq!(json["checkedAt"], serde_json::Value::Null);
        assert_eq!(serde_json::to_value(Phase::Idle).unwrap(), "idle");
    }
}
