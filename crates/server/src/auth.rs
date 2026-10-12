//! Signed session cookies, CSRF tokens and the sign-in rate limit.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const SESSION_COOKIE: &str = "bk_session";
pub const CSRF_COOKIE: &str = "bk_csrf";
/// Sessions last this long.
pub const SESSION_MS: i64 = 14 * 24 * 3600 * 1000;

type HmacSha256 = Hmac<Sha256>;

fn mac(secret: &[u8], data: &str) -> Option<HmacSha256> {
    let mut m = HmacSha256::new_from_slice(secret).ok()?;
    m.update(data.as_bytes());
    Some(m)
}

fn sign(secret: &[u8], data: &str) -> String {
    mac(secret, data).map(|m| B64.encode(m.finalize().into_bytes())).unwrap_or_default()
}

fn verify(secret: &[u8], data: &str, sig: &str) -> bool {
    let Ok(raw) = B64.decode(sig) else { return false };
    mac(secret, data).is_some_and(|m| m.verify_slice(&raw).is_ok())
}

/// What a valid session cookie says.
pub struct SessionData {
    pub person: u64,
    pub epoch: u64,
}

/// The cookie value for a session issued at `now_ms`.
pub fn issue_session(secret: &[u8], person: u64, epoch: u64, now_ms: i64) -> String {
    let payload = format!("{person}:{epoch}:{}", now_ms.saturating_add(SESSION_MS));
    format!("{}.{}", B64.encode(payload.as_bytes()), sign(secret, &payload))
}

/// Checks a session cookie's signature and expiry.
pub fn read_session(secret: &[u8], cookie: &str, now_ms: i64) -> Option<SessionData> {
    let (body, sig) = cookie.split_once('.')?;
    let payload = String::from_utf8(B64.decode(body).ok()?).ok()?;
    if !verify(secret, &payload, sig) {
        return None;
    }
    let mut parts = payload.split(':');
    let person = parts.next()?.parse().ok()?;
    let epoch = parts.next()?.parse().ok()?;
    let expires: i64 = parts.next()?.parse().ok()?;
    (expires > now_ms).then_some(SessionData { person, epoch })
}

/// The CSRF token for a form shown to the holder of `cookie` (the session cookie, or the anonymous
/// CSRF cookie before sign-in). A forged request cannot know it: it is bound to a secret cookie
/// value and the instance secret.
pub fn csrf_token(secret: &[u8], cookie: &str) -> String {
    sign(secret, &format!("csrf:{cookie}"))
}

pub fn csrf_ok(secret: &[u8], cookie: &str, token: &str) -> bool {
    !cookie.is_empty() && verify(secret, &format!("csrf:{cookie}"), token)
}

/// The value of cookie `name` in a `Cookie` header.
pub fn cookie_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v)
}

pub fn set_cookie(name: &str, value: &str, max_age_s: i64) -> String {
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_s}")
}

/// Limits sign-in attempts: `MAX` per key within `WINDOW`.
#[derive(Default)]
pub struct RateLimit {
    tries: HashMap<String, (u32, Instant)>,
}

impl RateLimit {
    const MAX: u32 = 8;
    const WINDOW: Duration = Duration::from_secs(300);

    /// Records an attempt; false when the key is over the limit.
    pub fn allow(&mut self, key: &str) -> bool {
        let now = Instant::now();
        if self.tries.len() > 10_000 {
            self.tries.retain(|_, (_, t)| now.duration_since(*t) < Self::WINDOW);
        }
        let entry = self.tries.entry(key.to_string()).or_insert((0, now));
        if now.duration_since(entry.1) >= Self::WINDOW {
            *entry = (0, now);
        }
        entry.0 = entry.0.saturating_add(1);
        entry.0 <= Self::MAX
    }

    /// A successful sign-in clears the key.
    pub fn clear(&mut self, key: &str) {
        self.tries.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trip_and_tamper() {
        let secret = [7u8; 32];
        let c = issue_session(&secret, 5, 2, 1000);
        let s = read_session(&secret, &c, 2000).unwrap();
        assert_eq!((s.person, s.epoch), (5, 2));
        assert!(read_session(&secret, &c, 1000 + SESSION_MS + 1).is_none(), "expired");
        assert!(read_session(&[8u8; 32], &c, 2000).is_none(), "other secret");
        let forged = c.replacen('.', "x.", 1);
        assert!(read_session(&secret, &forged, 2000).is_none());
    }

    #[test]
    fn csrf_binds_to_cookie() {
        let secret = [1u8; 32];
        let t = csrf_token(&secret, "abc");
        assert!(csrf_ok(&secret, "abc", &t));
        assert!(!csrf_ok(&secret, "abd", &t));
        assert!(!csrf_ok(&secret, "", &t));
    }

    #[test]
    fn rate_limit() {
        let mut r = RateLimit::default();
        for _ in 0..8 {
            assert!(r.allow("a"));
        }
        assert!(!r.allow("a"));
        assert!(r.allow("b"));
        r.clear("a");
        assert!(r.allow("a"));
    }
}
