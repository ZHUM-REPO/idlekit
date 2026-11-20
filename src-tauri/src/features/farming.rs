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
                Ok(true) => {
                    st.phase = "done".into();
                    st.message = format!("Finished — {} card(s) dropped this run", st.cards_farmed);
                    logbus::info("farming", st.message.clone());
                    app.farming.set(&st);
                    after_farming(&app, &steam_id).await;
                }
                Ok(false) => {
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

async fn run(app: &App, steam_id: &str, token: &CancellationToken, st: &mut FarmStatus) -> Result<bool> {
    let settings = app.settings().farming;
    st.games = FarmingManager::scan(app, steam_id).await?;
    let mut last_full_scan = Instant::now();
    let mut web_failures = 0u32;
    let check = Duration::from_secs(settings.check_minutes.clamp(1, 240) as u64 * 60);

    loop {
        if token.is_cancelled() {
            return Ok(false);
        }
        st.total_drops = st
            .games
            .iter()
            .filter(|g| g.status != "skipped" && g.status != "error")
            .map(|g| g.drops_remaining)
            .sum();
        let (phase, active) = pick_active(&st.games, &settings);
        if active.is_empty() {
            return Ok(true);
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
        if !to_start.is_empty() {
            for r in app.idler.start(app, steam_id, to_start, "farming", None).await? {
                if !r.ok {
                    if let Some(g) = st.games.iter_mut().find(|g| g.appid == r.appid) {
                        g.status = "error".into();
                        g.note = r.error.unwrap_or_default();
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
            // Everything we tried failed to start; avoid a hot loop.
            if !sleep_or_cancel(Duration::from_secs(30), token).await {
                return Ok(false);
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
        st.next_check_at = now_ms() + check.as_millis() as i64;
        app.farming.set(st);

        if !sleep_or_cancel(check, token).await {
            return Ok(false);
        }
        let idled_hours = check.as_secs_f64() / 3600.0;
        for g in st.games.iter_mut().filter(|g| running.contains(&g.appid)) {
            g.playtime_hours += idled_hours;
        }

        // Check drops.
        let ws = match app.web_session(steam_id).await {
            Ok(ws) => ws,
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
                    web_failures = 0;
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
                Err(e) => {
                    web_failures += 1;
                    logbus::warn("farming", format!("Badge scan failed: {e:#}"));
                }
            }
        } else if phase == "phase2" {
            for appid in &running {
                match ws.gamecard(*appid).await {
                    Ok(card) => {
                        web_failures = 0;
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
                    Err(e) => {
                        web_failures += 1;
                        logbus::warn("farming", format!("Drop check failed for {appid}: {e:#}"));
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        if web_failures >= 5 {
            bail!("Steam keeps refusing drop checks — the web session may have expired");
        }

        if settings.restart_on_check && phase == "phase2" {
            st.phase = "restarting".into();
            st.message = "Restarting games to trigger drops…".into();
            app.farming.set(st);
            app.idler.stop_all(app, steam_id, Some("farming")).await?;
            if !sleep_or_cancel(Duration::from_secs(4), token).await {
                return Ok(false);
            }
        }
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
    }
}
