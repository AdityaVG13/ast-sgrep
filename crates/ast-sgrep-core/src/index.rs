use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::store::{
    CallerRow, ImportRow, IndexStore, RefreshLinesInput, SymbolRow, UpsertFileInput,
};
use crate::Result;
use ast_sgrep_lang::{detect_language, ExtractionResult, Language, ParserRegistry};
use blake3::Hasher;
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

thread_local! {
    /// Test-only: when set, [`Indexer::rebuild_dirty_sidecars`] returns Err after the
    /// bulk SQLite commit so callers can pin Err-path cache invalidation.
    /// Thread-local so parallel `cargo test` workers do not cross-contaminate.
    static FORCE_SIDECAR_REBUILD_ERR: Cell<bool> = Cell::new(false);
}

/// RAII guard that forces sidecar rebuild to fail on this thread (simulates
/// mid-sidecar Err after durable bulk commit). Clears the flag on drop.
#[doc(hidden)]
pub struct ForceSidecarRebuildErr;

impl Drop for ForceSidecarRebuildErr {
    fn drop(&mut self) {
        FORCE_SIDECAR_REBUILD_ERR.with(|c| c.set(false));
    }
}

/// Arm the mid-sidecar rebuild failure inject for the current thread.
#[doc(hidden)]
pub fn force_sidecar_rebuild_err() -> ForceSidecarRebuildErr {
    FORCE_SIDECAR_REBUILD_ERR.with(|c| c.set(true));
    ForceSidecarRebuildErr
}

/// Maximum exact paths accepted by one incremental update request.
pub const MAX_INCREMENTAL_PATHS: usize = 1_024;

/// Indexed relative paths must be valid UTF-8. Lossy conversion is forbidden:
/// two distinct non-UTF8 `OsStr` paths must not collide into one DB key.
///
/// Also rejects absolute paths and `..` / root / prefix components so an
/// untrusted relative key cannot escape the project root when later joined
/// (defense-in-depth alongside MCP `parse_node_id` and search 89er).
pub fn indexed_rel_path(rel: &Path) -> Result<String> {
    use std::path::Component;
    if rel.as_os_str().is_empty() {
        return Err(crate::StoreError::Other(
            "empty relative path rejected (asgrep-kqhp)".into(),
        ));
    }
    if rel.is_absolute()
        || rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(crate::StoreError::Other(format!(
            "path traversal rejected (asgrep-kqhp): {}",
            rel.display()
        )));
    }
    let raw = rel.to_str().ok_or_else(|| {
        crate::StoreError::Other(format!(
            "non-UTF8 path rejected (asgrep-kqhp): {}",
            rel.display()
        ))
    })?;
    if raw.contains('\0') {
        return Err(crate::StoreError::Other(format!(
            "NUL in path rejected (asgrep-kqhp): {}",
            rel.display()
        )));
    }
    #[cfg(windows)]
    {
        Ok(raw.replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        // A backslash is an ordinary filename byte on Unix. Rewriting it after
        // component validation can turn `..\\escape.rs` into a traversal path
        // and can collide with the distinct `dir/file.rs` name.
        Ok(raw.to_owned())
    }
}
#[derive(Debug, Clone)]
pub struct SplitLines {
    pub lines: Vec<(u32, String)>,
    pub eol: &'static str,
}
pub fn split_content_lines(content: &str) -> SplitLines {
    if content.is_empty() {
        return SplitLines {
            lines: vec![(1, String::new())],
            eol: "lf",
        };
    }
    SplitLines {
        eol: if content.contains("\r\n") {
            "crlf"
        } else {
            "lf"
        },
        lines: content
            .split('\n')
            .enumerate()
            .map(|(i, line)| {
                (
                    (i + 1) as u32,
                    line.strip_suffix('\r').unwrap_or(line).into(),
                )
            })
            .collect(),
    }
}
type ExtractedRows = (
    Vec<SymbolRow>,
    Vec<CallerRow>,
    Vec<ImportRow>,
    Vec<ast_sgrep_lang::PatternNode>,
);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedBackend {
    #[default]
    Auto,
    Cloud,
    Ollama,
    Neural,
    Semantic,
}
impl EmbedBackend {
    pub fn to_preference(self) -> ast_sgrep_embed::EmbedPreference {
        match self {
            Self::Auto => ast_sgrep_embed::EmbedPreference::Auto,
            Self::Cloud => ast_sgrep_embed::EmbedPreference::Cloud,
            Self::Ollama => ast_sgrep_embed::EmbedPreference::Ollama,
            Self::Neural => ast_sgrep_embed::EmbedPreference::Neural,
            Self::Semantic => ast_sgrep_embed::EmbedPreference::Semantic,
        }
    }
    pub fn to_preference_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cloud => "cloud",
            Self::Ollama => "ollama",
            Self::Neural => "neural",
            // "semantic" is the legacy v1 marker (needs_semantic_v1_rewrite);
            // the versioned v2 identity is what gets stored and compared.
            Self::Semantic => "semantic-v2",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cloud" => Self::Cloud,
            "ollama" => Self::Ollama,
            "neural" | "fastembed" => Self::Neural,
            "semantic" | "semantic-v2" | "local" => Self::Semantic,
            _ => Self::Auto,
        }
    }
    pub fn from_flags(cloud: bool, ollama: bool, neural: bool, semantic_only: bool) -> Self {
        if cloud {
            Self::Cloud
        } else if ollama {
            Self::Ollama
        } else if neural {
            Self::Neural
        } else if semantic_only {
            Self::Semantic
        } else {
            Self::Auto
        }
    }
}
#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub lang_filter: Option<String>,
    pub respect_gitignore: bool,
    pub use_tantivy: bool,
    pub embed_semantic: bool,
    pub embed_backend: EmbedBackend,
    pub force_reindex: bool,
    pub ann_threshold: Option<usize>,
    /// Write-durability profile for the index database (0obi).
    pub durability: crate::store::Durability,
}
impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            index_path: None,
            lang_filter: None,
            respect_gitignore: true,
            use_tantivy: false,
            embed_semantic: true,
            embed_backend: EmbedBackend::Auto,
            force_reindex: false,
            ann_threshold: None,
            durability: crate::store::Durability::from_env(),
        }
    }
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub files_failed: usize,
    pub walk_errors: bool,
    pub symbols_extracted: usize,
    pub callers_extracted: usize,
    pub imports_extracted: usize,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct FileIndexStats {
    pub symbols: usize,
    pub callers: usize,
    pub imports: usize,
    pub skipped: bool,
}
struct IndexFileOutcome {
    stats: FileIndexStats,
    removed: bool,
}
pub struct Indexer {
    store: IndexStore,
    root_dir: crate::io_bounds::RootDir,
    parsers: ParserRegistry,
    options: IndexOptions,
    ignore: crate::gitignore::IgnoreMatcher,
    sidecars_dirty: SidecarsDirty,
}
#[derive(Debug, Clone, Copy, Default)]
struct SidecarsDirty {
    tantivy: bool,
    semantic_ivf: bool,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct WatchUpdateStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub files_failed: usize,
}

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

