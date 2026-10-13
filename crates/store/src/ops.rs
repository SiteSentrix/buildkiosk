//! Operations: offline backups and integrity checks of a data directory.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

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
        self.backup_begin(dest)?.finish()
    }

    /// The first half of a backup taken while a server is running. Call it while nothing is writing
    /// (the server holds its own lock): it copies the small files (instance, snapshot, credentials)
    /// and notes how long each journal is. [`OnlineBackup::finish`] then copies the journals up to
    /// those lengths and the blobs without holding anything up. The journal only grows and a blob
    /// is written before the entry that names it, so the copy is whole as of the moment of this
    /// call.
    pub fn backup_begin(&self, dest: &Path) -> Result<OnlineBackup> {
        if dest.exists() && fs::read_dir(dest).map_err(|e| StoreError::io(dest, e))?.next().is_some() {
            return Err(StoreError::Invalid(format!("{} is not empty; back up to a new folder", dest.display())));
        }
        if dest.starts_with(self.root()) {
            return Err(StoreError::Invalid("the backup folder cannot be inside the data directory".into()));
        }
        fs::create_dir_all(dest).map_err(|e| StoreError::io(dest, e))?;
        let instance = self.root().join("instance.json");
        fs::copy(&instance, dest.join("instance.json")).map_err(|e| StoreError::io(&instance, e))?;
        let mut backup = OnlineBackup { dest: dest.to_path_buf(), journals: Vec::new(), blobs: Vec::new() };
        for slug in self.companies()? {
            let from = self.root().join("companies").join(&slug);
            let to = dest.join("companies").join(&slug);
            fs::create_dir_all(&to).map_err(|e| StoreError::io(&to, e))?;
            for entry in fs::read_dir(&from).map_err(|e| StoreError::io(&from, e))? {
                let entry = entry.map_err(|e| StoreError::io(&from, e))?;
                let name = entry.file_name();
                let Some(name_text) = name.to_str() else { continue };
                let kind = entry.file_type().map_err(|e| StoreError::io(&entry.path(), e))?;
                if kind.is_dir() {
                    if name_text == "blobs" {
                        backup.blobs.push((entry.path(), to.join(&name)));
                    }
                } else if kind.is_file() && !name_text.ends_with(".tmp") {
                    if name_text == "journal.jsonl" {
                        let len = entry.metadata().map_err(|e| StoreError::io(&entry.path(), e))?.len();
                        backup.journals.push((entry.path(), to.join(&name), len));
                    } else {
                        fs::copy(entry.path(), to.join(&name)).map_err(|e| StoreError::io(&entry.path(), e))?;
                    }
                }
            }
        }
        Ok(backup)
    }
}

/// A backup whose small files are copied and whose journals and blobs are still to copy.
pub struct OnlineBackup {
    dest: PathBuf,
    /// (journal, copy, length at the moment the backup began)
    journals: Vec<(PathBuf, PathBuf, u64)>,
    /// (blob folder, copy)
    blobs: Vec<(PathBuf, PathBuf)>,
}

impl OnlineBackup {
    /// Copies the journals and blobs, then opens the copy and replays it to prove it loads.
    pub fn finish(self) -> Result<Vec<CompanyReport>> {
        for (from, to, len) in &self.journals {
            let source = fs::File::open(from).map_err(|e| StoreError::io(from, e))?;
            let mut out = fs::File::create(to).map_err(|e| StoreError::io(to, e))?;
            let copied = io::copy(&mut source.take(*len), &mut out).map_err(|e| StoreError::io(from, e))?;
            if copied != *len {
                return Err(StoreError::Invalid(format!("{} is shorter than it was a moment ago; the backup was not kept", from.display())));
            }
            out.sync_all().map_err(|e| StoreError::io(to, e))?;
        }
        for (from, to) in &self.blobs {
            copy_dir(from, to)?;
        }
        fsutil::sync_dir(&self.dest);
        DataDir::open(&self.dest)?.check()
    }
}

/// What `restore` did.
pub struct RestoreReport {
    pub companies: Vec<CompanyReport>,
    /// Where the previous data directory was moved to, when it was replaced.
    pub moved_aside: Option<PathBuf>,
}

