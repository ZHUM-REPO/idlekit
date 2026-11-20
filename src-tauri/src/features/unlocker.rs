//! Achievement unlocker: works through the account's unlock queue, unlocking
//! achievements one by one with randomized (or imported) delays, optionally
//! only inside a daily time window. Needs the local Steam client.
use crate::config::{ImportedTiming, UnlockerSettings};
use crate::logbus;
use crate::state::App;
use crate::steam::{schema, webapi};
use anyhow::{anyhow, bail, Result};
use chrono::{NaiveTime, Timelike};
use futures_util::StreamExt;
use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnlockGame {
    pub appid: u32,
    pub name: String,
    pub total: u32,
    pub remaining: u32,
    /// "queued" | "running" | "waiting" | "done" | "error"
    pub status: String,
    pub message: String,
    pub next_at: i64,
    pub next_name: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnlockStatus {
    pub steam_id: String,
    pub running: bool,
    /// "idle" | "running" | "done" | "stopped" | "error"
    pub phase: String,
    pub message: String,
    pub games: Vec<UnlockGame>,
    pub unlocked: u32,
}

#[derive(Default)]
pub struct UnlockerManager {
    runs: Mutex<HashMap<String, CancellationToken>>,
    status: Mutex<HashMap<String, Arc<Mutex<UnlockStatus>>>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub api_name: String,
    pub name: String,
    pub percent: Option<f64>,
}

/// Order achievements: custom order > imported order > settings order.
pub fn order_candidates(
    mut list: Vec<Candidate>,
    custom: Option<&Vec<String>>,
    imported: Option<&Vec<ImportedTiming>>,
    order: &str,
) -> Vec<Candidate> {
    // Base order first so unmatched items keep a sensible relative order.
    match order {
        "common" => list.sort_by(|a, b| b.percent.unwrap_or(0.0).total_cmp(&a.percent.unwrap_or(0.0))),
        "rare" => list.sort_by(|a, b| a.percent.unwrap_or(100.0).total_cmp(&b.percent.unwrap_or(100.0))),
        _ => {}
    }
    let rank: Option<HashMap<&str, usize>> = custom
        .filter(|c| !c.is_empty())
        .map(|c| c.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect())
        .or_else(|| {
            imported
                .filter(|i| !i.is_empty())
                .map(|i| i.iter().enumerate().map(|(n, t)| (t.api_name.as_str(), n)).collect())
        });
    if let Some(rank) = rank {
        list.sort_by_key(|c| rank.get(c.api_name.as_str()).copied().unwrap_or(usize::MAX));
    }
    list
}

/// Delay before unlocking `list[i]`.
pub fn delay_for(
    i: usize,
    list: &[Candidate],
    imported: Option<&Vec<ImportedTiming>>,
    s: &UnlockerSettings,
    rng: &mut impl Rng,
) -> Duration {
    let min = s.min_delay_minutes.max(1) as f64 * 60.0;
    let max = (s.max_delay_minutes as f64 * 60.0).max(min);
    if i > 0 {
        if let Some(imp) = imported {
            let off = |name: &str| imp.iter().find(|t| t.api_name == name).map(|t| t.offset_secs);
            if let (Some(a), Some(b)) = (off(&list[i - 1].api_name), off(&list[i].api_name)) {
                let gap = ((b - a).max(0) as f64) * s.imported_time_scale.clamp(0.05, 20.0);
                let jitter = rng.gen_range(0.85..1.15);
                return Duration::from_secs_f64((gap * jitter).clamp(60.0, 12.0 * 3600.0));
            }
        }
    }
    if i == 0 {
        // Start soon, but not instantly.
        return Duration::from_secs_f64(rng.gen_range(60.0..=min.max(61.0)));
    }
    Duration::from_secs_f64(rng.gen_range(min..=max))
}

fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

/// Seconds until the allowed window opens (0 when inside it). Handles windows
/// that cross midnight, e.g. 22:00–02:00.
pub fn secs_until_window(now: NaiveTime, start: NaiveTime, end: NaiveTime) -> i64 {
    if start == end {
        return 0;
    }
    let inside = if start < end { now >= start && now < end } else { now >= start || now < end };
    if inside {
        return 0;
    }
    let n = now.num_seconds_from_midnight() as i64;
    let s = start.num_seconds_from_midnight() as i64;
    (s - n).rem_euclid(86_400)
}

impl UnlockerManager {
    pub fn all(&self) -> Vec<UnlockStatus> {
        self.status.lock().unwrap().values().map(|s| s.lock().unwrap().clone()).collect()
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

    pub fn start(&self, app: App, steam_id: String) -> Result<()> {
        app.require_local_client(&steam_id)?;
        let account = app.account(&steam_id)?;
        if account.unlock_queue.is_empty() {
            bail!("The unlock queue is empty — add games from the Achievements page");
        }
        let token = CancellationToken::new();
        {
            let mut runs = self.runs.lock().unwrap();
            if runs.contains_key(&steam_id) {
                bail!("The unlocker is already running for this account");
            }
            runs.insert(steam_id.clone(), token.clone());
        }
        let shared = Arc::new(Mutex::new(UnlockStatus {
            steam_id: steam_id.clone(),
            running: true,
            phase: "running".into(),
            message: String::new(),
            games: account
                .unlock_queue
                .iter()
                .map(|&appid| UnlockGame {
                    appid,
                    name: app.game_name(&steam_id, appid),
                    status: "queued".into(),
                    ..Default::default()
                })
                .collect(),
            unlocked: 0,
        }));
        self.status.lock().unwrap().insert(steam_id.clone(), shared.clone());
        emit(&shared);

        tokio::spawn(async move {
            let parallel = app.settings().unlocker.parallel_games.clamp(1, 32) as usize;
            let queue = account.unlock_queue.clone();
            futures_util::stream::iter(queue)
                .for_each_concurrent(parallel, |appid| {
                    let app = app.clone();
                    let sid = steam_id.clone();
                    let token = token.clone();
                    let shared = shared.clone();
                    async move {
                        if token.is_cancelled() {
                            return;
                        }
                        let result = unlock_game(&app, &sid, appid, &token, &shared).await;
                        let _ = app.idler.stop(&app, &sid, &[appid], Some("unlocker")).await;
                        // Whatever happened, don't leave the game "running" in Steam.
                        let open = app.ach_open.lock().unwrap().contains(&appid);
                        if !app.idler.anyone_idling(appid) && !open {
                            app.helpers.stop(appid).await;
                        }
                        update(&shared, appid, |g| match &result {
                            Ok(true) => {
                                g.status = "done".into();
                                g.next_at = 0;
                                g.next_name.clear();
                            }
                            Ok(false) => {
                                g.status = "queued".into();
                                g.next_at = 0;
                            }
                            Err(e) => {
                                g.status = "error".into();
                                g.message = format!("{e:#}");
                            }
                        });
                        match result {
                            Ok(true) => app.update_config(|c| {
                                if let Some(a) = c.account_mut(&sid) {
                                    a.unlock_queue.retain(|x| *x != appid);
                                }
                            }),
                            Err(e) => logbus::error("unlocker", format!("{appid}: {e:#}")),
                            _ => {}
                        }
                    }
                })
                .await;
            app.unlocker.runs.lock().unwrap().remove(&steam_id);
            {
                let mut st = shared.lock().unwrap();
                st.running = false;
                st.phase = if token.is_cancelled() { "stopped" } else { "done" }.into();
                st.message = format!("{} achievement(s) unlocked", st.unlocked);
            }
            emit(&shared);
            app.emit_accounts();
        });
        Ok(())
    }
}

fn emit(shared: &Arc<Mutex<UnlockStatus>>) {
    let snapshot = shared.lock().unwrap().clone();
    logbus::emit("unlock://status", snapshot);
}

fn update(shared: &Arc<Mutex<UnlockStatus>>, appid: u32, f: impl FnOnce(&mut UnlockGame)) {
    {
        let mut st = shared.lock().unwrap();
        if let Some(g) = st.games.iter_mut().find(|g| g.appid == appid) {
            f(g);
        }
    }
    emit(shared);
}

async fn sleep_or_cancel(d: Duration, token: &CancellationToken) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => true,
        _ = token.cancelled() => false,
    }
}

