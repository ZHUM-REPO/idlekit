//! The log: every line goes to a ring buffer in memory, to the Logs page, and
//! to `log.txt` in the app's data folder. Each line is scrubbed of personal
//! and secret data first (see `redact`), so the same text is safe on screen,
//! on the clipboard and in the file.
//!
//! Also a global event emitter, so background tasks don't need an AppHandle.
use regex::Regex;
use serde::Serialize;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// Where the line comes in the order of everything sent to the window (see [`emit`]).
    pub seq: u64,
    pub ts: i64,
    pub level: &'static str,
    pub scope: String,
    pub message: String,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
/// Numbers everything sent to the window, in the order it happens. The
/// window takes its first look at the app's state with the number of that
/// moment and keeps only what came after it, so nothing that happens while
/// it starts is missed (a status that changed meanwhile) or applied twice.
static SEQ: AtomicU64 = AtomicU64::new(0);
static BUF: Mutex<VecDeque<LogLine>> = Mutex::new(VecDeque::new());
static FILE: Mutex<Option<LogFile>> = Mutex::new(None);
/// Words to keep out of the log that no pattern can recognise, such as a
/// Steam sign-in name, each with what to write instead.
static HIDDEN: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
/// This PC's user name as it appears in folder paths.
static USERS: Mutex<Vec<Regex>> = Mutex::new(Vec::new());
/// Words the log itself is written in. A sign-in name that is one of these is
/// not hidden as a word: no line carries sign-in names on purpose, and hiding
/// "steam" everywhere would leave the log unreadable.
const EVERYDAY_WORDS: &[&str] = &[
    "steam", "idlekit", "account", "accounts", "game", "games", "idle", "idling", "farm", "farming", "card", "cards",
    "badge", "badges", "market", "achievement", "achievements", "unlock", "unlocker", "booster", "update", "updates",
    "window", "tray", "settings", "setting", "start", "started", "stop", "stopped", "done", "failed", "error", "warn",
    "info", "debug", "action", "app", "helper", "direct", "local", "web", "session", "cookie", "password", "name",
    "sign", "code", "guard", "client", "user", "the", "and", "for", "not", "was", "with", "from", "page", "log",
    "file", "version", "windows", "running", "waiting", "connect", "connected", "library", "item", "items", "price",
];
const MAX_LINES: usize = 1500;
pub const FILE_NAME: &str = "log.txt";
const OLD_FILE_NAME: &str = "log.old.txt";
/// log.txt is set aside as log.old.txt when it reaches this size, so the two
/// together never take more than twice this.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 4000;

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::SeqCst) + 1
}

/// Sends an event to the window. One that carries an object gets the next
/// number in `seq` (see [`SEQ`]), taken after the change it reports.
pub fn emit<S: Serialize + Clone>(event: &str, payload: S) {
    let Some(app) = APP.get() else { return };
    match serde_json::to_value(&payload) {
        Ok(serde_json::Value::Object(mut map)) => {
            if !map.contains_key("seq") {
                map.insert("seq".into(), next_seq().into());
            }
            let _ = app.emit(event, serde_json::Value::Object(map));
        }
        _ => {
            let _ = app.emit(event, payload);
        }
    }
}

/// The lines in memory and the number of this moment, read together: every
/// line numbered up to it is among them, none after it.
pub fn snapshot() -> (u64, Vec<LogLine>) {
    match BUF.lock() {
        Ok(b) => (SEQ.load(Ordering::SeqCst), b.iter().cloned().collect()),
        Err(_) => (SEQ.load(Ordering::SeqCst), Vec::new()),
    }
}

// ------------------------------------------------------------ redaction ---

