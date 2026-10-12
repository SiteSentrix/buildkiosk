//! One company on disk: its journal, snapshot, history index, blobs and credentials.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use buildkiosk_engine::{Engine, JournalEntry, JournalSink};
use buildkiosk_model::{CompanyState, SnapshotIn};

use crate::blobs::BlobStore;
use crate::creds::Credentials;
use crate::{Result, StoreError, fsutil};

/// A loaded company: its engine (the state) and its storage.
pub struct Tenant {
    pub slug: String,
    pub engine: Engine,
    pub store: TenantStore,
}

/// The files of one company.
pub struct TenantStore {
    dir: PathBuf,
    journal_path: PathBuf,
    journal: File,
    /// Byte length of the journal (where the next entry goes).
    len: u64,
    /// Byte offset of each entry; entry `seq` is at index `seq - 1`.
    offsets: Vec<u64>,
    /// (table index in `CompanyState::TABLES`, row id) → the entries that changed that row.
    index: HashMap<(u16, u64), Vec<u64>>,
    snapshot_seq: u64,
    pub blobs: BlobStore,
}

impl JournalSink for TenantStore {
    fn append(&mut self, entry: &JournalEntry) -> std::result::Result<(), String> {
        let expected = u64::try_from(self.offsets.len()).unwrap_or(u64::MAX).saturating_add(1);
        if entry.seq != expected {
            return Err(format!("journal entry {} out of order (expected {expected})", entry.seq));
        }
        let mut line = serde_json::to_vec(entry).map_err(|e| format!("encoding the journal entry: {e}"))?;
        line.push(b'\n');
        let write = self.journal.write_all(&line).and_then(|()| self.journal.sync_data());
        if let Err(e) = write {
            // Cut off whatever part of the line reached the file, so the journal stays well-formed.
            let _ = self.journal.set_len(self.len);
            return Err(format!("writing {}: {e}", self.journal_path.display()));
        }
        self.offsets.push(self.len);
        self.len = self.len.saturating_add(u64::try_from(line.len()).unwrap_or(0));
        index_entry(&mut self.index, entry);
        Ok(())
    }
}

