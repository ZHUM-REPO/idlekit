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
static FILE: Mutex<Option<LogFile>> = Mutex::new(None);
/// Words to keep out of the log that no pattern can recognise, such as a
/// Steam sign-in name, each with what to write instead.
static HIDDEN: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
const MAX_LINES: usize = 1500;
pub const FILE_NAME: &str = "log.txt";
const OLD_FILE_NAME: &str = "log.old.txt";
/// log.txt is set aside as log.old.txt when it reaches this size, so the two
/// together never take more than twice this.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MESSAGE_CHARS: usize = 4000;

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

pub fn emit<S: Serialize + Clone>(event: &str, payload: S) {
    if let Some(app) = APP.get() {
        let _ = app.emit(event, payload);
    }
}

// ------------------------------------------------------------ redaction ---

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // Sign-in tokens (JWTs): refresh tokens, access tokens, the value of the login cookie.
            (r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}(?:\.[A-Za-z0-9_-]+)?", "<token>"),
            // Secrets passed as web address parameters.
            (
                r"(?i)([?&](?:key|access_token|refresh_token|token|sessionid|code|password|client_id|request_id|nonce|auth|steamguard)=)[^&\s)\x22']+",
                "$1<hidden>",
            ),
            // Cookies and secrets written as name=value or name: value.
            (
                r#"(?i)\b(steamLoginSecure|steamRefresh_steam|sessionid|steamMachineAuth\w*|refresh_token|access_token|guard_data|shared_secret|identity_secret|password)\b(\\?["']?\s*[=:]\s*\\?["']?)[^\s&;,"'\\)]+"#,
                "$1$2<hidden>",
            ),
            // The QR sign-in link.
            (r"https://s\.team/q/\S+", "<sign-in link>"),
            // A profile's custom address, which is a name its owner picked.
            (r#"(?i)(steamcommunity\.com/id/)[^/\s"'<>?#]+"#, "$1<profile>"),
            // SteamID64, shortened to its last four digits so accounts can still be told apart.
            (r"\b7656119\d{6}(\d{4})\b", "<steam id …$1>"),
            // The Windows, Linux or macOS user name inside a folder path.
            (r#"(?i)\b([a-z]:[\\/]+Users[\\/]+)[^\\/"'<>|:*?\r\n]+"#, "$1<user>"),
            (r#"(/home/|/Users/)[^/\s"'<>]+"#, "$1<user>"),
            // E-mail addresses.
            (r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}", "<email>"),
            // Network addresses.
            (r"\b(?:\d{1,3}\.){3}\d{1,3}\b", "<ip>"),
            (r"(?i)\b(?:[0-9a-f]{1,4}:){5,7}[0-9a-f]{1,4}\b", "<ip>"),
            // Anything else long and unreadable is more likely a key than a word.
            (r"\b[A-Za-z0-9+/_-]{40,}={0,2}", "<hidden>"),
        ]
        .into_iter()
        .map(|(pattern, with)| (Regex::new(pattern).expect("valid log pattern"), with))
        .collect()
    })
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
/// and the given words (sign-in names).
pub fn redact_with(text: &str, hidden: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (pattern, with) in patterns() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, *with).into_owned();
        }
    }
    for (word, with) in hidden {
        out = replace_word(&out, word, with);
    }
    out
}

pub fn redact(text: &str) -> String {
    let hidden = HIDDEN.lock().map(|h| h.clone()).unwrap_or_default();
    redact_with(text, &hidden)
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
    if word.chars().count() < 3 {
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

// ------------------------------------------------------------- the file ---

struct LogFile {
    dir: PathBuf,
    file: File,
    size: u64,
}

impl LogFile {
    fn open(dir: &Path) -> std::io::Result<LogFile> {
        let path = dir.join(FILE_NAME);
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) >= MAX_FILE_BYTES {
            let _ = std::fs::rename(&path, dir.join(OLD_FILE_NAME));
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(LogFile { dir: dir.to_path_buf(), file, size })
    }

    fn write(&mut self, text: &str) {
        if self.size >= MAX_FILE_BYTES {
            // Full: set it aside as the one older file and begin again.
            let _ = std::fs::rename(self.dir.join(FILE_NAME), self.dir.join(OLD_FILE_NAME));
            if let Ok(fresh) = LogFile::open(&self.dir) {
                *self = fresh;
            }
        }
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
    opened.write(&format!("\n======== {} ========\n", redact(started)));
    for line in recent() {
        opened.write(&format_line(&line));
    }
    if let Ok(mut file) = FILE.lock() {
        *file = Some(opened);
    }
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
    if let Ok(mut file) = FILE.lock() {
        if let Some(file) = file.as_mut() {
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
        assert_eq!(clean("password: hunter2 was wrong"), "password: <hidden> was wrong");
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
        assert_eq!(
            clean(r"could not read C:\Users\Zed Zedson\AppData\Roaming\dev.idlekit.app\config.json"),
            r"could not read C:\Users\<user>\AppData\Roaming\dev.idlekit.app\config.json"
        );
        assert_eq!(clean("c:/users/zz/steam and /home/zz/.steam and /Users/zz/Library"), "c:/users/<user>/steam and /home/<user>/.steam and /Users/<user>/Library");
        assert_eq!(clean("mail zz.top+steam@example.co.uk about it"), "mail <email> about it");
        assert_eq!(clean("connect to 203.0.113.42:27017 failed"), "connect to <ip>:27017 failed");
        assert_eq!(clean("peer 2001:db8:85a3:0:0:8a2e:370:7334 left"), "peer <ip> left");
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
    fn ordinary_lines_are_left_alone() {
        for line in [
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
        assert_eq!(label("76561197960265729"), "account …5729");
        assert_eq!(label(""), "account");
    }

    #[test]
    fn the_file_gets_dated_lines_and_starts_over_when_full() {
        let dir = std::env::temp_dir().join(format!("idlekit-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let line = LogLine { ts: 1_795_000_000_123, level: "warn", scope: "idle".into(), message: "first\nsecond".into() };
        assert_eq!(format_line(&line), "2026-11-18 11:06:40.123Z  warn   idle       first\n        second\n");

        let mut log = LogFile::open(&dir).unwrap();
        log.write(&format_line(&line));
        assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), format_line(&line));

        // full: the next write sets it aside and begins a new file
        log.size = MAX_FILE_BYTES;
        log.write("after\n");
        assert_eq!(std::fs::read_to_string(dir.join(FILE_NAME)).unwrap(), "after\n");
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
