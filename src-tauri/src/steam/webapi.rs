//! Calls to api.steampowered.com: protobuf-encoded service methods (used for the
//! QR sign-in and token refresh) and plain JSON endpoints.
use crate::net::Logged;
use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use protobuf::Message;
use serde_json::Value;
use steam_vent_proto_steam::steammessages_auth_steamclient::{
    CAuthentication_BeginAuthSessionViaQR_Request, CAuthentication_BeginAuthSessionViaQR_Response,
    CAuthentication_DeviceDetails, CAuthentication_PollAuthSessionStatus_Request,
    CAuthentication_PollAuthSessionStatus_Response, EAuthTokenPlatformType,
};

pub const API: &str = "https://api.steampowered.com";
/// Steam's own client user agent; Steam ties token audiences to the platform.
pub const UA_STEAM_CLIENT: &str = "Mozilla/5.0 (Windows; U; Windows NT 10.0; en-US; Valve Steam Client/default/1665786434; ) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0.4183.121 Safari/537.36";
pub const UA_BROWSER: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

#[derive(Debug)]
pub struct SteamApiError {
    pub eresult: i32,
    pub message: String,
}

impl std::fmt::Display for SteamApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.message.is_empty() {
            write!(f, "Steam returned error {} ({})", self.eresult, eresult_name(self.eresult))
        } else {
            write!(f, "{} (Steam error {})", self.message, self.eresult)
        }
    }
}

impl std::error::Error for SteamApiError {}

pub fn eresult_name(e: i32) -> &'static str {
    match e {
        1 => "OK",
        2 => "Fail",
        5 => "InvalidPassword",
        6 => "LoggedInElsewhere",
        8 => "InvalidParam",
        9 => "FileNotFound",
        10 => "Busy",
        15 => "AccessDenied",
        16 => "Timeout",
        20 => "ServiceUnavailable",
        25 => "LimitExceeded",
        27 => "Expired",
        29 => "DuplicateRequest",
        34 => "LogonSessionReplaced",
        42 => "NoMatch",
        65 => "InvalidLoginAuthCode",
        84 => "RateLimitExceeded",
        85 => "AccountLoginDeniedNeedTwoFactor",
        88 => "TwoFactorCodeMismatch",
        _ => "Unknown",
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(UA_BROWSER)
        // A connection that can't be opened fails in seconds, not at the end of the 30.
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("http client")
}

/// Call a protobuf service method over HTTPS (`input_protobuf_encoded`).
/// Returns the raw body and whether Steam answered with JSON instead of protobuf.
async fn service_raw<Req: Message>(
    http: &reqwest::Client,
    interface: &str,
    method: &str,
    version: u32,
    get: bool,
    req: &Req,
) -> Result<(Vec<u8>, bool)> {
    let url = format!("{API}/I{interface}Service/{method}/v{version}/");
    let encoded = STANDARD.encode(req.write_to_bytes()?);
    let rb = if get {
        http.get(&url).query(&[("input_protobuf_encoded", encoded)])
    } else {
        http.post(&url)
            .multipart(reqwest::multipart::Form::new().text("input_protobuf_encoded", encoded))
    };
    let resp = rb
        .header(reqwest::header::USER_AGENT, UA_STEAM_CLIENT)
        .header("Origin", "https://steamloopback.host")
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .logged()
        .await
        .with_context(|| format!("{interface}.{method}: network error"))?;
    let status = resp.status();
    let eresult = resp
        .headers()
        .get("x-eresult")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i32>().ok());
    let message = resp
        .headers()
        .get("x-error_message")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let is_json = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|c| c.contains("json"))
        .unwrap_or(false);
    let body = resp.bytes().await?.to_vec();
    if let Some(e) = eresult {
        if e != 1 {
            return Err(SteamApiError { eresult: e, message }.into());
        }
    }
    if !status.is_success() {
        bail!("{interface}.{method}: HTTP {status}");
    }
    Ok((body, is_json))
}

fn json_response(body: &[u8]) -> Result<Value> {
    let v: Value = serde_json::from_slice(body).context("unreadable JSON from Steam")?;
    Ok(v.get("response").cloned().unwrap_or(v))
}

