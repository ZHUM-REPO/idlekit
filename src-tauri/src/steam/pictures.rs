//! Pictures kept on disk: game covers, banners and icons, and account
//! avatars. Each one is downloaded once, a few at a time, and from then on the
//! window gets it from the app's own folder instead of the internet.
//!
//! A file is named `<kind>_<id>_<mark>.<ext>`, where the mark is a short hash
//! of the address it came from. That is how a picture that has since changed
//! at the source (a new avatar) is told apart from the copy on disk.
use crate::logbus;
use crate::net::Logged;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::Semaphore;

/// Downloads running at the same time, however many are asked for. Opening a
/// library of hundreds of games must not take the whole connection.
pub const AT_ONCE: usize = 3;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(25);
/// After a download fails, the same picture is not tried again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
const DAY: u64 = 86_400;
pub const FOLDER: &str = "pictures";
/// Which pictures Steam turned out not to have, and when that was found out.
const NONE_FILE: &str = "none.json";
/// A picture Steam doesn't have is asked for again after this long:
/// developers do add art later.
const NONE_SECS: i64 = 3 * DAY as i64;
/// The mark of a picture copied from this PC instead of downloaded.
const LOCAL_MARK: &str = "00000000";

/// One picture the window would like: which kind, of what, and (when known)
/// the address it currently has at the source.
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub current: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cached {
    /// The file on disk; empty when `none`.
    pub file: String,
    /// On disk, but old or changed at the source: worth fetching again when
    /// there is a moment. The copy on disk can be shown meanwhile.
    pub stale: bool,
    /// Steam was asked not long ago and has no such picture, so there is
    /// nothing to show and nothing to fetch.
    pub none: bool,
}

struct Entry {
    file: String,
    mark: String,
}

/// What has been fetched since the last line about it in the log. Hundreds of
/// pictures make one line, not hundreds.
#[derive(Default)]
struct Tally {
    saved: usize,
    bytes: usize,
    failed: usize,
    last_error: String,
    last_at: Option<Instant>,
    reporting: bool,
}

/// How long nothing has to be fetched before the tally is written down.
const QUIET: Duration = Duration::from_secs(10);

pub struct Pictures {
    dir: PathBuf,
    /// "kind:id" → the file on disk. Filled from the folder on first use.
    known: Mutex<Option<HashMap<String, Entry>>>,
    failed: Mutex<HashMap<String, Instant>>,
    /// "kind:id" → when Steam said it has no such picture. Kept in `none.json`.
    none: Mutex<Option<HashMap<String, i64>>>,
    slots: Semaphore,
    tally: Arc<Mutex<Tally>>,
    /// A development stand-in for Steam's image hosts (IDLEKIT_STEAM_CDN).
    extra_host: Option<String>,
}

/// How long a copy is used before it is fetched again, by kind. Avatars change
/// now and then; game art almost never does.
fn keep_secs(kind: &str) -> u64 {
    match kind {
        "avatar" => 14 * DAY,
        _ => 120 * DAY,
    }
}

fn kind_ok(kind: &str) -> bool {
    matches!(kind, "cover" | "header" | "icon" | "avatar")
}

fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

pub fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// A short, stable hash (FNV-1a) of an address's path and query. The host is
/// left out, so the same picture on a mirror gets the same mark.
fn mark_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let path = rest.split_once('/').map(|(_, p)| p).unwrap_or("");
    let mut hash: u32 = 0x811c_9dc5;
    for byte in path.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    // Never the mark reserved for local copies.
    format!("{:08x}", if hash == 0 { 1 } else { hash })
}

/// The picture format, from the file's first bytes. Anything else (an error
/// page, a truncated reply) is not kept.
fn extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// The development stand-in for Steam's image hosts draws its sample art as
/// SVG. Only pictures from that host are accepted in this format.
fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]);
    let head = head.trim_start();
    head.starts_with("<svg") || (head.starts_with("<?xml") && head.contains("<svg"))
}

pub fn content_type(file: &str) -> &'static str {
    match file.rsplit('.').next() {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "image/jpeg",
    }
}

