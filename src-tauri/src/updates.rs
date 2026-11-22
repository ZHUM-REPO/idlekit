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
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// The installer is a few megabytes; this only guards against a stalled connection.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// The first look after starting waits this long: Steam connections come first.
const FIRST_LOOK: Duration = Duration::from_secs(8);
/// IdleKit can run for weeks in the tray, so it keeps looking while it runs.
const LOOK_EVERY: Duration = Duration::from_secs(4 * 60 * 60);
/// After a look that failed (no connection, a slow one) it looks again sooner,
/// a few times, then settles for once an hour until one works.
const LOOK_AGAIN: Duration = Duration::from_secs(10 * 60);
const LOOK_AGAIN_TIMES: u32 = 3;
const LOOK_AGAIN_LATER: Duration = Duration::from_secs(60 * 60);
const CHECK_FAILED: &str = "Couldn't check for updates";

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
        other => other.to_string(),
    }
}

/// How long to wait before looking again, given how many looks in a row have failed.
fn next_look(failures_in_a_row: u32) -> Duration {
    match failures_in_a_row {
        0 => LOOK_EVERY,
        n if n <= LOOK_AGAIN_TIMES => LOOK_AGAIN,
        _ => LOOK_AGAIN_LATER,
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
    tokio::time::sleep(FIRST_LOOK).await;
    let mut failures = 0u32;
    let mut first = true;
    loop {
        if app.settings().check_updates {
            // The first look reports like one asked for by hand; the ones
            // after it stay out of the way unless they find something.
            let failed = look(&app, &handle, !first).await.1;
            failures = if failed { failures + 1 } else { 0 };
            first = false;
        } else {
            failures = 0;
        }
        tokio::time::sleep(every(next_look(failures))).await;
    }
}

/// Look for a newer version. Does nothing while a check or an install is under way.
pub async fn check(app: &App, handle: &AppHandle) -> UpdateState {
    look(app, handle, false).await.0
}

/// One look for a newer version; also says whether it failed. A quiet look is
/// one nobody asked for: when it fails, the failure goes to the log and is not
/// put in front of the user, and whatever was on screen stays.
async fn look(app: &App, handle: &AppHandle, quiet: bool) -> (UpdateState, bool) {
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

    let found = async {
        let updater = handle.updater_builder().endpoints(endpoints)?.timeout(CHECK_TIMEOUT).build()?;
        updater.check().await
    }
    .await;

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
    let failure = match download_and_hand_over(app, handle, &update).await {
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
    desktop::clear_show_after_update(app);
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

    desktop::mark_show_after_update(app);
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
        // every few hours while all is well
        assert_eq!(next_look(0), LOOK_EVERY);
        // sooner after a look that failed, a few times over
        for failures in 1..=LOOK_AGAIN_TIMES {
            assert_eq!(next_look(failures), LOOK_AGAIN);
        }
        // then hourly, so a PC that is offline for days isn't kept busy
        assert_eq!(next_look(LOOK_AGAIN_TIMES + 1), LOOK_AGAIN_LATER);
        assert_eq!(next_look(500), LOOK_AGAIN_LATER);
        assert!(LOOK_AGAIN < LOOK_AGAIN_LATER && LOOK_AGAIN_LATER < LOOK_EVERY);
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
