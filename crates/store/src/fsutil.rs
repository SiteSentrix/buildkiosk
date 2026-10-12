//! Small file-system helpers: atomic writes, randomness, hex.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::{Result, StoreError};

/// Writes `bytes` to `path` atomically: a temporary file in the same folder, fsynced, then renamed
/// over the target, then the folder fsynced. Readers see the old or the new file, never a mix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| StoreError::Invalid(format!("{} has no parent folder", path.display())))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", hex(&random_bytes::<6>()?)));
    let result = (|| {
        let mut f = File::create(&tmp).map_err(|e| StoreError::io(&tmp, e))?;
        f.write_all(bytes).map_err(|e| StoreError::io(&tmp, e))?;
        f.sync_all().map_err(|e| StoreError::io(&tmp, e))?;
        fs::rename(&tmp, path).map_err(|e| StoreError::io(path, e))?;
        sync_dir(dir);
        Ok(())
    })();
    if result.is_err() {
        // Best effort: a leftover temporary file is harmless, but don't leave clutter.
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Fsyncs a directory so a rename in it is durable. Some platforms can't open directories; there
/// the rename is as durable as the platform makes it, so failures are ignored.
pub fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

/// `N` random bytes from the operating system.
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| StoreError::Invalid(format!("the operating system has no randomness: {e}")))?;
    Ok(b)
}

pub(crate) fn random_32() -> Result<[u8; 32]> {
    random_bytes::<32>()
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS.get(usize::from(b >> 4)).copied().unwrap_or(b'0')));
        s.push(char::from(DIGITS.get(usize::from(b & 0xf)).copied().unwrap_or(b'0')));
    }
    s
}

/// 32 bytes from 64 hex digits.
pub(crate) fn unhex32(s: &str) -> Option<[u8; 32]> {
    let bytes = s.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = char::from(*bytes.get(i * 2)?).to_digit(16)?;
        let lo = char::from(*bytes.get(i * 2 + 1)?).to_digit(16)?;
        *slot = u8::try_from(hi * 16 + lo).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let b: [u8; 32] = core::array::from_fn(|i| u8::try_from(i * 7 % 256).unwrap());
        assert_eq!(unhex32(&hex(&b)), Some(b));
        assert_eq!(hex(&[0, 255, 16]), "00ff10");
        assert_eq!(unhex32("zz"), None);
    }
}
