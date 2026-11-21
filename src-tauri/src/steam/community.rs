//! Signed-in Steam Community/Store web session (cookie-based) and the page
//! scrapers built on it: badge pages (card drops), licenses (refund window),
//! the games list fallback, and another player's achievement unlock times.
use super::webapi::{OwnedGame, UA_BROWSER};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use rand::Rng;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const DOMAINS: [&str; 4] = [
    "steamcommunity.com",
    "store.steampowered.com",
    "help.steampowered.com",
    "checkout.steampowered.com",
];

pub struct WebSession {
    pub steam_id: String,
    pub session_id: String,
    pub client: reqwest::Client,
    /// Access token behind the cookie, when known (used for its expiry).
    pub access_token: String,
}

/// Normalise a pasted cookie: accepts `steamLoginSecure=...`, the raw value,
/// URL-encoded or not. Returns (steam_id, url-encoded cookie value, token).
pub fn parse_login_secure(input: &str) -> Result<(String, String, String)> {
    let mut v = input.trim().trim_matches('"').to_string();
    if let Some(rest) = v.strip_prefix("steamLoginSecure=") {
        v = rest.split(';').next().unwrap_or("").to_string();
    }
    let decoded = urlencoding::decode(&v).map(|c| c.into_owned()).unwrap_or(v.clone());
    let (sid, token) = decoded
        .split_once("||")
        .ok_or_else(|| anyhow!("That doesn't look like a steamLoginSecure cookie (expected 7656…||token)"))?;
    if sid.len() != 17 || !sid.chars().all(|c| c.is_ascii_digit()) {
        bail!("The cookie does not start with a 17-digit SteamID");
    }
    let encoded = urlencoding::encode(&decoded).into_owned();
    Ok((sid.to_string(), encoded, token.to_string()))
}

fn random_session_id() -> String {
    let mut rng = rand::thread_rng();
    (0..12).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

impl WebSession {
    pub fn from_access_token(steam_id: &str, access_token: &str) -> Result<Self> {
        let value = urlencoding::encode(&format!("{steam_id}||{access_token}")).into_owned();
        Self::build(steam_id, &value, access_token)
    }

    pub fn from_cookie(cookie: &str) -> Result<Self> {
        let (steam_id, value, token) = parse_login_secure(cookie)?;
        Self::build(&steam_id, &value, &token)
    }

    fn build(steam_id: &str, login_secure: &str, access_token: &str) -> Result<Self> {
        let jar = Arc::new(reqwest::cookie::Jar::default());
        let session_id = random_session_id();
        for domain in DOMAINS {
            let url: reqwest::Url = format!("https://{domain}/").parse()?;
            jar.add_cookie_str(
                &format!("steamLoginSecure={login_secure}; Domain={domain}; Path=/; Secure; HttpOnly"),
                &url,
            );
            jar.add_cookie_str(&format!("sessionid={session_id}; Domain={domain}; Path=/; Secure"), &url);
            jar.add_cookie_str(&format!("Steam_Language=english; Domain={domain}; Path=/"), &url);
        }
        let client = reqwest::Client::builder()
            .cookie_provider(jar)
            .user_agent(UA_BROWSER)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            steam_id: steam_id.to_string(),
            session_id,
            client,
            access_token: access_token.to_string(),
        })
    }

    pub fn profile_url(&self) -> String {
        format!("https://steamcommunity.com/profiles/{}", self.steam_id)
    }

    /// GET with retry on Steam's 429/5xx responses.
    pub async fn get_text(&self, url: &str) -> Result<String> {
        let mut delay = 4u64;
        for attempt in 0..4 {
            let resp = self.client.get(url).send().await.with_context(|| format!("GET {url}"))?;
            let status = resp.status();
            if status.as_u16() == 429 || status.is_server_error() {
                if attempt == 3 {
                    bail!("Steam is rate-limiting requests (HTTP {status}); try again later");
                }
                tokio::time::sleep(Duration::from_secs(delay)).await;
                delay *= 3;
                continue;
            }
            if !status.is_success() {
                bail!("HTTP {status} from {url}");
            }
            return Ok(resp.text().await?);
        }
        unreachable!()
    }

    pub async fn badges(&self) -> Result<Vec<BadgeGame>> {
        let mut all = Vec::new();
        let mut page = 1u32;
        loop {
            let html = self
                .get_text(&format!("{}/badges/?l=english&p={page}", self.profile_url()))
                .await?;
            let parsed = parse_badges_page(&html);
            if parsed.logged_in_as.as_deref() != Some(self.steam_id.as_str()) {
                bail!("Steam web session expired — reconnect this account's web session");
            }
            all.extend(parsed.games);
            if page >= parsed.max_page || page >= 50 {
                break;
            }
            page += 1;
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
        Ok(all)
    }

    pub async fn gamecard(&self, appid: u32) -> Result<BadgeGame> {
        let html = self
            .get_text(&format!("{}/gamecards/{appid}/?l=english", self.profile_url()))
            .await?;
        let parsed = parse_badges_page(&html);
        if parsed.logged_in_as.as_deref() != Some(self.steam_id.as_str()) {
            bail!("Steam web session expired — reconnect this account's web session");
        }
        let mut g = parse_gamecard_page(&html);
        g.appid = appid;
        Ok(g)
    }

    pub async fn licenses(&self) -> Result<Vec<License>> {
        let html = self
            .get_text("https://store.steampowered.com/account/licenses/?l=english")
            .await?;
        Ok(parse_licenses_page(&html))
    }

    pub async fn games_from_profile(&self) -> Result<Vec<OwnedGame>> {
        let html = self
            .get_text(&format!("{}/games/?tab=all&l=english", self.profile_url()))
            .await?;
        parse_games_page(&html)
    }

    /// Ensure the session is still signed in (cheap check on the badges page).
    pub async fn verify(&self) -> Result<()> {
        let html = self
            .get_text(&format!("{}/badges/?l=english", self.profile_url()))
            .await?;
        match parse_badges_page(&html).logged_in_as {
            Some(id) if id == self.steam_id => Ok(()),
            Some(other) => bail!("This cookie belongs to a different account ({other})"),
            None => bail!("Steam did not accept this session"),
        }
    }
}

