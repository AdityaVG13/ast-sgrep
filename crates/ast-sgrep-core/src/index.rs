use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::index_prepare::{
    body_structure_hash, hash_content, materialize_upsert, prepare_file, rows_from_extraction,
    should_prune_missing_files, system_time_to_parts, ExtractedRows, PrepareOutcome,
};
use crate::index_recovery::recover_corrupt_index;
use crate::index_watch::{normalize_watch_path, should_skip_watch_path};
use crate::store::{IndexStore, RefreshLinesInput, UpsertFileInput};
use crate::Result;
use ast_sgrep_lang::{detect_language, Language, ParserRegistry};
use rayon::prelude::*;
use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

pub use crate::index_watch::canonicalize_affected_path;

thread_local! {
    /// Test-only: when set, [`Indexer::rebuild_dirty_sidecars`] returns Err after the
    /// bulk SQLite commit so callers can pin Err-path cache invalidation.
    /// Thread-local so parallel `cargo test` workers do not cross-contaminate.
    static FORCE_SIDECAR_REBUILD_ERR: Cell<bool> = const { Cell::new(false) };
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

pub(crate) fn open_index_store(options: &IndexOptions) -> Result<IndexStore> {
    IndexStore::open_with_durability(
        &options.root,
        options.index_path.as_deref(),
        options.durability,
    )
}

pub(crate) fn quick_check(store: &IndexStore) -> Result<String> {
    Ok(store
        .connection()
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?)
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
        // Isolate `?` so a later path error cannot skip the writer stamp after
        // earlier paths already committed (watch batches / multi-path CLI).
        let result = (|| -> Result<()> {
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
                    if should_skip_watch_path(
                        &abs,
                        rel,
                        self.options.respect_gitignore,
                        &self.ignore,
                    ) {
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
            Ok(())
        })();
        if changed {
            // Advertise even when `result` is Err: earlier paths may already be
            // durable. Prefer peer reopen over silencing the stamp on mark failure.
            let mark = self.mark_sidecars_dirty();
            self.advertise_writer_generation();
            mark?;
        }
        result?;
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

    /// Bump the cross-process writer stamp after a durable index mutation.
    ///
    /// Fail-open by contract: stamp I/O must never fail the index once SQLite
    /// has committed. A missed bump only delays peer Searcher reopen; failing
    /// the command would report error after durable rows are already visible.
    /// See `docs/index-consistency.md` (writer-generation fail-open).
    fn advertise_writer_generation(&self) {
        let stamp = crate::store::writer_generation_path(
            &self.options.root,
            self.options.index_path.as_deref(),
        );
        if let Err(error) = crate::store::bump_writer_generation(
            &self.options.root,
            self.options.index_path.as_deref(),
        ) {
            eprintln!(
                "asgrep: writer_generation stamp skipped (index commit already durable; path {}): {error}",
                stamp.display()
            );
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

#[cfg(test)]
#[path = "../../../tests/unit/core/index.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/core/index__body_hash_tests.rs"]
mod body_hash_tests;
