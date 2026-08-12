mod embed_support;
mod module_resolve;
pub(crate) mod sql;
mod sqlite;
mod writer_generation;
pub use sql::integrity_check;
pub use sql::{
    assert_sql_ident, CALLER_COLUMN_ALLOWLIST, COUNT_TABLE_ALLOWLIST, FILE_CHILD_TABLE_ALLOWLIST,
};
pub use sqlite::{
    CallerRow, ImportRow, IndexStore, IndexedLineRow, RefreshLinesInput, SymbolLocationRow,
    SymbolRow, UpsertFileInput,
};
pub use writer_generation::{
    bump_writer_generation, read_writer_generation, writer_generation_home, writer_generation_path,
    WRITER_GENERATION_FILE,
};
use std::path::{Path, PathBuf};

/// Write-durability profile for the index database (0obi).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Durability {
    /// `WAL` + `synchronous = FULL`. Survives power loss; slowest to index.
    Strict,
    /// `WAL` + `synchronous = NORMAL`. Survives process crash. Default.
    #[default]
    Balanced,
    /// `synchronous = OFF`. Fastest, and can corrupt the index on power loss.
    FastUnsafe,
}

impl Durability {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(Self::Strict),
            "balanced" | "default" => Some(Self::Balanced),
            "fast-unsafe" | "fast_unsafe" | "unsafe" => Some(Self::FastUnsafe),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Balanced => "balanced",
            Self::FastUnsafe => "fast-unsafe",
        }
    }

    /// `PRAGMA synchronous` value held outside write transactions.
    pub fn steady_pragma(self) -> &'static str {
        match self {
            Self::Strict => "FULL",
            // A crashed process must not leave `OFF` behind, so even the unsafe
            // profile returns to NORMAL between write batches.
            Self::Balanced | Self::FastUnsafe => "NORMAL",
        }
    }

    /// `PRAGMA synchronous` value used inside bulk and per-file write batches.
    pub fn write_pragma(self) -> &'static str {
        match self {
            Self::Strict => "FULL",
            Self::Balanced => "NORMAL",
            Self::FastUnsafe => "OFF",
        }
    }

    /// Resolve from `ASGREP_DURABILITY`, falling back to the safe default.
    /// An unrecognized value must not silently downgrade durability.
    pub fn from_env() -> Self {
        std::env::var("ASGREP_DURABILITY")
            .ok()
            .and_then(|value| Self::parse(&value))
            .unwrap_or_default()
    }
}

pub const INDEX_DIR: &str = ".asgrep";
pub const INDEX_DB: &str = "index.db";
/// Directory holding candidate and retained index generations (jpbq).
pub const GENERATIONS_DIR: &str = "generations";
/// Directory holding the active-generation pointer (jpbq).
pub const MANIFESTS_DIR: &str = "manifests";
/// Atomically replaced pointer to the active generation (jpbq).
pub const ACTIVE_MANIFEST: &str = "active.json";

/// Pointer to the generation currently serving queries (jpbq).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveManifest {
    /// Generation directory name, e.g. `000184`.
    pub generation: String,
    /// Schema version the generation was built with.
    pub schema_version: i64,
    /// Previous generation, retained for rollback until this one is proven.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    /// Unix seconds when this generation was activated.
    pub activated_at: i64,
}

/// Root of the generation layout for an index directory (jpbq).
pub fn generations_root(index_dir: &Path) -> PathBuf {
    index_dir.join(GENERATIONS_DIR)
}

/// Path of the active-generation manifest (jpbq).
pub fn active_manifest_path(index_dir: &Path) -> PathBuf {
    index_dir.join(MANIFESTS_DIR).join(ACTIVE_MANIFEST)
}