/// `<kind>_<id>_<mark>.<ext>` and nothing else: the only names ever written,
/// and the only ones the window is given files for.
pub fn parse_name(file: &str) -> Option<(&str, &str, &str)> {
    let (stem, ext) = file.rsplit_once('.')?;
    if !matches!(ext, "jpg" | "png" | "gif" | "webp" | "svg") {
        return None;
    }
    let mut parts = stem.split('_');
    let (kind, id, mark) = (parts.next()?, parts.next()?, parts.next()?);
    let mark_ok = mark.len() == 8 && mark.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    (parts.next().is_none() && kind_ok(kind) && id_ok(id) && mark_ok).then_some((kind, id, mark))
}

impl Pictures {
    pub fn new(dir: PathBuf) -> Self {
        let extra_host = std::env::var("IDLEKIT_STEAM_CDN")
            .ok()
            .and_then(|u| reqwest::Url::parse(&u).ok())
            .and_then(|u| u.host_str().map(str::to_string));
        Self {
            dir,
            known: Mutex::new(None),
            failed: Mutex::new(HashMap::new()),
            none: Mutex::new(None),
            slots: Semaphore::new(AT_ONCE),
            tally: Arc::default(),
            extra_host,
        }
    }

    /// Only Steam's own image hosts, over HTTPS. The window can ask for any
    /// address, so this is what keeps the app from fetching anything else.
    fn allowed(&self, url: &str) -> bool {
        let Ok(parsed) = reqwest::Url::parse(url) else { return false };
        let Some(host) = parsed.host_str() else { return false };
        if self.extra_host.as_deref() == Some(host) {
            return true;
        }
        parsed.scheme() == "https"
            && (host.ends_with(".steamstatic.com")
                || host.ends_with(".steamusercontent.com")
                || host == "media.steampowered.com"
                || host == "steamcdn-a.akamaihd.net")
    }

