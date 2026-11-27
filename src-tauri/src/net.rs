//! Every web request IdleKit makes goes through [`send`], which writes one
//! line per attempt to the log: what was asked (method, host and path), what
//! came back (status, size) and how long it took, or why nothing came back.
//!
//! The query is never written: it can carry a sign-in (`access_token=`) or a
//! key. Other players' profile names in a path are left out too, and the log
//! scrubs every line again on top (Steam IDs, tokens, addresses).
use crate::logbus;
use std::time::Instant;

/// Sends a request and writes down how it went. Answers come back as they
/// are (an error status is the caller's to judge); a request that got no
/// answer at all is written down at info level, so it shows in the Logs page.
/// An error comes back without the address in it: the line written here
/// names the place, and an address can carry a sign-in in its query.
pub async fn send(req: reqwest::RequestBuilder) -> reqwest::Result<reqwest::Response> {
    let (client, built) = req.build_split();
    let request = match built {
        Ok(r) => r,
        Err(e) => {
            logbus::debug("net", format!("A request couldn't be made: {}", reason(&e)));
            return Err(e.without_url());
        }
    };
    let what = format!("{} {}", request.method(), place(request.url()));
    let started = Instant::now();
    let result = client.execute(request).await;
    let ms = started.elapsed().as_millis();
    match &result {
        Ok(resp) => {
            let size = resp.content_length().map(|n| format!(", {}", size(n))).unwrap_or_default();
            logbus::debug("net", format!("{what} → {}{size} in {ms} ms", resp.status().as_u16()));
        }
        Err(e) => logbus::info("net", format!("{what} → no answer after {ms} ms: {}", reason(e))),
    }
    result.map_err(reqwest::Error::without_url)
}

/// `request.logged()` in place of `request.send()`: the same request,
/// written down.
pub trait Logged {
    fn logged(self) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send;
}

impl Logged for reqwest::RequestBuilder {
    fn logged(self) -> impl std::future::Future<Output = reqwest::Result<reqwest::Response>> + Send {
        send(self)
    }
}

/// Host and path of an address, readable and without anything private: no
/// query, no sign-in or password, and no profile name after `/id/`.
pub fn place(url: &reqwest::Url) -> String {
    let host = url.host_str().unwrap_or("?");
    let path = urlencoding::decode(url.path()).map(|p| p.into_owned()).unwrap_or_else(|_| url.path().to_string());
    let mut parts: Vec<String> = path.split('/').map(str::to_string).collect();
    for i in 1..parts.len() {
        if parts[i - 1] == "id" && !parts[i].is_empty() {
            parts[i] = "…".into();
        }
    }
    let query = if url.query().is_some_and(|q| !q.is_empty()) { "?…" } else { "" };
    format!("{host}{}{query}", parts.join("/"))
}

/// Why no answer came, in a few words, from the innermost cause.
pub fn reason(e: &reqwest::Error) -> String {
    let mut inner: &dyn std::error::Error = e;
    while let Some(next) = inner.source() {
        inner = next;
    }
    let cause = inner.to_string();
    let kind = if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "couldn't connect"
    } else if e.is_body() || e.is_decode() {
        "the answer was cut off"
    } else if e.is_redirect() {
        "too many redirects"
    } else {
        "failed"
    };
    // The outer error's own words name the address; the innermost cause doesn't.
    if cause.is_empty() || cause.contains("://") {
        kind.to_string()
    } else {
        format!("{kind} ({cause})")
    }
}

/// A size in bytes, readable.
fn size(n: u64) -> String {
    match n {
        0..=1023 => format!("{n} B"),
        1024..=1_048_575 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> reqwest::Url {
        s.parse().unwrap()
    }

    #[test]
    fn a_place_never_shows_the_query_or_a_profile_name() {
        assert_eq!(
            place(&url("https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/?access_token=eyJ.secret&steamid=1")),
            "api.steampowered.com/IPlayerService/GetOwnedGames/v1/?…"
        );
        assert_eq!(place(&url("https://steamcommunity.com/id/somebody/stats/440/achievements/")), "steamcommunity.com/id/…/stats/440/achievements/");
        assert_eq!(
            place(&url("https://steamcommunity.com/market/listings/753/730-Anarchist%20(Trading%20Card)")),
            "steamcommunity.com/market/listings/753/730-Anarchist (Trading Card)"
        );
        assert_eq!(place(&url("https://user:pass@example.com/a")), "example.com/a");
        assert_eq!(place(&url("https://example.com/")), "example.com/");
    }

    #[test]
    fn sizes_read_as_words() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(2048), "2.0 KB");
        assert_eq!(size(3 * 1_048_576), "3.0 MB");
    }

    #[tokio::test]
    async fn a_request_with_no_answer_says_why_without_the_address() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        // Nothing listens on port 9 of this machine.
        let client = reqwest::Client::new();
        let e = send(client.get("http://127.0.0.1:9/secret?token=abc")).await.unwrap_err();
        let why = reason(&e);
        assert!(why.starts_with("couldn't connect"), "{why}");
        assert!(!why.contains("token") && !why.contains("secret"), "{why}");
        // Nor does the error itself carry the address any more.
        let all = format!("{:#}", anyhow::Error::new(e));
        assert!(!all.contains("token") && !all.contains("secret") && !all.contains("127.0.0.1"), "{all}");
    }
}
