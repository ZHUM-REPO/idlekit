//! Card farming: reads badge pages for games with card drops left and idles
//! them in two phases.
//!
//! Phase 1 — games under the playtime threshold (default 2h; Steam often holds
//! back drops until then) idle together, up to 32 at once, to build playtime.
//! Phase 2 — games past the threshold idle a few at a time (default one); drops
//! are re-checked every N minutes and games are briefly restarted at each
//! check, which tends to release pending drops sooner.
use crate::config::FarmingSettings;
use crate::logbus;
use crate::state::App;
use crate::steam::community::{self, BadgeGame};
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FarmGame {
    pub appid: u32,
    pub name: String,
    pub drops_remaining: u32,
    pub playtime_hours: f64,
    /// "queued" | "idling" | "done" | "skipped" | "error"
    pub status: String,
    pub note: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FarmStatus {
    pub steam_id: String,
    pub running: bool,
    /// "idle" | "scanning" | "phase1" | "phase2" | "restarting" | "done" | "stopped" | "error"
    pub phase: String,
    pub message: String,
    pub games: Vec<FarmGame>,
    pub active: Vec<u32>,
    pub total_drops: u32,
    pub cards_farmed: u32,
    pub started_at: i64,
    pub next_check_at: i64,
}

#[derive(Default)]
pub struct FarmingManager {
    runs: Mutex<HashMap<String, CancellationToken>>,
    status: Mutex<HashMap<String, FarmStatus>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn order_games(games: &mut [FarmGame], order: &str) {
    match order {
        "leastDrops" => games.sort_by(|a, b| a.drops_remaining.cmp(&b.drops_remaining)),
        "name" => games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "playtime" => games.sort_by(|a, b| b.playtime_hours.total_cmp(&a.playtime_hours)),
        _ => games.sort_by(|a, b| b.drops_remaining.cmp(&a.drops_remaining)),
    }
}

/// Apply black/whitelists and the refund check to badge results.
pub fn plan(
    badges: Vec<BadgeGame>,
    blacklist: &[u32],
    whitelist: &[u32],
    licenses: &[community::License],
    settings: &FarmingSettings,
    today: chrono::NaiveDate,
) -> Vec<FarmGame> {
    let mut out: Vec<FarmGame> = badges
        .into_iter()
        .filter(|b| b.drops_remaining > 0)
        .map(|b| {
            let (status, note) = if blacklist.contains(&b.appid) {
                ("skipped", "Blacklisted")
            } else if !whitelist.is_empty() && !whitelist.contains(&b.appid) {
                ("skipped", "Not in whitelist")
            } else if settings.skip_refundable
                && community::likely_refundable(&b.name, b.playtime_hours, licenses, today)
            {
                ("skipped", "Bought in the last 14 days, so idling could void a refund")
            } else {
                ("queued", "")
            };
            FarmGame {
                appid: b.appid,
                name: b.name,
                drops_remaining: b.drops_remaining,
                playtime_hours: b.playtime_hours,
                status: status.into(),
                note: note.into(),
            }
        })
        .collect();
    order_games(&mut out, &settings.order);
    out
}

/// Which games to idle next: (phase, appids).
pub fn pick_active(games: &[FarmGame], settings: &FarmingSettings) -> (&'static str, Vec<u32>) {
    let eligible: Vec<&FarmGame> = games
        .iter()
        .filter(|g| (g.status == "queued" || g.status == "idling") && g.drops_remaining > 0)
        .collect();
    if settings.threshold_hours > 0.0 {
        let under: Vec<u32> = eligible
            .iter()
            .filter(|g| g.playtime_hours < settings.threshold_hours)
            .take(settings.max_games.clamp(1, 32) as usize)
            .map(|g| g.appid)
            .collect();
        if !under.is_empty() {
            return ("phase1", under);
        }
    }
    (
        "phase2",
        eligible
            .iter()
            .take(settings.solo_games.clamp(1, 32) as usize)
            .map(|g| g.appid)
            .collect(),
    )
}

/// A game that fails to start is tried this many times in a run before it is
/// given up on: Steam is often only busy for a moment.
pub const START_TRIES: u32 = 3;
/// How long a game that failed to start waits before it is tried again.
pub const START_RETRY_AFTER: Duration = Duration::from_secs(2 * 60);
/// A game whose card page fails this many drop checks in a row, while Steam's
/// other pages answer, is set aside for the run so the others can be farmed.
pub const CHECK_TRIES: u32 = 3;
/// Checks in a row at which Steam says the web session has ended (after a
/// fresh session was tried) before the run stops.
pub const ENDED_TRIES: u32 = 3;

/// Why a game was given up on for the rest of a run.
#[derive(Clone, Debug, PartialEq)]
pub enum GaveUp {
    /// It never started; what Steam said.
    Start(String),
    /// Its card page kept failing while Steam's other pages answered.
    Checks(String),
}

/// What went wrong during one run, kept from check to check.
#[derive(Default)]
pub struct Setbacks {
    /// Failed starts so far, per game.
    starts: HashMap<u32, u32>,
    /// Games that failed to start and wait for another try, and from when.
    held: HashMap<u32, Instant>,
    /// Failed drop checks in a row, per game, while Steam answered otherwise.
    checks: HashMap<u32, u32>,
    /// Games given up on for the run.
    gave_up: HashMap<u32, GaveUp>,
    /// Checks in a row at which Steam's pages didn't answer at all.
    pub outage: u32,
    /// Checks in a row at which Steam said the web session has ended.
    pub ended: u32,
}

impl Setbacks {
    /// A game failed to start. It waits and is tried again, or after
    /// [`START_TRIES`] it is given up on for the run (`true`).
    pub fn start_failed(&mut self, g: &mut FarmGame, why: &str, now: Instant) -> bool {
        let tries = self.starts.entry(g.appid).or_insert(0);
        *tries += 1;
        if *tries >= START_TRIES {
            self.held.remove(&g.appid);
            self.gave_up.insert(g.appid, GaveUp::Start(why.to_string()));
            g.status = "error".into();
            g.note = format!("Couldn't start: {why}");
            return true;
        }
        self.held.insert(g.appid, now + START_RETRY_AFTER);
        g.status = "queued".into();
        g.note = format!(
            "Couldn't start ({why}). Trying again in {} minutes, try {} of {START_TRIES}",
            START_RETRY_AFTER.as_secs() / 60,
            *tries + 1
        );
        false
    }

    /// A game that had failed to start is running now.
    pub fn started(&mut self, g: &mut FarmGame) {
        if self.starts.remove(&g.appid).is_some() {
            g.note.clear();
        }
    }

    /// Lets games whose wait after a failed start is over be picked again.
    pub fn release_due(&mut self, now: Instant) {
        self.held.retain(|_, at| *at > now);
    }

    pub fn is_held(&self, appid: u32) -> bool {
        self.held.contains_key(&appid)
    }

    /// When the next game waiting after a failed start may be tried again.
    pub fn next_retry(&self) -> Option<Instant> {
        self.held.values().min().copied()
    }

    /// A drop check for this game failed while Steam answered otherwise. After
    /// [`CHECK_TRIES`] in a row the game is set aside for the run (`true`).
    pub fn check_failed(&mut self, g: &mut FarmGame, why: &str) -> bool {
        let fails = self.checks.entry(g.appid).or_insert(0);
        *fails += 1;
        if *fails < CHECK_TRIES {
            return false;
        }
        self.gave_up.insert(g.appid, GaveUp::Checks(why.to_string()));
        g.status = "error".into();
        g.note = format!("Set aside: its card page failed {CHECK_TRIES} checks in a row ({why})");
        true
    }

    pub fn check_worked(&mut self, appid: u32) {
        self.checks.remove(&appid);
    }

    /// When the games ran out only because every one of them failed to start,
    /// the reason the first one gave. That is a failure, not "finished".
    pub fn nothing_started(&self, games: &[FarmGame]) -> Option<String> {
        let mut wanted = games.iter().filter(|g| g.status != "skipped");
        if !wanted.clone().all(|g| g.status == "error" && matches!(self.gave_up.get(&g.appid), Some(GaveUp::Start(_)))) {
            return None;
        }
        wanted.next().map(|g| match self.gave_up.get(&g.appid) {
            Some(GaveUp::Start(why)) if !why.is_empty() => why.clone(),
            _ => "Steam gave no reason".to_string(),
        })
    }

    /// The names of the games given up on, in the order of the list.
    pub fn given_up<'a>(&self, games: &'a [FarmGame]) -> Vec<&'a str> {
        games.iter().filter(|g| self.gave_up.contains_key(&g.appid)).map(|g| g.name.as_str()).collect()
    }
}