fn jstr(v: &Value, k: &str) -> Option<String> {
    match v.get(k)? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn ju64(v: &Value, k: &str) -> Option<u64> {
    match v.get(k)? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

pub fn device_details() -> CAuthentication_DeviceDetails {
    let mut d = CAuthentication_DeviceDetails::new();
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "PC".into());
    d.set_device_friendly_name(format!("{host} (IdleKit)"));
    d.set_platform_type(EAuthTokenPlatformType::k_EAuthTokenPlatformType_SteamClient);
    d.set_os_type(20); // Windows 11 — Steam only uses it for display
    d.set_gaming_device_type(1);
    d
}

pub struct QrStart {
    pub client_id: u64,
    pub request_id: Vec<u8>,
    pub challenge_url: String,
    pub interval: f32,
}

pub async fn begin_qr(http: &reqwest::Client) -> Result<QrStart> {
    let mut req = CAuthentication_BeginAuthSessionViaQR_Request::new();
    let details = device_details();
    req.set_device_friendly_name(details.device_friendly_name().to_string());
    req.set_platform_type(EAuthTokenPlatformType::k_EAuthTokenPlatformType_SteamClient);
    req.device_details = protobuf::MessageField::some(details);
    req.set_website_id("Client".into());
    let (body, is_json) = service_raw(http, "Authentication", "BeginAuthSessionViaQR", 1, false, &req).await?;
    if is_json {
        let v = json_response(&body)?;
        return Ok(QrStart {
            client_id: ju64(&v, "client_id").ok_or_else(|| anyhow!("no client_id"))?,
            request_id: jstr(&v, "request_id")
                .and_then(|s| STANDARD.decode(s).ok())
                .ok_or_else(|| anyhow!("no request_id"))?,
            challenge_url: jstr(&v, "challenge_url").ok_or_else(|| anyhow!("no challenge_url"))?,
            interval: v.get("interval").and_then(Value::as_f64).unwrap_or(5.0) as f32,
        });
    }
    let r = CAuthentication_BeginAuthSessionViaQR_Response::parse_from_bytes(&body)?;
    if r.challenge_url().is_empty() {
        bail!("Steam did not return a QR challenge");
    }
    Ok(QrStart {
        client_id: r.client_id(),
        request_id: r.request_id().to_vec(),
        challenge_url: r.challenge_url().to_string(),
        interval: if r.interval() > 0.0 { r.interval() } else { 5.0 },
    })
}

#[derive(Default, Debug)]
pub struct PollResult {
    pub new_client_id: Option<u64>,
    pub new_challenge_url: Option<String>,
    pub refresh_token: Option<String>,
    pub access_token: Option<String>,
    pub had_remote_interaction: bool,
    pub account_name: Option<String>,
    pub new_guard_data: Option<String>,
}

pub async fn poll_auth(http: &reqwest::Client, client_id: u64, request_id: &[u8]) -> Result<PollResult> {
    let mut req = CAuthentication_PollAuthSessionStatus_Request::new();
    req.set_client_id(client_id);
    req.set_request_id(request_id.to_vec());
    let (body, is_json) = service_raw(http, "Authentication", "PollAuthSessionStatus", 1, false, &req).await?;
    if is_json {
        let v = json_response(&body)?;
        return Ok(PollResult {
            new_client_id: ju64(&v, "new_client_id"),
            new_challenge_url: jstr(&v, "new_challenge_url"),
            refresh_token: jstr(&v, "refresh_token"),
            access_token: jstr(&v, "access_token"),
            had_remote_interaction: v.get("had_remote_interaction").and_then(Value::as_bool).unwrap_or(false),
            account_name: jstr(&v, "account_name"),
            new_guard_data: jstr(&v, "new_guard_data"),
        });
    }
    let r = CAuthentication_PollAuthSessionStatus_Response::parse_from_bytes(&body)?;
    let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
    Ok(PollResult {
        new_client_id: r.has_new_client_id().then(|| r.new_client_id()),
        new_challenge_url: opt(r.new_challenge_url()),
        refresh_token: opt(r.refresh_token()),
        access_token: opt(r.access_token()),
        had_remote_interaction: r.had_remote_interaction(),
        account_name: opt(r.account_name()),
        new_guard_data: opt(r.new_guard_data()),
    })
}

// A new access token is never asked for here, over HTTPS: for a Steam-client
// sign-in Steam answers AccountLogonDenied (63), then AccessDenied (15), and
// the saved sign-in stops working everywhere. See `cm::generate_web_token`.

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct OwnedGame {
    pub appid: u32,
    pub name: String,
    pub playtime_minutes: u32,
    pub icon_hash: String,
    pub has_stats: bool,
    pub last_played: u32,
}

/// IPlayerService/GetOwnedGames with an access token (no API key needed).
pub async fn owned_games_web(http: &reqwest::Client, access_token: &str, steam_id: &str) -> Result<Vec<OwnedGame>> {
    let resp = http
        .get(format!("{API}/IPlayerService/GetOwnedGames/v1/"))
        .query(&[
            ("access_token", access_token),
            ("steamid", steam_id),
            ("include_appinfo", "1"),
            ("include_played_free_games", "1"),
            ("skip_unvetted_apps", "0"),
        ])
        .logged()
        .await?;
    if !resp.status().is_success() {
        bail!("GetOwnedGames: HTTP {}", resp.status());
    }
    let v: Value = resp.json().await?;
    let games = v
        .pointer("/response/games")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(games
        .iter()
        .filter_map(|g| {
            Some(OwnedGame {
                appid: g.get("appid")?.as_u64()? as u32,
                name: g.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
                playtime_minutes: g.get("playtime_forever").and_then(Value::as_u64).unwrap_or(0) as u32,
                icon_hash: g.get("img_icon_url").and_then(Value::as_str).unwrap_or("").to_string(),
                has_stats: g.get("has_community_visible_stats").and_then(Value::as_bool).unwrap_or(false),
                last_played: g.get("rtime_last_played").and_then(Value::as_u64).unwrap_or(0) as u32,
            })
        })
        .collect())
}

/// Global unlock percentages per achievement API name (public endpoint).
pub async fn global_achievement_percentages(
    http: &reqwest::Client,
    appid: u32,
) -> Result<std::collections::HashMap<String, f64>> {
    let resp = http
        .get(format!("{API}/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/"))
        .query(&[("gameid", appid.to_string())])
        .logged()
        .await?;
    if !resp.status().is_success() {
        bail!("GetGlobalAchievementPercentagesForApp: HTTP {}", resp.status());
    }
    let v: Value = resp.json().await?;
    let mut out = std::collections::HashMap::new();
    if let Some(list) = v.pointer("/achievementpercentages/achievements").and_then(Value::as_array) {
        for a in list {
            let Some(name) = a.get("name").and_then(Value::as_str) else { continue };
            let pct = match a.get("percent") {
                Some(Value::Number(n)) => n.as_f64(),
                Some(Value::String(s)) => s.parse().ok(),
                _ => None,
            };
            if let Some(p) = pct {
                out.insert(name.to_string(), p);
            }
        }
    }
    Ok(out)
}
