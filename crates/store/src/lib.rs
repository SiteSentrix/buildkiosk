//! BuildKiosk's storage on disk (layer 6).
//!
//! A data directory holds every company (tenant) separately:
//!
//! ```text
//! data/
//!   .lock                     held exclusively by the process using the directory
//!   instance.json             schema version and the instance secret (signs session cookies)
//!   companies/<slug>/
//!     journal.jsonl           one JournalEntry per line, append-only, fsynced per command
//!     snapshot.json           the state after some entry, written atomically
//!     credentials.json        password hashes, session epochs and API token hashes (never journaled)
//!     blobs/ab/abcdef…        uploaded files, named by the SHA-256 of their content
//! ```
//!
//! Loading a company reads the snapshot (if any) and replays the journal entries after it. A torn
//! last line (a crash mid-write) is cut off with a warning; damage anywhere else refuses to load, so
//! nothing ever runs on a partial state.
#![forbid(unsafe_code)]

pub mod blobs;
pub mod creds;
mod fsutil;
pub mod ops;
pub mod tenant;

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use buildkiosk_core::text;
use serde::{Deserialize, Serialize};

pub use blobs::{BlobStore, BlobWriter};
pub use creds::{Credentials, hash_password, verify_password};
pub use fsutil::{hex, random_bytes};
pub use ops::CompanyReport;
pub use tenant::{Tenant, TenantStore};

/// Why the store failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("the data directory {0} is in use by another BuildKiosk process")]
    Locked(String),
    #[error("{path}: {msg}")]
    Corrupt { path: String, msg: String },
    #[error("{0}")]
    Invalid(String),
}

impl StoreError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> StoreError {
        StoreError::Io { path: path.display().to_string(), source }
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// Settings of this installation.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Instance {
    schema: u32,
    /// Hex of 32 random bytes; signs session cookies and CSRF tokens.
    secret: String,
}

/// An open data directory. Only one process may hold it at a time.
#[derive(Debug)]
pub struct DataDir {
    root: PathBuf,
    secret: [u8; 32],
    _lock: File,
}

impl DataDir {
    /// Opens (creating if needed) the data directory and takes its lock.
    pub fn open(root: impl AsRef<Path>) -> Result<DataDir> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("companies")).map_err(|e| StoreError::io(&root, e))?;
        let lock_path = root.join(".lock");
        let lock = OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path).map_err(|e| StoreError::io(&lock_path, e))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(StoreError::Locked(root.display().to_string())),
            Err(fs::TryLockError::Error(e)) => return Err(StoreError::io(&lock_path, e)),
        }
        let secret = load_or_create_instance(&root)?;
        Ok(DataDir { root, secret, _lock: lock })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The instance secret (session cookies and CSRF tokens are signed with it).
    pub fn secret(&self) -> &[u8; 32] {
        &self.secret
    }

    /// Slugs of the companies in this directory, sorted.
    pub fn companies(&self) -> Result<Vec<String>> {
        let dir = self.root.join("companies");
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| StoreError::io(&dir, e))? {
            let entry = entry.map_err(|e| StoreError::io(&dir, e))?;
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
                && valid_slug(name)
            {
                out.push(name.to_string());
            }
        }
        out.sort();
        Ok(out)
    }

    fn company_dir(&self, slug: &str) -> Result<PathBuf> {
        if !valid_slug(slug) {
            return Err(StoreError::Invalid(format!("`{slug}` is not a valid company id (lowercase letters, digits and hyphens)")));
        }
        Ok(self.root.join("companies").join(slug))
    }

    /// Loads a company: snapshot plus journal replay.
    pub fn load(&self, slug: &str) -> Result<Tenant> {
        let dir = self.company_dir(slug)?;
        if !dir.is_dir() {
            return Err(StoreError::Invalid(format!("no company `{slug}` in {}", self.root.display())));
        }
        tenant::load(slug, &dir)
    }

    /// Creates the folders of a new, empty company. Fails if it exists.
    pub fn create(&self, slug: &str) -> Result<Tenant> {
        let dir = self.company_dir(slug)?;
        if dir.exists() {
            return Err(StoreError::Invalid(format!("a company `{slug}` already exists")));
        }
        fs::create_dir_all(dir.join("blobs")).map_err(|e| StoreError::io(&dir, e))?;
        tenant::load(slug, &dir)
    }
}

/// Company ids: 1–63 lowercase ASCII letters, digits and inner hyphens (what `text::slug` makes).
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && text::slug(s, 63) == s
}

fn load_or_create_instance(root: &Path) -> Result<[u8; 32]> {
    let path = root.join("instance.json");
    if path.exists() {
        let text = fs::read_to_string(&path).map_err(|e| StoreError::io(&path, e))?;
        let inst: Instance = serde_json::from_str(&text).map_err(|e| StoreError::Corrupt { path: path.display().to_string(), msg: e.to_string() })?;
        if inst.schema > buildkiosk_model::SCHEMA {
            return Err(StoreError::Invalid(format!(
                "{} was written by a newer BuildKiosk (schema {}); upgrade this build",
                path.display(),
                inst.schema
            )));
        }
        return fsutil::unhex32(&inst.secret).ok_or_else(|| StoreError::Corrupt { path: path.display().to_string(), msg: "bad secret".into() });
    }
    let secret = fsutil::random_32()?;
    let inst = Instance { schema: buildkiosk_model::SCHEMA, secret: hex(&secret) };
    let text = serde_json::to_string_pretty(&inst).map_err(|e| StoreError::Invalid(e.to_string()))?;
    fsutil::write_atomic(&path, text.as_bytes())?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("buildkiosk-store-{name}-{}-{}", std::process::id(), hex(&random_bytes::<6>().unwrap())));
        p
    }

    #[test]
    fn slugs() {
        assert!(valid_slug("summit-ridge"));
        for bad in ["", "Summit", "a/b", "..", "-x", "x-", "a b", "é"] {
            assert!(!valid_slug(bad), "{bad:?}");
        }
    }

    #[test]
    fn lock_and_secret() {
        let root = temp_dir("lock");
        let a = DataDir::open(&root).unwrap();
        assert!(matches!(DataDir::open(&root), Err(StoreError::Locked(_))));
        let secret = *a.secret();
        drop(a);
        let b = DataDir::open(&root).unwrap();
        assert_eq!(*b.secret(), secret, "the secret persists");
        assert!(b.companies().unwrap().is_empty());
        b.create("acme").unwrap();
        assert!(b.create("acme").is_err());
        assert_eq!(b.companies().unwrap(), vec!["acme".to_string()]);
        assert!(b.load("../etc").is_err());
        drop(b);
        let _ = fs::remove_dir_all(&root);
    }
}
