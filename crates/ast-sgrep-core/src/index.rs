use crate::gitignore::{should_skip_dir, should_skip_file};
use crate::store::{
    CallerRow, ImportRow, IndexStore, RefreshLinesInput, SymbolRow, UpsertFileInput,
};
use crate::Result;
use ast_sgrep_lang::{detect_language, ExtractionResult, Language, ParserRegistry};
use blake3::Hasher;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;
/// Indexed relative paths must be valid UTF-8. Lossy conversion is forbidden:
/// two distinct non-UTF8 `OsStr` paths must not collide into one DB key.
pub fn indexed_rel_path(rel: &Path) -> Result<String> {
    let raw = rel.to_str().ok_or_else(|| {
        crate::StoreError::Other(format!(
            "non-UTF8 path rejected (asgrep-kqhp): {}",
            rel.display()
        ))
    })?;
    Ok(raw.replace('\\', "/"))
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
pub struct Indexer {
    store: IndexStore,
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
#[derive(Debug, Clone, Default)]
pub struct WatchUpdateStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub files_failed: usize,
}
impl Indexer {
    pub fn new(mut options: IndexOptions) -> Result<Self> {
        options.root = options.root.canonicalize().unwrap_or(options.root.clone());
        let store = IndexStore::open_with_durability(
            &options.root,
            options.index_path.as_deref(),
            options.durability,
        )?;
        store.set_meta("root", &options.root.display().to_string())?;
        let ignore = crate::gitignore::IgnoreMatcher::new(&options.root);
        Ok(Self {
            store,
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
        let _perf_run = crate::perf_profile::Run::start("index_all");
        self.ignore.clear();
        let (candidates, mut stats, prepared) = {
            let _span = crate::perf_profile::Span::start(
                "index_walk_parse",
                "index",
                "WalkDir + prepare_file (read/hash/tree-sitter extract)",
            );
            let (candidates, stats) = self.collect_index_candidates();
            let force = self.options.force_reindex;
            let lang_filter = self.options.lang_filter.clone();
            let embed_semantic = self.options.embed_semantic;
            // 28vo: the hash-only fast path must not skip when the stored semantic
            // identity (backend/model) differs from the active preference.
            let semantic_identity_ok =
                !embed_semantic || self.semantic_identity_matches()?;
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
                        force,
                        current_hash.as_deref(),
                        lang_filter.as_deref(),
                        embed_semantic,
                        semantic_identity_ok,
                    )
                })
                .collect();
            (candidates, stats, prepared)
        };
        let mut seen_paths = HashSet::new();
        let mut semantic_ivf_dirty = false;
        {
            let _span = crate::perf_profile::Span::start(
                "sqlite_upsert",
                "index",
                "bulk upsert_file transaction",
            );
            self.store.begin_bulk_tx()?;
            let write_result = (|| -> Result<()> {
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
                                semantic_ivf_dirty = true;
                            }
                        }
                    }
                }
                if should_prune_missing_files(stats.walk_errors) {
                    self.prune_missing_files(&seen_paths, &mut stats, &mut semantic_ivf_dirty)?;
                }
                Ok(())
            })();
            match write_result {
                Ok(()) => self.store.commit_bulk_tx()?,
                Err(e) => {
                    let _ = self.store.rollback_bulk_tx();
                    return Err(e);
                }
            }
        }
        self.rebuild_dirty_sidecars(&stats, semantic_ivf_dirty)?;
        // e2hc.13: a full index_all rewrites every reachable file, so a legacy
        // v1 store may now promote to v2 (persist_embed_metadata keeps v1
        // during partial updates to protect unrewritten siblings).
        if self.options.embed_semantic && self.store.needs_semantic_v1_rewrite()? {
            self.store.set_meta("embed_backend", "semantic-v2")?;
            if self.options.embed_backend == EmbedBackend::Auto {
                self.store.set_meta("embed_backend_pref", "auto")?;
            } else {
                self.store.delete_meta("embed_backend_pref")?;
            }
        }
        Ok(stats)
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
        self.store.clear_all_data()?;
        self.index_all()
    }
    pub fn update_paths(&mut self, paths: &[PathBuf]) -> Result<WatchUpdateStats> {
        // Single-file updates reuse the existing gitignore matcher.
        if paths.len() != 1 {
            self.ignore.clear();
        }
        let mut stats = WatchUpdateStats::default();
        for input_path in paths {
            let Some(abs) = normalize_watch_path(&self.options.root, input_path) else {
                continue;
            };
            let Ok(rel) = abs.strip_prefix(&self.options.root) else {
                continue;
            };
            if rel.as_os_str().is_empty() || abs.is_dir() {
                continue;
            }
            let rel_str = indexed_rel_path(rel)?;
            if rel
                .components()
                .any(|c| should_skip_dir(Path::new(c.as_os_str())))
                || should_skip_file(&abs)
                || (self.options.respect_gitignore && self.ignore.is_ignored(rel))
            {
                stats.files_skipped += 1;
                continue;
            }
            if abs.is_file() {
                match self.index_file(&abs, &rel_str) {
                    Ok(fs) if fs.skipped => stats.files_skipped += 1,
                    Ok(_) => {
                        stats.files_indexed += 1;
                        self.mark_sidecars_dirty()?;
                    }
                    Err(e) => {
                        eprintln!("[asgrep] failed to index {rel_str}: {e}");
                        stats.files_failed += 1;
                    }
                }
            } else if self.store.file_hash(&rel_str)?.is_some() {
                self.store.remove_file(&rel_str)?;
                stats.files_removed += 1;
                self.mark_sidecars_dirty()?;
            }
        }
        Ok(stats)
    }
    pub fn flush_deferred_rebuilds(&mut self) -> Result<()> {
        if self.sidecars_dirty.tantivy {
            self.rebuild_tantivy_sidecar()?;
            self.sidecars_dirty.tantivy = false;
        }
        if self.sidecars_dirty.semantic_ivf {
            self.rebuild_semantic_ivf_sidecar()?;
            self.sidecars_dirty.semantic_ivf = false;
        }
        Ok(())
    }
    pub fn deferred_rebuilds_pending(&self) -> bool {
        self.sidecars_dirty.tantivy || self.sidecars_dirty.semantic_ivf
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
        let metadata = fs::metadata(abs_path)?;
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
        let content = crate::io_bounds::read_text_capped(abs_path, crate::io_bounds::MAX_INDEX_FILE_BYTES)?;
        self.index_content_at(rel_path, &content, abs_path, mtime_secs, mtime_nanos)
    }
    pub fn index_content(&mut self, rel_path: &str, content: &str) -> Result<FileIndexStats> {
        let (mtime_secs, mtime_nanos) = system_time_to_parts(SystemTime::now());
        self.index_content_at(
            rel_path,
            content,
            Path::new(rel_path),
            mtime_secs,
            mtime_nanos,
        )
    }
    fn index_content_at(
        &mut self,
        rel_path: &str,
        content: &str,
        lang_path: &Path,
        mtime_secs: i64,
        mtime_nanos: u32,
    ) -> Result<FileIndexStats> {
        let hash = hash_content(content);
        if self.is_unchanged(rel_path, &hash)? {
            return Ok(FileIndexStats {
                skipped: true,
                ..Default::default()
            });
        }
        let language = detect_language(lang_path, Some(content));
        if !self.language_filter_allows(rel_path, language)? {
            return Ok(FileIndexStats::default());
        }
        let body_hash = body_structure_hash(content, language);
        let body_key = format!("body:{rel_path}");
        if !self.options.embed_semantic {
            if let Some(file_id) = self.store.file_id(rel_path)? {
                if self.store.get_meta(&body_key)?.as_deref() == Some(body_hash.as_str()) {
                    let split = split_content_lines(content);
                    self.store.begin_file_tx()?;
                    match self.store.refresh_lines_only(RefreshLinesInput {
                        file_id,
                        language: language.map(|l| l.as_str()),
                        mtime_secs,
                        mtime_nanos,
                        content_hash: &hash,
                        lines: &split.lines,
                        eol: split.eol,
                        rel_path,
                    }) {
                        Ok(_) => {
                            self.store.commit_file_tx()?;
                            return Ok(FileIndexStats::default());
                        }
                        Err(e) => {
                            self.store.rollback_file_tx()?;
                            return Err(e);
                        }
                    }
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
        Ok(FileIndexStats {
            symbols: symbols.len(),
            callers: callers.len(),
            imports: imports.len(),
            skipped: false,
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
    fn language_filter_allows(&self, rel_path: &str, language: Option<Language>) -> Result<bool> {
        let Some(lang_filter) = self.options.lang_filter.as_ref() else {
            return Ok(true);
        };
        if language.is_some_and(|lang| lang.as_str() == lang_filter.as_str()) {
            return Ok(true);
        }
        if self.store.file_hash(rel_path)?.is_some() {
            self.store.remove_file(rel_path)?;
        }
        Ok(false)
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
) -> UpsertMaterial {
    let split = split_content_lines(content);
    let body_hash = body_structure_hash(content, language);
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
    if input_path.starts_with(root) {
        return Some(input_path.to_path_buf());
    }
    input_path.canonicalize().ok().or_else(|| {
        let parent = input_path.parent()?.canonicalize().ok()?;
        Some(parent.join(input_path.file_name()?))
    })
}

fn prepare_file(
    abs: &Path,
    rel: &str,
    force: bool,
    current_hash: Option<&str>,
    lang_filter: Option<&str>,
    embed_semantic: bool,
    semantic_identity_ok: bool,
) -> PrepareOutcome {
    let metadata = match fs::metadata(abs) {
        Ok(m) => m,
        Err(e) => return PrepareOutcome::Failed(e.to_string()),
    };
    let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
    let content = match crate::io_bounds::read_text_capped(abs, crate::io_bounds::MAX_INDEX_FILE_BYTES)
    {
        Ok(c) => c,
        Err(e) => return PrepareOutcome::Failed(e.to_string()),
    };
    let hash = {
        let t0 = std::time::Instant::now();
        let h = hash_content(&content);
        if crate::perf_profile::enabled() {
            crate::perf_profile::record_sample(
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
    if let Some(filter) = lang_filter {
        if language.is_none_or(|l| l.as_str() != filter) {
            return PrepareOutcome::Filtered;
        }
    }
    if !force && current_hash == Some(hash.as_str()) && semantic_identity_ok {
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
        embed_semantic,
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