async fn set_one(app: &App, appid: u32, api_name: &str) -> Result<bool> {
    let helper = app.helpers.ensure(appid).await?;
    let v = helper
        .request(
            "set_achievements",
            json!({ "changes": [{ "apiName": api_name, "achieved": true }] }),
            Duration::from_secs(30),
        )
        .await?;
    Ok(v.pointer(&format!("/results/{api_name}")).and_then(Value::as_bool).unwrap_or(false))
}

/// Returns Ok(true) when the game's queue finished, Ok(false) when cancelled.
async fn unlock_game(
    app: &App,
    steam_id: &str,
    appid: u32,
    token: &CancellationToken,
    shared: &Arc<Mutex<UnlockStatus>>,
) -> Result<bool> {
    let settings = app.settings();
    let us = settings.unlocker.clone();
    let account = app.account(steam_id)?;
    app.require_local_client(steam_id)?;
    update(shared, appid, |g| {
        g.status = "running".into();
        g.message = "Loading achievements…".into();
    });

    let name = app.game_name(steam_id, appid);
    // Achievements always go through the local Steam client. A direct-mode
    // account idling the same game over its own connection would make Steam
    // pause one of the two, so for those the helper alone does the idling.
    if us.idle_while_unlocking && account.mode != "direct" {
        let r = app.idler.start(app, steam_id, vec![(appid, name.clone())], "unlocker", None).await?;
        if let Some(err) = r.into_iter().find(|r| !r.ok).and_then(|r| r.error) {
            return Err(anyhow!(err));
        }
    }
    let helper = app.helpers.ensure(appid).await?;
    let data = helper.request("achievements", Value::Null, Duration::from_secs(40)).await?;
    let list = data.get("achievements").and_then(Value::as_array).cloned().unwrap_or_default();

    let protected: HashSet<String> = app
        .steam_path()
        .ok()
        .and_then(|p| schema::load(&p, appid, &settings.language).ok())
        .map(|s| {
            s.achievements
                .into_iter()
                .filter(|a| a.permission & 3 != 0)
                .map(|a| a.api_name)
                .collect()
        })
        .unwrap_or_default();
    let percents = webapi::global_achievement_percentages(&app.http, appid).await.unwrap_or_default();
    let skip: HashSet<&String> = account.unlock_skip.get(&appid).map(|v| v.iter().collect()).unwrap_or_default();

    let candidates: Vec<Candidate> = list
        .iter()
        .filter(|a| !a.get("achieved").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|a| {
            let api = a.get("apiName")?.as_str()?.to_string();
            if protected.contains(&api) || skip.contains(&api) {
                return None;
            }
            let display = a.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(&api).to_string();
            Some(Candidate { percent: percents.get(&api).copied(), api_name: api, name: display })
        })
        .collect();
    let imported = account.unlock_imported.get(&appid);
    let todo = order_candidates(candidates, account.unlock_order.get(&appid), imported, &us.order);
    let total = list.len() as u32;
    update(shared, appid, |g| {
        g.total = total;
        g.remaining = todo.len() as u32;
        g.message.clear();
    });
    if todo.is_empty() {
        return Ok(true);
    }
    logbus::info("unlocker", format!("{name}: {} achievement(s) to unlock", todo.len()));

    for i in 0..todo.len() {
        let delay = delay_for(i, &todo, imported, &us, &mut rand::thread_rng());
        update(shared, appid, |g| {
            g.status = "waiting".into();
            g.next_at = now_ms() + delay.as_millis() as i64;
            g.next_name = todo[i].name.clone();
        });
        if !sleep_or_cancel(delay, token).await {
            return Ok(false);
        }
        if us.schedule_enabled {
            if let (Some(start), Some(end)) = (parse_hhmm(&us.schedule_start), parse_hhmm(&us.schedule_end)) {
                let wait = secs_until_window(chrono::Local::now().time(), start, end);
                if wait > 0 {
                    update(shared, appid, |g| {
                        g.message = format!("Outside the unlock window; resuming at {}", us.schedule_start);
                        g.next_at = now_ms() + wait * 1000;
                    });
                    if !sleep_or_cancel(Duration::from_secs(wait as u64 + 5), token).await {
                        return Ok(false);
                    }
                }
            }
        }
        app.require_local_client(steam_id)?;
        let c = &todo[i];
        match set_one(app, appid, &c.api_name).await {
            Ok(true) => {
                logbus::info("unlocker", format!("{name}: unlocked \"{}\"", c.name));
                shared.lock().unwrap().unlocked += 1;
            }
            Ok(false) => logbus::warn("unlocker", format!("{name}: Steam refused \"{}\"", c.name)),
            Err(e) => logbus::warn("unlocker", format!("{name}: \"{}\" failed: {e:#}", c.name)),
        }
        update(shared, appid, |g| {
            g.status = "running".into();
            g.remaining = (todo.len() - i - 1) as u32;
            g.message.clear();
        });
        if !us.idle_while_unlocking && !app.idler.anyone_idling(appid) && !app.ach_open.lock().unwrap().contains(&appid) {
            app.helpers.stop(appid).await;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn c(name: &str, pct: f64) -> Candidate {
        Candidate { api_name: name.into(), name: name.into(), percent: Some(pct) }
    }

    #[test]
    fn ordering_rules() {
        let list = vec![c("A", 10.0), c("B", 80.0), c("C", 50.0)];
        let names = |v: Vec<Candidate>| v.into_iter().map(|c| c.api_name).collect::<Vec<_>>();
        assert_eq!(names(order_candidates(list.clone(), None, None, "common")), ["B", "C", "A"]);
        assert_eq!(names(order_candidates(list.clone(), None, None, "rare")), ["A", "C", "B"]);
        assert_eq!(names(order_candidates(list.clone(), None, None, "schema")), ["A", "B", "C"]);
        let custom = vec!["C".to_string(), "A".to_string()];
        assert_eq!(names(order_candidates(list.clone(), Some(&custom), None, "common")), ["C", "A", "B"]);
        let imp = vec![
            ImportedTiming { api_name: "A".into(), offset_secs: 0 },
            ImportedTiming { api_name: "B".into(), offset_secs: 600 },
        ];
        assert_eq!(names(order_candidates(list, None, Some(&imp), "common")), ["A", "B", "C"]);
    }

    #[test]
    fn delays_respect_settings() {
        let s = UnlockerSettings { min_delay_minutes: 10, max_delay_minutes: 20, ..Default::default() };
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let list = vec![c("A", 1.0), c("B", 1.0), c("C", 1.0)];
        for _ in 0..50 {
            let d = delay_for(1, &list, None, &s, &mut rng).as_secs();
            assert!((600..=1200).contains(&d), "{d}");
            let first = delay_for(0, &list, None, &s, &mut rng).as_secs();
            assert!((60..=600).contains(&first), "{first}");
        }
        let imp = vec![
            ImportedTiming { api_name: "A".into(), offset_secs: 0 },
            ImportedTiming { api_name: "B".into(), offset_secs: 3600 },
        ];
        let d = delay_for(1, &list, Some(&imp), &s, &mut rng).as_secs();
        assert!((3060..=4140).contains(&d), "{d}");
        // C has no imported time: falls back to the random range
        let d = delay_for(2, &list, Some(&imp), &s, &mut rng).as_secs();
        assert!((600..=1200).contains(&d), "{d}");
    }

    #[test]
    fn time_window() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        assert_eq!(secs_until_window(t(19, 0), t(18, 0), t(23, 0)), 0);
        assert_eq!(secs_until_window(t(17, 0), t(18, 0), t(23, 0)), 3600);
        assert_eq!(secs_until_window(t(23, 30), t(18, 0), t(23, 0)), 18 * 3600 + 1800);
        // overnight window 22:00–02:00
        assert_eq!(secs_until_window(t(1, 0), t(22, 0), t(2, 0)), 0);
        assert_eq!(secs_until_window(t(23, 0), t(22, 0), t(2, 0)), 0);
        assert_eq!(secs_until_window(t(12, 0), t(22, 0), t(2, 0)), 10 * 3600);
        assert_eq!(secs_until_window(t(12, 0), t(9, 0), t(9, 0)), 0);
    }
}