/// Puts a backup in place as the data directory `into`. The backup is opened and replayed first, so a
/// damaged backup changes nothing. `into` must be missing or empty; with `replace` an existing data
/// directory (no server may be using it) is moved aside to `<into>.before-restore-<seconds>` rather
/// than deleted. The restored copy is opened and replayed too.
pub fn restore(from: &Path, into: &Path, replace: bool, now_seconds: u64) -> Result<RestoreReport> {
    if !from.join("instance.json").is_file() {
        return Err(StoreError::Invalid(format!("{} is not a BuildKiosk backup (no instance.json)", from.display())));
    }
    if from == into || into.starts_with(from) || from.starts_with(into) {
        return Err(StoreError::Invalid("the backup and the data directory can't be the same folder or inside one another".into()));
    }
    let source = DataDir::open(from)?;
    source.check().map_err(|e| StoreError::Invalid(format!("the backup does not load, so nothing was changed: {e}")))?;
    let mut moved_aside = None;
    if into.exists() && fs::read_dir(into).map_err(|e| StoreError::io(into, e))?.next().is_some() {
        if !replace {
            return Err(StoreError::Invalid(format!("{} is not empty; use --replace to move it aside and restore", into.display())));
        }
        if !into.join("instance.json").is_file() {
            return Err(StoreError::Invalid(format!("{} is not a BuildKiosk data directory, so it won't be moved aside", into.display())));
        }
        // Taking its lock proves no server is running on it.
        drop(DataDir::open(into)?);
        let name = into.file_name().and_then(|n| n.to_str()).unwrap_or("data");
        let aside = into.with_file_name(format!("{name}.before-restore-{now_seconds}"));
        if aside.exists() {
            return Err(StoreError::Invalid(format!("{} already exists", aside.display())));
        }
        fs::rename(into, &aside).map_err(|e| StoreError::io(into, e))?;
        moved_aside = Some(aside);
    }
    fs::create_dir_all(into).map_err(|e| StoreError::io(into, e))?;
    let instance = from.join("instance.json");
    fs::copy(&instance, into.join("instance.json")).map_err(|e| StoreError::io(&instance, e))?;
    copy_dir(&from.join("companies"), &into.join("companies"))?;
    fsutil::sync_dir(into);
    drop(source);
    let companies = DataDir::open(into)?.check()?;
    Ok(RestoreReport { companies, moved_aside })
}

/// Puts one company of a backup back into an existing data directory (no server may be using it), leaving the
/// others alone. The company's current folder is moved aside to `<data>.before-restore-<company>-<seconds>`
/// rather than deleted, and a company that is not in the data directory yet is simply added. The backup is
/// replayed first, and the restored company is replayed after, so a damaged backup changes nothing.
pub fn restore_company(from: &Path, into: &Path, slug: &str, now_seconds: u64) -> Result<CompanyReport> {
    if !crate::valid_slug(slug) {
        return Err(StoreError::Invalid(format!("`{slug}` is not a valid company id (lowercase letters, digits and hyphens)")));
    }
    if !from.join("instance.json").is_file() {
        return Err(StoreError::Invalid(format!("{} is not a BuildKiosk backup (no instance.json)", from.display())));
    }
    if from == into || into.starts_with(from) || from.starts_with(into) {
        return Err(StoreError::Invalid("the backup and the data directory can't be the same folder or inside one another".into()));
    }
    let source = DataDir::open(from)?;
    if !source.companies()?.iter().any(|c| c == slug) {
        return Err(StoreError::Invalid(format!("the backup has no company `{slug}`")));
    }
    source.load(slug).map_err(|e| StoreError::Invalid(format!("company `{slug}` in the backup does not load, so nothing was changed: {e}")))?;
    if !into.join("instance.json").is_file() {
        return Err(StoreError::Invalid(format!("{} is not a BuildKiosk data directory", into.display())));
    }
    // Taking the lock proves no server is running on it.
    let target = DataDir::open(into)?;
    let name = into.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let staged = into.with_file_name(format!("{name}.restoring-{slug}-{now_seconds}"));
    let aside = into.with_file_name(format!("{name}.before-restore-{slug}-{now_seconds}"));
    if staged.exists() || aside.exists() {
        return Err(StoreError::Invalid(format!("{} or {} already exists", staged.display(), aside.display())));
    }
    copy_dir(&from.join("companies").join(slug), &staged)?;
    let live = into.join("companies").join(slug);
    let had_one = live.exists();
    let swap = || -> Result<()> {
        if had_one {
            fs::rename(&live, &aside).map_err(|e| StoreError::io(&live, e))?;
        }
        fs::rename(&staged, &live).map_err(|e| StoreError::io(&staged, e))?;
        Ok(())
    };
    if let Err(e) = swap() {
        // Put things back as they were.
        if had_one && aside.exists() && !live.exists() {
            let _ = fs::rename(&aside, &live);
        }
        let _ = fs::remove_dir_all(&staged);
        return Err(e);
    }
    fsutil::sync_dir(&into.join("companies"));
    let t = target.load(slug).map_err(|e| StoreError::Invalid(format!("company `{slug}` was restored but does not load: {e}")))?;
    let rows = t.engine.state().row_counts().into_iter().filter(|(_, n)| *n > 0).collect();
    Ok(CompanyReport { slug: slug.to_string(), journal_entries: t.store.len(), rows })
}

