//! Uploaded files, stored by the SHA-256 of their content.
//!
//! Content addressing makes uploads idempotent (the same file twice is stored once), lets the
//! journal refer to a file by its hash, and means a stored blob never changes.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{Result, StoreError, fsutil};

/// The largest file accepted (4 GiB).
pub const MAX_BLOB: u64 = 4 * 1024 * 1024 * 1024;

/// A company's blob folder.
#[derive(Clone, Debug)]
pub struct BlobStore {
    dir: PathBuf,
}

impl BlobStore {
    pub(crate) fn new(dir: PathBuf) -> Result<BlobStore> {
        fs::create_dir_all(dir.join("tmp")).map_err(|e| StoreError::io(&dir, e))?;
        Ok(BlobStore { dir })
    }

    /// Where the blob with this hash lives; `None` for anything that isn't 64 lowercase hex digits
    /// (so a hash from a request can never point outside the folder).
    pub fn path(&self, sha256: &str) -> Option<PathBuf> {
        if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return None;
        }
        let (head, _) = sha256.split_at(2);
        Some(self.dir.join(head).join(sha256))
    }

    pub fn exists(&self, sha256: &str) -> bool {
        self.path(sha256).is_some_and(|p| p.is_file())
    }

    /// Opens a stored blob for reading.
    pub fn open(&self, sha256: &str) -> Result<File> {
        let path = self.path(sha256).ok_or_else(|| StoreError::Invalid(format!("`{sha256}` is not a file hash")))?;
        File::open(&path).map_err(|e| StoreError::io(&path, e))
    }

    /// Starts writing a new blob.
    pub fn writer(&self, max: u64) -> Result<BlobWriter> {
        let tmp = self.dir.join("tmp").join(format!("{}.part", fsutil::hex(&fsutil::random_bytes::<12>()?)));
        let file = File::create(&tmp).map_err(|e| StoreError::io(&tmp, e))?;
        Ok(BlobWriter { store: self.clone(), tmp, file: Some(file), hasher: Sha256::new(), size: 0, max: max.min(MAX_BLOB) })
    }

    /// Stores `bytes` and returns (sha256, size).
    pub fn put(&self, bytes: &[u8]) -> Result<(String, u64)> {
        let mut w = self.writer(MAX_BLOB)?;
        w.write(bytes)?;
        w.finish()
    }
}

/// A blob being written: data goes to a temporary file while it is hashed, then moves into place.
pub struct BlobWriter {
    store: BlobStore,
    tmp: PathBuf,
    file: Option<File>,
    hasher: Sha256,
    size: u64,
    max: u64,
}

impl BlobWriter {
    pub fn write(&mut self, chunk: &[u8]) -> Result<()> {
        let len = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        self.size = self.size.saturating_add(len);
        if self.size > self.max {
            return Err(StoreError::Invalid(format!("the file is larger than the {} MB limit", self.max / (1024 * 1024))));
        }
        let Some(file) = self.file.as_mut() else { return Err(StoreError::Invalid("the upload was already finished".into())) };
        file.write_all(chunk).map_err(|e| StoreError::io(&self.tmp, e))?;
        self.hasher.update(chunk);
        Ok(())
    }

    /// Finishes the blob: fsyncs it and moves it into place. Returns (sha256, size).
    pub fn finish(mut self) -> Result<(String, u64)> {
        let file = self.file.take().ok_or_else(|| StoreError::Invalid("the upload was already finished".into()))?;
        file.sync_all().map_err(|e| StoreError::io(&self.tmp, e))?;
        drop(file);
        let sha = fsutil::hex(&std::mem::take(&mut self.hasher).finalize());
        let dest = self.store.path(&sha).ok_or_else(|| StoreError::Invalid("bad hash".into()))?;
        if dest.exists() {
            let _ = fs::remove_file(&self.tmp); // the same content is already stored
        } else {
            let parent = dest.parent().unwrap_or(Path::new("."));
            fs::create_dir_all(parent).map_err(|e| StoreError::io(parent, e))?;
            fs::rename(&self.tmp, &dest).map_err(|e| StoreError::io(&dest, e))?;
            fsutil::sync_dir(parent);
        }
        Ok((sha, self.size))
    }
}

impl Drop for BlobWriter {
    fn drop(&mut self) {
        if self.file.is_some() {
            // An abandoned upload: remove its partial file (best effort).
            self.file = None;
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn content_addressed_and_capped() {
        let root = crate::tests::temp_dir("blobs");
        let store = BlobStore::new(root.join("blobs")).unwrap();
        let (sha, size) = store.put(b"hello").unwrap();
        assert_eq!(sha, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        assert_eq!(size, 5);
        assert!(store.exists(&sha));
        assert_eq!(store.put(b"hello").unwrap().0, sha, "same content, same blob");
        let mut w = store.writer(4).unwrap();
        assert!(w.write(b"12345").is_err());
        drop(w);
        assert_eq!(fs::read_dir(root.join("blobs/tmp")).unwrap().count(), 0, "partial uploads are cleaned up");
        for bad in ["../../etc/passwd", "ABC", &"g".repeat(64), &"a".repeat(63)] {
            assert!(store.path(bad).is_none(), "{bad}");
        }
        let _ = fs::remove_dir_all(&root);
    }
}