fn compile(list: &[(&str, &'static str)]) -> Vec<(Regex, &'static str)> {
    list.iter().map(|(pattern, with)| (Regex::new(pattern).expect("valid log pattern"), *with)).collect()
}

/// Secrets with a recognisable shape. These run first.
fn secrets() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        compile(&[
            // Sign-in tokens (JWTs): refresh tokens, access tokens, the value of the login cookie.
            (r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}(?:\.[A-Za-z0-9_-]+)?", "<token>"),
            // Secrets passed as web address parameters.
            (
                r"(?i)([?&](?:key|access_token|refresh_token|token|sessionid|code|password|client_id|request_id|nonce|auth|steamguard)=)[^&\s)\x22']+",
                "$1<hidden>",
            ),
            // Cookies and secrets written as name=value or name: value.
            (
                r#"(?i)\b(steamLoginSecure|steamRefresh_steam|sessionid|steamMachineAuth\w*|refresh_token|access_token|guard_data|shared_secret|identity_secret)\b(\\?["']?\s*[=:]\s*\\?["']?)[^\s&;,"'\\)]+"#,
                "$1$2<hidden>",
            ),
            // A password in a form or in JSON. Not "password: " in a sentence,
            // which is how the app's own lines and Steam's messages use the word.
            (r#"(?i)\b(password)(\\?["']\s*:\s*\\?["']?|\\?["']?\s*=\s*\\?["']?)[^\s&;,"'\\)]+"#, "$1$2<hidden>"),
            // The QR sign-in link.
            (r"https://s\.team/q/\S+", "<sign-in link>"),
            // A profile's custom address, which is a name its owner picked.
            (r#"(?i)(steamcommunity\.com/id/)[^/\s"'<>?#]+"#, "$1<profile>"),
        ])
    })
}

/// People and places. These run after Steam IDs and this PC's own user name.
fn places() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        compile(&[
            // A user name inside a Windows folder path. A name of several words
            // only counts when the path goes on after it; otherwise it ends at
            // the first space, so the rest of the sentence is kept.
            (
                r#"(?i)\b([a-z]:[\\/]+Users[\\/]+)(?:[^\\/"'<>|:*?\s]+(?: [^\\/"'<>|:*?\s]+){0,2}([\\/])|[^\\/"'<>|:*?\s]+)"#,
                "${1}<user>${2}",
            ),
            // The same on Linux and macOS.
            (r#"(/home/|/Users/)[^/\s"'<>]+"#, "$1<user>"),
            // E-mail addresses.
            (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}", "<email>"),
            // IPv6 addresses written out in full,
            (r"(?i)\b(?:[0-9a-f]{1,4}:){5,7}[0-9a-f]{1,4}\b", "<ip>"),
            // and shortened with "::", which needs a group on both sides or two before it.
            (r"(?i)\b[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,5}::[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){0,5}\b", "<ip>"),
            (r"(?i)\b[0-9a-f]{1,4}(?::[0-9a-f]{1,4}){1,6}::", "<ip>"),
        ])
    })
}

fn digits() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\d+").expect("valid log pattern"))
}

fn ipv4() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("valid log pattern"))
}

fn long_run() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\b[A-Za-z0-9+/_-]{40,}={0,2}").expect("valid log pattern"))
}

