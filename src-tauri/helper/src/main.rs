//! steam-helper: one process per game, started by IdleKit.
//!
//! Starting the process with `--appid <id>` makes the local Steam client see that
//! game as running (that is all "idling" is). While it runs, it answers JSON-line
//! commands on stdin so IdleKit can read and change achievements and stats.
//! The process exits when stdin closes, so it never outlives IdleKit.
#![cfg_attr(windows, windows_subsystem = "windows")]

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use steamworks::{Client, SteamAPIInitError, UserStatsReceived, UserStatsStored};

const STATS_WAIT: Duration = Duration::from_secs(12);
const STORE_WAIT: Duration = Duration::from_secs(8);

// 0 = waiting, 1 = received OK, 2 = received with an error (game has no stats, etc.)
const STATS_PENDING: u8 = 0;
const STATS_OK: u8 = 1;
const STATS_FAILED: u8 = 2;

fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

fn parse_appid(args: &[String]) -> Option<u32> {
    let pos = args.iter().position(|a| a == "--appid")?;
    args.get(pos + 1)?.parse().ok()
}

#[derive(Deserialize)]
struct Request {
    id: Option<u64>,
    cmd: String,
    #[serde(default)]
    args: Value,
}

#[derive(Deserialize)]
struct AchChange {
    #[serde(rename = "apiName")]
    api_name: String,
    achieved: bool,
}

#[derive(Deserialize)]
struct StatRef {
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    value: Option<f64>,
}

struct Helper {
    client: Client,
    stats_state: Arc<AtomicU8>,
    stored: Arc<AtomicBool>,
}

