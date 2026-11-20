//! In-memory log ring buffer that also streams every line to the UI, plus a
//! global event emitter so background tasks don't need to carry an AppHandle.
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub ts: i64,
    pub level: &'static str,
    pub scope: String,
    pub message: String,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static BUF: Mutex<VecDeque<LogLine>> = Mutex::new(VecDeque::new());
const MAX_LINES: usize = 1500;

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

pub fn emit<S: Serialize + Clone>(event: &str, payload: S) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

fn push(level: &'static str, scope: &str, message: String) {
    let line = LogLine {
        ts: chrono::Utc::now().timestamp_millis(),
        level,
        scope: scope.to_string(),
        message,
    };
    eprintln!("[{}] {}: {}", line.level, line.scope, line.message);
    if let Ok(mut buf) = BUF.lock() {
        if buf.len() >= MAX_LINES {
            buf.pop_front();
        }
        buf.push_back(line.clone());
    }
    emit("log://line", line);
}

pub fn info(scope: &str, message: impl Into<String>) {
    push("info", scope, message.into());
}

pub fn warn(scope: &str, message: impl Into<String>) {
    push("warn", scope, message.into());
}

pub fn error(scope: &str, message: impl Into<String>) {
    push("error", scope, message.into());
}

pub fn debug(scope: &str, message: impl Into<String>) {
    push("debug", scope, message.into());
}

pub fn recent() -> Vec<LogLine> {
    BUF.lock().map(|b| b.iter().cloned().collect()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut b) = BUF.lock() {
        b.clear();
    }
}