    fn with_known<R>(&self, f: impl FnOnce(&mut HashMap<String, Entry>) -> R) -> R {
        let mut guard = self.known.lock().unwrap();
        let known = guard.get_or_insert_with(|| {
            let mut map = HashMap::new();
            for entry in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
                let file = entry.file_name().to_string_lossy().into_owned();
                if let Some((kind, id, mark)) = parse_name(&file) {
                    map.insert(key(kind, id), Entry { mark: mark.to_string(), file });
                }
            }
            map
        });
        f(known)
    }

    fn with_none<R>(&self, f: impl FnOnce(&mut HashMap<String, i64>) -> R) -> R {
        let mut guard = self.none.lock().unwrap();
        let none = guard.get_or_insert_with(|| {
            std::fs::read(self.dir.join(NONE_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
        });
        f(none)
    }

    /// Whether Steam said, not long ago, that it has no such picture.
    fn known_none(&self, name: &str) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.with_none(|none| none.get(name).is_some_and(|at| now - at < NONE_SECS))
    }

    fn remember_none(&self, name: &str, is_none: bool) {
        let now = chrono::Utc::now().timestamp();
        let changed = self.with_none(|none| {
            let before = none.len();
            none.retain(|_, at| now - *at < NONE_SECS);
            let had = none.remove(name).is_some();
            if is_none {
                none.insert(name.to_string(), now);
            }
            is_none || had || none.len() != before
        });
        if changed {
            let text = self.with_none(|none| serde_json::to_vec(none).unwrap_or_default());
            let _ = std::fs::create_dir_all(&self.dir).and_then(|_| std::fs::write(self.dir.join(NONE_FILE), text));
        }
    }

    fn age_secs(&self, file: &str) -> u64 {
        std::fs::metadata(self.dir.join(file))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .map(|d| d.as_secs())
            .unwrap_or(u64::MAX)
    }

    /// What is on disk for this picture, without touching the network.
    pub fn cached(&self, ask: &Ask) -> Option<Cached> {
        if !kind_ok(&ask.kind) || !id_ok(&ask.id) {
            return None;
        }
        let name = key(&ask.kind, &ask.id);
        let on_disk = self.with_known(|known| known.get(&name).map(|e| (e.file.clone(), e.mark.clone())));
        let Some((file, mark)) = on_disk else {
            return self.known_none(&name).then(|| Cached { file: String::new(), stale: false, none: true });
        };
        let age = self.age_secs(&file);
        if age == u64::MAX {
            // Deleted behind the app's back: forget it, so it is fetched again.
            self.with_known(|known| known.remove(&key(&ask.kind, &ask.id)));
            return None;
        }
        let changed = ask.current.as_deref().is_some_and(|url| mark_of(url) != mark);
        Some(Cached { file, stale: changed || age > keep_secs(&ask.kind), none: false })
    }

    /// Puts a picture on disk under this kind and id, replacing an older copy.
    fn store(&self, kind: &str, id: &str, mark: &str, bytes: &[u8]) -> Option<String> {
        let ext = extension(bytes).or_else(|| (self.extra_host.is_some() && is_svg(bytes)).then_some("svg"))?;
        let file = format!("{kind}_{id}_{mark}.{ext}");
        std::fs::create_dir_all(&self.dir).ok()?;
        let tmp = self.dir.join(format!("{file}.part"));
        std::fs::write(&tmp, bytes).ok()?;
        std::fs::rename(&tmp, self.dir.join(&file)).ok()?;
        let old = self.with_known(|known| known.insert(key(kind, id), Entry { file: file.clone(), mark: mark.to_string() }));
        if let Some(old) = old.filter(|o| o.file != file) {
            let _ = std::fs::remove_file(self.dir.join(old.file));
        }
        Some(file)
    }

    /// Takes a picture that is already on this PC (Steam keeps the avatars of
    /// its signed-in accounts), unless one is on disk already.
    pub fn adopt_local(&self, kind: &str, id: &str, source: &Path) -> Option<String> {
        if !kind_ok(kind) || !id_ok(id) || self.with_known(|known| known.contains_key(&key(kind, id))) {
            return None;
        }
        let bytes = std::fs::read(source).ok()?;
        (bytes.len() <= MAX_BYTES).then(|| self.store(kind, id, LOCAL_MARK, &bytes)).flatten()
    }

    /// Counts one fetch, and makes sure a line about the whole run reaches the
    /// log once it has gone quiet.
    fn count(&self, outcome: Result<usize, &str>) {
        let mut tally = self.tally.lock().unwrap();
        match outcome {
            Ok(bytes) => {
                tally.saved += 1;
                tally.bytes += bytes;
            }
            Err(why) => {
                tally.failed += 1;
                tally.last_error = why.to_string();
            }
        }
        tally.last_at = Some(Instant::now());
        if tally.reporting {
            return;
        }
        tally.reporting = true;
        let shared = self.tally.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(QUIET).await;
                let mut tally = shared.lock().unwrap();
                if tally.last_at.is_some_and(|at| at.elapsed() < QUIET) {
                    continue;
                }
                let done = std::mem::take(&mut *tally);
                drop(tally);
                let mut line = format!(
                    "Fetched {} picture(s) from Steam, {} KB in all, {AT_ONCE} at a time; they are kept on disk",
                    done.saved,
                    done.bytes.div_ceil(1024)
                );
                if done.failed > 0 {
                    line.push_str(&format!(". {} could not be fetched (the last one: {})", done.failed, done.last_error));
                }
                if done.saved == 0 {
                    logbus::warn("pictures", line);
                } else {
                    logbus::info("pictures", line);
                }
                break;
            }
        });
    }

    /// The bytes of one picture. An error says why not, and whether that is
    /// the host's answer (`true`: it has no such picture) or only trouble
    /// reaching it (`false`).
    async fn download(&self, http: &reqwest::Client, url: &str) -> Result<Vec<u8>, (String, bool)> {
        let resp = http.get(url).timeout(TIMEOUT).logged().await.map_err(|e| (e.without_url().to_string(), false))?;
        let status = resp.status();
        if !status.is_success() {
            // 404 and its relatives are an answer; a 429 or a 5xx is trouble.
            let answered = status.is_client_error() && status.as_u16() != 429 && status.as_u16() != 408;
            return Err((format!("HTTP {status}"), answered));
        }
        if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
            return Err(("too large".into(), true));
        }
        let bytes = resp.bytes().await.map_err(|e| (e.without_url().to_string(), false))?;
        if bytes.len() > MAX_BYTES {
            return Err(("too large".into(), true));
        }
        let stand_in = self.extra_host.is_some() && is_svg(&bytes);
        if extension(&bytes).is_none() && !stand_in {
            return Err(("not a picture".into(), true));
        }
        Ok(bytes.to_vec())
    }

    /// The picture's file on disk, downloading it from the first of `urls`
    /// that works when it isn't there, is old, or has changed at the source.
    /// At most `AT_ONCE` downloads run at a time; the rest wait their turn.
    /// Gives the copy on disk when a fresh one can't be had, and nothing when
    /// there is neither.
    ///
    /// `certain` says the addresses are the picture's real ones (from Steam's
    /// own listing), not guesses. Only then is "none of them exists"
    /// remembered, so the picture isn't asked for again at every start.
    pub async fn fetch(&self, http: &reqwest::Client, ask: &Ask, urls: &[String], certain: bool) -> Option<String> {
        if !kind_ok(&ask.kind) || !id_ok(&ask.id) {
            return None;
        }
        let name = key(&ask.kind, &ask.id);
        let have = self.cached(ask);
        if let Some(c) = have.as_ref().filter(|c| !c.stale) {
            return (!c.none).then(|| c.file.clone());
        }
        let old = have.map(|c| c.file);
        let recently_failed = self.failed.lock().unwrap().get(&name).is_some_and(|at| at.elapsed() < RETRY_AFTER);
        if recently_failed {
            return old;
        }

        let _slot = self.slots.acquire().await.ok()?;
        // Someone else may have fetched it while this call waited its turn.
        if let Some(c) = self.cached(ask).filter(|c| !c.stale) {
            return (!c.none).then_some(c.file);
        }
        let mut why = String::from("no address to fetch it from");
        let (mut tried, mut all_answered) = (0, true);
        for url in urls.iter().filter(|u| self.allowed(u)).take(6) {
            tried += 1;
            match self.download(http, url).await {
                Ok(bytes) => {
                    if let Some(file) = self.store(&ask.kind, &ask.id, &mark_of(url), &bytes) {
                        self.failed.lock().unwrap().remove(&name);
                        self.remember_none(&name, false);
                        self.count(Ok(bytes.len()));
                        return Some(file);
                    }
                    why = "could not be written to disk".into();
                    all_answered = false;
                }
                Err((e, answered)) => {
                    why = e;
                    all_answered &= answered;
                }
            }
        }
        if certain && tried > 0 && all_answered && old.is_none() {
            // Steam has no such picture. That is not a failure worth a line.
            self.remember_none(&name, true);
            return None;
        }
        self.failed.lock().unwrap().insert(name, Instant::now());
        self.count(Err(&why));
        old
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3, 4];

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("idlekit-pictures-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn ask(kind: &str, id: &str, current: Option<&str>) -> Ask {
        Ask { kind: kind.into(), id: id.into(), current: current.map(str::to_string) }
    }

    /// A tiny image host on 127.0.0.1: every request is answered with a JPEG
    /// after `delay`, unless its path contains "missing". Counts requests and
    /// the most that were being served at once.
    async fn serve(delay: Duration) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (hits, busy, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (h, m) = (hits.clone(), most.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let (h, b, m) = (h.clone(), busy.clone(), m.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    h.fetch_add(1, Ordering::SeqCst);
                    let now = b.fetch_add(1, Ordering::SeqCst) + 1;
                    m.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(delay).await;
                    let reply = if request.lines().next().unwrap_or("").contains("missing") {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                    } else {
                        let mut r = format!("HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", JPEG.len()).into_bytes();
                        r.extend_from_slice(JPEG);
                        r
                    };
                    b.fetch_sub(1, Ordering::SeqCst);
                    let _ = socket.write_all(&reply).await;
                });
            }
        });
        (format!("http://{addr}"), hits, most)
    }

    fn store_for(base: &str, dir: PathBuf) -> Pictures {
        let mut pictures = Pictures::new(dir);
        pictures.extra_host = Some("127.0.0.1".into());
        assert!(pictures.allowed(&format!("{base}/x.jpg")));
        pictures
    }

    fn client() -> reqwest::Client {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        reqwest::Client::new()
    }

    #[test]
    fn only_steams_image_hosts_are_fetched_from() {
        let pictures = Pictures { extra_host: None, ..Pictures::new(temp_dir("hosts")) };
        for good in [
            "https://shared.fastly.steamstatic.com/store_item_assets/steam/apps/440/library_600x900.jpg?t=1",
            "https://cdn.cloudflare.steamstatic.com/steam/apps/440/header.jpg",
            "https://avatars.fastly.steamstatic.com/abc_full.jpg",
            "https://media.steampowered.com/steamcommunity/public/images/apps/440/e3f5.jpg",
        ] {
            assert!(pictures.allowed(good), "{good}");
        }
        for bad in [
            "http://shared.fastly.steamstatic.com/x.jpg",
            "https://example.com/steamstatic.com/x.jpg",
            "https://steamstatic.com.example.com/x.jpg",
            "https://127.0.0.1/x.jpg",
            "file:///C:/Windows/win.ini",
            "not an address",
        ] {
            assert!(!pictures.allowed(bad), "{bad}");
        }
    }

    #[test]
    fn file_names_are_strict() {
        assert_eq!(parse_name("cover_440_1a2b3c4d.jpg"), Some(("cover", "440", "1a2b3c4d")));
        assert_eq!(parse_name("avatar_76561197960265729_00000000.png"), Some(("avatar", "76561197960265729", "00000000")));
        for bad in ["cover_440.jpg", "cover_440_1A2B3C4D.jpg", "cover_44a_1a2b3c4d.jpg", "secret_440_1a2b3c4d.jpg", "cover_440_1a2b3c4d.exe", "../config.json", "cover_440_1a2b3c4d_x.jpg", "cover_440_1a2b3c4d.jpg.part"] {
            assert_eq!(parse_name(bad), None, "{bad}");
        }
        assert_eq!(content_type("icon_1_00000001.png"), "image/png");
        // the same picture on a mirror has the same mark; another path does not
        assert_eq!(mark_of("https://a.steamstatic.com/x/y.jpg?t=1"), mark_of("https://b.steamstatic.com/x/y.jpg?t=1"));
        assert_ne!(mark_of("https://a.steamstatic.com/x/y.jpg?t=1"), mark_of("https://a.steamstatic.com/x/y.jpg?t=2"));
        assert_eq!(extension(b"<html>"), None);
        assert_eq!(extension(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None, "SVG is only taken from the stand-in host");
        assert!(is_svg(b"<?xml version='1.0'?>\n<svg/>") && !is_svg(b"<html><svg/></html>"));
        assert_eq!(extension(JPEG), Some("jpg"));
    }

    #[tokio::test]
    async fn each_picture_is_downloaded_once_and_kept() {
        let (base, hits, _) = serve(Duration::ZERO).await;
        let dir = temp_dir("once");
        let pictures = store_for(&base, dir.clone());
        let http = client();
        let url = format!("{base}/apps/440/library_600x900.jpg");
        let cover = ask("cover", "440", None);

        assert_eq!(pictures.cached(&cover), None);
        let file = pictures.fetch(&http, &cover, &[format!("{base}/missing.jpg"), url.clone()], true).await.unwrap();
        assert_eq!(parse_name(&file).map(|p| (p.0, p.1)), Some(("cover", "440")));
        assert_eq!(std::fs::read(dir.join(&file)).unwrap(), JPEG);
        assert_eq!(hits.load(Ordering::SeqCst), 2, "the missing one, then the one that worked");

        // asked again, also by a copy of the app that has just started: no request
        assert_eq!(pictures.fetch(&http, &cover, &[url.clone()], true).await.as_deref(), Some(file.as_str()));
        let restarted = store_for(&base, dir.clone());
        assert_eq!(restarted.cached(&cover), Some(Cached { file: file.clone(), stale: false, none: false }));
        assert_eq!(restarted.fetch(&http, &cover, &[url], true).await.as_deref(), Some(file.as_str()));
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // nothing to fetch it from, and nothing on disk: no picture, no request
        assert_eq!(pictures.fetch(&http, &ask("icon", "440", None), &[], true).await, None);
        // an address outside Steam's hosts is not fetched
        assert_eq!(pictures.fetch(&http, &ask("icon", "441", None), &["https://example.com/x.jpg".into()], true).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_picture_steam_does_not_have_is_not_asked_for_at_every_start() {
        let (base, hits, _) = serve(Duration::ZERO).await;
        let dir = temp_dir("none");
        let http = client();
        let cover = ask("cover", "268910", None);
        let urls = [format!("{base}/apps/268910/missing_cover.jpg"), format!("{base}/mirror/268910/missing_cover.jpg")];

        // a guess that isn't there proves nothing: it is tried again next time
        let pictures = store_for(&base, dir.clone());
        assert_eq!(pictures.fetch(&http, &cover, &urls[..1], false).await, None);
        assert_eq!(pictures.cached(&cover), None);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // the real addresses, and Steam has neither: remembered, also after a restart
        let pictures = store_for(&base, dir.clone());
        assert_eq!(pictures.fetch(&http, &cover, &urls, true).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let restarted = store_for(&base, dir.clone());
        assert_eq!(restarted.cached(&cover), Some(Cached { file: String::new(), stale: false, none: true }));
        assert_eq!(restarted.fetch(&http, &cover, &urls, true).await, None);
        assert_eq!(hits.load(Ordering::SeqCst), 3, "not asked for again");

        // a host that can't be reached is not an answer, so nothing is remembered
        let other = ask("cover", "4000", None);
        assert_eq!(restarted.fetch(&http, &other, &["http://127.0.0.1:1/apps/4000/cover.jpg".into()], true).await, None);
        assert_eq!(restarted.cached(&other), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_few_at_a_time_however_many_are_asked_for() {
        let (base, hits, most) = serve(Duration::from_millis(60)).await;
        let dir = temp_dir("few");
        let pictures = Arc::new(store_for(&base, dir.clone()));
        let http = client();
        let mut tasks = Vec::new();
        for appid in 0..24 {
            let (pictures, http, base) = (pictures.clone(), http.clone(), base.clone());
            tasks.push(tokio::spawn(async move {
                pictures.fetch(&http, &ask("cover", &(1000 + appid).to_string(), None), &[format!("{base}/apps/{appid}/cover.jpg")], true).await
            }));
        }
        for task in tasks {
            assert!(task.await.unwrap().is_some());
        }
        assert_eq!(hits.load(Ordering::SeqCst), 24);
        assert!(most.load(Ordering::SeqCst) <= AT_ONCE, "{} at once", most.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_changed_picture_is_fetched_again_and_a_failure_keeps_the_old_one() {
        let (base, hits, _) = serve(Duration::ZERO).await;
        let dir = temp_dir("changed");
        let pictures = store_for(&base, dir.clone());
        let http = client();
        let old_url = format!("{base}/avatars/old_full.jpg");
        let new_url = format!("{base}/avatars/new_full.jpg");

        let first = pictures.fetch(&http, &ask("avatar", "76561197960265729", Some(&old_url)), &[old_url.clone()], true).await.unwrap();
        assert_eq!(pictures.cached(&ask("avatar", "76561197960265729", Some(&old_url))).unwrap().stale, false);
        // the profile now has another avatar
        let changed = ask("avatar", "76561197960265729", Some(&new_url));
        assert_eq!(pictures.cached(&changed), Some(Cached { file: first.clone(), stale: true, none: false }));
        let second = pictures.fetch(&http, &changed, &[new_url.clone()], true).await.unwrap();
        assert_ne!(first, second);
        assert!(!dir.join(&first).exists(), "the old copy is removed");
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // changed again, but the new one can't be fetched: the copy on disk is given
        let gone = format!("{base}/avatars/missing_full.jpg");
        let again = ask("avatar", "76561197960265729", Some(&gone));
        assert_eq!(pictures.fetch(&http, &again, &[gone.clone()], true).await.as_deref(), Some(second.as_str()));
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        // and it is not tried again straight away
        assert_eq!(pictures.fetch(&http, &again, &[gone], true).await.as_deref(), Some(second.as_str()));
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_already_on_this_pc_is_taken_without_the_network() {
        let dir = temp_dir("local");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("steam-avatar.png");
        std::fs::write(&source, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
        let pictures = Pictures::new(dir.join("pictures"));
        let file = pictures.adopt_local("avatar", "76561197960265729", &source).unwrap();
        assert_eq!(file, "avatar_76561197960265729_00000000.png");
        // not taken twice, and not when the file is no picture
        assert_eq!(pictures.adopt_local("avatar", "76561197960265729", &source), None);
        std::fs::write(&source, "<html>").unwrap();
        assert_eq!(pictures.adopt_local("avatar", "76561197960265730", &source), None);
        // once the profile's own address is known, the local copy counts as old
        let known = ask("avatar", "76561197960265729", Some("https://avatars.fastly.steamstatic.com/abc_full.jpg"));
        assert_eq!(pictures.cached(&known), Some(Cached { file, stale: true, none: false }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
