//! Cross-process writer epoch for warm Searcher caches (R-XPROC-MULTIWRITER).
//!
//! Option C lite: index writers bump a durable stamp next to the index home;
//! long-lived MCP / Code Mode Searcher caches poll it and reopen when it changes.
//! This is not a lease or IPC bus — peers detect staleness by reading the file.

use super::{as_db_path, INDEX_DIR};

/// Historical generation-layout directory name. Origin no longer uses
/// build-then-swap, but leftover `generations/<id>/index.db` paths still
/// stamp the enclosing `.asgrep/` home so peers share one epoch file.
const GENERATIONS_DIR: &str = "generations";
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Filename under the index home advertising external writer epochs.
pub const WRITER_GENERATION_FILE: &str = "writer_generation";

/// Resolve the directory that owns [`WRITER_GENERATION_FILE`] for this index.
///
/// Default and generation layouts use `root/.asgrep/`. Pinned `index_path` /
/// `ASGREP_INDEX_PATH` use the parent of the DB file. Candidate DBs under
/// `generations/<id>/` still resolve to the enclosing index home so activation
/// and candidate builds share one stamp peers can poll.
pub fn writer_generation_home(root: &Path, index_path: Option<&Path>) -> PathBuf {
    if let Some(path) = index_path {
        return home_for_db(&as_db_path(path.to_path_buf()), root);
    }
    if let Ok(env_path) = std::env::var("ASGREP_INDEX_PATH") {
        return home_for_db(&as_db_path(PathBuf::from(env_path)), root);
    }
    root.join(INDEX_DIR)
}

fn home_for_db(db: &Path, root: &Path) -> PathBuf {
    if let Some(home) = generation_layout_home(db) {
        return home;
    }
    db.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.join(INDEX_DIR))
}

fn generation_layout_home(db: &Path) -> Option<PathBuf> {
    let gen_dir = db.parent()?;
    let gens = gen_dir.parent()?;
    if gens.file_name()?.to_str()? != GENERATIONS_DIR {
        return None;
    }
    Some(gens.parent()?.to_path_buf())
}

/// Absolute path of the writer-generation stamp file.
pub fn writer_generation_path(root: &Path, index_path: Option<&Path>) -> PathBuf {
    writer_generation_home(root, index_path).join(WRITER_GENERATION_FILE)
}

/// Read the current writer generation, or `0` when the stamp is absent/unreadable.
pub fn read_writer_generation(root: &Path, index_path: Option<&Path>) -> u64 {
    let path = writer_generation_path(root, index_path);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

/// Unique epoch for one bump. Not a +1 counter: two writers that both read
/// `N` must not both publish `N+1`, or a peer that already observed `N+1`
/// will skip the second mutation (`!=` check).
fn unique_epoch(sequence: u64) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = u64::from(std::process::id());
    nanos
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(pid.wrapping_shl(32))
        .wrapping_add(sequence)
}

/// Atomically publish a new writer epoch and return it.
///
/// Writers call this after durable index mutations so peer processes with
/// warm Searcher caches can detect change. The value is unique per bump
/// (time + pid + sequence), not `read+1`, so concurrent writers cannot
/// publish a duplicate epoch.
pub fn bump_writer_generation(root: &Path, index_path: Option<&Path>) -> crate::Result<u64> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let path = writer_generation_path(root, index_path);
    let parent = path.parent().ok_or_else(|| {
        crate::StoreError::Other(format!(
            "writer_generation path has no parent: {}",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(parent).map_err(|e| {
        crate::StoreError::Other(format!(
            "create writer_generation dir {}: {e}",
            parent.display()
        ))
    })?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let next = unique_epoch(sequence);
    let temp = parent.join(format!(
        ".writer_generation.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let body = format!("{next}\n");
    let result = (|| -> crate::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| crate::StoreError::Other(format!("create {}: {e}", temp.display())))?;
        file.write_all(body.as_bytes())
            .map_err(|e| crate::StoreError::Other(format!("write {}: {e}", temp.display())))?;
        file.sync_all()
            .map_err(|e| crate::StoreError::Other(format!("fsync {}: {e}", temp.display())))?;
        drop(file);
        std::fs::rename(&temp, &path)
            .map_err(|e| crate::StoreError::Other(format!("activate {}: {e}", path.display())))?;
        #[cfg(unix)]
        {
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| {
                    crate::StoreError::Other(format!("fsync parent {}: {e}", parent.display()))
                })?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    Ok(next)
}

#[cfg(test)]
#[path = "../../../../tests/unit/core/store__writer_generation.rs"]
mod tests;
