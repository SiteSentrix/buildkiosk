//! Credentials: password hashes, session epochs and API tokens.
//!
//! They live in `credentials.json` next to the journal, never in it: the journal is history that
//! can be exported and replayed, and secrets don't belong there. Passwords are hashed with
//! argon2id; API tokens are stored as SHA-256 hashes of their secret part.

use std::collections::BTreeMap;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use buildkiosk_core::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Result, StoreError, fsutil};

/// The shortest password accepted.
pub const MIN_PASSWORD: usize = 10;
/// The longest password accepted (bounds the hashing work an attacker can cause).
pub const MAX_PASSWORD: usize = 256;

/// One person's sign-in secret.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonCred {
    /// argon2id PHC string.
    pub hash: String,
    /// Bumped to sign the person out everywhere (sessions carry the epoch they were issued in).
    #[serde(default)]
    pub epoch: u64,
    pub changed: Timestamp,
}

/// An API token: `bk_<id>_<secret>`; only the hash of the secret is kept.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub person: u64,
    pub name: String,
    pub hash: String,
    pub created: Timestamp,
}

/// A company's credentials.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub people: BTreeMap<u64, PersonCred>,
    #[serde(default)]
    pub tokens: Vec<ApiToken>,
}

/// Why a password was refused.
pub fn check_password_rules(password: &str) -> std::result::Result<(), String> {
    let n = password.chars().count();
    if n < MIN_PASSWORD {
        return Err(format!("the password must have at least {MIN_PASSWORD} characters"));
    }
    if n > MAX_PASSWORD {
        return Err(format!("the password must have at most {MAX_PASSWORD} characters"));
    }
    if password.trim().is_empty() || password.chars().all(|c| Some(c) == password.chars().next()) {
        return Err("choose a password that isn't a single repeated character".into());
    }
    Ok(())
}

fn argon2() -> Argon2<'static> {
    // OWASP-recommended argon2id settings: 19 MiB, 2 iterations, 1 lane.
    let params = Params::new(19 * 1024, 2, 1, None).unwrap_or_default();
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// An argon2id hash of `password` with a random salt.
pub fn hash_password(password: &str) -> Result<String> {
    check_password_rules(password).map_err(StoreError::Invalid)?;
    let salt_bytes = fsutil::random_bytes::<16>()?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| StoreError::Invalid(format!("salt: {e}")))?;
    argon2().hash_password(password.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| StoreError::Invalid(format!("hashing: {e}")))
}

/// Whether `password` matches `hash` (constant-time comparison inside argon2).
pub fn verify_password(hash: &str, password: &str) -> bool {
    if password.chars().count() > MAX_PASSWORD {
        return false;
    }
    PasswordHash::new(hash).is_ok_and(|parsed| argon2().verify_password(password.as_bytes(), &parsed).is_ok())
}

/// A hash to verify against when the account doesn't exist, so a wrong email takes as long as a
/// wrong password.
pub fn dummy_verify(password: &str) {
    const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$J8bHuKZSFtqhlXqKMZ71UjE0/3B4lV4jQm0lZoG6nS8";
    let _ = verify_password(DUMMY, password);
}

fn sha256_hex(s: &str) -> String {
    fsutil::hex(&Sha256::digest(s.as_bytes()))
}

/// Compares two strings in constant time (for token hashes).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Credentials {
    /// Sets a person's password (and signs them out everywhere).
    pub fn set_password(&mut self, person: u64, password: &str, at: Timestamp) -> Result<()> {
        let hash = hash_password(password)?;
        let epoch = self.people.get(&person).map(|c| c.epoch.saturating_add(1)).unwrap_or(0);
        self.people.insert(person, PersonCred { hash, epoch, changed: at });
        Ok(())
    }

    pub fn has_password(&self, person: u64) -> bool {
        self.people.contains_key(&person)
    }

    pub fn check_password(&self, person: u64, password: &str) -> bool {
        match self.people.get(&person) {
            Some(c) => verify_password(&c.hash, password),
            None => {
                dummy_verify(password);
                false
            }
        }
    }

    pub fn epoch(&self, person: u64) -> u64 {
        self.people.get(&person).map(|c| c.epoch).unwrap_or(0)
    }

    /// Signs a person out of every session.
    pub fn bump_epoch(&mut self, person: u64) {
        if let Some(c) = self.people.get_mut(&person) {
            c.epoch = c.epoch.saturating_add(1);
        }
    }

    /// Creates an API token for `person`; returns the full token, shown once.
    pub fn create_token(&mut self, person: u64, name: &str, at: Timestamp) -> Result<String> {
        if self.tokens.iter().filter(|t| t.person == person).count() >= 50 {
            return Err(StoreError::Invalid("a person can have at most 50 API tokens".into()));
        }
        let id = fsutil::hex(&fsutil::random_bytes::<6>()?);
        let secret = fsutil::hex(&fsutil::random_bytes::<32>()?);
        self.tokens.push(ApiToken { id: id.clone(), person, name: name.chars().take(100).collect(), hash: sha256_hex(&secret), created: at });
        Ok(format!("bk_{id}_{secret}"))
    }

    /// The person a token belongs to.
    pub fn token_person(&self, token: &str) -> Option<u64> {
        let rest = token.strip_prefix("bk_")?;
        let (id, secret) = rest.split_once('_')?;
        let t = self.tokens.iter().find(|t| t.id == id)?;
        same(&t.hash, &sha256_hex(secret)).then_some(t.person)
    }

    pub fn revoke_token(&mut self, id: &str) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|t| t.id != id);
        self.tokens.len() != before
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn passwords() {
        assert!(hash_password("short").is_err());
        assert!(hash_password("aaaaaaaaaaaa").is_err());
        let h = hash_password("a long enough password").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password(&h, "a long enough password"));
        assert!(!verify_password(&h, "a long enough passworD"));
        assert!(!verify_password("not a hash", "x"));
        let mut c = Credentials::default();
        c.set_password(3, "a long enough password", Timestamp::from_millis(0)).unwrap();
        assert_eq!(c.epoch(3), 0);
        c.set_password(3, "another long password", Timestamp::from_millis(0)).unwrap();
        assert_eq!(c.epoch(3), 1, "changing the password signs out other sessions");
        assert!(!c.check_password(4, "anything at all"));
    }

    #[test]
    fn the_dummy_hash_is_a_real_hash() {
        // Otherwise a missing account would answer faster than a wrong password.
        assert!(PasswordHash::new("$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$J8bHuKZSFtqhlXqKMZ71UjE0/3B4lV4jQm0lZoG6nS8").is_ok());
    }

    #[test]
    fn tokens() {
        let mut c = Credentials::default();
        let t = c.create_token(7, "ci", Timestamp::from_millis(0)).unwrap();
        assert_eq!(c.token_person(&t), Some(7));
        assert_eq!(c.token_person(&format!("{t}x")), None);
        assert_eq!(c.token_person("bk_nope_nope"), None);
        let id = c.tokens[0].id.clone();
        assert!(c.revoke_token(&id));
        assert_eq!(c.token_person(&t), None);
    }
}