/// The line a run that ran out of games ends with, and whether it is a
/// warning: games given up on are named, so "finished" never hides them.
pub fn finish_message(games: &[FarmGame], setbacks: &Setbacks, cards: u32) -> (String, bool) {
    let done = format!("Finished — {cards} card(s) dropped this run");
    let names = setbacks.given_up(games);
    if names.is_empty() {
        return (done, false);
    }
    (format!("{done}, but {} couldn't be farmed (the list says why)", join_names(&names)), true)
}

/// "A", "A and B", or "A, B and 3 more".
fn join_names(names: &[&str]) -> String {
    match names {
        [] => "a game".into(),
        [one] => one.to_string(),
        [first, second] => format!("{first} and {second}"),
        [first, second, rest @ ..] => format!("{first}, {second} and {} more", rest.len()),
    }
}

/// The names of these games, for a line.
fn named(games: &[FarmGame], appids: impl IntoIterator<Item = u32>) -> String {
    let names: Vec<&str> =
        appids.into_iter().filter_map(|a| games.iter().find(|g| g.appid == a)).map(|g| g.name.as_str()).collect();
    join_names(&names)
}

impl FarmingManager {
    pub fn status(&self, steam_id: &str) -> FarmStatus {
        self.status
            .lock()
            .unwrap()
            .get(steam_id)
            .cloned()
            .unwrap_or_else(|| FarmStatus { steam_id: steam_id.to_string(), phase: "idle".into(), ..Default::default() })
    }