fn open_index_store(options: &IndexOptions) -> Result<IndexStore> {
    IndexStore::open_with_durability(
        &options.root,
        options.index_path.as_deref(),
        options.durability,
    )
}

fn quick_check(store: &IndexStore) -> Result<String> {
    Ok(store
        .connection()
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?)
}

fn replacement_generation_seed() -> i64 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    nanos.min((i64::MAX / 2) as u128) as i64
}

fn recover_corrupt_index(
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

impl Indexer {
    pub fn new(mut options: IndexOptions) -> Result<Self> {
        options.root = options.root.canonicalize().unwrap_or(options.root.clone());
        let root_dir = crate::io_bounds::RootDir::open(&options.root)?;
        let store = match open_index_store(&options) {
            Ok(store) if options.force_reindex => match quick_check(&store) {
                Ok(result) if result.eq_ignore_ascii_case("ok") => store,
                Ok(detail) => {
                    drop(store);
                    recover_corrupt_index(&options, detail)?
                }
                Err(error) => {
                    if !error.is_corrupt_database() {
                        return Err(error);
                    }
                    drop(store);
                    recover_corrupt_index(&options, error)?
                }
            },
            Ok(store) => store,
            Err(error) if options.force_reindex && error.is_corrupt_database() => {
                recover_corrupt_index(&options, error)?
            }
            Err(error) => return Err(error),
        };
        store.set_meta("root", &options.root.display().to_string())?;
        let ignore = crate::gitignore::IgnoreMatcher::new(&options.root);
        Ok(Self {
            store,
            root_dir,
            parsers: ParserRegistry::new(),
            options,
            ignore,
            sidecars_dirty: SidecarsDirty::default(),
        })
    }
    pub fn store(&self) -> &IndexStore {
        &self.store
    }
    pub fn index_all(&mut self) -> Result<IndexStats> {
        let perf_run = crate::perf_profile::Run::start("index_all");
        let perf_run_id = perf_run.id();
        self.ignore.clear();
        let (candidates, mut stats, prepared, semantic_rewrite_required) = {
            let _span = crate::perf_profile::Span::start(
                "index_walk_parse",
                "index",
                "WalkDir + prepare_file (read/hash/tree-sitter extract)",
            );
            let (candidates, stats) = self.collect_index_candidates();
            let options = &self.options;
            // 28vo: the hash-only fast path must not skip when the stored semantic
            // identity (backend/model) differs from the active preference.
            let semantic_rewrite_required =
                options.embed_semantic && !self.semantic_identity_matches()?;
            let semantic_identity_ok = !options.embed_semantic || !semantic_rewrite_required;
            let current_hashes = candidates
                .iter()
                .map(|(_, rel)| self.store.file_hash(rel))
                .collect::<Result<Vec<_>>>()?;
            let prepared: Vec<PrepareOutcome> = candidates
                .par_iter()
                .zip(current_hashes.par_iter())
                .map(|((abs, rel), current_hash)| {
                    prepare_file(
                        abs,
                        rel,
                        current_hash.as_deref(),
                        options,
                        &self.root_dir,
                        semantic_identity_ok,
                        perf_run_id,
                    )
                })
                .collect();
            (candidates, stats, prepared, semantic_rewrite_required)
        };
        if self.options.force_reindex && stats.walk_errors {
            return Err(crate::StoreError::Other(
                "strict reindex aborted because the repository walk was incomplete".into(),
            ));
        }
        // A legacy marker can be cleared only when every reachable candidate
        // was prepared. Clearing it before the upserts lets their existing
        // metadata path persist the backend that actually produced the new
        // vectors instead of hard-coding the local backend after the fact.
        let complete_semantic_rewrite = semantic_rewrite_required
            && self.options.lang_filter.is_none()
            && !stats.walk_errors
            && prepared.iter().all(|outcome| {
                matches!(
                    outcome,
                    PrepareOutcome::Ready(_) | PrepareOutcome::Unchanged
                )
            });
        let mut semantic_ivf_dirty = false;
        {
            let _span = crate::perf_profile::Span::start(
                "sqlite_upsert",
                "index",
                "bulk upsert_file transaction",
            );
            self.store.begin_bulk_tx()?;
            let write_result = (|| {
                if complete_semantic_rewrite {
                    self.store.reset_semantic_index_for_rewrite()?;
                }
                self.commit_prepared_files(
                    &candidates,
                    prepared,
                    &mut stats,
                    &mut semantic_ivf_dirty,
                )
            })();
            self.store.apply_bulk_write_result(write_result)?;
        }
        // Durable rows may already be visible; advertise before sidecar work so
        // peer Searcher caches cannot keep a pre-mutation snapshot (Option C lite).
        self.advertise_writer_generation();
        self.rebuild_dirty_sidecars(&stats, semantic_ivf_dirty)?;
        self.rebuild_lexicon_if_dirty();
        Ok(stats)
    }

    /// Bulk-tx body: upsert prepared outcomes, then prune missing files when safe.
    /// Callers own `begin_bulk_tx` / `apply_bulk_write_result` so rollback pairing stays visible.
    fn commit_prepared_files(
        &self,
        candidates: &[(PathBuf, String)],
        prepared: Vec<PrepareOutcome>,
        stats: &mut IndexStats,
        semantic_ivf_dirty: &mut bool,
    ) -> Result<()> {
        if self.options.force_reindex {
            let failures = prepared
                .iter()
                .filter(|outcome| matches!(outcome, PrepareOutcome::Failed(_)))
                .count();
            if failures > 0 {
                stats.files_failed = failures;
                return Err(crate::StoreError::Other(format!(
                    "strict reindex aborted because {failures} file(s) could not be prepared"
                )));
            }
        }
        let mut seen_paths = HashSet::new();
        for (rel_str, outcome) in candidates.iter().map(|(_, r)| r).zip(prepared) {
            match outcome {
                PrepareOutcome::Unchanged => {
                    seen_paths.insert(rel_str.clone());
                    stats.files_skipped += 1;
                }
                PrepareOutcome::Filtered => {
                    // --lang must not destructively wipe other languages (y1oy.8):
                    // filtered paths are skipped here; prune_missing_files also
                    // respects lang_filter when removing absent files.
                }
                PrepareOutcome::Failed(msg) => {
                    eprintln!("[asgrep] failed to index {rel_str}: {msg}");
                    // Preserve the last usable row for an unreadable/invalid
                    // file rather than treating a preparation failure as a
                    // confirmed deletion during stale-row pruning.
                    seen_paths.insert(rel_str.clone());
                    stats.files_failed += 1;
                }
                PrepareOutcome::Ready(prep) => {
                    seen_paths.insert(rel_str.clone());
                    self.store.upsert_file(UpsertFileInput {
                        rel_path: rel_str,
                        language: prep.language.as_deref(),
                        mtime_secs: prep.mtime_secs,
                        mtime_nanos: prep.mtime_nanos,
                        content_hash: &prep.hash,
                        lines: &prep.lines,
                        eol: &prep.eol,
                        symbols: &prep.symbols,
                        callers: &prep.callers,
                        imports: &prep.imports,
                        pattern_nodes: &prep.pattern_nodes,
                        semantic_chunks: &prep.semantic_chunks,
                        embed_semantic: self.options.embed_semantic,
                        embed_backend: self.options.embed_backend.to_preference(),
                    })?;
                    // Same bulk_tx as upsert: fail + rollback if body meta cannot land
                    // (structure-skip must not use a stale body fingerprint).
                    self.store
                        .set_meta(&format!("body:{rel_str}"), &prep.body_hash)?;
                    stats.files_indexed += 1;
                    stats.symbols_extracted += prep.symbols.len();
                    stats.callers_extracted += prep.callers.len();
                    stats.imports_extracted += prep.imports.len();
                    if self.options.embed_semantic {
                        *semantic_ivf_dirty = true;
                    }
                }
            }
        }
        if should_prune_missing_files(stats.walk_errors) {
            self.prune_missing_files(&seen_paths, stats, semantic_ivf_dirty)?;
        }
        Ok(())
    }

    fn rebuild_lexicon_if_dirty(&self) {
        if self.store.lexicon_is_dirty().unwrap_or(true) {
            if let Err(error) = self.rebuild_lexicon() {
                // Search sees an empty lexicon after invalidation, never stale
                // associations. Learning remains a best-effort enhancement.
                eprintln!("asgrep: lexicon rebuild skipped: {error}");
            }
        }
    }

    /// Rebuild the repository semantic lexicon from indexed symbols (ufk7).
    ///
    /// Pairs each symbol's identifier subtokens with the prose terms around it
    /// (doc comments and the surrounding line text) and with its neighbours,
    /// then scores the pairs with PPMI under a support floor.
    fn rebuild_lexicon(&self) -> Result<()> {
        use crate::lexicon::{prose_terms, subtokens, LexiconBuilder, Observation};

        let mut builder = LexiconBuilder::new();
        self.store.for_each_symbol_context(|name, context| {
            let identifier_terms = subtokens(name);
            if identifier_terms.is_empty() {
                return;
            }
            let mut prose = prose_terms(context);
            prose.sort();
            prose.dedup();
            prose.truncate(crate::lexicon::MAX_PROSE_TERMS);
            // A symbol with no surrounding vocabulary teaches nothing.
            if prose.is_empty() {
                return;
            }
            builder.observe(&Observation {
                identifier_terms,
                prose_terms: prose,
            });
        })?;
        let associations = builder.finish();
        crate::lexicon::store_lexicon(&self.store, &associations)
    }
    /// Walk the project once using the Indexer's IgnoreMatcher for both directory
    /// pruning and file skips (single ownership story — no second matcher).
    fn collect_index_candidates(&self) -> (Vec<(PathBuf, String)>, IndexStats) {
        let mut stats = IndexStats::default();
        let mut candidates: Vec<(PathBuf, String)> = Vec::new();
        let root = &self.options.root;
        let ignore = &self.ignore;
        let respect_gitignore = self.options.respect_gitignore;
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if should_skip_dir(e.path()) {
                    return false;
                }
                if respect_gitignore && e.file_type().is_dir() {
                    if let Ok(rel) = e.path().strip_prefix(root) {
                        if !rel.as_os_str().is_empty() && ignore.is_dir_ignored(rel) {
                            return false;
                        }
                    }
                }
                true
            })
        {
            match entry {
                Ok(entry) if entry.file_type().is_file() => {
                    let path = entry.path().to_path_buf();
                    let Ok(rel) = path.strip_prefix(root) else {
                        continue;
                    };
                    // kqhp: non-UTF8 rel paths are rejected, never lossy-collapsed.
                    let Ok(rel_str) = indexed_rel_path(rel) else {
                        stats.files_skipped += 1;
                        continue;
                    };
                    if (respect_gitignore && ignore.is_ignored(rel)) || should_skip_file(&path) {
                        stats.files_skipped += 1;
                        continue;
                    }
                    candidates.push((path, rel_str));
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[asgrep] walk error: {e}");
                    stats.walk_errors = true;
                }
            }
        }
        (candidates, stats)
    }
    fn prune_missing_files(
        &self,
        seen_paths: &HashSet<String>,
        stats: &mut IndexStats,
        semantic_ivf_dirty: &mut bool,
    ) -> Result<()> {
        for path in self.store.all_file_paths()? {
            if seen_paths.contains(&path) {
                continue;
            }
            // With --lang, only prune missing files for that language so other
            // languages remain searchable (y1oy.8).
            if let Some(filter) = self.options.lang_filter.as_ref() {
                match self.store.file_language(&path)? {
                    Some(lang) if lang == *filter => {}
                    Some(_) => continue,
                    None => {}
                }
            }
            self.store.remove_file(&path)?;
            stats.files_removed += 1;
            if self.options.embed_semantic {
                *semantic_ivf_dirty = true;
            }
        }
        Ok(())
    }
    fn rebuild_dirty_sidecars(&self, _stats: &IndexStats, semantic_ivf_dirty: bool) -> Result<()> {
        // After bulk commit: injectable Err so MCP/CM tests pin invalidate-on-Err.
        if FORCE_SIDECAR_REBUILD_ERR.with(|c| c.get()) {
            return Err(crate::StoreError::Other(
                "forced sidecar rebuild failure after bulk commit (test inject)".into(),
            ));
        }
        let file_count = self.store.status()?.file_count;
        if crate::tantivy_index::should_use_tantivy(file_count, self.options.use_tantivy) {
            self.rebuild_tantivy_sidecar()?;
        }
        if self.options.embed_semantic && semantic_ivf_dirty {
            self.rebuild_semantic_ivf_sidecar()?;
        }
        Ok(())
    }
    fn rebuild_semantic_ivf_sidecar(&self) -> Result<()> {
        let stats = self.store.semantic_chunk_stats(None)?;
        if !crate::semantic_ann::should_use_ann(stats.count, self.options.ann_threshold) {
            crate::semantic_ivf::invalidate_semantic_ivf(self.store.db_path())?;
            return Ok(());
        }
        let chunks = self.store.all_semantic_chunks(None)?;
        crate::semantic_ann::rebuild_semantic_ivf_sidecar(
            self.store(),
            &chunks,
            self.options.ann_threshold,
        )
    }
    fn rebuild_tantivy_sidecar(&self) -> Result<()> {
        let before = self.store.index_data_version()?;
        let lines = self.store.all_indexed_lines()?;
        let after = self.store.index_data_version()?;
        if before != after {
            return Err(crate::StoreError::Other(
                "index changed while preparing lexical sidecar; retry the rebuild".into(),
            ));
        }
        crate::tantivy_index::TantivySidecar::open_for_index(
            &self.options.root,
            self.options.index_path.as_deref(),
        )?
        .rebuild_from_lines_with_generation(&lines, after)
    }
    pub fn reindex_all(&mut self) -> Result<IndexStats> {
        // Rebuild in place instead of committing an empty database before the
        // repository walk. Existing rows remain usable until index_all's bulk
        // transaction commits, and a crash cannot strand an empty index.
        let previous = self.options.force_reindex;
        self.options.force_reindex = true;
        let result = self.index_all();
        self.options.force_reindex = previous;
        result
    }
    pub fn update_paths(&mut self, paths: &[PathBuf]) -> Result<WatchUpdateStats> {
        if paths.len() > MAX_INCREMENTAL_PATHS {
            return Err(crate::StoreError::Other(format!(
                "incremental update exceeds max {MAX_INCREMENTAL_PATHS} paths"
            )));
        }
        // Single-file updates reuse the existing gitignore matcher.
        if paths.len() != 1 {
            self.ignore.clear();
        }
        let mut stats = WatchUpdateStats::default();
        let mut changed = false;
        for input_path in paths {
            let Some(abs) = normalize_watch_path(&self.options.root, input_path) else {
                continue;
            };
            let Ok(rel) = abs.strip_prefix(&self.options.root) else {
                continue;
            };
            // Empty relative path or directory events are not "skipped" files.
            if rel.as_os_str().is_empty() || abs.is_dir() {
                continue;
            }
            let rel_str = indexed_rel_path(rel)?;
            let mut path_stats = WatchUpdateStats::default();
            let mut path_changed = false;
            let mut index_failure = false;
            let descendant_prefix = format!("{rel_str}/");
            let transactional_replace = self.store.has_file_with_prefix(&descendant_prefix)?;
            if transactional_replace {
                self.store.begin_file_tx()?;
            }
            let update_result = (|| -> Result<()> {
                // Filesystem paths cannot simultaneously be files and directories.
                // Delete stale descendants in the same transaction as a replacement
                // file upsert, so an unreadable replacement preserves the last usable
                // directory-shaped index state.
                let descendants = if transactional_replace {
                    self.store.remove_files_with_prefix(&descendant_prefix)?
                } else {
                    0
                };
                path_stats.files_removed += descendants;
                path_changed |= descendants > 0;
                if !abs.exists() {
                    if self.store.file_hash(&rel_str)?.is_some() {
                        self.store.remove_file(&rel_str)?;
                        path_stats.files_removed += 1;
                        path_changed = true;
                    }
                    return Ok(());
                }
                if should_skip_watch_path(&abs, rel, self.options.respect_gitignore, &self.ignore) {
                    if self.store.file_hash(&rel_str)?.is_some() {
                        self.store.remove_file(&rel_str)?;
                        path_stats.files_removed += 1;
                        path_changed = true;
                    } else {
                        path_stats.files_skipped += 1;
                    }
                    return Ok(());
                }
                let leaf_is_symlink = fs::symlink_metadata(&abs)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink());
                if leaf_is_symlink {
                    if self.store.file_hash(&rel_str)?.is_some() {
                        self.store.remove_file(&rel_str)?;
                        path_stats.files_removed += 1;
                        path_changed = true;
                    }
                } else if abs.is_file() {
                    match self.index_file_outcome(&abs, &rel_str) {
                        Ok(outcome) if outcome.removed => {
                            path_stats.files_removed += 1;
                            path_changed = true;
                        }
                        Ok(outcome) if outcome.stats.skipped => path_stats.files_skipped += 1,
                        Ok(_) => {
                            path_stats.files_indexed += 1;
                            path_changed = true;
                        }
                        Err(error) => {
                            index_failure = true;
                            return Err(error);
                        }
                    }
                } else if self.store.file_hash(&rel_str)?.is_some() {
                    self.store.remove_file(&rel_str)?;
                    path_stats.files_removed += 1;
                    path_changed = true;
                }
                Ok(())
            })();
            match update_result {
                Ok(()) => {
                    if transactional_replace {
                        self.store.commit_file_tx()?;
                    }
                    stats.files_indexed += path_stats.files_indexed;
                    stats.files_skipped += path_stats.files_skipped;
                    stats.files_removed += path_stats.files_removed;
                    changed |= path_changed;
                }
                Err(error) => {
                    if transactional_replace {
                        self.store.rollback_file_tx()?;
                    }
                    if !index_failure {
                        return Err(error);
                    }
                    eprintln!("[asgrep] failed to index {rel_str}: {error}");
                    stats.files_failed += 1;
                }
            }
        }
        if changed {
            self.mark_sidecars_dirty()?;
            self.advertise_writer_generation();
        }
        Ok(stats)
    }
    pub fn flush_deferred_rebuilds(&mut self) -> Result<()> {
        let pending = self.deferred_rebuilds_pending();
        if self.sidecars_dirty.tantivy {
            self.rebuild_tantivy_sidecar()?;
            self.sidecars_dirty.tantivy = false;
        }
        if self.sidecars_dirty.semantic_ivf {
            self.rebuild_semantic_ivf_sidecar()?;
            self.sidecars_dirty.semantic_ivf = false;
        }
        self.rebuild_lexicon_if_dirty();
        if pending {
            self.advertise_writer_generation();
        }
        Ok(())
    }

    /// Bump the cross-process writer stamp (best-effort; never fails the index).
    fn advertise_writer_generation(&self) {
        if let Err(error) = crate::store::bump_writer_generation(
            &self.options.root,
            self.options.index_path.as_deref(),
        ) {
            eprintln!("asgrep: writer_generation stamp skipped: {error}");
        }
    }
    pub fn deferred_rebuilds_pending(&self) -> bool {
        self.sidecars_dirty.tantivy
            || self.sidecars_dirty.semantic_ivf
            || self.store.lexicon_is_dirty().unwrap_or(true)
    }
    fn mark_sidecars_dirty(&mut self) -> Result<()> {
        let lexical_exists = crate::tantivy_index::sidecar_path(
            &self.options.root,
            self.options.index_path.as_deref(),
        )
        .exists();
        let file_count = self.store.status()?.file_count;
        self.sidecars_dirty.tantivy = self.sidecars_dirty.tantivy
            || lexical_exists
            || crate::tantivy_index::should_use_tantivy(file_count, self.options.use_tantivy);
        if self.options.embed_semantic {
            self.sidecars_dirty.semantic_ivf = true;
        }
        Ok(())
    }
    pub fn index_file(&mut self, abs_path: &Path, rel_path: &str) -> Result<FileIndexStats> {
        Ok(self.index_file_outcome(abs_path, rel_path)?.stats)
    }
    fn index_file_outcome(&mut self, abs_path: &Path, rel_path: &str) -> Result<IndexFileOutcome> {
        let rel_path = indexed_rel_path(Path::new(rel_path))?;
        let source = self
            .root_dir
            .read_text_capped(Path::new(&rel_path), crate::io_bounds::MAX_INDEX_FILE_BYTES)?;
        let mtime = source.metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
        self.index_content_at(&rel_path, &source.text, abs_path, mtime_secs, mtime_nanos)
    }
    pub fn index_content(&mut self, rel_path: &str, content: &str) -> Result<FileIndexStats> {
        let rel_path = indexed_rel_path(Path::new(rel_path))?;
        let (mtime_secs, mtime_nanos) = system_time_to_parts(SystemTime::now());
        Ok(self
            .index_content_at(
                &rel_path,
                content,
                Path::new(&rel_path),
                mtime_secs,
                mtime_nanos,
            )?
            .stats)
    }
    fn index_content_at(
        &mut self,
        rel_path: &str,
        content: &str,
        lang_path: &Path,
        mtime_secs: i64,
        mtime_nanos: u32,
    ) -> Result<IndexFileOutcome> {
        // Callers (`index_file` / `index_content` / walk) already validated; re-check
        // so internal paths cannot skip the fence.
        let rel_path = indexed_rel_path(Path::new(rel_path))?;
        let rel_path = rel_path.as_str();
        let hash = hash_content(content);
        let language = detect_language(lang_path, Some(content));
        if !self.language_filter_allows(language) {
            let removed = self.store.file_hash(rel_path)?.is_some();
            if removed {
                self.store.remove_file(rel_path)?;
            }
            return Ok(IndexFileOutcome {
                stats: FileIndexStats {
                    skipped: !removed,
                    ..Default::default()
                },
                removed,
            });
        }
        if self.is_unchanged(rel_path, &hash)? {
            return Ok(IndexFileOutcome {
                stats: FileIndexStats {
                    skipped: true,
                    ..Default::default()
                },
                removed: false,
            });
        }
        let body_hash = body_structure_hash(content, language);
        let body_key = format!("body:{rel_path}");
        // Structure-skip fast path: when embeddings are disabled and the body
        // fingerprint is unchanged, refresh lines without reparsing graph rows.
        if !self.options.embed_semantic {
            if let Some(file_id) = self.store.file_id(rel_path)? {
                if self.store.get_meta(&body_key)?.as_deref() == Some(body_hash.as_str()) {
                    let split = split_content_lines(content);
                    self.store.with_file_tx(|| {
                        self.store.refresh_lines_only(RefreshLinesInput {
                            file_id,
                            language: language.map(|l| l.as_str()),
                            mtime_secs,
                            mtime_nanos,
                            content_hash: &hash,
                            lines: &split.lines,
                            eol: split.eol,
                            rel_path,
                        })
                    })?;
                    return Ok(IndexFileOutcome {
                        stats: FileIndexStats::default(),
                        removed: false,
                    });
                }
            }
        }
        let (symbols, callers, imports, pattern_nodes) =
            self.extract_rows(rel_path, content, language)?;
        let material = materialize_upsert(
            content,
            language,
            &symbols,
            &callers,
            &pattern_nodes,
            self.options.embed_semantic,
            body_hash,
        );
        // Nest under with_file_tx so body meta and upsert commit or roll back together.
        // Without this, a post-upsert set_meta failure leaves content_hash advanced and
        // body: meta stale → next structure-skip can keep wrong graph rows.
        self.store.with_file_tx(|| {
            self.store.upsert_file(UpsertFileInput {
                rel_path,
                language: language.map(|l| l.as_str()),
                mtime_secs,
                mtime_nanos,
                content_hash: &hash,
                lines: &material.split.lines,
                eol: material.split.eol,
                symbols: &symbols,
                callers: &callers,
                imports: &imports,
                pattern_nodes: &pattern_nodes,
                semantic_chunks: &material.semantic_chunks,
                embed_semantic: self.options.embed_semantic,
                embed_backend: self.options.embed_backend.to_preference(),
            })?;
            self.store.set_meta(&body_key, &material.body_hash)?;
            Ok(())
        })?;
        Ok(IndexFileOutcome {
            stats: FileIndexStats {
                symbols: symbols.len(),
                callers: callers.len(),
                imports: imports.len(),
                skipped: false,
            },
            removed: false,
        })
    }
    fn is_unchanged(&self, rel_path: &str, hash: &str) -> Result<bool> {
        if self.options.force_reindex {
            return Ok(false);
        }
        if self.store.file_hash(rel_path)?.is_none_or(|h| h != hash) {
            return Ok(false);
        }
        if self.options.embed_semantic && !self.semantic_identity_matches()? {
            return Ok(false);
        }
        Ok(true)
    }
    /// Full semantic identity check (28vo/e2hc.13): the stored embed backend
    /// must equal the active preference exactly, no legacy v1 rewrite pending,
    /// and the configured model must match what was recorded at index time.
    fn semantic_identity_matches(&self) -> Result<bool> {
        // Legacy unversioned semantic-v1 (e2hc.13): force rewrite even under
        // Auto. Without this, Auto skips the backend mismatch check and a
        // single-file update can flip meta to semantic-v2 while sibling
        // chunks remain v1.
        if self.store.needs_semantic_v1_rewrite()? {
            return Ok(false);
        }
        // Exact backend identity only (ast-sgrep-28vo): Auto is not a
        // wildcard for concrete stored backends, and stored "auto" does not
        // match a concrete active preference. Builds made under Auto record
        // "embed_backend_pref=auto", so an Auto reopen over an Auto build
        // stays a no-op (parity) while an Auto reopen over an explicit build
        // reindexes.
        let stored = self.store.get_meta("embed_backend")?;
        let active = self.options.embed_backend.to_preference_str();
        let stored_pref = self.store.get_meta("embed_backend_pref")?;
        // Auto reopen: no-op only over an Auto build (parity idempotency);
        // over an explicit build it reindexes (ast-sgrep-28vo). Explicit
        // reopen: exact resolved-kind match (semantic-v2 round-trips).
        let identity_ok = if active == "auto" {
            stored_pref.as_deref() == Some("auto")
        } else {
            stored.as_deref() == Some(active)
        };
        if !identity_ok {
            return Ok(false);
        }
        let dim = self
            .store
            .get_meta("embed_dim")?
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_else(ast_sgrep_embed::default_semantic_dim);
        let current_model = stored
            .as_deref()
            .and_then(ast_sgrep_embed::EmbedBackendKind::parse)
            .and_then(|backend| ast_sgrep_embed::configured_backend_model_id(backend, dim));
        if self.store.get_meta("embed_model")? != current_model {
            return Ok(false);
        }
        Ok(true)
    }
    fn language_filter_allows(&self, language: Option<Language>) -> bool {
        let Some(lang_filter) = self.options.lang_filter.as_ref() else {
            return true;
        };
        if language.is_some_and(|lang| lang.as_str() == lang_filter.as_str()) {
            return true;
        }
        false
    }
    fn extract_rows(
        &self,
        rel_path: &str,
        content: &str,
        language: Option<Language>,
    ) -> Result<ExtractedRows> {
        let Some(lang) = language else {
            return Ok((vec![], vec![], vec![], vec![]));
        };
        let extraction = self.parsers.parse(lang, content).map_err(|e| {
            crate::StoreError::Other(format!(
                "failed to parse {rel_path} as {}: {e}",
                lang.as_str()
            ))
        })?;
        Ok(rows_from_extraction(&extraction))
    }
}
struct PreparedFile {
    hash: String,
    body_hash: String,
    language: Option<String>,
    mtime_secs: i64,
    mtime_nanos: u32,
    lines: Vec<(u32, String)>,
    eol: String,
    symbols: Vec<SymbolRow>,
    callers: Vec<CallerRow>,
    imports: Vec<ImportRow>,
    pattern_nodes: Vec<ast_sgrep_lang::PatternNode>,
    semantic_chunks: Vec<crate::semantic_chunk::SemanticChunkInput>,
}
#[allow(clippy::large_enum_variant)]
enum PrepareOutcome {
    Unchanged,
    Filtered,
    Failed(String),
    Ready(PreparedFile),
}
/// Hash with trailing blank/line-comment trivia removed. Equal ⇒ structure unchanged for trailing edits.
fn body_structure_hash(content: &str, language: Option<Language>) -> String {
    let mut end = content.len();
    let bytes = content.as_bytes();
    while end > 0 {
        while end > 0 && (bytes[end - 1] == b'\n' || bytes[end - 1] == b'\r') {
            end -= 1;
        }
        if end == 0 {
            break;
        }
        let line_start = content[..end].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = content[line_start..end].trim();
        if !is_trailing_trivia_line(line, language) {
            break;
        }
        end = line_start;
        if end > 0 && bytes[end - 1] == b'\n' {
            end -= 1;
        }
        if end > 0 && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    let mut h = Hasher::new();
    h.update(&bytes[..end]);
    h.finalize().to_hex().to_string()
}

/// Table-driven trailing trivia: hash-style vs C-family line/block comment prefixes.
fn is_trailing_trivia_line(line: &str, language: Option<Language>) -> bool {
    if line.is_empty() {
        return true;
    }
    const HASH_PREFIXES: &[&str] = &["#"];
    const C_FAMILY_PREFIXES: &[&str] = &["//", "/*", "*"];
    let prefixes: &[&str] = match language {
        Some(Language::Python | Language::Ruby) => HASH_PREFIXES,
        Some(
            Language::Rust
            | Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::Java
            | Language::CSharp
            | Language::Swift
            | Language::C
            | Language::Cpp
            | Language::Kotlin
            | Language::Php,
        ) => C_FAMILY_PREFIXES,
        None => return false,
    };
    prefixes.iter().any(|p| line.starts_with(p))
}

fn hash_content(content: &str) -> String {
    let mut h = Hasher::new();
    h.update(content.as_bytes());
    h.finalize().to_hex().to_string()
}

/// Shared prepare→upsert materialization: line split, body hash, optional semantic chunks.
struct UpsertMaterial {
    split: SplitLines,
    body_hash: String,
    semantic_chunks: Vec<crate::semantic_chunk::SemanticChunkInput>,
}

fn materialize_upsert(
    content: &str,
    language: Option<Language>,
    symbols: &[SymbolRow],
    callers: &[CallerRow],
    pattern_nodes: &[ast_sgrep_lang::PatternNode],
    embed_semantic: bool,
    body_hash: String,
) -> UpsertMaterial {
    let split = split_content_lines(content);
    let semantic_chunks = if embed_semantic {
        crate::semantic_chunk::build_semantic_chunks_with_patterns(
            symbols,
            callers,
            pattern_nodes,
            &split.lines,
            language.map(|l| l.as_str()),
        )
    } else {
        vec![]
    };
    UpsertMaterial {
        split,
        body_hash,
        semantic_chunks,
    }
}

/// Normalize a watcher path against a canonicalized index root.
fn normalize_watch_path(root: &Path, input_path: &Path) -> Option<PathBuf> {
    let candidate = if input_path.is_absolute() {
        input_path.to_path_buf()
    } else {
        root.join(input_path)
    };
    canonicalize_affected_path(&candidate)
        .ok()
        .filter(|canonical| canonical.starts_with(root))
}

/// Resolve the nearest existing ancestor without following the final path
/// component. This confines intermediate symlinks while preserving the indexed
/// key for a newly created or deleted file.
pub fn canonicalize_affected_path(path: &Path) -> std::io::Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return path.canonicalize();
    };
    let mut existing = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut suffix = vec![name.to_os_string()];
    loop {
        match existing.canonicalize() {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
            {
                let Some(name) = existing.file_name().map(ToOwned::to_owned) else {
                    return Err(error);
                };
                suffix.push(name);
                if !existing.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Guard predicate for watch updates: skip-dir components, skip-file policy, gitignore.
/// Callers still handle empty-rel / directory continues separately (those do not bump files_skipped).
fn should_skip_watch_path(
    abs: &Path,
    rel: &Path,
    respect_gitignore: bool,
    ignore: &crate::gitignore::IgnoreMatcher,
) -> bool {
    // Same short-circuit order as the former inline condition in `update_paths`.
    rel.components()
        .any(|c| should_skip_dir(Path::new(c.as_os_str())))
        || should_skip_file(abs)
        || (respect_gitignore && ignore.is_ignored(rel))
}

fn prepare_file(
    abs: &Path,
    rel: &str,
    current_hash: Option<&str>,
    options: &IndexOptions,
    root_dir: &crate::io_bounds::RootDir,
    semantic_identity_ok: bool,
    perf_run_id: Option<u64>,
) -> PrepareOutcome {
    let source =
        match root_dir.read_text_capped(Path::new(rel), crate::io_bounds::MAX_INDEX_FILE_BYTES) {
            Ok(source) => source,
            Err(error) => return PrepareOutcome::Failed(error.to_string()),
        };
    let mtime = source.metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
    let content = source.text;
    let hash = {
        let t0 = std::time::Instant::now();
        let h = hash_content(&content);
        if crate::perf_profile::enabled() {
            crate::perf_profile::record_sample_for(
                perf_run_id,
                "embed_hash",
                "index",
                "blake3 hash_content per file",
                t0.elapsed().as_micros() as u64,
                false,
            );
        }
        h
    };
    let language = detect_language(abs, Some(&content));
    if let Some(filter) = options.lang_filter.as_deref() {
        if language.is_none_or(|l| l.as_str() != filter) {
            return PrepareOutcome::Filtered;
        }
    }
    if !options.force_reindex && current_hash == Some(hash.as_str()) && semantic_identity_ok {
        return PrepareOutcome::Unchanged;
    }
    let (symbols, callers, imports, pattern_nodes) = match language {
        Some(lang) => {
            // One ParserRegistry per rayon worker — building all language parsers
            // on every file was pure fixed cost on the hot index path.
            thread_local! {
                static REGISTRY: ParserRegistry = ParserRegistry::new();
            }
            match REGISTRY.with(|registry| registry.parse(lang, &content)) {
                Ok(extraction) => rows_from_extraction(&extraction),
                Err(e) => {
                    return PrepareOutcome::Failed(format!(
                        "failed to parse {rel} as {}: {e}",
                        lang.as_str()
                    ))
                }
            }
        }
        None => (vec![], vec![], vec![], vec![]),
    };
    let material = materialize_upsert(
        &content,
        language,
        &symbols,
        &callers,
        &pattern_nodes,
        options.embed_semantic,
        body_structure_hash(&content, language),
    );
    PrepareOutcome::Ready(PreparedFile {
        hash,
        body_hash: material.body_hash,
        language: language.map(|l| l.as_str().to_string()),
        mtime_secs,
        mtime_nanos,
        lines: material.split.lines,
        eol: material.split.eol.to_string(),
        symbols,
        callers,
        imports,
        pattern_nodes,
        semantic_chunks: material.semantic_chunks,
    })
}
fn rows_from_extraction(extraction: &ExtractionResult) -> ExtractedRows {
    (
        extraction
            .symbols
            .iter()
            .map(|s| SymbolRow {
                name: s.name.clone(),
                kind: format!("{:?}", s.kind).to_lowercase(),
                line_start: s.line_start,
                line_end: s.line_end,
                byte_start: s.byte_start,
                byte_end: s.byte_end,
            })
            .collect(),
        extraction
            .calls
            .iter()
            .map(|c| CallerRow {
                caller: c.caller.clone(),
                callee: c.callee.clone(),
                line_no: c.line,
                byte_start: c.byte_start,
                byte_end: c.byte_end,
            })
            .collect(),
        extraction
            .imports
            .iter()
            .map(|i| ImportRow {
                module_path: i.module_path.clone(),
                line_no: i.line,
            })
            .collect(),
        extraction.pattern_nodes.clone(),
    )
}
fn should_prune_missing_files(walk_errors: bool) -> bool {
    !walk_errors
}
fn system_time_to_parts(time: SystemTime) -> (i64, u32) {
    let d = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    (d.as_secs() as i64, d.subsec_nanos())
}
#[cfg(test)]
mod tests {
    use super::should_prune_missing_files;
    #[test]
    fn walk_error_prevents_pruning_from_incomplete_seen_paths() {
        assert!(!should_prune_missing_files(true));
        assert!(should_prune_missing_files(false));
    }
}
#[cfg(test)]
mod body_hash_tests {
    use super::body_structure_hash;
    use ast_sgrep_lang::Language;

    #[test]
    fn trailing_comment_preserves_body_hash_for_its_language() {
        let a = "export function x() {\n  return 1;\n}\n";
        let js_comment = format!("{a}\n// sub1ms-bench-marker\n");
        assert_eq!(
            body_structure_hash(a, Some(Language::JavaScript)),
            body_structure_hash(&js_comment, Some(Language::JavaScript))
        );
        let hash_line = format!("{a}\n# not-a-javascript-comment\n");
        assert_ne!(
            body_structure_hash(a, Some(Language::JavaScript)),
            body_structure_hash(&hash_line, Some(Language::JavaScript))
        );
        assert_eq!(
            body_structure_hash(a, Some(Language::Python)),
            body_structure_hash(&hash_line, Some(Language::Python))
        );
    }
}