/// Another player's unlock order/timing for a game, from their public profile.
pub async fn profile_unlock_times(
    http: &reqwest::Client,
    profile: &str,
    appid: u32,
) -> Result<Vec<(String, i64)>> {
    let base = profile_base_url(profile)?;
    let url = format!("{base}/stats/{appid}/achievements/?l=english");
    let resp = http.get(&url).send().await?;
    if !resp.status().is_success() {
        bail!("HTTP {} loading {url}", resp.status());
    }
    let html = resp.text().await?;
    let list = parse_profile_achievements(&html, chrono::Local::now().year());
    if list.is_empty() {
        bail!("No unlocked achievements found — the profile or its game details may be private");
    }
    Ok(list)
}

/// Accepts a profile URL, vanity name or 17-digit SteamID.
pub fn profile_base_url(input: &str) -> Result<String> {
    let s = input.trim().trim_end_matches('/');
    if let Some(caps) = re(r"steamcommunity\.com/(id|profiles)/([^/?#]+)").captures(s) {
        return Ok(format!("https://steamcommunity.com/{}/{}", &caps[1], &caps[2]));
    }
    if s.len() == 17 && s.chars().all(|c| c.is_ascii_digit()) {
        return Ok(format!("https://steamcommunity.com/profiles/{s}"));
    }
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Ok(format!("https://steamcommunity.com/id/{s}"));
    }
    bail!("Enter a Steam profile URL, custom URL name or SteamID64")
}

// ---------------------------------------------------------------- parsers --

fn re(pattern: &str) -> Regex {
    // Small fixed set of patterns; compile on demand is fine at our call rates.
    Regex::new(pattern).expect("valid regex")
}

fn sel(css: &str) -> Selector {
    Selector::parse(css).expect("valid selector")
}

