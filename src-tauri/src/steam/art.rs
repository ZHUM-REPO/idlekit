//! Game art lookup: each game's icon, banner and cover image, resolved through
//! Steam's public store API (`IStoreBrowseService/GetItems`). That endpoint
//! needs no key and no sign-in and is keyed only by app id, so it works for
//! every account type, including ones whose games list came from local files.
//!
//! The lookup is needed because the paths can't always be built from the app
//! id: a game's icon lives under a hash, and many newer games serve their
//! banner and cover from hashed folders too. Art rarely changes, so results are
//! cached on disk. Ids Steam couldn't be reached for are left out of the reply;
//! the UI then falls back to the guessable URLs and asks again later.
//!
//! The same reply says what kind of content a game has, which is what the
//! "Hide NSFW games" setting goes by.
use crate::logbus;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

const DEFAULT_API: &str = "https://api.steampowered.com";
/// Hashed asset folders are only reliably served from the `shared.*` hosts.
const DEFAULT_CDN: &str = "https://shared.fastly.steamstatic.com";
const CACHE_FILE: &str = "game_art.json";
/// Ids per request; keeps the query string a few kilobytes long.
const BATCH: usize = 100;
/// Upper bound on one call, so a runaway caller can't queue thousands of requests.
const MAX_IDS: usize = 5000;
const TIMEOUT: Duration = Duration::from_secs(10);
const DAY: i64 = 86_400;
const KEEP_FOUND_SECS: i64 = 30 * DAY;
/// Games with no art are rechecked sooner: developers do add it later.
const KEEP_EMPTY_SECS: i64 = 3 * DAY;
/// A failed lookup is reported as a warning this often at most; in between it
/// only goes to the debug log, so being offline doesn't fill the Logs page.
const WARN_EVERY_SECS: i64 = 30 * 60;
/// Steam's content labels that make a game one to hide: "Adult Only Sexual
/// Content" and "Frequent Nudity or Sexual Content". The others ("Some Nudity
/// or Sexual Content" 1, "Frequent Violence or Gore" 2, "General Mature
/// Content" 5) cover a large share of ordinary games and are left alone.
const NSFW_DESCRIPTORS: [u64; 2] = [3, 4];

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GameArt {
    /// The small square icon Steam shows next to a game's name.
    pub icon: Option<String>,
    /// The 460x215 banner ("header").
    pub header: Option<String>,
    /// The 600x900 portrait cover ("library capsule").
    pub cover: Option<String>,
    /// Whether Steam labels the game as adult-only or as frequent nudity or
    /// sexual content. `None` only in entries saved before this was read.
    pub nsfw: Option<bool>,
}

