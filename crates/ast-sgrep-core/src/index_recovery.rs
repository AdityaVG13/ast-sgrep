//! Corrupt-index recovery and SQLite sidecar quarantine helpers.
//! Extracted from `index.rs` (EXP-002 / F-001 C5+C6). Leaf FS helpers only;
//! `open_index_store` / `quick_check` stay in `index` (Indexer main open path).

use crate::index::{open_index_store, quick_check, IndexOptions};
use crate::store::IndexStore;
use crate::Result;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn suffixed_path(path: &Path, suffix: &str) -> Result<PathBuf> {
    let mut name = path
        .file_name()
        .map(OsString::from)
        .ok_or_else(|| crate::StoreError::Other("index path has no file name".into()))?;
    name.push(suffix);
    Ok(path.with_file_name(name))
}

const SQLITE_SIDECAR_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Derived sidecars are not recovery sources. Remove them before replacing the
/// authoritative DB so a coincidentally equal generation cannot admit stale
/// lexical or ANN rows from the corrupt index.
fn remove_derived_sidecars(index_path: &Path) -> Result<()> {
    let lexical = index_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(crate::tantivy_index::LEXICAL_DB);
    remove_file_if_present(&lexical)?;
    for suffix in SQLITE_SIDECAR_SUFFIXES {
        remove_file_if_present(&suffixed_path(&lexical, suffix)?)?;
    }
    remove_file_if_present(&crate::semantic_ivf::semantic_ivf_path(index_path))
}

/// Preserve a corrupt database and its SQLite sidecars without overwriting an
/// earlier quarantine. Recovery callers hold the adjacent recovery lock, and
/// hard-link admission prevents accidental overwrite. If the filesystem cannot
/// preserve the old inode, recovery fails closed and leaves the original path.
fn quarantine_corrupt_index(path: &Path) -> Result<PathBuf> {
    'candidate: for attempt in 0..1_000 {
        let suffix = if attempt == 0 {
            ".corrupt".to_owned()
        } else {
            format!(".corrupt.{attempt}")
        };
        let quarantine = suffixed_path(path, &suffix)?;
        match fs::hard_link(path, &quarantine) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }

        let mut preserved = vec![quarantine.clone()];
        for sidecar_suffix in SQLITE_SIDECAR_SUFFIXES {
            let source = suffixed_path(path, sidecar_suffix)?;
            let destination = suffixed_path(&quarantine, sidecar_suffix)?;
            match fs::hard_link(&source, &destination) {
                Ok(()) => preserved.push(destination),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    for created in preserved.into_iter().rev() {
                        let _ = fs::remove_file(created);
                    }
                    if error.kind() == ErrorKind::AlreadyExists {
                        continue 'candidate;
                    }
                    return Err(error.into());
                }
            }
        }

        // Remove sidecars before the main name so a failed cleanup never lets
        // SQLite attach an old WAL to a newly created replacement database.
        for sidecar_suffix in SQLITE_SIDECAR_SUFFIXES {
            remove_file_if_present(&suffixed_path(path, sidecar_suffix)?)?;
        }
        fs::remove_file(path)?;
        return Ok(quarantine);
    }
    Err(crate::StoreError::Other(
        "could not allocate a unique corrupt-index quarantine path".into(),
    ))
}

fn recovery_lock(path: &Path) -> Result<File> {
    let lock_path = suffixed_path(path, ".reindex.lock")?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.lock()?;
    Ok(lock)
}

fn replacement_generation_seed() -> i64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    nanos.min((i64::MAX / 2) as u128) as i64
}

pub(crate) fn recover_corrupt_index(
    options: &IndexOptions,
    cause: impl std::fmt::Display,
) -> Result<IndexStore> {
    let db_path = crate::try_index_db_path(&options.root, options.index_path.as_deref())?;
    let _recovery_lock = recovery_lock(&db_path).map_err(|error| {
        crate::StoreError::Other(format!(
            "corrupt index at {} could not acquire its recovery lock ({cause}): {error}",
            db_path.display()
        ))
    })?;

    // Another explicit reindex may have repaired the path while this caller
    // waited for the lock. Re-check before moving any inode.
    match open_index_store(options) {
        Ok(store) => match quick_check(&store) {
            Ok(result) if result.eq_ignore_ascii_case("ok") => return Ok(store),
            Ok(_) => drop(store),
            Err(error) if error.is_corrupt_database() => drop(store),
            Err(error) => return Err(error),
        },
        Err(error) if error.is_corrupt_database() => {}
        Err(error) => return Err(error),
    }

    remove_derived_sidecars(&db_path).map_err(|error| {
        crate::StoreError::Other(format!(
            "corrupt index at {} could not invalidate derived sidecars ({cause}): {error}",
            db_path.display()
        ))
    })?;
    let quarantine = quarantine_corrupt_index(&db_path).map_err(|error| {
        crate::StoreError::Other(format!(
            "corrupt index at {} could not be quarantined ({cause}): {error}",
            db_path.display()
        ))
    })?;
    let replacement = open_index_store(options).map_err(|error| {
        crate::StoreError::Other(format!(
            "corrupt index was quarantined at {}, but its replacement could not be created: {error}",
            quarantine.display()
        ))
    })?;
    // A fresh database would otherwise restart both counters at zero. Seed
    // them once so any undeletable/out-of-process stale sidecar fails identity
    // checks even when the rebuilt row counts happen to match the old index.
    let seed = replacement_generation_seed().to_string();
    replacement.set_meta("index_data_version", &seed)?;
    replacement.set_meta("semantic_data_version", &seed)?;
    Ok(replacement)
}
