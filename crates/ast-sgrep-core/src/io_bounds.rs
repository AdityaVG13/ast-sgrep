//! Bounded file reads for indexing (dx4g / zbpc).

use crate::{Result, StoreError};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

/// Maximum bytes accepted by `index_file` / walk indexing (64 MiB).
pub const MAX_INDEX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Read a UTF-8 source file with an explicit size cap.
pub fn read_text_capped(path: &Path, max_bytes: u64) -> Result<String> {
    let meta = fs::metadata(path)?;
    if meta.len() > max_bytes {
        return Err(StoreError::Other(format!(
            "file exceeds {} byte index cap: {}",
            max_bytes,
            path.display()
        )));
    }
    let mut file = File::open(path)?;
    let mut buf = String::new();
    // Cap the read even if the file grows between stat and read.
    file.by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_string(&mut buf)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                StoreError::Other(format!("binary file: {}", path.display()))
            } else {
                StoreError::Io(e)
            }
        })?;
    if buf.len() as u64 > max_bytes {
        return Err(StoreError::Other(format!(
            "file exceeds {} byte index cap: {}",
            max_bytes,
            path.display()
        )));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rejects_oversized_files() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(&[b'a'; 64]).unwrap();
        tmp.flush().unwrap();
        let err = read_text_capped(tmp.path(), 32).unwrap_err();
        assert!(err.to_string().contains("index cap"), "{err}");
    }
}