/// Replaces the matches `with` returns something for. It is given the match,
/// the text before it and the text after it.
fn replace_each(text: &str, pattern: &Regex, with: impl Fn(&str, &str, &str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for found in pattern.find_iter(text) {
        if let Some(new) = with(found.as_str(), &text[..found.start()], &text[found.end()..]) {
            out.push_str(&text[copied..found.start()]);
            out.push_str(&new);
            copied = found.end();
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// SteamID64s, shortened to their last four digits so accounts can still be
/// told apart. Found as whole runs of digits, so one is recognised next to
/// anything: "steamid_7656…", "%3D7656…".
fn shorten_steam_ids(text: &str) -> String {
    replace_each(text, digits(), |run, _, _| {
        (run.len() == 17 && run.starts_with("7656119")).then(|| format!("<steam id …{}>", &run[13..]))
    })
}

/// IPv4 addresses. Four numbers with dots are also how some versions are
/// written, so those are left alone when the text says that is what they are.
fn hide_ipv4(text: &str) -> String {
    replace_each(text, ipv4(), |found, before, after| {
        let octets = found.split('.').all(|part| part.parse::<u16>().is_ok_and(|n| n <= 255));
        let longer = before.ends_with('.') || after.starts_with('.') && after[1..].starts_with(|c: char| c.is_ascii_digit());
        let word = before.trim_end().rsplit(|c: char| !c.is_alphanumeric()).next().unwrap_or("").to_ascii_lowercase();
        let version = matches!(word.as_str(), "version" | "ver" | "v" | "view" | "build");
        (octets && !longer && !version).then(|| "<ip>".to_string())
    })
}

/// Whether a long run of letters, digits and separators reads as names
/// ("ACH_WIN_100_GAMES", "ISteamUserStats/GetSchemaForGame/v2") and not as a
/// key: every piece between separators is a word or a short number, never
/// letters and digits mixed.
fn reads_as_names(run: &str) -> bool {
    let pieces: Vec<&str> = run.split(['/', '_', '-', '+', '=']).filter(|p| !p.is_empty()).collect();
    pieces.len() >= 2
        && pieces.iter().all(|piece| {
            let letters = piece.chars().all(|c| c.is_ascii_alphabetic()) && piece.len() < 40;
            let number = piece.chars().all(|c| c.is_ascii_digit()) && piece.len() <= 8;
            let version = piece.len() <= 4 && piece.starts_with(['v', 'V']) && piece[1..].chars().all(|c| c.is_ascii_digit());
            letters || number || version
        })
}

/// Anything else long and unreadable is more likely a key than a word.
fn hide_long_runs(text: &str) -> String {
    replace_each(text, long_run(), |run, _, _| (!reads_as_names(run)).then(|| "<hidden>".to_string()))
}

/// A pattern for this PC's user name wherever it is a folder in a path, also
/// in a web-encoded one: "D:\Profiles\name\…", "%5CUsers%5Cname%5C…".
fn user_in_paths(name: &str) -> Option<Regex> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Regex::new(&format!(
        r#"(?i)([\\/]|%5C|%2F){}([\\/]|%5C|%2F|[\s"'<>|:*?),;]|$)"#,
        regex::escape(name)
    ))
    .ok()
}

/// Replaces whole occurrences of `word`, whatever their capitals: not when it
/// is only part of a longer word, so hiding the sign-in name "max" leaves
/// "maximum" alone.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let (bytes, needle) = (text.as_bytes(), word.as_bytes());
    if needle.is_empty() || bytes.len() < needle.len() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at + needle.len() <= bytes.len() {
        let end = at + needle.len();
        let found = bytes[at..end].eq_ignore_ascii_case(needle) && text.is_char_boundary(at) && text.is_char_boundary(end);
        let alone = found
            && !text[..at].chars().next_back().is_some_and(is_word)
            && !text[end..].chars().next().is_some_and(is_word);
        if alone {
            out.push_str(&text[copied..at]);
            out.push_str(with);
            copied = end;
            at = end;
        } else {
            at += 1;
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Takes personal and secret data out of a line: tokens, cookies, Steam IDs,
/// user names in paths, e-mail and network addresses, long opaque strings,
/// this PC's user name (`users`) and the given words (sign-in names).
fn redact_all(text: &str, hidden: &[(String, String)], users: &[Regex]) -> String {
    let mut out = text.to_string();
    for (pattern, with) in secrets() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, *with).into_owned();
        }
    }
    out = shorten_steam_ids(&out);
    for user in users {
        if user.is_match(&out) {
            out = user.replace_all(&out, "${1}<user>${2}").into_owned();
        }
    }
    for (pattern, with) in places() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, *with).into_owned();
        }
    }
    out = hide_ipv4(&out);
    out = hide_long_runs(&out);
    for (word, with) in hidden {
        out = replace_word(&out, word, with);
    }
    out
}

#[cfg(test)]
pub fn redact_with(text: &str, hidden: &[(String, String)]) -> String {
    redact_all(text, hidden, &[])
}

pub fn redact(text: &str) -> String {
    let hidden = HIDDEN.lock().map(|h| h.clone()).unwrap_or_default();
    let users = USERS.lock().map(|u| u.clone()).unwrap_or_default();
    redact_all(text, &hidden, &users)
}

/// How an account is named in the log: by the last four digits of its Steam
/// ID, never by its sign-in name.
pub fn label(steam_id: &str) -> String {
    let digits: String = steam_id.chars().filter(char::is_ascii_digit).collect();
    match digits.len() {
        0..=3 => "account".to_string(),
        n => format!("account …{}", &digits[n - 4..]),
    }
}