    pub fn all(&self) -> Vec<FarmStatus> {
        self.status.lock().unwrap().values().cloned().collect()
    }

    fn set(&self, st: &FarmStatus) {
        self.status.lock().unwrap().insert(st.steam_id.clone(), st.clone());
        logbus::emit("farm://status", st.clone());
    }

    pub async fn scan(app: &App, steam_id: &str) -> Result<Vec<FarmGame>> {
        let account = app.account(steam_id)?;
        let settings = app.settings().farming;
        let ws = app.web_session(steam_id).await?;
        let badges = ws.badges().await?;
        let licenses = if settings.skip_refundable {
            ws.licenses().await.unwrap_or_else(|e| {
                logbus::warn("farming", format!("Could not read licenses for the refund check: {e:#}"));
                vec![]
            })
        } else {
            vec![]
        };
        Ok(plan(
            badges,
            &account.farm_blacklist,
            &account.farm_whitelist,
            &licenses,
            &settings,
            chrono::Local::now().date_naive(),
        ))
    }

    pub fn start(&self, app: App, steam_id: String) -> Result<()> {
        let token = CancellationToken::new();
        {
            let mut runs = self.runs.lock().unwrap();
            if runs.contains_key(&steam_id) {
                bail!("Card farming is already running for this account");
            }
            runs.insert(steam_id.clone(), token.clone());
        }
        tokio::spawn(async move {
            let mut st = FarmStatus {
                steam_id: steam_id.clone(),
                running: true,
                phase: "scanning".into(),
                message: "Reading your badge pages…".into(),
                started_at: now_ms(),
                ..Default::default()
            };
            app.farming.set(&st);
            let result = run(&app, &steam_id, &token, &mut st).await;
            let _ = app.idler.stop_all(&app, &steam_id, Some("farming")).await;
            app.farming.runs.lock().unwrap().remove(&steam_id);
            st.running = false;
            st.active.clear();
            st.next_check_at = 0;
            for g in st.games.iter_mut().filter(|g| g.status == "idling") {
                g.status = "queued".into();
            }
            match result {
                Ok(Ended::Finished { warn }) => {
                    // run() put how it went in the message.
                    st.phase = "done".into();
                    if warn {
                        logbus::warn("farming", st.message.clone());
                    } else {
                        logbus::info("farming", st.message.clone());
                    }
                    app.farming.set(&st);
                    after_farming(&app, &steam_id).await;
                }
                Ok(Ended::Stopped) => {
                    st.phase = "stopped".into();
                    st.message = "Stopped".into();
                    app.farming.set(&st);
                }
                Err(e) => {
                    st.phase = "error".into();
                    st.message = format!("{e:#}");
                    logbus::error("farming", st.message.clone());
                    app.farming.set(&st);
                }
            }
        });
        Ok(())
    }