impl Helper {
    /// Pump callbacks until the condition holds or the timeout passes.
    fn pump_until(&self, timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        loop {
            self.client.run_callbacks();
            if done() {
                return true;
            }
            if start.elapsed() > timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    fn wait_for_stats(&self) -> Result<(), String> {
        let state = self.stats_state.clone();
        self.pump_until(STATS_WAIT, || state.load(Ordering::SeqCst) != STATS_PENDING);
        match self.stats_state.load(Ordering::SeqCst) {
            STATS_FAILED => Err("Steam returned no stats for this game (it may have no achievements)".into()),
            // On timeout, try anyway: some clients never post the callback but serve the data.
            _ => Ok(()),
        }
    }

    fn store(&self) -> bool {
        self.stored.store(false, Ordering::SeqCst);
        if self.client.user_stats().store_stats().is_err() {
            return false;
        }
        let stored = self.stored.clone();
        self.pump_until(STORE_WAIT, || stored.load(Ordering::SeqCst))
    }

    fn achievements(&self) -> Result<Value, String> {
        self.wait_for_stats()?;
        let stats = self.client.user_stats();
        let count = stats
            .get_num_achievements()
            .map_err(|_| "Steam did not return the achievement list".to_string())?;
        if count == 0 {
            return Ok(json!({ "achievements": [] }));
        }
        let names = stats.get_achievement_names().unwrap_or_default();
        let mut list = Vec::with_capacity(names.len());
        for name in names.into_iter().filter(|n| !n.is_empty()) {
            let ach = stats.achievement(&name);
            let (achieved, unlock_time) = ach.get_achievement_and_unlock_time().unwrap_or((false, 0));
            let display = ach.get_achievement_display_attribute("name").unwrap_or("").to_string();
            let desc = ach.get_achievement_display_attribute("desc").unwrap_or("").to_string();
            let hidden = ach.get_achievement_display_attribute("hidden").unwrap_or("0") == "1";
            list.push(json!({
                "apiName": name,
                "name": display,
                "description": desc,
                "hidden": hidden,
                "achieved": achieved,
                "unlockTime": unlock_time,
            }));
        }
        Ok(json!({ "achievements": list }))
    }

    fn get_stats(&self, args: &Value) -> Result<Value, String> {
        self.wait_for_stats()?;
        let refs: Vec<StatRef> = serde_json::from_value(args.get("stats").cloned().unwrap_or(json!([])))
            .map_err(|e| format!("bad stats list: {e}"))?;
        let stats = self.client.user_stats();
        let mut out = serde_json::Map::new();
        for r in refs {
            let value = if r.kind == "float" {
                stats.get_stat_f32(&r.name).ok().map(|v| json!(v))
            } else {
                stats.get_stat_i32(&r.name).ok().map(|v| json!(v))
            };
            out.insert(r.name, value.unwrap_or(Value::Null));
        }
        Ok(json!({ "values": out }))
    }

    fn set_achievements(&self, args: &Value) -> Result<Value, String> {
        self.wait_for_stats()?;
        let changes: Vec<AchChange> = serde_json::from_value(args.get("changes").cloned().unwrap_or(json!([])))
            .map_err(|e| format!("bad change list: {e}"))?;
        let stats = self.client.user_stats();
        let mut results = serde_json::Map::new();
        for c in changes {
            let ach = stats.achievement(&c.api_name);
            let ok = if c.achieved { ach.set().is_ok() } else { ach.clear().is_ok() };
            results.insert(c.api_name, json!(ok));
        }
        let stored = self.store();
        Ok(json!({ "results": results, "stored": stored }))
    }

    fn set_stats(&self, args: &Value) -> Result<Value, String> {
        self.wait_for_stats()?;
        let refs: Vec<StatRef> = serde_json::from_value(args.get("stats").cloned().unwrap_or(json!([])))
            .map_err(|e| format!("bad stats list: {e}"))?;
        let stats = self.client.user_stats();
        let mut results = serde_json::Map::new();
        for r in refs {
            let Some(v) = r.value else { continue };
            let ok = if r.kind == "float" {
                stats.set_stat_f32(&r.name, v as f32).is_ok()
            } else {
                stats.set_stat_i32(&r.name, v.round() as i32).is_ok()
            };
            results.insert(r.name, json!(ok));
        }
        let stored = self.store();
        Ok(json!({ "results": results, "stored": stored }))
    }

    fn reset_all(&self, args: &Value) -> Result<Value, String> {
        self.wait_for_stats()?;
        let with_ach = args.get("achievements").and_then(Value::as_bool).unwrap_or(false);
        self.client
            .user_stats()
            .reset_all_stats(with_ach)
            .map_err(|_| "Steam refused to reset stats".to_string())?;
        Ok(json!({ "stored": self.store() }))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(appid) = parse_appid(&args) else {
        emit(&json!({ "event": "error", "message": "usage: steam-helper --appid <id>" }));
        std::process::exit(2);
    };

    let client = match Client::init_app(appid) {
        Ok(c) => c,
        Err(e) => {
            let (what, detail) = match &e {
                SteamAPIInitError::NoSteamClient(d) => ("Steam isn't running, or IdleKit can't reach it", d),
                SteamAPIInitError::VersionMismatch(d) => ("The Steam client needs an update", d),
                SteamAPIInitError::FailedGeneric(d) => ("Steam wouldn't start this game", d),
            };
            emit(&json!({
                "event": "error",
                "message": format!(
                    "{what}. Make sure Steam is running and signed in to an account that owns app {appid}. (Steam said: {detail})"
                )
            }));
            std::process::exit(3);
        }
    };

    let stats_state = Arc::new(AtomicU8::new(STATS_PENDING));
    let stored = Arc::new(AtomicBool::new(false));
    let s = stats_state.clone();
    let _stats_cb = client.register_callback(move |r: UserStatsReceived| {
        s.store(if r.result.is_ok() { STATS_OK } else { STATS_FAILED }, Ordering::SeqCst);
    });
    let st = stored.clone();
    let _stored_cb = client.register_callback(move |r: UserStatsStored| {
        st.store(r.result.is_ok(), Ordering::SeqCst);
    });

    let helper = Helper { client, stats_state, stored };
    emit(&json!({ "event": "ready", "appid": appid, "pid": std::process::id() }));

    // Read stdin on a thread; EOF means IdleKit went away, so we exit too.
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(l) => {
                    if tx.send(Some(l)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(None);
    });

    loop {
        helper.client.run_callbacks();
        let line = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(l)) => l,
            Ok(None) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => continue,
        };
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                emit(&json!({ "event": "warn", "message": format!("bad request: {e}") }));
                continue;
            }
        };
        let result = match req.cmd.as_str() {
            "ping" => Ok(json!({ "appid": appid })),
            "achievements" => helper.achievements(),
            "get_stats" => helper.get_stats(&req.args),
            "set_achievements" => helper.set_achievements(&req.args),
            "set_stats" => helper.set_stats(&req.args),
            "reset_all" => helper.reset_all(&req.args),
            "quit" => {
                emit(&json!({ "id": req.id, "ok": true, "data": null }));
                break;
            }
            other => Err(format!("unknown command {other}")),
        };
        match result {
            Ok(data) => emit(&json!({ "id": req.id, "ok": true, "data": data })),
            Err(message) => emit(&json!({ "id": req.id, "ok": false, "error": message })),
        }
    }
}