/// Copies a finished backup folder to another place (a second disk, a mounted share) as it is.
pub fn copy_backup(from: &Path, to: &Path) -> Result<()> {
    if to.starts_with(from) || from.starts_with(to) {
        return Err(StoreError::Invalid("the copy can't be inside the backup or the other way round".into()));
    }
    copy_dir(from, to)
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| StoreError::io(to, e))?;
    for entry in fs::read_dir(from).map_err(|e| StoreError::io(from, e))? {
        let entry = entry.map_err(|e| StoreError::io(from, e))?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| StoreError::io(&entry.path(), e))?;
        if kind.is_dir() {
            // Half-written uploads are not data.
            if entry.file_name().to_str() == Some("tmp") {
                continue;
            }
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

    fn setup(root: &Path) -> (DataDir, crate::Tenant) {
        let data = DataDir::open(root).unwrap();
        let mut t = data.create("acme").unwrap();
        let p = serde_json::json!({"name": "Acme", "slug": "acme", "admin_first_name": "A", "admin_last_name": "B", "admin_email": "a@example.com"});
        let at = buildkiosk_core::Timestamp::from_millis(1);
        t.engine.execute(&mut t.store, buildkiosk_engine::Actor::System, at, "company.setup", &p).unwrap();
        (data, t)
    }

    fn add_project(t: &mut crate::Tenant, name: &str) {
        let at = buildkiosk_core::Timestamp::from_millis(2);
        t.engine.execute(&mut t.store, buildkiosk_engine::Actor::System, at, "project.create", &serde_json::json!({"name": name})).unwrap();
    }

    #[test]
    fn a_backup_taken_while_writing_goes_on_is_whole_as_of_its_start() {
        let src = temp("online-src");
        let dst = temp("online-dst");
        let (data, mut t) = setup(&src);
        add_project(&mut t, "Tower One");
        let (sha, _) = t.store.blobs.put(b"a file").unwrap();
        let entries_at_start = t.store.len();
        // The first half runs under the server's lock; the writes after it are not in the copy.
        let backup = data.backup_begin(&dst).unwrap();
        add_project(&mut t, "Tower Two");
        t.store.blobs.put(b"a later file").unwrap();
        let report = backup.finish().unwrap();
        assert_eq!(report[0].journal_entries, entries_at_start, "journal copied up to the moment the backup began");
        assert!(report[0].rows.iter().any(|(table, n)| *table == "projects" && *n == 1), "{:?}", report[0].rows);
        let copy = DataDir::open(&dst).unwrap();
        assert!(copy.load("acme").unwrap().store.blobs.exists(&sha));
        drop(copy);
        // The live data still has both.
        assert_eq!(data.check().unwrap()[0].rows.iter().find(|(t, _)| *t == "projects").map(|(_, n)| *n), Some(2));
        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
    }

    #[test]
    fn restore_puts_a_backup_in_place_and_moves_the_old_data_aside() {
        let src = temp("restore-src");
        let bak = temp("restore-bak");
        let live = temp("restore-live");
        {
            let (data, mut t) = setup(&src);
            add_project(&mut t, "Tower One");
            drop(t);
            data.backup_to(&bak).unwrap();
        }
        // Into a missing folder.
        let r = restore(&bak, &live, false, 100).unwrap();
        assert_eq!((r.companies.len(), r.moved_aside), (1, None));
        // Into a folder that has data: refused without --replace, and nothing changes.
        assert!(restore(&bak, &live, false, 101).is_err());
        {
            let data = DataDir::open(&live).unwrap();
            let mut t = data.load("acme").unwrap();
            add_project(&mut t, "Added after the backup");
        }
        let r = restore(&bak, &live, true, 102).unwrap();
        let aside = r.moved_aside.unwrap();
        assert!(aside.ends_with(format!("{}.before-restore-102", live.file_name().unwrap().to_str().unwrap())));
        let rows = |p: &Path| DataDir::open(p).unwrap().check().unwrap()[0].rows.iter().find(|(t, _)| *t == "projects").map(|(_, n)| *n);
        assert_eq!(rows(&live), Some(1), "back to the backup");
        assert_eq!(rows(&aside), Some(2), "the replaced data is kept");
        assert!(restore(&bak, &live, true, 102).is_err(), "won't overwrite an earlier move-aside");

        // A running server's directory is left alone; so are things that aren't backups or data directories.
        let held = DataDir::open(&live).unwrap();
        assert!(matches!(restore(&bak, &live, true, 103), Err(StoreError::Locked(_))));
        drop(held);
        let not_a_backup = temp("restore-nothing");
        fs::create_dir_all(&not_a_backup).unwrap();
        assert!(restore(&not_a_backup, &live, true, 104).is_err());
        let stranger = temp("restore-stranger");
        fs::create_dir_all(&stranger).unwrap();
        fs::write(stranger.join("notes.txt"), "mine").unwrap();
        assert!(restore(&bak, &stranger, true, 105).is_err());
        assert!(stranger.join("notes.txt").is_file());
        assert!(restore(&bak, &bak, true, 106).is_err());
        assert!(restore(&bak, &bak.join("inside"), true, 107).is_err());

        // A damaged backup is refused before anything is touched.
        fs::write(bak.join("companies").join("acme").join("journal.jsonl"), "{\"seq\":1}\nnot json\n{\"seq\":3}\n").unwrap();
        assert!(restore(&bak, &live, true, 108).is_err());
        assert_eq!(rows(&live), Some(1));
        for p in [&src, &bak, &live, &aside, &not_a_backup, &stranger] {
            let _ = fs::remove_dir_all(p);
        }
    }

    #[test]
    fn one_company_is_restored_and_the_others_are_left_alone() {
        let src = temp("co-src");
        let bak = temp("co-bak");
        let at = buildkiosk_core::Timestamp::from_millis(3);
        let setup_two =
            serde_json::json!({"name": "Beta", "slug": "beta", "admin_first_name": "B", "admin_last_name": "B", "admin_email": "b@example.com"});
        {
            let (data, mut t) = setup(&src);
            add_project(&mut t, "Tower One");
            let mut b = data.create("beta").unwrap();
            b.engine.execute(&mut b.store, buildkiosk_engine::Actor::System, at, "company.setup", &setup_two).unwrap();
            drop((t, b));
            data.backup_to(&bak).unwrap();
        }
        // After the backup both companies move on.
        {
            let data = DataDir::open(&src).unwrap();
            let mut t = data.load("acme").unwrap();
            add_project(&mut t, "Tower Two");
            let mut b = data.load("beta").unwrap();
            b.engine.execute(&mut b.store, buildkiosk_engine::Actor::System, at, "project.create", &serde_json::json!({"name": "Beta job"})).unwrap();
        }
        let projects = |p: &Path, slug: &str| {
            let d = DataDir::open(p).unwrap();
            d.load(slug).unwrap().engine.state().row_counts().into_iter().find(|(t, _)| *t == "projects").map(|(_, n)| n)
        };
        assert_eq!((projects(&src, "acme"), projects(&src, "beta")), (Some(2), Some(1)));
        // Refused: a bad name, a company the backup lacks, a running server, a backup that is not one.
        assert!(restore_company(&bak, &src, "Acme!", 10).is_err());
        assert!(restore_company(&bak, &src, "gamma", 10).is_err());
        {
            let _held = DataDir::open(&src).unwrap();
            assert!(matches!(restore_company(&bak, &src, "acme", 10), Err(StoreError::Locked(_))));
        }
        let nothing = temp("co-nothing");
        fs::create_dir_all(&nothing).unwrap();
        assert!(restore_company(&nothing, &src, "acme", 10).is_err());
        assert_eq!(projects(&src, "acme"), Some(2), "nothing changed");
        // Acme goes back to the backup; Beta keeps what it had; the old Acme is kept aside.
        let r = restore_company(&bak, &src, "acme", 11).unwrap();
        assert_eq!(r.slug, "acme");
        assert_eq!((projects(&src, "acme"), projects(&src, "beta")), (Some(1), Some(1)));
        let aside = src.with_file_name(format!("{}.before-restore-acme-11", src.file_name().unwrap().to_str().unwrap()));
        assert!(aside.join("journal.jsonl").is_file(), "the replaced company is kept");
        assert!(restore_company(&bak, &src, "acme", 11).is_err(), "won't overwrite an earlier move-aside");
        // A company that was lost altogether is simply put back.
        fs::remove_dir_all(src.join("companies").join("beta")).unwrap();
        restore_company(&bak, &src, "beta", 12).unwrap();
        assert_eq!(projects(&src, "beta"), Some(0), "as of the backup");
        for p in [&src, &bak, &nothing, &aside] {
            let _ = fs::remove_dir_all(p);
        }
        let _ = fs::remove_dir_all(src.with_file_name(format!("{}.before-restore-beta-12", src.file_name().unwrap().to_str().unwrap())));
    }
}