fn text_of(el: ElementRef) -> String {
    el.text().collect::<Vec<_>>().join(" ").replace('\u{a0}', " ")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BadgeGame {
    pub appid: u32,
    pub name: String,
    pub drops_remaining: u32,
    pub playtime_hours: f64,
}

pub struct BadgesPage {
    pub games: Vec<BadgeGame>,
    pub max_page: u32,
    pub logged_in_as: Option<String>,
}

fn logged_in_as(html: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r#"g_steamID\s*=\s*"(\d{17})""#))
        .captures(html)
        .map(|c| c[1].to_string())
}

fn drops_from_text(t: &str) -> u32 {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r"(\d+)\s+card drops?\s+remaining"))
        .captures(t)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0)
}

fn hours_from_text(t: &str) -> f64 {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| re(r"([\d,]+(?:\.\d+)?)\s+hrs?\s+on record"))
        .captures(t)
        .and_then(|c| c[1].replace(',', "").parse().ok())
        .unwrap_or(0.0)
}

pub fn parse_badges_page(html: &str) -> BadgesPage {
    let doc = Html::parse_document(html);
    let appid_re = re(r"/gamecards/(\d+)");
    let mut games = Vec::new();
    for row in doc.select(&sel("div.badge_row")) {
        let Some(href) = row
            .select(&sel("a.badge_row_overlay"))
            .next()
            .and_then(|a| a.value().attr("href"))
        else {
            continue;
        };
        let Some(appid) = appid_re.captures(href).and_then(|c| c[1].parse::<u32>().ok()) else {
            continue; // community/event badges have no app
        };
        // A game's foil badge has a row of its own (…/gamecards/730/?border=1)
        // that never shows card drops. Left in, it would stand for the game
        // wherever one row per game is expected, and report no drops left.
        if href.contains("border=1") {
            continue;
        }
        let drops = row
            .select(&sel(".progress_info_bold"))
            .next()
            .map(|e| drops_from_text(&text_of(e)))
            .unwrap_or(0);
        let hours = row
            .select(&sel(".badge_title_stats_playtime"))
            .next()
            .map(|e| hours_from_text(&text_of(e)))
            .unwrap_or(0.0);
        let name = row
            .select(&sel(".badge_title"))
            .next()
            .map(|e| {
                let full = text_of(e);
                squash(&full.replace("View details", ""))
            })
            .unwrap_or_default();
        games.push(BadgeGame { appid, name, drops_remaining: drops, playtime_hours: hours });
    }
    let max_page = doc
        .select(&sel("a.pagelink"))
        .filter_map(|a| text_of(a).trim().parse::<u32>().ok())
        .max()
        .unwrap_or(1)
        .max(1);
    BadgesPage { games, max_page, logged_in_as: logged_in_as(html) }
}

pub fn parse_gamecard_page(html: &str) -> BadgeGame {
    let doc = Html::parse_document(html);
    let drops = doc
        .select(&sel(".progress_info_bold"))
        .next()
        .map(|e| drops_from_text(&text_of(e)))
        .unwrap_or(0);
    let hours = doc
        .select(&sel(".badge_title_stats_playtime"))
        .next()
        .map(|e| hours_from_text(&text_of(e)))
        .unwrap_or(0.0);
    let name = doc
        .select(&sel(".badge_title"))
        .next()
        .map(|e| squash(&text_of(e).replace("View details", "")))
        .unwrap_or_default();
    BadgeGame { appid: 0, name, drops_remaining: drops, playtime_hours: hours }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct License {
    pub date: NaiveDate,
    pub name: String,
    pub acquisition: String,
}

pub fn parse_date_loose(s: &str) -> Option<NaiveDate> {
    let s = squash(&s.replace('\u{a0}', " "));
    for fmt in ["%d %b, %Y", "%b %d, %Y", "%d %B, %Y", "%B %d, %Y", "%d %b %Y", "%Y-%m-%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(&s, fmt) {
            return Some(d);
        }
    }
    None
}

pub fn parse_licenses_page(html: &str) -> Vec<License> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    for tr in doc.select(&sel("table.account_table tr")) {
        let Some(date) = tr
            .select(&sel("td.license_date_col"))
            .next()
            .and_then(|td| parse_date_loose(&text_of(td)))
        else {
            continue;
        };
        let tds: Vec<ElementRef> = tr.select(&sel("td")).collect();
        let Some(item) = tds.get(1) else { continue };
        let remove_text: String = item
            .select(&sel(".free_license_remove_link"))
            .map(text_of)
            .collect::<Vec<_>>()
            .join(" ");
        let mut name = squash(&text_of(*item));
        let remove_text = squash(&remove_text);
        if !remove_text.is_empty() {
            name = squash(&name.replacen(&remove_text, "", 1));
        }
        let acquisition = tds.get(2).map(|t| squash(&text_of(*t))).unwrap_or_default();
        out.push(License { date, name, acquisition });
    }
    out
}

