//! Cross-process writer epoch for warm Searcher caches (R-XPROC-MULTIWRITER).
//!
//! Option C lite: index writers bump a durable stamp next to the index home;
//! long-lived MCP / Code Mode Searcher caches poll it and reopen when it changes.
//! This is not a lease or IPC bus — peers detect staleness by reading the file.

use super::{as_db_path, GENERATIONS_DIR, INDEX_DIR};
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

/// Atomically bump the writer generation and return the new value.
///
/// Writers call this after durable index mutations (and after generation
/// activation) so peer processes with warm Searcher caches can detect change.
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
    let next = read_writer_generation(root, index_path).saturating_add(1);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
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
            .map_err(|e| {
                crate::StoreError::Other(format!("create {}: {e}", temp.display()))
            })?;
        file.write_all(body.as_bytes()).map_err(|e| {
            crate::StoreError::Other(format!("write {}: {e}", temp.display()))
        })?;
        file.sync_all().map_err(|e| {
            crate::StoreError::Other(format!("fsync {}: {e}", temp.display()))
        })?;
        drop(file);
        std::fs::rename(&temp, &path).map_err(|e| {
            crate::StoreError::Other(format!("activate {}: {e}", path.display()))
        })?;
        #[cfg(unix)]
        {
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|e| {
                    crate::StoreError::Other(format!(
                        "fsync parent {}: {e}",
                        parent.display()
                    ))
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn bump_advances_and_peers_observe() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        assert_eq!(read_writer_generation(root, None), 0);
        let g1 = bump_writer_generation(root, None).unwrap();
        assert_eq!(g1, 1);
        assert_eq!(read_writer_generation(root, None), 1);
        let g2 = bump_writer_generation(root, None).unwrap();
        assert_eq!(g2, 2);
        let path = writer_generation_path(root, None);
        assert!(path.starts_with(root.join(INDEX_DIR)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap().trim(),
            "2"
        );
    }

    #[test]
    fn pinned_db_stamp_lives_beside_db() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let db = root.join("custom").join("index.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let g = bump_writer_generation(root, Some(&db)).unwrap();
        assert_eq!(g, 1);
        assert_eq!(
            writer_generation_path(root, Some(&db)),
            root.join("custom").join(WRITER_GENERATION_FILE)
        );
    }

    #[test]
    fn generation_candidate_db_stamps_index_home() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let candidate = root
            .join(INDEX_DIR)
            .join(GENERATIONS_DIR)
            .join("000001")
            .join("index.db");
        let g = bump_writer_generation(root, Some(&candidate)).unwrap();
        assert_eq!(g, 1);
        assert_eq!(
            writer_generation_path(root, Some(&candidate)),
            root.join(INDEX_DIR).join(WRITER_GENERATION_FILE)
        );
    }
}
