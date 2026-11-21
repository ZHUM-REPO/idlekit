//! The one place games get started/stopped. Every feature (manual idling,
//! auto-idle, playtime booster, card farming, unlocker) registers a "source"
//! on a game; the game keeps running until no source holds it.
//! Backend: local accounts -> steam-helper processes; direct accounts -> CM.
use crate::logbus;
use crate::state::App;
use anyhow::Result;
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    started_at: i64,
    sources: BTreeSet<String>,
    target_minutes: Option<u32>,
    /// When the playtime booster was set on this game, which can be long
    /// after the game began idling for another reason.
    boost_started_at: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IdleGame {
    pub appid: u32,
    pub name: String,
    pub started_at: i64,
    pub sources: Vec<String>,
    pub target_minutes: Option<u32>,
    /// What `target_minutes` is counted from.
    pub boost_started_at: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IdleSnapshot {
    pub steam_id: String,
    pub games: Vec<IdleGame>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    pub appid: u32,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Idler {
    accounts: Mutex<HashMap<String, BTreeMap<u32, Entry>>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Whether a playtime boost has run its course. The minutes count from when
/// the boost was set, not from when the game began idling.
fn boost_reached(now: i64, started_at: i64, boost_started_at: Option<i64>, target_minutes: u32) -> bool {
    (now - boost_started_at.unwrap_or(started_at)) / 60_000 >= target_minutes as i64
}

impl Idler {
    pub fn snapshot(&self, steam_id: &str) -> Vec<IdleGame> {
        self.accounts
            .lock()
            .unwrap()
            .get(steam_id)
            .map(|m| {
                m.iter()
                    .map(|(&appid, e)| IdleGame {
                        appid,
                        name: e.name.clone(),
                        started_at: e.started_at,
                        sources: e.sources.iter().cloned().collect(),
                        target_minutes: e.target_minutes,
                        boost_started_at: e.boost_started_at,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn all(&self) -> Vec<IdleSnapshot> {
        let ids: Vec<String> = self.accounts.lock().unwrap().keys().cloned().collect();
        ids.into_iter()
            .map(|id| IdleSnapshot { games: self.snapshot(&id), steam_id: id })
            .collect()
    }

    fn emit(&self, steam_id: &str) {
        logbus::emit(
            "idle://status",
            IdleSnapshot { steam_id: steam_id.to_string(), games: self.snapshot(steam_id) },
        );
    }

    pub fn appids(&self, steam_id: &str) -> Vec<u32> {
        self.accounts
            .lock()
            .unwrap()
            .get(steam_id)
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default()
    }

    pub fn appids_with_source(&self, steam_id: &str, source: &str) -> Vec<u32> {
        self.accounts
            .lock()
            .unwrap()
            .get(steam_id)
            .map(|m| {
                m.iter()
                    .filter(|(_, e)| e.sources.contains(source))
                    .map(|(a, _)| *a)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True if any account is idling this app (helpers are per-app, not per-account).
    pub fn anyone_idling(&self, appid: u32) -> bool {
        self.accounts.lock().unwrap().values().any(|m| m.contains_key(&appid))
    }

    fn add_entry(&self, steam_id: &str, appid: u32, name: &str, source: &str, target: Option<u32>) {
        let mut all = self.accounts.lock().unwrap();
        let map = all.entry(steam_id.to_string()).or_default();
        let e = map.entry(appid).or_insert_with(|| Entry {
            name: name.to_string(),
            started_at: now_ms(),
            sources: BTreeSet::new(),
            target_minutes: None,
            boost_started_at: None,
        });
        e.sources.insert(source.to_string());
        if target.is_some() {
            e.target_minutes = target;
            e.boost_started_at = Some(now_ms());
        }
    }

    /// Take back games that were just registered but never started.
    fn forget(&self, steam_id: &str, appids: &[u32]) {
        if let Some(map) = self.accounts.lock().unwrap().get_mut(steam_id) {
            for appid in appids {
                map.remove(appid);
            }
        }
    }

    pub async fn start(
        &self,
        app: &App,
        steam_id: &str,
        games: Vec<(u32, String)>,
        source: &str,
        target_minutes: Option<u32>,
    ) -> Result<Vec<StartResult>> {
        let account = app.account(steam_id)?;
        let max = app.settings().max_idle_games.clamp(1, 32) as usize;
        let current: BTreeSet<u32> = self.appids(steam_id).into_iter().collect();
        let mut results = Vec::new();
        let mut new_games = Vec::new();
        for (appid, name) in games {
            if current.contains(&appid) {
                self.add_entry(steam_id, appid, &name, source, target_minutes);
                results.push(StartResult { appid, ok: true, error: None });
            } else if current.len() + new_games.len() >= max {
                results.push(StartResult {
                    appid,
                    ok: false,
                    error: Some(format!("Limit reached: at most {max} games can idle at once")),
                });
            } else {
                new_games.push((appid, name));
            }
        }

        if account.mode == "direct" {
            if !app.cm.is_online(steam_id) {
                let s = app.settings();
                app.cm.configure(steam_id, &account.account_name, account.persona_state, &s.idle_status_text);
                app.cm.connect(steam_id).await?;
            }
            for (appid, name) in &new_games {
                self.add_entry(steam_id, *appid, name, source, target_minutes);
                results.push(StartResult { appid: *appid, ok: true, error: None });
            }
            if let Err(e) = app.cm.set_games(steam_id, self.appids(steam_id)).await {
                // Steam was never told, so these must not be shown as idling.
                let added: Vec<u32> = new_games.iter().map(|(appid, _)| *appid).collect();
                self.forget(steam_id, &added);
                self.emit(steam_id);
                return Err(e);
            }
        } else if !new_games.is_empty() {
            app.require_local_client(steam_id)?;
            let started: Vec<(u32, String, Result<()>)> = futures_util::stream::iter(new_games)
                .map(|(appid, name)| async move {
                    let r = app.helpers.ensure(appid).await.map(|_| ());
                    (appid, name, r)
                })
                .buffer_unordered(4)
                .collect()
                .await;
            for (appid, name, r) in started {
                match r {
                    Ok(()) => {
                        self.add_entry(steam_id, appid, &name, source, target_minutes);
                        results.push(StartResult { appid, ok: true, error: None });
                    }
                    Err(e) => {
                        logbus::warn("idle", format!("Could not start {name} ({appid}): {e:#}"));
                        results.push(StartResult { appid, ok: false, error: Some(format!("{e:#}")) });
                    }
                }
            }
        }
        let ok = results.iter().filter(|r| r.ok).count();
        if ok > 0 {
            logbus::info("idle", format!("{ok} game(s) idling ({source})"));
        }
        self.emit(steam_id);
        Ok(results)
    }

    /// Remove `source` from these games (or every source when `source` is None)
    /// and stop the games nothing else is holding.
    pub async fn stop(&self, app: &App, steam_id: &str, appids: &[u32], source: Option<&str>) -> Result<()> {
        let removed: Vec<u32> = {
            let mut all = self.accounts.lock().unwrap();
            let Some(map) = all.get_mut(steam_id) else { return Ok(()) };
            let mut removed = Vec::new();
            for appid in appids {
                let drop_it = match (map.get_mut(appid), source) {
                    (Some(e), Some(src)) => {
                        e.sources.remove(src);
                        e.sources.is_empty()
                    }
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if drop_it {
                    map.remove(appid);
                    removed.push(*appid);
                }
            }
            removed
        };
        if removed.is_empty() {
            self.emit(steam_id);
            return Ok(());
        }
        let mode = app.account(steam_id).map(|a| a.mode).unwrap_or_default();
        if mode == "direct" {
            app.cm.set_games(steam_id, self.appids(steam_id)).await?;
        } else {
            let keep: Vec<u32> = app.ach_open.lock().unwrap().iter().copied().collect();
            for appid in removed {
                if !keep.contains(&appid) && !self.anyone_idling(appid) {
                    app.helpers.stop(appid).await;
                }
            }
        }
        self.emit(steam_id);
        Ok(())
    }

    pub async fn stop_all(&self, app: &App, steam_id: &str, source: Option<&str>) -> Result<()> {
        let ids = match source {
            Some(s) => self.appids_with_source(steam_id, s),
            None => self.appids(steam_id),
        };
        self.stop(app, steam_id, &ids, source).await
    }

    /// A helper process died on its own (game closed by Steam, Steam exited…).
    pub fn on_helper_exit(&self, appid: u32) {
        let mut affected = Vec::new();
        {
            let mut all = self.accounts.lock().unwrap();
            for (sid, map) in all.iter_mut() {
                if let Some(e) = map.remove(&appid) {
                    affected.push((sid.clone(), e.name));
                }
            }
        }
        for (sid, name) in affected {
            logbus::warn("idle", format!("{name} ({appid}) stopped unexpectedly — is Steam still running?"));
            self.emit(&sid);
        }
    }

    /// Stop playtime-booster games that reached their target.
    pub async fn tick(&self, app: &App) {
        let now = now_ms();
        let due: Vec<(String, u32, String)> = {
            let all = self.accounts.lock().unwrap();
            all.iter()
                .flat_map(|(sid, map)| {
                    map.iter().filter_map(move |(appid, e)| {
                        let target = e.target_minutes?;
                        (e.sources.contains("booster") && boost_reached(now, e.started_at, e.boost_started_at, target))
                            .then(|| (sid.clone(), *appid, e.name.clone()))
                    })
                })
                .collect()
        };
        for (sid, appid, name) in due {
            logbus::info("booster", format!("{name} reached its playtime target"));
            if let Err(e) = self.stop(app, &sid, &[appid], Some("booster")).await {
                logbus::warn("booster", format!("{e:#}"));
            }
            app.update_config(|c| {
                if let Some(a) = c.account_mut(&sid) {
                    a.booster_targets.remove(&appid);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boost_counts_from_when_it_was_set() {
        let minute = 60_000;
        // idling for three hours, boost of 60 minutes set five minutes ago
        let (started, boosted, now) = (0, 175 * minute, 180 * minute);
        assert!(!boost_reached(now, started, Some(boosted), 60));
        assert!(boost_reached(boosted + 60 * minute, started, Some(boosted), 60));
        // no separate boost time recorded: count from the start of idling
        assert!(boost_reached(now, started, None, 60));
        assert!(!boost_reached(59 * minute, started, None, 60));
    }

    #[test]
    fn setting_a_boost_on_a_running_game_restarts_the_count() {
        let idler = Idler::default();
        idler.add_entry("1", 440, "Team Fortress 2", "manual", None);
        assert_eq!(idler.snapshot("1")[0].boost_started_at, None);
        idler.add_entry("1", 440, "Team Fortress 2", "booster", Some(60));
        let game = &idler.snapshot("1")[0];
        assert_eq!(game.target_minutes, Some(60));
        assert!(game.boost_started_at.is_some_and(|at| at >= game.started_at));
        assert_eq!(game.sources, ["booster", "manual"]);

        idler.forget("1", &[440]);
        assert!(idler.snapshot("1").is_empty());
    }
}