impl GameArt {
    fn is_empty(&self) -> bool {
        self.icon.is_none() && self.header.is_none() && self.cover.is_none()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Cached {
    #[serde(flatten)]
    art: GameArt,
    /// Unix time of the lookup.
    at: i64,
}

fn is_fresh(entry: &Cached, now: i64) -> bool {
    // Saved by a version that didn't read the content labels: look it up again.
    if entry.art.nsfw.is_none() {
        return false;
    }
    let keep = if entry.art.is_empty() { KEEP_EMPTY_SECS } else { KEEP_FOUND_SECS };
    // An entry stamped in the future means the clock moved; look it up again.
    entry.at <= now + DAY && now - entry.at < keep
}

fn icon_url(cdn: &str, appid: u32, hash: &str) -> String {
    let file = if hash.contains('.') { hash.to_string() } else { format!("{hash}.jpg") };
    format!("{cdn}/community_assets/images/apps/{appid}/{file}")
}

/// Build the art URLs from one store item's `assets` object.
fn art_from_assets(cdn: &str, appid: u32, assets: &Value) -> GameArt {
    let text = |key: &str| {
        assets
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    // e.g. "steam/apps/570/${FILENAME}?t=1791407503"
    let format = text("asset_url_format").filter(|f| f.contains("${FILENAME}"));
    let asset = |file: Option<&str>| -> Option<String> {
        let path = format?.replace("${FILENAME}", file?);
        Some(if path.starts_with("https://") {
            path
        } else {
            format!("{cdn}/store_item_assets/{}", path.trim_start_matches('/'))
        })
    };
    GameArt {
        icon: text("community_icon").map(|hash| icon_url(cdn, appid, hash)),
        header: asset(text("header")),
        cover: asset(text("library_capsule").or_else(|| text("library_capsule_2x"))),
        nsfw: None,
    }
}

/// Whether a store item carries one of the content labels that hide a game.
fn is_nsfw(item: &Value) -> bool {
    item.get("content_descriptorids")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().filter_map(Value::as_u64).any(|id| NSFW_DESCRIPTORS.contains(&id)))
}

fn as_appid(v: Option<&Value>) -> Option<u32> {
    match v? {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .filter(|id| *id != 0)
}

/// Turn a `GetItems` reply into art for every requested id. Ids the reply has
/// nothing for (delisted or unknown apps come back with `appid` zeroed) get an
/// empty entry, so they're remembered instead of being looked up on every render.
/// Every entry says whether the game is one to hide; a game Steam's store has
/// nothing on is not.
pub fn parse_items(cdn: &str, requested: &[u32], body: &Value) -> HashMap<u32, GameArt> {
    let answered = GameArt { nsfw: Some(false), ..GameArt::default() };
    let mut out: HashMap<u32, GameArt> = requested.iter().map(|id| (*id, answered.clone())).collect();
    let items = body.pointer("/response/store_items").and_then(Value::as_array);
    for item in items.into_iter().flatten() {
        let Some(appid) = as_appid(item.get("appid")) else { continue };
        let Some(slot) = out.get_mut(&appid) else { continue };
        if let Some(assets) = item.get("assets") {
            *slot = art_from_assets(cdn, appid, assets);
        }
        slot.nsfw = Some(is_nsfw(item));
    }
    out
}

pub struct ArtResolver {
    file: PathBuf,
    api: String,
    cdn: String,
    /// Loaded from disk on first use.
    cache: tokio::sync::Mutex<Option<HashMap<u32, Cached>>>,
    /// Unix time of the last warning about a failed lookup.
    warned_at: AtomicI64,
}

impl ArtResolver {
    pub fn new(data_dir: &Path) -> Self {
        // The overrides exist so the lookup can be exercised against a local stand-in for Steam.
        let env = |key: &str, default: &str| {
            std::env::var(key)
                .ok()
                .map(|v| v.trim_end_matches('/').to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        Self::with_endpoints(
            data_dir,
            &env("IDLEKIT_STEAM_API", DEFAULT_API),
            &env("IDLEKIT_STEAM_CDN", DEFAULT_CDN),
        )
    }

    pub fn with_endpoints(data_dir: &Path, api: &str, cdn: &str) -> Self {
        Self {
            file: data_dir.join(CACHE_FILE),
            api: api.to_string(),
            cdn: cdn.to_string(),
            cache: tokio::sync::Mutex::new(None),
            warned_at: AtomicI64::new(0),
        }
    }

    fn load(&self) -> HashMap<u32, Cached> {
        // A missing or damaged file only costs lookups that can be repeated.
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, cache: &HashMap<u32, Cached>) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(cache)?)?;
        std::fs::rename(&tmp, &self.file)?;
        Ok(())
    }

    async fn fetch(&self, http: &reqwest::Client, language: &str, ids: &[u32]) -> Result<HashMap<u32, GameArt>> {
        let input = json!({
            "ids": ids.iter().map(|id| json!({ "appid": id })).collect::<Vec<_>>(),
            "context": { "language": language, "country_code": "US" },
            "data_request": { "include_assets": true },
        });
        let resp = http
            .get(format!("{}/IStoreBrowseService/GetItems/v1/", self.api))
            .query(&[("input_json", input.to_string())])
            .timeout(TIMEOUT)
            .send()
            .await
            // The URL lists every requested id, which would swamp the log line.
            .map_err(reqwest::Error::without_url)
            .context("could not reach Steam's store API")?;
        if !resp.status().is_success() {
            bail!("Steam's store API answered HTTP {}", resp.status());
        }
        let body: Value = resp
            .json()
            .await
            .map_err(reqwest::Error::without_url)
            .context("unreadable reply from Steam's store API")?;
        // A reply to a request for games is expected to carry the item list.
        // One without it is not an answer worth remembering for days.
        if body.pointer("/response/store_items").and_then(Value::as_array).is_none() {
            bail!("Steam's store API sent a reply with no item list");
        }
        Ok(parse_items(&self.cdn, ids, &body))
    }

    /// Art for each of `appids`. Ids are missing from the result only when
    /// Steam couldn't be reached for them.
    pub async fn resolve(&self, http: &reqwest::Client, language: &str, appids: &[u32]) -> HashMap<u32, GameArt> {
        let mut wanted: Vec<u32> = appids.iter().copied().filter(|id| *id != 0).collect();
        wanted.sort_unstable();
        wanted.dedup();
        wanted.truncate(MAX_IDS);
        let now = chrono::Utc::now().timestamp();

        let mut found = HashMap::with_capacity(wanted.len());
        let mut missing = Vec::new();
        // What was known before it went out of date: better than nothing if
        // Steam can't be reached now, above all for which games to hide.
        let mut outdated: HashMap<u32, GameArt> = HashMap::new();
        {
            let mut guard = self.cache.lock().await;
            let cache = guard.get_or_insert_with(|| self.load());
            for id in &wanted {
                match cache.get(id) {
                    Some(entry) if is_fresh(entry, now) => {
                        found.insert(*id, entry.art.clone());
                    }
                    other => {
                        if let Some(entry) = other.filter(|e| e.art.nsfw.is_some()) {
                            outdated.insert(*id, entry.art.clone());
                        }
                        missing.push(*id);
                    }
                }
            }
        }
        if missing.is_empty() {
            return found;
        }

        // The lock is not held across the network, so cached lookups never wait on Steam.
        let language = if language.trim().is_empty() { "english" } else { language };
        let mut fetched: HashMap<u32, GameArt> = HashMap::new();
        for batch in missing.chunks(BATCH) {
            match self.fetch(http, language, batch).await {
                Ok(arts) => fetched.extend(arts),
                Err(e) => {
                    if now - self.warned_at.load(Ordering::Relaxed) >= WARN_EVERY_SECS {
                        self.warned_at.store(now, Ordering::Relaxed);
                        logbus::warn("art", format!("Game pictures couldn't be looked up, so some may be missing: {e:#}"));
                    } else {
                        logbus::debug("art", format!("{} game(s) not looked up: {e:#}", batch.len()));
                    }
                    // Steam is unreachable or unhappy; don't hammer it with the remaining batches.
                    break;
                }
            }
        }
        // Games Steam didn't answer for keep their earlier answer, if there is one.
        outdated.retain(|id, _| !fetched.contains_key(id));
        if fetched.is_empty() {
            found.extend(outdated);
            return found;
        }

        {
            let mut guard = self.cache.lock().await;
            let cache = guard.get_or_insert_with(|| self.load());
            for (id, art) in &fetched {
                cache.insert(*id, Cached { art: art.clone(), at: now });
            }
            if let Err(e) = self.save(cache) {
                logbus::debug("art", format!("could not save the art cache: {e:#}"));
            }
        }
        found.extend(fetched);
        found.extend(outdated);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const CDN: &str = "https://cdn.test";

    #[test]
    fn builds_urls_for_plain_and_hashed_assets() {
        let body = json!({ "response": { "store_items": [
            { "item_type": 0, "id": 440, "success": 1, "visible": true, "appid": 440, "assets": {
                "asset_url_format": "steam/apps/440/${FILENAME}?t=1745368595",
                "header": "header.jpg",
                "library_capsule": "library_600x900.jpg",
                "community_icon": "e3f595a92552da3d664ad00277fad2107345f743"
            }},
            { "appid": "570", "assets": {
                "asset_url_format": "steam/apps/570/${FILENAME}?t=1791407503",
                "header": "a1b2c3/header.jpg",
                "library_capsule": "6cddc26c/library_capsule.jpg",
                "community_icon": "0bbb630d63262dd66d2fdd0f7d37e8661a410075.jpg"
            }}
        ]}});
        let out = parse_items(CDN, &[440, 570], &body);
        assert_eq!(
            out[&440],
            GameArt {
                icon: Some(format!("{CDN}/community_assets/images/apps/440/e3f595a92552da3d664ad00277fad2107345f743.jpg")),
                header: Some(format!("{CDN}/store_item_assets/steam/apps/440/header.jpg?t=1745368595")),
                cover: Some(format!("{CDN}/store_item_assets/steam/apps/440/library_600x900.jpg?t=1745368595")),
                nsfw: Some(false),
            }
        );
        // appid sent as a string, assets in hashed folders
        assert_eq!(
            out[&570].cover.as_deref(),
            Some("https://cdn.test/store_item_assets/steam/apps/570/6cddc26c/library_capsule.jpg?t=1791407503")
        );
        assert_eq!(
            out[&570].header.as_deref(),
            Some("https://cdn.test/store_item_assets/steam/apps/570/a1b2c3/header.jpg?t=1791407503")
        );
        // an icon given as a file name keeps its one extension
        assert_eq!(
            out[&570].icon.as_deref(),
            Some("https://cdn.test/community_assets/images/apps/570/0bbb630d63262dd66d2fdd0f7d37e8661a410075.jpg")
        );
    }

    #[test]
    fn missing_pieces_stay_empty() {
        let body = json!({ "response": { "store_items": [
            // delisted: Steam zeroes the appid, so it can't be matched to the request
            { "item_type": 0, "id": 480, "success": 15, "visible": false, "appid": 0 },
            // has an icon but no url format, and only a 2x cover name
            { "appid": 400, "assets": { "community_icon": "abc", "header": "header.jpg", "library_capsule_2x": "x2.jpg" } },
            // 2x cover is used when the regular one is absent; blank strings are ignored
            { "appid": 620, "assets": { "asset_url_format": "steam/apps/620/${FILENAME}", "library_capsule": " ", "library_capsule_2x": "cover_2x.jpg" } },
            // not asked for: ignored
            { "appid": 999, "assets": { "community_icon": "zzz" } }
        ]}});
        let out = parse_items(CDN, &[480, 400, 620, 10], &body);
        assert_eq!(out.len(), 4);
        assert!(out[&480].is_empty());
        assert!(out[&10].is_empty());
        assert_eq!(out[&400].icon.as_deref(), Some("https://cdn.test/community_assets/images/apps/400/abc.jpg"));
        assert_eq!((out[&400].header.clone(), out[&400].cover.clone()), (None, None));
        assert_eq!(out[&620].cover.as_deref(), Some("https://cdn.test/store_item_assets/steam/apps/620/cover_2x.jpg"));
        assert_eq!(out[&620].icon, None);
        // a reply with no items at all still yields an entry per id
        assert_eq!(parse_items(CDN, &[1, 2], &json!({ "response": {} })).len(), 2);
    }

    #[test]
    fn empty_results_expire_sooner() {
        let now = 100 * DAY;
        let entry = |icon: Option<&str>, age: i64| Cached {
            art: GameArt { icon: icon.map(String::from), nsfw: Some(false), ..Default::default() },
            at: now - age,
        };
        assert!(is_fresh(&entry(Some("u"), KEEP_EMPTY_SECS + 1), now));
        assert!(!is_fresh(&entry(None, KEEP_EMPTY_SECS + 1), now));
        assert!(is_fresh(&entry(None, KEEP_EMPTY_SECS - 1), now));
        assert!(!is_fresh(&entry(Some("u"), KEEP_FOUND_SECS + 1), now));
        // stamped far in the future (clock changed): not trusted
        assert!(!is_fresh(&entry(Some("u"), -3 * DAY), now));
        // saved before content labels were read: looked up again, however new
        let mut old = entry(Some("u"), 60);
        old.art.nsfw = None;
        assert!(!is_fresh(&old, now));
        let saved: Cached = serde_json::from_value(json!({ "icon": "u", "header": null, "cover": null, "at": now })).unwrap();
        assert_eq!(saved.art.nsfw, None);
    }

    #[test]
    fn content_labels_decide_which_games_to_hide() {
        // label sets as Steam sends them: some nudity (1), violence (2), adult only (3),
        // frequent nudity or sexual content (4), general mature (5)
        let body = json!({ "response": { "store_items": [
            { "appid": 10, "assets": { "community_icon": "a" } },
            { "appid": 20, "content_descriptorids": [1, 5] },
            { "appid": 30, "content_descriptorids": [1, 2, 5], "assets": { "community_icon": "c" } },
            { "appid": 40, "content_descriptorids": [1, 3, 5, 4], "assets": { "community_icon": "d" } },
            { "appid": 50, "content_descriptorids": [4] },
            { "appid": 60, "content_descriptorids": [3, 5] },
            { "appid": 70, "content_descriptorids": "3" },
            { "id": 80, "success": 15, "appid": 0, "content_descriptorids": [3] }
        ]}});
        let out = parse_items(CDN, &[10, 20, 30, 40, 50, 60, 70, 80, 90], &body);
        let hidden = |id: u32| out[&id].nsfw;
        assert_eq!([hidden(10), hidden(20), hidden(30)], [Some(false); 3]);
        assert_eq!([hidden(40), hidden(50), hidden(60)], [Some(true); 3]);
        // labels in a shape that isn't a list, a game Steam's store has nothing on,
        // and one the reply leaves out: answered, and not hidden
        assert_eq!([hidden(70), hidden(80), hidden(90)], [Some(false); 3]);
        // a hidden game keeps its pictures: hiding is the window's business
        assert!(out[&40].icon.is_some());
    }

    /// A stand-in for Steam's store API: answers every request with assets for
    /// each requested id below 1000 and counts the requests it served. A request
    /// that includes id 9999 gets a reply in a shape the app doesn't know.
    async fn fake_store() -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    let head = String::from_utf8_lossy(&buf);
                    let target = head.split_whitespace().nth(1).unwrap_or("");
                    let raw = target.split("input_json=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
                    let input: Value = serde_json::from_str(&urlencoding::decode(&raw.replace('+', " ")).unwrap()).unwrap();
                    assert_eq!(input.pointer("/data_request/include_assets"), Some(&json!(true)));
                    let ids: Vec<u64> = input["ids"].as_array().unwrap().iter().map(|id| id["appid"].as_u64().unwrap()).collect();
                    let items: Vec<Value> = ids
                        .iter()
                        .map(|&id| {
                            if id < 1000 {
                                json!({ "appid": id, "assets": {
                                    "asset_url_format": format!("steam/apps/{id}/${{FILENAME}}"),
                                    "header": "header.jpg",
                                    "library_capsule": "library_600x900.jpg",
                                    "community_icon": format!("icon{id}")
                                }})
                            } else {
                                json!({ "id": id, "success": 15, "appid": 0 })
                            }
                        })
                        .collect();
                    let body = if ids.contains(&9999) {
                        json!({ "response": {} }).to_string()
                    } else {
                        json!({ "response": { "store_items": items } }).to_string()
                    };
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn client() -> reqwest::Client {
        // Same TLS provider choice the app makes at startup.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        reqwest::Client::new()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("idlekit-art-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn looks_up_in_batches_and_caches_to_disk() {
        let (api, hits) = fake_store().await;
        let dir = temp_dir("batches");
        let http = client();
        let resolver = ArtResolver::with_endpoints(&dir, &api, CDN);

        // 150 distinct ids (with duplicates and a zero mixed in) -> two requests
        let mut ids: Vec<u32> = (1..=149).collect();
        ids.extend([5000, 7, 7, 0]);
        let out = resolver.resolve(&http, "english", &ids).await;
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(out.len(), 150);
        assert_eq!(out[&7].icon.as_deref(), Some("https://cdn.test/community_assets/images/apps/7/icon7.jpg"));
        assert_eq!(out[&7].cover.as_deref(), Some("https://cdn.test/store_item_assets/steam/apps/7/library_600x900.jpg"));
        assert!(out[&5000].is_empty(), "an app Steam has nothing for is still answered");

        // the same ids again: served from memory
        let again = resolver.resolve(&http, "english", &ids).await;
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(again, out);

        // a new resolver (a restart) reads the file; only the new id is fetched
        let restarted = ArtResolver::with_endpoints(&dir, &api, CDN);
        let more = restarted.resolve(&http, "", &[7, 5000, 150]).await;
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        assert_eq!(more.len(), 3);
        assert_eq!(more[&7], out[&7]);
        assert!(more[&150].header.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unreachable_store_leaves_ids_out() {
        let (api, hits) = fake_store().await;
        let dir = temp_dir("offline");
        let http = client();
        ArtResolver::with_endpoints(&dir, &api, CDN).resolve(&http, "english", &[1]).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // nothing listens on port 9: the cached id is answered, the rest are left out
        let offline = ArtResolver::with_endpoints(&dir, "http://127.0.0.1:9", CDN);
        let out = offline.resolve(&http, "english", &[1, 2, 3]).await;
        assert_eq!(out.len(), 1);
        assert!(out[&1].icon.is_some());

        // an answer that has gone out of date is still given when Steam can't be asked again
        let aged = json!({
            "1": { "icon": "old-icon", "header": null, "cover": null, "nsfw": true, "at": 1000 },
            "2": { "icon": "before-labels", "header": null, "cover": null, "at": 1000 }
        });
        std::fs::write(dir.join(CACHE_FILE), aged.to_string()).unwrap();
        let later = ArtResolver::with_endpoints(&dir, "http://127.0.0.1:9", CDN);
        let out = later.resolve(&http, "english", &[1, 2, 3]).await;
        assert_eq!(out.len(), 1, "only the game whose content was known");
        assert_eq!((out[&1].nsfw, out[&1].icon.as_deref()), (Some(true), Some("old-icon")));
        // and replaced by a fresh answer once Steam is there again
        let back = ArtResolver::with_endpoints(&dir, &api, CDN);
        let out = back.resolve(&http, "english", &[1, 2]).await;
        assert_eq!((out[&1].nsfw, out[&2].nsfw), (Some(false), Some(false)));
        assert!(out[&2].icon.as_deref().is_some_and(|u| u.ends_with("icon2.jpg")));

        // a damaged cache file is ignored rather than fatal
        std::fs::write(dir.join(CACHE_FILE), "{ not json").unwrap();
        let damaged = ArtResolver::with_endpoints(&dir, "http://127.0.0.1:9", CDN);
        assert!(damaged.resolve(&http, "english", &[1]).await.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn reply_without_an_item_list_is_not_remembered() {
        let (api, hits) = fake_store().await;
        let dir = temp_dir("shape");
        let http = client();
        let resolver = ArtResolver::with_endpoints(&dir, &api, CDN);
        assert!(resolver.resolve(&http, "english", &[5, 9999]).await.is_empty());
        assert!(!dir.join(CACHE_FILE).exists());
        // asked again next time, rather than answered from a cache of nothing
        assert!(resolver.resolve(&http, "english", &[5, 9999]).await.is_empty());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // and a normal reply afterwards is taken as usual
        assert!(resolver.resolve(&http, "english", &[5]).await[&5].icon.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
