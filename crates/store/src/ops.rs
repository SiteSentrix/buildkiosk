//! Operations: offline backups and integrity checks of a data directory.

use std::fs;
use std::path::Path;

use crate::{DataDir, Result, StoreError, fsutil};

/// What `check` found in one company.
pub struct CompanyReport {
    pub slug: String,
    pub journal_entries: u64,
    pub rows: Vec<(&'static str, usize)>,
}

impl DataDir {
    /// Replays every company's journal from its snapshot and reports what loaded. A company that
    /// fails to load comes back as an error naming it.
    pub fn check(&self) -> Result<Vec<CompanyReport>> {
        let mut out = Vec::new();
        for slug in self.companies()? {
            let t = self.load(&slug).map_err(|e| StoreError::Invalid(format!("company `{slug}`: {e}")))?;
            let rows = t.engine.state().row_counts().into_iter().filter(|(_, n)| *n > 0).collect();
            out.push(CompanyReport { slug, journal_entries: t.store.len(), rows });
        }
        Ok(out)
    }

    /// Copies the instance file and every company (journal, snapshot, blobs, credentials) to `dest`,
    /// which must be missing or empty, then opens the copy and replays it to prove it loads. The
    /// data directory is locked while this runs, so no server is writing.
    pub fn backup_to(&self, dest: &Path) -> Result<Vec<CompanyReport>> {
        if dest.exists() && fs::read_dir(dest).map_err(|e| StoreError::io(dest, e))?.next().is_some() {
            return Err(StoreError::Invalid(format!("{} is not empty; back up to a new folder", dest.display())));
        }
        if dest.starts_with(self.root()) {
            return Err(StoreError::Invalid("the backup folder cannot be inside the data directory".into()));
        }
        fs::create_dir_all(dest).map_err(|e| StoreError::io(dest, e))?;
        let instance = self.root().join("instance.json");
        fs::copy(&instance, dest.join("instance.json")).map_err(|e| StoreError::io(&instance, e))?;
        copy_dir(&self.root().join("companies"), &dest.join("companies"))?;
        fsutil::sync_dir(dest);
        let copy = DataDir::open(dest)?;
        copy.check()
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| StoreError::io(to, e))?;
    for entry in fs::read_dir(from).map_err(|e| StoreError::io(from, e))? {
        let entry = entry.map_err(|e| StoreError::io(from, e))?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| StoreError::io(&entry.path(), e))?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            let name = entry.file_name();
            // Leftover temporary files from an interrupted atomic write are not data.
            if name.to_str().is_some_and(|n| n.ends_with(".tmp")) {
                continue;
            }
            fs::copy(entry.path(), &target).map_err(|e| StoreError::io(&entry.path(), e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("bk-ops-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn backup_copies_and_loads() {
        let src = temp("src");
        let dst = temp("dst");
        {
            let data = DataDir::open(&src).unwrap();
            let mut t = data.create("acme").unwrap();
            let p =
                serde_json::json!({"name": "Acme", "slug": "acme", "admin_first_name": "A", "admin_last_name": "B", "admin_email": "a@example.com"});
            t.engine
                .execute(&mut t.store, buildkiosk_engine::Actor::System, buildkiosk_core::Timestamp::from_millis(1), "company.setup", &p)
                .unwrap();
            drop(t);
            let report = data.backup_to(&dst).unwrap();
            assert_eq!(report.len(), 1);
            assert!(report[0].journal_entries >= 1);
            assert!(data.backup_to(&dst).is_err(), "destination must be empty");
            assert!(data.backup_to(&src.join("inner")).is_err(), "not inside the data directory");
        }
        let copy = DataDir::open(&dst).unwrap();
        assert_eq!(copy.check().unwrap().len(), 1);
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }
}
