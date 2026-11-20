//! Minimal JWT payload decoding for Steam tokens (no signature check needed:
//! we only read our own tokens' subject and expiry).
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub exp: i64,
    pub aud: Vec<String>,
}

pub fn decode(token: &str) -> Option<Claims> {
    let part = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(part.trim_end_matches('=')).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let aud = match v.get("aud") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => vec![],
    };
    Some(Claims {
        iss: v.get("iss").and_then(Value::as_str).unwrap_or("").to_string(),
        sub: v
            .get("sub")
            .map(|s| match s {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        exp: v.get("exp").and_then(Value::as_i64).unwrap_or(0),
        aud,
    })
}

/// True when the token expires within `margin_secs` (or can't be read).
pub fn expires_soon(token: &str, margin_secs: i64) -> bool {
    match decode(token) {
        Some(c) if c.exp > 0 => c.exp - chrono::Utc::now().timestamp() < margin_secs,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(payload: &str) -> String {
        format!("eyJhbGciOiJFZERTQSJ9.{}.sig", URL_SAFE_NO_PAD.encode(payload))
    }

    #[test]
    fn decodes_steam_style_token() {
        let t = make(r#"{"iss":"steam","sub":"76561197960265729","aud":["client","web"],"exp":4102444800}"#);
        let c = decode(&t).unwrap();
        assert_eq!(c.iss, "steam");
        assert_eq!(c.sub, "76561197960265729");
        assert_eq!(c.aud, vec!["client", "web"]);
        assert!(!expires_soon(&t, 3600));
    }

    #[test]
    fn expired_and_garbage() {
        let t = make(r#"{"iss":"r:abc","sub":"1","aud":"web","exp":1000}"#);
        assert!(expires_soon(&t, 0));
        assert_eq!(decode(&t).unwrap().aud, vec!["web"]);
        assert!(decode("not-a-jwt").is_none());
        assert!(expires_soon("x.y.z", 0));
    }
}