/// Keeps a word, such as a Steam sign-in name, out of everything logged from
/// now on. Very short words are left alone: hiding "ab" would chew up
/// ordinary text and protects nothing.
pub fn hide(word: &str, with: &str) {
    let word = word.trim();
    if word.chars().count() < 3 || EVERYDAY_WORDS.contains(&word.to_ascii_lowercase().as_str()) {
        return;
    }
    if let Ok(mut hidden) = HIDDEN.lock() {
        if !hidden.iter().any(|(w, _)| w.eq_ignore_ascii_case(word)) {
            hidden.push((word.to_string(), with.to_string()));
            // Longest first, so a name that contains another is replaced whole.
            hidden.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        }
    }
}

/// Keeps this PC's user name out of every folder path in the log, wherever
/// the folder is: profile folders are not always under C:\Users.
pub fn hide_user(name: &str) {
    if let (Some(pattern), Ok(mut users)) = (user_in_paths(name), USERS.lock()) {
        if !users.iter().any(|u| u.as_str() == pattern.as_str()) {
            users.push(pattern);
        }
    }
}

// ------------------------------------------------------------- the file ---

struct LogFile {
    dir: PathBuf,
    file: File,
    size: u64,
    /// What this run of the app is, repeated at the top of the file that
    /// follows a full one.
    header: String,
}

impl LogFile {
    fn open(dir: &Path) -> std::io::Result<LogFile> {
        let path = dir.join(FILE_NAME);
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= MAX_FILE_BYTES {
            let _ = std::fs::rename(&path, dir.join(OLD_FILE_NAME));
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if size >= MAX_FILE_BYTES {
            // It could not be set aside (something else has it open), so it is
            // emptied instead: the file must not grow without end.
            file = OpenOptions::new().create(true).write(true).truncate(true).open(&path)?;
            size = 0;
        }
        Ok(LogFile { dir: dir.to_path_buf(), file, size, header: String::new() })
    }

    fn write(&mut self, text: &str) {
        if self.size >= MAX_FILE_BYTES {
            // Full: set it aside as the one older file and begin again.
            let _ = std::fs::rename(self.dir.join(FILE_NAME), self.dir.join(OLD_FILE_NAME));
            if let Ok(mut fresh) = LogFile::open(&self.dir) {
                fresh.header = std::mem::take(&mut self.header);
                *self = fresh;
                if !self.header.is_empty() {
                    let again = format!("======== {}, continued from {OLD_FILE_NAME} ========\n", self.header);
                    self.append(&again);
                }
            }
        }
        self.append(text);
    }

    fn append(&mut self, text: &str) {
        if self.file.write_all(text.as_bytes()).is_ok() {
            self.size += text.len() as u64;
        }
    }
}

/// One line of log.txt: UTC time, level, where it came from, what happened.
fn format_line(line: &LogLine) -> String {
    let when = chrono::DateTime::from_timestamp_millis(line.ts)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S%.3fZ").to_string())
        .unwrap_or_default();
    // A message of several lines stays readable and can't pass for other entries.
    let message = line.message.replace('\r', "").replace('\n', "\n        ");
    format!("{when}  {:<5}  {:<9}  {message}\n", line.level, line.scope)
}

/// Starts writing the log to `log.txt` in this folder, under a header for
/// this run of the app. Lines logged before this are written out now.
pub fn open_file(dir: &Path, started: &str) {
    let mut opened = match LogFile::open(dir) {
        Ok(f) => f,
        Err(e) => {
            warn("app", format!("No log file: {e}"));
            return;
        }
    };
    if opened.size == 0 {
        opened.write(
            "What IdleKit did and what went wrong, newest at the bottom. Times are UTC.\n\
             Sign-in names, Steam IDs, passwords, codes, cookies, tokens, e-mail and network\n\
             addresses and your user name are replaced before a line is written. Games,\n\
             achievements and market items are named. Nothing here is sent anywhere.\n",
        );
    }
    opened.header = redact(started);
    let header = format!("\n======== {} ========\n", opened.header);
    opened.write(&header);
    // Holding the file's lock from before the catch-up until the file is in
    // place: a line logged meanwhile waits, and is then written once.
    let Ok(mut file) = FILE.lock() else { return };
    for line in recent() {
        opened.write(&format_line(&line));
    }
    *file = Some(opened);
}

pub fn file_path() -> Option<PathBuf> {
    FILE.lock().ok()?.as_ref().map(|f| f.dir.join(FILE_NAME))
}

// -------------------------------------------------------------- logging ---

fn push(level: &'static str, scope: &str, message: String) {
    let mut message = redact(&message);
    if message.chars().count() > MAX_MESSAGE_CHARS {
        message = message.chars().take(MAX_MESSAGE_CHARS).collect::<String>() + " … (cut short)";
    }
    let mut line = LogLine {
        seq: 0,
        ts: chrono::Utc::now().timestamp_millis(),
        level,
        scope: scope.to_string(),
        message,
    };
    eprintln!("[{}] {}: {}", line.level, line.scope, line.message);
    {
        // The file's lock is taken first and held over both steps (see open_file).
        let mut file = FILE.lock().ok();
        // Numbered under the lock, so a snapshot holds exactly the lines up to its number.
        match BUF.lock() {
            Ok(mut buf) => {
                line.seq = next_seq();
                if buf.len() >= MAX_LINES {
                    buf.pop_front();
                }
                buf.push_back(line.clone());
            }
            Err(_) => line.seq = next_seq(),
        }
        if let Some(file) = file.as_mut().and_then(|f| f.as_mut()) {
            file.write(&format_line(&line));
        }
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

/// A line reported by the window: an action and how it ended, or a fault in
/// the interface. Level and scope are limited to known values.
pub fn from_window(level: &str, scope: &str, message: String) {
    let level = match level {
        "warn" => "warn",
        "error" => "error",
        "debug" => "debug",
        _ => "info",
    };
    let scope = if scope == "ui" { "ui" } else { "action" };
    push(level, scope, message);
}

/// A crash in the app's own code ends up in the log with where it happened.
pub fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".to_string());
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        // try_lock inside push would be safer still, but a panic while the log
        // itself holds a lock is not something the log could report anyway.
        error("crash", format!("The app hit a fault at {place}: {what}"));
        default(info);
    }));
}