/// Read the active manifest, if this index uses the generation layout (jpbq).
pub fn read_active_manifest(index_dir: &Path) -> Option<ActiveManifest> {
    let raw = std::fs::read_to_string(active_manifest_path(index_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Replace the active-generation pointer atomically and durably (jpbq).
pub fn write_active_manifest(index_dir: &Path, manifest: &ActiveManifest) -> crate::Result<()> {
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let path = active_manifest_path(index_dir);
    let parent = path.parent().expect("manifest path has a parent");
    std::fs::create_dir_all(parent).map_err(|e| {
        crate::StoreError::Other(format!("create manifest dir {}: {e}", parent.display()))
    })?;
    let body = serde_json::to_string_pretty(manifest)
        .map_err(|e| crate::StoreError::Other(format!("serialize active manifest: {e}")))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".active.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
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
        // Close before rename so Windows can replace if the target is open.
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
    result
}

/// Database path for a named generation (jpbq).
pub fn generation_db_path(index_dir: &Path, generation: &str) -> PathBuf {
    generations_root(index_dir).join(generation).join(INDEX_DB)
}

/// Next generation directory name after the current one (jpbq).
pub fn next_generation_name(current: Option<&str>) -> String {
    let next = current
        .and_then(|name| name.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    format!("{next:06}")
}
pub(crate) fn as_db_path(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|e| e == "db") {
        path
    } else {
        path.join(INDEX_DB)
    }
}
pub fn index_db_path(root: &Path, index_path: Option<&Path>) -> PathBuf {
    try_index_db_path(root, index_path).unwrap_or_else(|_| {
        // Prefer naming a broken active generation for diagnostics over a
        // leftover flat legacy path (same integrity rule as try_index_db_path).
        let index_dir = root.join(INDEX_DIR);
        if let Some(manifest) = read_active_manifest(&index_dir) {
            let candidate = generation_db_path(&index_dir, &manifest.generation);
            if !candidate.exists() {
                return candidate;
            }
        }
        root.join(INDEX_DIR).join(INDEX_DB)
    })
}

pub fn try_index_db_path(root: &Path, index_path: Option<&Path>) -> crate::Result<PathBuf> {
    if let Some(path) = index_path {
        return Ok(as_db_path(path.to_path_buf()));
    }
    if let Ok(env_path) = std::env::var("ASGREP_INDEX_PATH") {
        return Ok(as_db_path(PathBuf::from(env_path)));
    }
    let index_dir = root.join(INDEX_DIR);
    // jpbq: an active-generation pointer wins over the legacy flat layout.
    if let Some(manifest) = read_active_manifest(&index_dir) {
        let candidate = generation_db_path(&index_dir, &manifest.generation);
        if candidate.exists() {
            return Ok(candidate);
        }
        // Corrupt activation: do not fall through to a leftover flat
        // `.asgrep/index.db` (stale corpus) or create an empty DB while the
        // manifest still claims a generation layout (wave2 loop9 / data-integrity).
        return Err(crate::StoreError::Other(format!(
            "active generation '{}' is missing at {}; refusing legacy/empty fallthrough \
             (restore the generation directory or reindex)",
            manifest.generation,
            candidate.display()
        )));
    }
    let local = index_dir.join(INDEX_DB);
    if local.exists() {
        return Ok(local);
    }
    if std::env::var("ASGREP_USE_CACHE")
        .ok()
        .as_deref()
        .is_some_and(crate::env_flag::is_boolish_true)
    {
        return cache_index_path(root);
    }
    Ok(local)
}
/// Resolve a private cache index path. Refuses shared `/tmp` when HOME/XDG_CACHE_HOME unset (i5ef).
fn cache_index_path(root: &Path) -> crate::Result<PathBuf> {
    let hash = blake3::hash(root.to_string_lossy().as_bytes());
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })
        .ok_or_else(|| {
            crate::StoreError::Other(
                "ASGREP_USE_CACHE requires HOME or XDG_CACHE_HOME; refusing shared /tmp fallback"
                    .into(),
            )
        })?;
    Ok(base
        .join("asgrep")
        .join(hash.to_hex().to_string())
        .join(INDEX_DB))
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexStatus {
    pub root: String,
    pub index_path: String,
    pub file_count: usize,
    pub line_count: usize,
    pub symbol_count: usize,
    pub caller_count: usize,
    pub import_count: usize,
    pub semantic_chunk_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_backend: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_dim: Option<usize>,
    pub embed_cache_entries: usize,
    pub embed_cache_capacity: usize,
    pub embed_cache_hits: u64,
    pub embed_cache_misses: u64,
    pub semantic_ivf_present: bool,
    /// Active write-durability profile (0obi).
    pub durability: String,
    /// Cross-process writer epoch stamped beside the index home (R-XPROC-MULTIWRITER).
    pub writer_generation: u64,
}