impl TenantStore {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Entries in the journal.
    pub fn len(&self) -> u64 {
        u64::try_from(self.offsets.len()).unwrap_or(u64::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Entries written since the last snapshot.
    pub fn since_snapshot(&self) -> u64 {
        self.len().saturating_sub(self.snapshot_seq)
    }

    fn read_at(&self, reader: &mut BufReader<File>, offset: u64) -> Result<JournalEntry> {
        reader.seek(SeekFrom::Start(offset)).map_err(|e| StoreError::io(&self.journal_path, e))?;
        let mut buf = Vec::new();
        reader.read_until(b'\n', &mut buf).map_err(|e| StoreError::io(&self.journal_path, e))?;
        serde_json::from_slice(trim_newline(&buf))
            .map_err(|e| StoreError::Corrupt { path: self.journal_path.display().to_string(), msg: format!("entry at byte {offset}: {e}") })
    }

    fn reader(&self) -> Result<BufReader<File>> {
        File::open(&self.journal_path).map(BufReader::new).map_err(|e| StoreError::io(&self.journal_path, e))
    }

    /// Journal entry `seq`.
    pub fn entry(&self, seq: u64) -> Result<Option<JournalEntry>> {
        let Some(offset) = seq.checked_sub(1).and_then(|i| usize::try_from(i).ok()).and_then(|i| self.offsets.get(i)).copied() else {
            return Ok(None);
        };
        let mut r = self.reader()?;
        self.read_at(&mut r, offset).map(Some)
    }

    /// Every entry that changed row `id` of `table`, oldest first: the item's change history.
    pub fn history(&self, table: &str, id: u64) -> Result<Vec<JournalEntry>> {
        let Some(t) = table_index(table) else { return Ok(Vec::new()) };
        let Some(seqs) = self.index.get(&(t, id)) else { return Ok(Vec::new()) };
        let mut r = self.reader()?;
        let mut out = Vec::with_capacity(seqs.len());
        for seq in seqs.iter().take(10_000) {
            if let Some(offset) = seq.checked_sub(1).and_then(|i| usize::try_from(i).ok()).and_then(|i| self.offsets.get(i)) {
                out.push(self.read_at(&mut r, *offset)?);
            }
        }
        Ok(out)
    }

    /// The last `n` entries that `keep` accepts, newest first, looking back at most `scan` entries.
    pub fn recent(&self, n: usize, scan: usize, keep: impl Fn(&JournalEntry) -> bool) -> Result<Vec<JournalEntry>> {
        let mut r = self.reader()?;
        let mut out = Vec::new();
        for offset in self.offsets.iter().rev().take(scan) {
            let e = self.read_at(&mut r, *offset)?;
            if keep(&e) {
                out.push(e);
                if out.len() >= n {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Writes a snapshot of `engine`'s state, so the next load replays only later entries.
    pub fn snapshot(&mut self, engine: &Engine) -> Result<()> {
        let path = self.dir.join("snapshot.json");
        let bytes =
            serde_json::to_vec(&engine.state().snapshot(engine.seq())).map_err(|e| StoreError::Invalid(format!("encoding the snapshot: {e}")))?;
        fsutil::write_atomic(&path, &bytes)?;
        self.snapshot_seq = engine.seq();
        Ok(())
    }

    /// The credential store (empty if none yet).
    pub fn credentials(&self) -> Result<Credentials> {
        let path = self.dir.join("credentials.json");
        if !path.exists() {
            return Ok(Credentials::default());
        }
        let text = fs::read_to_string(&path).map_err(|e| StoreError::io(&path, e))?;
        serde_json::from_str(&text).map_err(|e| StoreError::Corrupt { path: path.display().to_string(), msg: e.to_string() })
    }

    pub fn save_credentials(&self, creds: &Credentials) -> Result<()> {
        let path = self.dir.join("credentials.json");
        let text = serde_json::to_vec_pretty(creds).map_err(|e| StoreError::Invalid(e.to_string()))?;
        fsutil::write_atomic(&path, &text)
    }
}

fn table_index(table: &str) -> Option<u16> {
    CompanyState::TABLES.iter().position(|t| *t == table).and_then(|i| u16::try_from(i).ok())
}

fn index_entry(index: &mut HashMap<(u16, u64), Vec<u64>>, entry: &JournalEntry) {
    for (table, id) in entry.touched() {
        if let Some(t) = table_index(table) {
            let seqs = index.entry((t, id)).or_default();
            if seqs.last() != Some(&entry.seq) {
                seqs.push(entry.seq);
            }
        }
    }
}

fn trim_newline(b: &[u8]) -> &[u8] {
    b.strip_suffix(b"\n").map(|s| s.strip_suffix(b"\r").unwrap_or(s)).unwrap_or(b)
}

fn corrupt(path: &Path, msg: impl Into<String>) -> StoreError {
    StoreError::Corrupt { path: path.display().to_string(), msg: msg.into() }
}

/// Loads a company from `dir`: the snapshot, then every later journal entry.
pub(crate) fn load(slug: &str, dir: &Path) -> Result<Tenant> {
    let snapshot_path = dir.join("snapshot.json");
    let (mut engine, snapshot_seq) = if snapshot_path.exists() {
        let file = File::open(&snapshot_path).map_err(|e| StoreError::io(&snapshot_path, e))?;
        let snap: SnapshotIn = serde_json::from_reader(BufReader::new(file)).map_err(|e| corrupt(&snapshot_path, e.to_string()))?;
        let (state, seq) = snap.into_state().map_err(|e| corrupt(&snapshot_path, e.to_string()))?;
        (Engine::from_state(state, seq), seq)
    } else {
        (Engine::new(), 0)
    };

    let journal_path = dir.join("journal.jsonl");
    let mut offsets = Vec::new();
    let mut index = HashMap::new();
    let mut good_end: u64 = 0;
    if journal_path.exists() {
        let file = File::open(&journal_path).map_err(|e| StoreError::io(&journal_path, e))?;
        let mut reader = BufReader::new(file);
        let mut buf = Vec::new();
        let mut offset: u64 = 0;
        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf).map_err(|e| StoreError::io(&journal_path, e))?;
            if n == 0 {
                break;
            }
            let complete = buf.ends_with(b"\n");
            let parsed = serde_json::from_slice::<JournalEntry>(trim_newline(&buf));
            let entry = match parsed {
                Ok(e) if complete => e,
                _ if !complete => {
                    // A torn last line: the process stopped mid-write. Its command never took effect.
                    log::warn!("{}: dropping an incomplete last entry ({n} bytes) at byte {offset}", journal_path.display());
                    break;
                }
                Ok(_) => return Err(corrupt(&journal_path, format!("unexpected data at byte {offset}"))),
                Err(e) => return Err(corrupt(&journal_path, format!("entry at byte {offset}: {e}"))),
            };
            let expected = u64::try_from(offsets.len()).unwrap_or(u64::MAX).saturating_add(1);
            if entry.seq != expected {
                return Err(corrupt(&journal_path, format!("entry {} where entry {expected} was expected", entry.seq)));
            }
            if entry.seq > snapshot_seq {
                engine.apply(&entry).map_err(|e| corrupt(&journal_path, format!("replaying entry {}: {e}", entry.seq)))?;
            }
            index_entry(&mut index, &entry);
            offsets.push(offset);
            offset = offset.saturating_add(u64::try_from(n).unwrap_or(0));
            good_end = offset;
        }
    }
    let last_seq = u64::try_from(offsets.len()).unwrap_or(u64::MAX);
    if snapshot_seq > last_seq {
        return Err(corrupt(
            &snapshot_path,
            format!("the snapshot is at entry {snapshot_seq} but the journal ends at entry {last_seq}; restore the journal from a backup"),
        ));
    }
    let journal = OpenOptions::new().create(true).read(true).append(true).open(&journal_path).map_err(|e| StoreError::io(&journal_path, e))?;
    let on_disk = journal.metadata().map_err(|e| StoreError::io(&journal_path, e))?.len();
    if on_disk != good_end {
        journal.set_len(good_end).map_err(|e| StoreError::io(&journal_path, e))?;
        journal.sync_data().map_err(|e| StoreError::io(&journal_path, e))?;
    }
    let blobs = BlobStore::new(dir.join("blobs"))?;
    let store = TenantStore { dir: dir.to_path_buf(), journal_path, journal, len: good_end, offsets, index, snapshot_seq, blobs };
    Ok(Tenant { slug: slug.to_string(), engine, store })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use crate::DataDir;
    use buildkiosk_core::Timestamp;
    use buildkiosk_engine::Actor;
    use serde_json::json;

    fn setup(dir: &DataDir) -> Tenant {
        let mut t = dir.create("acme").unwrap();
        let at = Timestamp::from_millis(1_791_795_600_000);
        t.engine
            .execute(
                &mut t.store,
                Actor::System,
                at,
                "company.setup",
                &json!({"name": "Acme", "admin_first_name": "Ada", "admin_last_name": "Admin", "admin_email": "ada@acme.example"}),
            )
            .unwrap();
        t
    }

    #[test]
    fn journal_survives_reload_and_snapshots() {
        let root = crate::tests::temp_dir("tenant");
        let dir = DataDir::open(&root).unwrap();
        let mut t = setup(&dir);
        let admin = Actor::Person(buildkiosk_core::Id::new(1));
        let at = Timestamp::from_millis(1_791_795_700_000);
        t.engine.execute(&mut t.store, admin, at, "project.create", &json!({"name": "Clinic"})).unwrap();
        let state = t.engine.state().clone();
        drop(t);

        let t = dir.load("acme").unwrap();
        assert!(t.engine.state() == &state);
        assert_eq!(t.store.len(), 2);
        let hist = t.store.history("projects", 1).unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].cmd, "project.create");

        // Snapshot, add an entry, reload: snapshot plus one replayed entry.
        let mut t = t;
        t.store.snapshot(&t.engine).unwrap();
        t.engine.execute(&mut t.store, admin, at, "project.update", &json!({"project": 1, "name": "Clinic B"})).unwrap();
        let state = t.engine.state().clone();
        drop(t);
        let t = dir.load("acme").unwrap();
        assert!(t.engine.state() == &state);
        assert_eq!(t.store.since_snapshot(), 1);
        assert_eq!(t.store.recent(10, 100, |_| true).unwrap().len(), 3);
        drop(t);
        drop(dir);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_torn_last_line_is_dropped_but_damage_elsewhere_refuses_to_load() {
        let root = crate::tests::temp_dir("torn");
        let dir = DataDir::open(&root).unwrap();
        let t = setup(&dir);
        let journal = t.store.dir().join("journal.jsonl");
        drop(t);
        let good = fs::read(&journal).unwrap();
        // Torn write: half a line at the end.
        let mut torn = good.clone();
        torn.extend_from_slice(b"{\"seq\":2,\"at\":\"2026-");
        fs::write(&journal, &torn).unwrap();
        let mut t = dir.load("acme").unwrap();
        assert_eq!(t.store.len(), 1);
        assert_eq!(fs::read(&journal).unwrap(), good, "the torn tail is cut off");
        // Appending after recovery works.
        let admin = Actor::Person(buildkiosk_core::Id::new(1));
        t.engine.execute(&mut t.store, admin, Timestamp::from_millis(1_791_795_800_000), "project.create", &json!({"name": "X"})).unwrap();
        drop(t);
        assert_eq!(dir.load("acme").unwrap().store.len(), 2);
        // A damaged middle line refuses to load.
        let mut bad = fs::read(&journal).unwrap();
        bad[5] = b'#';
        fs::write(&journal, &bad).unwrap();
        assert!(matches!(dir.load("acme"), Err(StoreError::Corrupt { .. })));
        drop(dir);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn credentials_round_trip() {
        let root = crate::tests::temp_dir("creds");
        let dir = DataDir::open(&root).unwrap();
        let t = setup(&dir);
        let mut c = t.store.credentials().unwrap();
        c.set_password(1, "correct horse battery staple", Timestamp::from_millis(0)).unwrap();
        t.store.save_credentials(&c).unwrap();
        let back = t.store.credentials().unwrap();
        assert!(back.check_password(1, "correct horse battery staple"));
        assert!(!back.check_password(1, "wrong"));
        drop(t);
        drop(dir);
        let _ = fs::remove_dir_all(&root);
    }
}
