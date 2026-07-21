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
            Self::Semantic => "semantic",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cloud" => Self::Cloud,
            "ollama" => Self::Ollama,
            "neural" | "fastembed" => Self::Neural,
            "semantic" | "local" => Self::Semantic,
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
        let store = IndexStore::open(&options.root, options.index_path.as_deref())?;
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
        self.ignore.clear();
        let mut stats = IndexStats::default();
        let mut seen_paths = HashSet::new();
        let mut semantic_ivf_dirty = false;
        let root = self.options.root.clone();
        let ignore = crate::gitignore::IgnoreMatcher::new(&root);
        let respect_gitignore = self.options.respect_gitignore;
        let mut candidates: Vec<(PathBuf, String)> = Vec::new();
        for entry in WalkDir::new(&self.options.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if should_skip_dir(e.path()) {
                    return false;
                }
                if respect_gitignore && e.file_type().is_dir() {
                    if let Ok(rel) = e.path().strip_prefix(&root) {
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
                    let Ok(rel) = path.strip_prefix(&self.options.root) else {
                        continue;
                    };
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if (self.options.respect_gitignore && self.ignore.is_ignored(rel))
                        || should_skip_file(&path)
                    {
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
        let force = self.options.force_reindex;
        let lang_filter = self.options.lang_filter.clone();
        let embed_semantic = self.options.embed_semantic;
        let prepared: Vec<PrepareOutcome> = candidates
            .par_iter()
            .map(|(abs, rel)| prepare_file(abs, rel, force, lang_filter.as_deref(), embed_semantic))
            .collect();
        self.store.begin_bulk_tx()?;
        let write_result = (|| -> Result<()> {
            for (rel_str, outcome) in candidates.iter().map(|(_, r)| r).zip(prepared) {
                match outcome {
                    PrepareOutcome::Filtered => {
                        if self.store.file_hash(rel_str)?.is_some() {
                            self.store.remove_file(rel_str)?;
                        }
                    }
                    PrepareOutcome::Failed(msg) => {
                        eprintln!("[asgrep] failed to index {rel_str}: {msg}");
                        stats.files_failed += 1;
                    }
                    PrepareOutcome::Ready(prep) => {
                        seen_paths.insert(rel_str.clone());
                        if self.is_unchanged(rel_str, &prep.hash)? {
                            stats.files_skipped += 1;
                            continue;
                        }
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
                        let _ = self
                            .store
                            .set_meta(&format!("body:{rel_str}"), &prep.body_hash);
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
        self.rebuild_dirty_sidecars(&stats, semantic_ivf_dirty)?;
        Ok(stats)
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
            self.store.remove_file(&path)?;
            stats.files_removed += 1;
            if self.options.embed_semantic {
                *semantic_ivf_dirty = true;
            }
        }
        Ok(())
    }
    fn rebuild_dirty_sidecars(&self, stats: &IndexStats, semantic_ivf_dirty: bool) -> Result<()> {
        if self.options.use_tantivy
            || crate::tantivy_index::should_use_tantivy(stats.files_indexed, false)
        {
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
            let _ = crate::semantic_ivf::invalidate_semantic_ivf(self.store.db_path());
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
        let lines = self.store.all_indexed_lines()?;
        crate::tantivy_index::TantivySidecar::open_for_index(
            &self.options.root,
            self.options.index_path.as_deref(),
        )?
        .rebuild_from_lines(&lines)
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
            // The root is canonicalized, while watcher paths may retain platform symlink prefixes.
            let normalized;
            let abs = if input_path.starts_with(&self.options.root) {
                input_path.as_path()
            } else {
                normalized = input_path.canonicalize().ok().or_else(|| {
                    let parent = input_path.parent()?.canonicalize().ok()?;
                    Some(parent.join(input_path.file_name()?))
                });
                let Some(path) = normalized.as_deref() else {
                    continue;
                };
                path
            };
            let Ok(rel) = abs.strip_prefix(&self.options.root) else {
                continue;
            };
            if rel.as_os_str().is_empty() || abs.is_dir() {
                continue;
            }
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if rel
                .components()
                .any(|c| should_skip_dir(Path::new(c.as_os_str())))
                || should_skip_file(abs)
                || (self.options.respect_gitignore && self.ignore.is_ignored(rel))
            {
                stats.files_skipped += 1;
                continue;
            }
            if abs.is_file() {
                match self.index_file(abs, &rel_str) {
                    Ok(fs) if fs.skipped => stats.files_skipped += 1,
                    Ok(_) => {
                        stats.files_indexed += 1;
                        self.mark_sidecars_dirty();
                    }
                    Err(e) => {
                        eprintln!("[asgrep] failed to index {rel_str}: {e}");
                        stats.files_failed += 1;
                    }
                }
            } else if self.store.file_hash(&rel_str)?.is_some() {
                self.store.remove_file(&rel_str)?;
                stats.files_removed += 1;
                self.mark_sidecars_dirty();
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
    fn mark_sidecars_dirty(&mut self) {
        if self.options.use_tantivy {
            self.sidecars_dirty.tantivy = true;
        }
        if self.options.embed_semantic {
            self.sidecars_dirty.semantic_ivf = true;
        }
    }
    pub fn index_file(&mut self, abs_path: &Path, rel_path: &str) -> Result<FileIndexStats> {
        let metadata = fs::metadata(abs_path)?;
        let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
        let content = match fs::read_to_string(abs_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                return Err(crate::StoreError::Other(format!("binary file: {rel_path}")));
            }
            Err(e) => return Err(e.into()),
        };
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
        let hash = {
            let mut h = Hasher::new();
            h.update(content.as_bytes());
            h.finalize().to_hex().to_string()
        };
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
        let split = split_content_lines(content);
        let body_hash = body_structure_hash(content);
        let body_key = format!("body:{rel_path}");
        if let Some(file_id) = self.store.file_id(rel_path)? {
            if self.store.get_meta(&body_key)?.as_deref() == Some(body_hash.as_str()) {
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
        let (symbols, callers, imports, pattern_nodes) =
            self.extract_rows(rel_path, content, language)?;
        let semantic_chunks = if self.options.embed_semantic {
            crate::semantic_chunk::build_semantic_chunks(&symbols, &callers, &split.lines)
        } else {
            vec![]
        };
        self.store.upsert_file(UpsertFileInput {
            rel_path,
            language: language.map(|l| l.as_str()),
            mtime_secs,
            mtime_nanos,
            content_hash: &hash,
            lines: &split.lines,
            eol: split.eol,
            symbols: &symbols,
            callers: &callers,
            imports: &imports,
            pattern_nodes: &pattern_nodes,
            semantic_chunks: &semantic_chunks,
            embed_semantic: self.options.embed_semantic,
            embed_backend: self.options.embed_backend.to_preference(),
        })?;
        let _ = self.store.set_meta(&body_key, &body_hash);
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
        if self.options.embed_semantic {
            let stored = self.store.get_meta("embed_backend")?;
            let active = self.options.embed_backend.to_preference_str();
            if stored.as_deref() != Some(active)
                && stored.as_deref() != Some("auto")
                && active != "auto"
            {
                return Ok(false);
            }
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
    Filtered,
    Failed(String),
    Ready(PreparedFile),
}
/// Hash with trailing blank/line-comment trivia removed. Equal ⇒ structure unchanged for trailing edits.
fn body_structure_hash(content: &str) -> String {
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
        let trivia = line.is_empty()
            || line.starts_with("//")
            || line.starts_with('#')
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with("--");
        if !trivia {
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
fn prepare_file(
    abs: &Path,
    rel: &str,
    _force: bool,
    lang_filter: Option<&str>,
    embed_semantic: bool,
) -> PrepareOutcome {
    let metadata = match fs::metadata(abs) {
        Ok(m) => m,
        Err(e) => return PrepareOutcome::Failed(e.to_string()),
    };
    let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let (mtime_secs, mtime_nanos) = system_time_to_parts(mtime);
    let content = match fs::read_to_string(abs) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            return PrepareOutcome::Failed(format!("binary file: {rel}"));
        }
        Err(e) => return PrepareOutcome::Failed(e.to_string()),
    };
    let mut hasher = Hasher::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize().to_hex().to_string();
    let language = detect_language(abs, Some(&content));
    if let Some(filter) = lang_filter {
        if language.is_none_or(|l| l.as_str() != filter) {
            return PrepareOutcome::Filtered;
        }
    }
    let split = split_content_lines(&content);
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
    let semantic_chunks = if embed_semantic {
        crate::semantic_chunk::build_semantic_chunks(&symbols, &callers, &split.lines)
    } else {
        vec![]
    };
    PrepareOutcome::Ready(PreparedFile {
        hash,
        body_hash: body_structure_hash(&content),
        language: language.map(|l| l.as_str().to_string()),
        mtime_secs,
        mtime_nanos,
        lines: split.lines,
        eol: split.eol.to_string(),
        symbols,
        callers,
        imports,
        pattern_nodes,
        semantic_chunks,
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
    #[test]
    fn trailing_comment_preserves_body_hash() {
        let a = "export function x() {\n  return 1;\n}\n";
        let b = format!("{a}\n// sub1ms-bench-marker\n");
        assert_eq!(body_structure_hash(a), body_structure_hash(&b));
    }
}