pub fn recent() -> Vec<LogLine> {
    BUF.lock().map(|b| b.iter().cloned().collect()).unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut b) = BUF.lock() {
        b.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(text: &str) -> String {
        redact_with(text, &[])
    }

    #[test]
    fn tokens_and_cookies_never_reach_the_log() {
        let jwt = "eyJ0eXAiOiJKV1QiLCJhbGciOiJFZERTQSJ9.eyJpc3MiOiJzdGVhbSIsInN1YiI6Ijc2NTYxMTk3OTYwMjY1NzI5In0.c2lnbmF0dXJlLWJ5dGVz";
        assert_eq!(clean(&format!("refresh failed for {jwt}")), "refresh failed for <token>");
        let cookie = format!("steamLoginSecure=76561197960265729%7C%7C{jwt}; sessionid=0123456789abcdef01234567");
        let out = clean(&cookie);
        assert_eq!(out, "steamLoginSecure=<hidden>; sessionid=<hidden>");
        assert_eq!(clean(r#"{"refresh_token":"abc.def","guard_data":"xyz"}"#), r#"{"refresh_token":"<hidden>","guard_data":"<hidden>"}"#);
        assert_eq!(clean("login failed: password=hunter2&remember=1"), "login failed: password=<hidden>&remember=1");
        assert_eq!(clean(r#"{"username":"x","password": "hunter2"}"#), r#"{"username":"x","password": "<hidden>"}"#);
        assert_eq!(
            clean("error sending request for url (https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key=AB12CD34EF56&steamid=76561197960265729&access_token=abc)"),
            "error sending request for url (https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?key=<hidden>&steamid=<steam id …5729>&access_token=<hidden>)"
        );
        assert_eq!(clean("scan https://s.team/q/1/1234567890123456789 with the app"), "scan <sign-in link> with the app");
        assert_eq!(clean("blob QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVowMTIzNDU2Nzg5YWJjZGVm=="), "blob <hidden>");
    }

    #[test]
    fn people_and_places_never_reach_the_log() {
        assert_eq!(clean("76561197960265729 connected to Steam"), "<steam id …5729> connected to Steam");
        // next to anything, not only between spaces
        assert_eq!(
            clean("steamid_76561197960265729 id76561197960265729x 76561197960265729_440.bin %3D76561197960265729%7C"),
            "steamid_<steam id …5729> id<steam id …5729>x <steam id …5729>_440.bin %3D<steam id …5729>%7C"
        );
        assert_eq!(clean("76561197960265729,76561197960265730"), "<steam id …5729>,<steam id …5730>");
        // but other long numbers are not Steam IDs
        assert_eq!(clean("listing 4638273645182736451 asset 27364518273645"), "listing 4638273645182736451 asset 27364518273645");
        // a path that ends at the user folder keeps the rest of the sentence
        assert_eq!(clean(r"C:\Users\Zed could not be found"), r"C:\Users\<user> could not be found");
        assert_eq!(clean(r"\\?\C:\Users\zed\AppData and file:///C:/Users/zed/x"), r"\\?\C:\Users\<user>\AppData and file:///C:/Users/<user>/x");
        assert_eq!(
            clean(r"could not read C:\Users\Zed Zedson\AppData\Roaming\dev.idlekit.app\config.json"),
            r"could not read C:\Users\<user>\AppData\Roaming\dev.idlekit.app\config.json"
        );
        assert_eq!(clean("c:/users/zz/steam and /home/zz/.steam and /Users/zz/Library"), "c:/users/<user>/steam and /home/<user>/.steam and /Users/<user>/Library");
        assert_eq!(clean("mail zz.top+steam@example.co.uk about it"), "mail <email> about it");
        assert_eq!(clean("connect to 203.0.113.42:27017 failed"), "connect to <ip>:27017 failed");
        assert_eq!(clean("peer 2001:db8:85a3:0:0:8a2e:370:7334 left"), "peer <ip> left");
        // shortened IPv6
        assert_eq!(clean("connect to [2a02:26f0:1180::5435:b6c2]:443 failed"), "connect to [<ip>]:443 failed");
        assert_eq!(clean("tcp connect error: 2606:4700::6810:84e5"), "tcp connect error: <ip>");
        assert_eq!(clean("peer fe80::1ff:fe23:4567:890a%eth0 dropped, then 2001:db8:: too"), "peer <ip>%eth0 dropped, then <ip> too");
        assert_eq!(
            clean("error sending request for url (https://steamcommunity.com/id/nightowl/badges/?l=english&p=2)"),
            "error sending request for url (https://steamcommunity.com/id/<profile>/badges/?l=english&p=2)"
        );
        assert_eq!(
            clean("https://steamcommunity.com/profiles/76561197960265729/gamecards/440/ timed out"),
            "https://steamcommunity.com/profiles/<steam id …5729>/gamecards/440/ timed out"
        );
    }

    #[test]
    fn this_pcs_user_name_is_hidden_in_any_folder_path() {
        let users = vec![user_in_paths("Zed Zedson").unwrap(), user_in_paths("zz").unwrap()];
        let clean = |text: &str| redact_all(text, &[], &users);
        assert_eq!(clean(r"D:\Profiles\Zed Zedson\AppData\Roaming\x"), r"D:\Profiles\<user>\AppData\Roaming\x");
        assert_eq!(clean(r"\\NAS\home\zz\steam and E:\zz"), r"\\NAS\home\<user>\steam and E:\<user>");
        assert_eq!(clean("https://asset.localhost/C%3A%5CUsers%5Czz%5CAppData%5Cart.jpg"), "https://asset.localhost/C%3A%5CUsers%5C<user>%5CAppData%5Cart.jpg");
        assert_eq!(clean("/mnt/c/Users/ZZ/x"), "/mnt/c/Users/<user>/x");
        // only as a folder: the same letters elsewhere are left alone
        assert_eq!(clean(r"pizza, jazz and C:\Games\jazz\zzz"), r"pizza, jazz and C:\Games\jazz\zzz");
        assert!(user_in_paths("  ").is_none());
    }

    #[test]
    fn ordinary_lines_are_left_alone() {
        for line in [
            // the app's own lines about signing in by password, and Steam's answer
            "Start sign-in by name and password: as direct account: started",
            "Start sign-in by name and password: as direct account: FAILED after 812 ms: Wrong password: check it and try again",
            // names and addresses made of words, however long
            "ACH_WIN_ONE_HUNDRED_GAMES_IN_A_ROW_WITHOUT_LOSING unlocked",
            "753-Counter-Strike_Global_Offensive_Trading_Card_Foil_x: price too low to list",
            "HTTP 500 from https://api.steampowered.com/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/",
            // versions are not network addresses, and Rust paths are not IPv6
            "web view 157.0.4341.46 on version 10.0.19045.1, driver version 1.2.3.4, build 31.0.15.3623",
            "serde::de::Error at std::fs::read; done at 12:30:45",
            "IdleKit 0.2.2 started",
            "1 game(s) idling (auto)",
            "Could not start Team Fortress 2 (440): Steam isn't running",
            "Start idling: 3 game(s) (440, 570, 730): done in 412 ms",
            "The installer ended with exit code 1 at 12:30:45",
            "settings changed: farming.checkMinutes 10 → 15",
            "Listing 753-Trading Card (Foil) at 1.15",
        ] {
            assert_eq!(clean(line), line);
        }
    }

    #[test]
    fn sign_in_names_are_hidden_as_whole_words() {
        let hidden = vec![("nightowl_88".to_string(), "account …5729".to_string()), ("max".to_string(), "account …0001".to_string())];
        assert_eq!(redact_with("Signed in as nightowl_88", &hidden), "Signed in as account …5729");
        assert_eq!(redact_with("NightOwl_88: waiting; max is next; maximum 32 games; climax", &hidden), "account …5729: waiting; account …0001 is next; maximum 32 games; climax");
        assert_eq!(redact_with("max", &hidden), "account …0001");
        assert_eq!(redact_with("(max) max, max", &hidden), "(account …0001) account …0001, account …0001");
        assert_eq!(redact_with("naïve Ünïcode stays", &hidden), "naïve Ünïcode stays");
        // a sign-in name that is an everyday word is not registered at all
        hide("Steam", "account …0002");
        hide("st", "account …0003");
        assert!(!HIDDEN.lock().unwrap().iter().any(|(w, _)| w.eq_ignore_ascii_case("steam") || w == "st"));
        assert_eq!(label("76561197960265729"), "account …5729");
        assert_eq!(label(""), "account");
    }

    #[test]
    fn the_file_gets_dated_lines_and_starts_over_when_full() {
        let dir = std::env::temp_dir().join(format!("idlekit-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let line = LogLine { seq: 1, ts: 1_795_000_000_123, level: "warn", scope: "idle".into(), message: "first\nsecond".into() };
        assert_eq!(format_line(&line), "2026-11-18 11:06:40.123Z  warn   idle       first\n        second\n");

        let mut log = LogFile::open(&dir).unwrap();
        log.write(&format_line(&line));
        assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), format_line(&line));

        // full: the next write sets it aside and begins a new file, which
        // says again what this run is
        log.size = MAX_FILE_BYTES;
        log.header = "IdleKit 0.5 started on windows".into();
        log.write("after\n");
        assert_eq!(
            std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(),
            "======== IdleKit 0.5 started on windows, continued from log.old.txt ========\nafter\n"
        );
        assert_eq!(std::fs::read_to_string(dir.join(OLD_FILE_NAME)).unwrap(), format_line(&line));
        // and a file found full at start is set aside too, replacing the older one
        drop(log);
        std::fs::write(dir.join(FILE_NAME), vec![b'x'; MAX_FILE_BYTES as usize]).unwrap();
        let mut log = LogFile::open(&dir).unwrap();
        log.write("fresh\n");
        assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), "fresh\n");
        assert_eq!(std::fs::metadata(dir.join(OLD_FILE_NAME)).unwrap().len(), MAX_FILE_BYTES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