    pub fn stop(&self, steam_id: &str) {
        if let Some(t) = self.runs.lock().unwrap().get(steam_id) {
            t.cancel();
        }
    }

    pub fn stop_everything(&self) {
        for t in self.runs.lock().unwrap().values() {
            t.cancel();
        }
    }
}

async fn sleep_or_cancel(d: Duration, token: &CancellationToken) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => true,
        _ = token.cancelled() => false,
    }
}

/// How a run ended, when not with an error.
enum Ended {
    /// It ran out of games. `st.message` says how it went; a warning when
    /// games had to be given up on.
    Finished { warn: bool },
    Stopped,
}

async fn run(app: &App, steam_id: &str, token: &CancellationToken, st: &mut FarmStatus) -> Result<Ended> {
    let settings = app.settings().farming;
    st.games = FarmingManager::scan(app, steam_id).await?;
    let mut last_full_scan = Instant::now();
    let mut web_failures = 0u32;
    let mut setbacks = Setbacks::default();
    let check = Duration::from_secs(settings.check_minutes.clamp(1, 240) as u64 * 60);

    loop {
        if token.is_cancelled() {
            return Ok(Ended::Stopped);
        }
        setbacks.release_due(Instant::now());
        st.total_drops = st
            .games
            .iter()
            .filter(|g| g.status != "skipped" && g.status != "error")
            .map(|g| g.drops_remaining)
            .sum();
        // Games waiting for another try after failing to start sit this one out.
        let pickable: Vec<FarmGame> = st.games.iter().filter(|g| !setbacks.is_held(g.appid)).cloned().collect();
        let (phase, active) = pick_active(&pickable, &settings);
        if active.is_empty() {
            if let Some(next) = setbacks.next_retry() {
                // All that is left waits for another try: nothing idles meanwhile.
                let previous = app.idler.appids_with_source(steam_id, "farming");
                if !previous.is_empty() {
                    app.idler.stop(app, steam_id, &previous, Some("farming")).await?;
                }
                for g in st.games.iter_mut().filter(|g| g.status == "idling") {
                    g.status = "queued".into();
                }
                let waiting: Vec<u32> = st.games.iter().filter(|g| setbacks.is_held(g.appid)).map(|g| g.appid).collect();
                let wait = next.saturating_duration_since(Instant::now());
                st.active.clear();
                st.message = format!("Waiting to try {} again", named(&st.games, waiting));
                st.next_check_at = now_ms() + wait.as_millis() as i64;
                app.farming.set(st);
                if !sleep_or_cancel(wait, token).await {
                    return Ok(Ended::Stopped);
                }
                continue;
            }
            if let Some(why) = setbacks.nothing_started(&st.games) {
                bail!("None of the games could be started: {why}");
            }
            let (message, warn) = finish_message(&st.games, &setbacks, st.cards_farmed);
            st.message = message;
            return Ok(Ended::Finished { warn });
        }

        // Sync the idler with the new active set.
        let previous = app.idler.appids_with_source(steam_id, "farming");
        let to_stop: Vec<u32> = previous.iter().copied().filter(|a| !active.contains(a)).collect();
        if !to_stop.is_empty() {
            app.idler.stop(app, steam_id, &to_stop, Some("farming")).await?;
        }
        let to_start: Vec<(u32, String)> = st
            .games
            .iter()
            .filter(|g| active.contains(&g.appid) && !previous.contains(&g.appid))
            .map(|g| (g.appid, g.name.clone()))
            .collect();
        let tried = !to_start.is_empty();
        if tried {
            let now = Instant::now();
            for r in app.idler.start(app, steam_id, to_start, "farming", None).await? {
                let Some(g) = st.games.iter_mut().find(|g| g.appid == r.appid) else { continue };
                if r.ok {
                    setbacks.started(g);
                } else {
                    let why = r.error.unwrap_or_else(|| "Steam gave no reason".into());
                    if setbacks.start_failed(g, &why, now) {
                        logbus::warn("farming", format!("{}: given up for this run after {START_TRIES} failed starts", g.name));
                    }
                }
            }
        }
        let running: Vec<u32> = app.idler.appids_with_source(steam_id, "farming");
        for g in st.games.iter_mut() {
            if running.contains(&g.appid) {
                g.status = "idling".into();
            } else if g.status == "idling" {
                g.status = "queued".into();
            }
        }
        if running.is_empty() {
            if tried {
                // Those failed to start, and now wait or are given up on: on to
                // the next games straight away.
                continue;
            }
            // Nothing new was tried and nothing runs: don't spin.
            if !sleep_or_cancel(Duration::from_secs(30), token).await {
                return Ok(Ended::Stopped);
            }
            continue;
        }
        st.phase = phase.into();
        st.active = running.clone();
        st.message = if phase == "phase1" {
            format!(
                "Phase 1: building playtime on {} game(s) under {}h",
                running.len(),
                settings.threshold_hours
            )
        } else {
            format!("Phase 2: farming {} game(s) for drops", running.len())
        };
        if setbacks.outage > 0 {
            st.message.push_str(". Steam's pages didn't answer the last check; still idling");
        }
        st.next_check_at = now_ms() + check.as_millis() as i64;
        app.farming.set(st);

        if !sleep_or_cancel(check, token).await {
            return Ok(Ended::Stopped);
        }
        let idled_hours = check.as_secs_f64() / 3600.0;
        for g in st.games.iter_mut().filter(|g| running.contains(&g.appid)) {
            g.playtime_hours += idled_hours;
        }

        // Check drops.
        let ws = match app.web_session(steam_id).await {
            Ok(ws) => {
                web_failures = 0;
                ws
            }
            Err(e) => {
                web_failures += 1;
                logbus::warn("farming", format!("Web session problem: {e:#}"));
                if web_failures >= 5 {
                    bail!("Lost the Steam web session: {e:#}");
                }
                continue;
            }
        };
        let full_scan_due = phase == "phase1"
            && (last_full_scan.elapsed() >= Duration::from_secs(30 * 60)
                || st
                    .games
                    .iter()
                    .filter(|g| running.contains(&g.appid))
                    .all(|g| g.playtime_hours >= settings.threshold_hours));
        if full_scan_due {
            match ws.badges().await {
                Ok(badges) => {
                    steam_back(&mut setbacks);
                    last_full_scan = Instant::now();
                    let fresh: HashMap<u32, BadgeGame> = badges.into_iter().map(|b| (b.appid, b)).collect();
                    for g in st.games.iter_mut() {
                        match fresh.get(&g.appid) {
                            Some(b) => {
                                if b.drops_remaining < g.drops_remaining {
                                    st.cards_farmed += g.drops_remaining - b.drops_remaining;
                                }
                                g.drops_remaining = b.drops_remaining;
                                if b.playtime_hours > 0.0 {
                                    g.playtime_hours = b.playtime_hours;
                                }
                            }
                            None => g.drops_remaining = 0,
                        }
                        if g.drops_remaining == 0 && g.status != "skipped" {
                            g.status = "done".into();
                        }
                    }
                }
                Err(e) => trouble(&mut setbacks, &e, check),
            }
        } else if phase == "phase2" {
            let mut answered = false;
            let mut failed: Vec<(u32, anyhow::Error)> = Vec::new();
            for appid in &running {
                match ws.gamecard(*appid).await {
                    Ok(card) => {
                        answered = true;
                        setbacks.check_worked(*appid);
                        if let Some(g) = st.games.iter_mut().find(|g| g.appid == *appid) {
                            if card.drops_remaining < g.drops_remaining {
                                let got = g.drops_remaining - card.drops_remaining;
                                st.cards_farmed += got;
                                logbus::info("farming", format!("{}: {got} card(s) dropped", g.name));
                            }
                            g.drops_remaining = card.drops_remaining;
                            if card.playtime_hours > 0.0 {
                                g.playtime_hours = card.playtime_hours;
                            }
                            if g.drops_remaining == 0 {
                                g.status = "done".into();
                                logbus::info("farming", format!("{}: all card drops received", g.name));
                            }
                        }
                    }
                    Err(e) => failed.push((*appid, e)),
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            if let Some((_, e)) = failed.iter().find(|(_, e)| community::session_ended(e)) {
                trouble(&mut setbacks, e, check);
            } else if !failed.is_empty() {
                // One game's page, or Steam (or the connection) as a whole? A
                // page that is always there tells them apart.
                let steam_answers = if answered { Ok(()) } else { ws.answers().await };
                match steam_answers {
                    Ok(()) => {
                        steam_back(&mut setbacks);
                        for (appid, e) in &failed {
                            let Some(g) = st.games.iter_mut().find(|g| g.appid == *appid) else { continue };
                            logbus::warn("farming", format!("Drop check failed for {}: {e:#}", g.name));
                            if setbacks.check_failed(g, &e.root_cause().to_string()) {
                                logbus::warn(
                                    "farming",
                                    format!(
                                        "{}: set aside for this run; its card page failed {CHECK_TRIES} checks in a row while Steam's other pages answered",
                                        g.name
                                    ),
                                );
                            }
                        }
                    }
                    Err(e) => trouble(&mut setbacks, &e, check),
                }
            } else if answered {
                steam_back(&mut setbacks);
            }
        }
        if setbacks.ended >= ENDED_TRIES {
            bail!(
                "Steam says this account's web session has ended. Sign in to the account again (or paste a new cookie), then start farming again"
            );
        }
        if setbacks.ended > 0 {
            // The next check gets a fresh session, made from the saved sign-in where there is one.
            app.forget_web_session(steam_id).await;
        }

        if settings.restart_on_check && phase == "phase2" {
            st.phase = "restarting".into();
            st.message = "Restarting games to trigger drops…".into();
            app.farming.set(st);
            app.idler.stop_all(app, steam_id, Some("farming")).await?;
            if !sleep_or_cancel(Duration::from_secs(4), token).await {
                return Ok(Ended::Stopped);
            }
        }
    }
}

/// A check Steam didn't answer as asked. When it says the web session has
/// ended, that is counted towards stopping. Anything else is trouble on
/// Steam's side or with the connection: farming keeps idling through it, as
/// drops keep coming while the pages are down, and says so once.
fn trouble(setbacks: &mut Setbacks, e: &anyhow::Error, check: Duration) {
    if community::session_ended(e) {
        setbacks.ended += 1;
        let next = if setbacks.ended < ENDED_TRIES { "; trying a fresh one" } else { "" };
        logbus::warn(
            "farming",
            format!("Steam says the web session has ended ({} of {ENDED_TRIES}){next}", setbacks.ended),
        );
        return;
    }
    setbacks.ended = 0;
    setbacks.outage += 1;
    if setbacks.outage == 1 {
        logbus::warn(
            "farming",
            format!(
                "Steam's pages aren't answering ({e:#}). Farming keeps idling and checks again in {} min",
                check.as_secs().div_ceil(60)
            ),
        );
    }
}

/// Steam's pages answer (again).
fn steam_back(setbacks: &mut Setbacks) {
    setbacks.ended = 0;
    if setbacks.outage > 0 {
        setbacks.outage = 0;
        logbus::info("farming", "Steam's pages are answering again");
    }
}

async fn after_farming(app: &App, steam_id: &str) {
    let action = app.settings().after_farming;
    let Ok(account) = app.account(steam_id) else { return };
    match action.as_str() {
        "autoIdle" if !account.auto_idle.is_empty() => {
            let games = account
                .auto_idle
                .iter()
                .map(|a| (*a, app.game_name(steam_id, *a)))
                .collect();
            if let Err(e) = app.idler.start(app, steam_id, games, "auto", None).await {
                logbus::warn("farming", format!("Auto-idle after farming failed: {e:#}"));
            }
        }
        "unlocker" if !account.unlock_queue.is_empty() => {
            if let Err(e) = app.unlocker.start(app.clone(), steam_id.to_string()) {
                logbus::warn("farming", format!("Could not start the unlocker: {e:#}"));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(appid: u32, drops: u32, hours: f64) -> BadgeGame {
        BadgeGame { appid, name: format!("Game {appid}"), drops_remaining: drops, playtime_hours: hours }
    }

    #[test]
    fn plan_filters_and_orders() {
        let s = FarmingSettings::default();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let lic = vec![community::License {
            date: today,
            name: "Game 4".into(),
            acquisition: "Steam Store".into(),
        }];
        let p = plan(vec![g(1, 2, 5.0), g(2, 0, 1.0), g(3, 4, 0.0), g(4, 1, 0.5), g(5, 3, 0.0)], &[5], &[], &lic, &s, today);
        let ids: Vec<u32> = p.iter().map(|x| x.appid).collect();
        assert_eq!(ids, vec![3, 5, 1, 4]);
        assert_eq!(p[1].status, "skipped");
        assert_eq!(p[3].status, "skipped");
        assert!(p[3].note.contains("refund"));
        let wl = plan(vec![g(1, 2, 5.0), g(3, 4, 0.0)], &[], &[1], &[], &s, today);
        assert_eq!(wl.iter().find(|x| x.appid == 3).unwrap().status, "skipped");
    }

    #[test]
    fn two_phase_selection() {
        let s = FarmingSettings { solo_games: 1, ..Default::default() };
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let mut games = plan(vec![g(1, 2, 5.0), g(3, 4, 0.0), g(6, 1, 1.0)], &[], &[], &[], &s, today);
        let (phase, active) = pick_active(&games, &s);
        assert_eq!(phase, "phase1");
        assert_eq!(active, vec![3, 6]);
        for x in games.iter_mut() {
            x.playtime_hours = 3.0;
        }
        let (phase, active) = pick_active(&games, &s);
        assert_eq!(phase, "phase2");
        assert_eq!(active, vec![3]);
        let no_threshold = FarmingSettings { threshold_hours: 0.0, solo_games: 2, ..Default::default() };
        let (phase, active) = pick_active(&games, &no_threshold);
        assert_eq!((phase, active.len()), ("phase2", 2));
        for x in games.iter_mut() {
            x.status = "done".into();
        }
        assert!(pick_active(&games, &s).1.is_empty());
        assert_eq!(Setbacks::default().nothing_started(&games), None, "all done is a normal finish");

        // every game that was wanted failed to start, every time it was tried
        let mut setbacks = Setbacks::default();
        let now = Instant::now();
        for x in games.iter_mut() {
            for _ in 0..START_TRIES {
                setbacks.start_failed(x, "Steam isn't running", now);
            }
        }
        games[0].status = "skipped".into();
        assert_eq!(setbacks.nothing_started(&games).as_deref(), Some("Steam isn't running"));
        // one that worked makes it a finish again
        games[1].status = "done".into();
        assert_eq!(setbacks.nothing_started(&games), None);
        assert_eq!(setbacks.nothing_started(&[]), None);
    }

    fn three_games() -> (FarmingSettings, Vec<FarmGame>) {
        let s = FarmingSettings { threshold_hours: 0.0, solo_games: 1, ..Default::default() };
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        let games = plan(vec![g(1, 2, 5.0), g(2, 1, 5.0), g(3, 1, 5.0)], &[], &[], &[], &s, today);
        (s, games)
    }

    fn pick(games: &[FarmGame], s: &FarmingSettings, setbacks: &Setbacks) -> Vec<u32> {
        let pickable: Vec<FarmGame> = games.iter().filter(|g| !setbacks.is_held(g.appid)).cloned().collect();
        pick_active(&pickable, s).1
    }

    #[test]
    fn a_game_that_fails_to_start_is_tried_again_before_it_is_given_up() {
        let (s, mut games) = three_games();
        let mut setbacks = Setbacks::default();
        let start = Instant::now();
        assert_eq!(pick(&games, &s, &setbacks), vec![1]);
        games[0].status = "done".into();

        // Game 2 fails once: it waits, and game 3 goes in the meantime.
        assert_eq!(pick(&games, &s, &setbacks), vec![2]);
        assert!(!setbacks.start_failed(&mut games[1], "Steam was busy", start));
        assert_eq!(games[1].status, "queued");
        assert!(games[1].note.contains("Steam was busy"), "{}", games[1].note);
        assert_eq!(pick(&games, &s, &setbacks), vec![3]);
        games[2].status = "done".into();

        // Only game 2 is left, and it isn't its turn yet: wait for it, don't finish.
        assert!(pick(&games, &s, &setbacks).is_empty());
        assert_eq!(setbacks.next_retry(), Some(start + START_RETRY_AFTER));
        setbacks.release_due(start + START_RETRY_AFTER - Duration::from_secs(1));
        assert!(pick(&games, &s, &setbacks).is_empty());
        setbacks.release_due(start + START_RETRY_AFTER);
        assert_eq!(pick(&games, &s, &setbacks), vec![2]);

        // Started the second time: no setback is left over.
        setbacks.started(&mut games[1]);
        assert!(games[1].note.is_empty());
        assert_eq!(finish_message(&games, &setbacks, 4), ("Finished — 4 card(s) dropped this run".into(), false));
    }

    #[test]
    fn a_game_that_never_starts_is_named_when_the_run_finishes() {
        let (s, mut games) = three_games();
        let mut setbacks = Setbacks::default();
        let now = Instant::now();
        for tries in 1..=START_TRIES {
            let gave_up = setbacks.start_failed(&mut games[1], "Steam was busy", now);
            assert_eq!(gave_up, tries == START_TRIES);
            setbacks.release_due(now + START_RETRY_AFTER);
        }
        assert_eq!((games[1].status.as_str(), games[1].note.as_str()), ("error", "Couldn't start: Steam was busy"));
        assert_eq!(setbacks.next_retry(), None, "nothing waits for it any more");
        assert_eq!(pick(&games, &s, &setbacks), vec![1]);
        games[0].status = "done".into();
        games[2].status = "done".into();
        assert!(pick(&games, &s, &setbacks).is_empty());
        assert_eq!(setbacks.nothing_started(&games), None, "the others were farmed");
        let (line, warn) = finish_message(&games, &setbacks, 3);
        assert!(warn);
        assert_eq!(line, "Finished — 3 card(s) dropped this run, but Game 2 couldn't be farmed (the list says why)");
    }

    #[test]
    fn only_the_game_whose_card_page_keeps_failing_is_set_aside() {
        let (s, mut games) = three_games();
        let mut setbacks = Setbacks::default();
        games[0].status = "done".into();
        assert_eq!(pick(&games, &s, &setbacks), vec![2]);
        // A check that works in between starts the count again.
        assert!(!setbacks.check_failed(&mut games[1], "HTTP 404 Not Found"));
        setbacks.check_worked(2);
        for fails in 1..=CHECK_TRIES {
            assert_eq!(setbacks.check_failed(&mut games[1], "HTTP 404 Not Found"), fails == CHECK_TRIES);
        }
        assert_eq!(games[1].status, "error");
        assert!(games[1].note.contains("HTTP 404 Not Found"), "{}", games[1].note);
        // The run goes on with the next game.
        assert_eq!(pick(&games, &s, &setbacks), vec![3]);
        games[2].status = "done".into();
        assert_eq!(setbacks.nothing_started(&games), None, "set aside is not \"never started\"");
        let (line, warn) = finish_message(&games, &setbacks, 3);
        assert!(warn && line.contains("Game 2"), "{line}");
    }

    #[test]
    fn several_games_given_up_are_named_briefly() {
        let mut setbacks = Setbacks::default();
        let mut games: Vec<FarmGame> =
            (1..=5).map(|i| FarmGame { appid: i, name: format!("Game {i}"), status: "queued".into(), ..Default::default() }).collect();
        for g in games.iter_mut().skip(1) {
            for _ in 0..CHECK_TRIES {
                setbacks.check_failed(g, "HTTP 500");
            }
        }
        let (line, _) = finish_message(&games, &setbacks, 0);
        assert!(line.ends_with("but Game 2, Game 3 and 2 more couldn't be farmed (the list says why)"), "{line}");
    }
}