fn normalize_title(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words that may follow a game's name in a package/license name without
/// making it a different game ("Hades Standard Edition" vs "Hades II").
const EDITION_WORDS: &[&str] = &[
    "standard", "edition", "deluxe", "digital", "commercial", "license", "retail", "steam", "store",
    "package", "complete", "goty", "game", "of", "the", "year", "bundle", "ultimate", "gold", "pack",
    "limited", "free", "weekend", "key", "base", "full", "version", "pc", "special", "premium",
];

fn same_game(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    match long.strip_prefix(short).and_then(|rest| rest.strip_prefix(' ')) {
        Some(rest) => rest.split_whitespace().all(|w| EDITION_WORDS.contains(&w)),
        None => false,
    }
}

/// Heuristic refund check: a license for this game in the last 14 days, and
/// under 2 hours on record. Matching is by name since the licenses page has no app ids.
pub fn likely_refundable(game_name: &str, playtime_hours: f64, licenses: &[License], today: NaiveDate) -> bool {
    if playtime_hours >= 2.0 || game_name.is_empty() {
        return false;
    }
    let g = normalize_title(game_name);
    licenses.iter().any(|l| {
        let age = (today - l.date).num_days();
        if !(0..=14).contains(&age) {
            return false;
        }
        if l.acquisition.to_lowercase().contains("complimentary") {
            return false;
        }
        same_game(&normalize_title(&l.name), &g)
    })
}

pub fn parse_games_page(html: &str) -> Result<Vec<OwnedGame>> {
    let doc = Html::parse_document(html);
    let raw = doc
        .select(&sel("[data-profile-gameslist]"))
        .next()
        .and_then(|e| e.value().attr("data-profile-gameslist"))
        .map(str::to_string)
        .or_else(|| {
            // Older layout: var rgGames = [...];
            re(r"(?s)var rgGames\s*=\s*(\[.*?\]);")
                .captures(html)
                .map(|c| format!("{{\"rgGames\":{}}}", &c[1]))
        })
        .ok_or_else(|| anyhow!("games list not found on profile page (is it private?)"))?;
    let v: serde_json::Value = serde_json::from_str(&raw).context("games list JSON")?;
    let list = v
        .get("rgGames")
        .and_then(|g| g.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(list
        .iter()
        .filter_map(|g| {
            let appid = g.get("appid")?.as_u64()? as u32;
            let minutes = match g.get("playtime_forever") {
                Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
                // Old layout gives "hours_forever": "1,234.5"
                _ => g
                    .get("hours_forever")
                    .and_then(|h| h.as_str())
                    .and_then(|h| h.replace(',', "").parse::<f64>().ok())
                    .map(|h| (h * 60.0) as u64)
                    .unwrap_or(0),
            };
            Some(OwnedGame {
                appid,
                name: g.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                playtime_minutes: minutes as u32,
                icon_hash: g.get("img_icon_url").and_then(|n| n.as_str()).unwrap_or("").to_string(),
                has_stats: false,
                last_played: g.get("rtime_last_played").and_then(|n| n.as_u64()).unwrap_or(0) as u32,
            })
        })
        .collect())
}

/// Parse "Unlocked 5 Mar, 2024 @ 10:41pm" (and the US/no-year variants).
pub fn parse_unlock_time(text: &str, current_year: i32) -> Option<NaiveDateTime> {
    let t = squash(&text.replace('\u{a0}', " "));
    let t = t.trim_start_matches("Unlocked").trim();
    let (date_part, time_part) = t.split_once('@')?;
    let date_part = date_part.trim();
    let time = NaiveTime::parse_from_str(&time_part.trim().to_lowercase(), "%I:%M%P")
        .or_else(|_| NaiveTime::parse_from_str(time_part.trim(), "%H:%M"))
        .ok()?;
    let date = parse_date_loose(date_part).or_else(|| {
        let with_year = format!("{date_part}, {current_year}");
        parse_date_loose(&with_year)
    })?;
    Some(date.and_time(time))
}

/// Returns (achievement display name, seconds since the first unlock), oldest first.
pub fn parse_profile_achievements(html: &str, current_year: i32) -> Vec<(String, i64)> {
    let doc = Html::parse_document(html);
    let mut rows: Vec<(String, NaiveDateTime)> = Vec::new();
    for row in doc.select(&sel(".achieveRow")) {
        let Some(time) = row
            .select(&sel(".achieveUnlockTime"))
            .next()
            .and_then(|e| parse_unlock_time(&text_of(e), current_year))
        else {
            continue;
        };
        let name = row
            .select(&sel(".achieveTxt h3"))
            .next()
            .map(|e| squash(&text_of(e)))
            .unwrap_or_default();
        if !name.is_empty() {
            rows.push((name, time));
        }
    }
    rows.sort_by_key(|(_, t)| *t);
    let Some(first) = rows.first().map(|(_, t)| *t) else { return vec![] };
    rows.into_iter()
        .map(|(n, t)| (n, (t - first).num_seconds()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BADGES: &str = r#"<html><script>g_steamID = "76561197960265729";</script>
<div class="badges_sheet">
 <div class="badge_row is_link">
  <a class="badge_row_overlay" href="https://steamcommunity.com/id/me/gamecards/730/"></a>
  <div class="badge_title_stats_playtime">&nbsp;1,234.5 hrs on record</div>
  <span class="progress_info_bold">3 card drops remaining</span>
  <div class="badge_title">Counter-Strike 2&nbsp;<span class="badge_view_details">View details</span></div>
 </div>
 <div class="badge_row is_link">
  <a class="badge_row_overlay" href="https://steamcommunity.com/id/me/gamecards/440/"></a>
  <div class="badge_title_stats_playtime">0.4 hrs on record</div>
  <span class="progress_info_bold">1 card drop remaining</span>
  <div class="badge_title">Team Fortress 2 <span class="badge_view_details">View details</span></div>
 </div>
 <div class="badge_row is_link">
  <a class="badge_row_overlay" href="https://steamcommunity.com/id/me/gamecards/570/"></a>
  <span class="progress_info_bold">No card drops remaining</span>
  <div class="badge_title">Dota 2</div>
 </div>
 <div class="badge_row is_link">
  <a class="badge_row_overlay" href="https://steamcommunity.com/id/me/badges/13"></a>
  <div class="badge_title">Years of Service</div>
 </div>
</div>
<div class="pageLinks"><a class="pagelink" href="?p=2">2</a><a class="pagelink" href="?p=3">3</a></div></html>"#;

    #[test]
    fn foil_badge_rows_do_not_stand_in_for_the_game() {
        let html = r#"<html><script>g_steamID = "76561197960265729";</script>
<div class="badge_row is_link">
 <a class="badge_row_overlay" href="https://steamcommunity.com/id/me/gamecards/730/"></a>
 <span class="progress_info_bold">3 card drops remaining</span>
 <div class="badge_title">Counter-Strike 2</div>
</div>
<div class="badge_row is_link">
 <a class="badge_row_overlay" href="https://steamcommunity.com/id/me/gamecards/730/?border=1"></a>
 <div class="badge_title">Counter-Strike 2 Foil Badge</div>
</div></html>"#;
        let games = parse_badges_page(html).games;
        assert_eq!(games.len(), 1);
        assert_eq!((games[0].appid, games[0].drops_remaining), (730, 3));
    }

    #[test]
    fn badges_page() {
        let p = parse_badges_page(BADGES);
        assert_eq!(p.logged_in_as.as_deref(), Some("76561197960265729"));
        assert_eq!(p.max_page, 3);
        assert_eq!(p.games.len(), 3);
        assert_eq!(
            p.games[0],
            BadgeGame { appid: 730, name: "Counter-Strike 2".into(), drops_remaining: 3, playtime_hours: 1234.5 }
        );
        assert_eq!(p.games[1].drops_remaining, 1);
        assert_eq!(p.games[1].name, "Team Fortress 2");
        assert_eq!(p.games[2].drops_remaining, 0);
    }

    #[test]
    fn logged_out_page() {
        let p = parse_badges_page(r#"<script>g_steamID = false;</script>"#);
        assert!(p.logged_in_as.is_none());
        assert_eq!(p.max_page, 1);
    }

    #[test]
    fn licenses_and_refund_heuristic() {
        let html = r#"<table class="account_table">
          <tr><th class="license_date_col">Date</th><th>Item</th><th>Acquisition Method</th></tr>
          <tr><td class="license_date_col">15 Nov, 2026</td><td>
             <div class="free_license_remove_link"><a href="javascript:RemoveFreeLicense( 1, 'x' );">Remove</a></div>
             Hades II</td><td class="license_acquisition_col">Steam Store</td></tr>
          <tr><td class="license_date_col">Mar 3, 2020</td><td>Portal 2</td><td>Steam Store</td></tr>
        </table>"#;
        let l = parse_licenses_page(html);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "Hades II");
        assert_eq!(l[0].date, NaiveDate::from_ymd_opt(2026, 11, 15).unwrap());
        assert_eq!(l[1].date, NaiveDate::from_ymd_opt(2020, 3, 3).unwrap());
        let today = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert!(likely_refundable("Hades II", 0.5, &l, today));
        assert!(!likely_refundable("Hades II", 2.5, &l, today));
        assert!(!likely_refundable("Hades", 0.5, &l, today));
        assert!(!likely_refundable("Portal 2", 0.1, &l, today));
        assert!(same_game("hades standard edition", "hades"));
        assert!(same_game("hades", "hades ii") == false);
        assert!(same_game("elden ring", "elden ring deluxe edition"));
    }

    #[test]
    fn unlock_times() {
        assert_eq!(
            parse_unlock_time("Unlocked 5 Mar, 2024 @ 10:41pm", 2026).unwrap().to_string(),
            "2024-03-05 22:41:00"
        );
        assert_eq!(
            parse_unlock_time("Unlocked Mar 5, 2024 @ 9:05am", 2026).unwrap().to_string(),
            "2024-03-05 09:05:00"
        );
        assert_eq!(
            parse_unlock_time("Unlocked 7 Jan @ 12:00am", 2026).unwrap().to_string(),
            "2026-01-07 00:00:00"
        );
        let html = r#"<div class="achieveRow"><div class="achieveUnlockTime">Unlocked 5 Mar, 2024 @ 10:41pm</div><div class="achieveTxt"><h3>Second</h3></div></div>
          <div class="achieveRow"><div class="achieveUnlockTime">Unlocked 5 Mar, 2024 @ 10:11pm</div><div class="achieveTxt"><h3>First</h3></div></div>
          <div class="achieveRow"><div class="achieveTxt"><h3>Locked one</h3></div></div>"#;
        let list = parse_profile_achievements(html, 2026);
        assert_eq!(list, vec![("First".to_string(), 0), ("Second".to_string(), 1800)]);
    }

    #[test]
    fn games_page_and_cookie() {
        let html = r#"<template id="gameslist_config" data-profile-gameslist="{&quot;rgGames&quot;:[{&quot;appid&quot;:730,&quot;name&quot;:&quot;Counter-Strike 2&quot;,&quot;playtime_forever&quot;:120}]}"></template>"#;
        let g = parse_games_page(html).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].appid, 730);
        assert_eq!(g[0].playtime_minutes, 120);

        let (sid, enc, tok) = parse_login_secure("steamLoginSecure=76561197960265729%7C%7CeyJ.abc.def; path=/").unwrap();
        assert_eq!(sid, "76561197960265729");
        assert_eq!(tok, "eyJ.abc.def");
        assert_eq!(enc, "76561197960265729%7C%7CeyJ.abc.def");
        assert!(parse_login_secure("garbage").is_err());
        assert_eq!(
            profile_base_url("https://steamcommunity.com/id/gaben/").unwrap(),
            "https://steamcommunity.com/id/gaben"
        );
        assert_eq!(
            profile_base_url("76561197960287930").unwrap(),
            "https://steamcommunity.com/profiles/76561197960287930"
        );
    }
}
