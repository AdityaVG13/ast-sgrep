use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ast_sgrep_lang::{detect_language, ParserRegistry};
use blake3::Hasher;
use walkdir::WalkDir;

use crate::store::{CallerRow, ImportRow, IndexStore, SymbolRow};
use crate::Result;

/// Embedding backend preference for symbol-level semantic chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedBackend {
    /// Cloud API → Ollama → semantic local (default).
    #[default]
    Auto,
    Cloud,
    Ollama,
    /// Offline code-aware semantic embeddings only.
    Semantic,
}

impl EmbedBackend {
    pub fn to_preference(self) -> ast_sgrep_embed::EmbedPreference {
        match self {
            EmbedBackend::Auto => ast_sgrep_embed::EmbedPreference::Auto,
            EmbedBackend::Cloud => ast_sgrep_embed::EmbedPreference::Cloud,
            EmbedBackend::Ollama => ast_sgrep_embed::EmbedPreference::Ollama,
            EmbedBackend::Semantic => ast_sgrep_embed::EmbedPreference::Semantic,
        }
    }
}

/// Options for indexing a repository.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub lang_filter: Option<String>,
    pub respect_gitignore: bool,
    pub use_tantivy: bool,
    /// Index symbol-level semantic chunks (default on).
    pub embed_semantic: bool,
    pub embed_backend: EmbedBackend,
    pub force_reindex: bool,
    /// Override ANN threshold (`ASGREP_ANN_THRESHOLD` when unset).
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

/// Statistics from an indexing run.
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

/// Per-file indexing outcome.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileIndexStats {
    pub symbols: usize,
    pub callers: usize,
    pub imports: usize,
    pub skipped: bool,
}

/// Indexes source files into the SQLite store.
pub struct Indexer {
    store: IndexStore,
    parsers: ParserRegistry,
    options: IndexOptions,
    ignore_patterns: Vec<String>,
}

impl Indexer {
    pub fn new(mut options: IndexOptions) -> Result<Self> {
        options.root = options
            .root
            .canonicalize()
            .unwrap_or(options.root.clone());
        let store = IndexStore::open(&options.root, options.index_path.as_deref())?;
        store.set_meta("root", &options.root.display().to_string())?;
        let ignore_patterns = if options.respect_gitignore {
            load_ignore_patterns(&options.root)
        } else {
            Vec::new()
        };
        Ok(Self {
            store,
            parsers: ParserRegistry::new(),
            options,
            ignore_patterns,
        })
    }

    pub fn store(&self) -> &IndexStore {
        &self.store
    }

    pub fn index_all(&mut self) -> Result<IndexStats> {
        let mut stats = IndexStats::default();
        let mut seen_paths = HashSet::new();
        let mut walk_errors = false;

        for entry in WalkDir::new(&self.options.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !should_skip_dir(e.path()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[asgrep] walk error: {e}");
                    walk_errors = true;
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let rel = match path.strip_prefix(&self.options.root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");

            if is_ignored(rel, &self.ignore_patterns) {
                stats.files_skipped += 1;
                continue;
            }

            seen_paths.insert(rel_str.clone());

            if self.should_skip_file(path) {
                stats.files_skipped += 1;
                continue;
            }

            match self.index_file(path, &rel_str) {
                Ok(file_stats) => {
                    if file_stats.skipped {
                        stats.files_skipped += 1;
                    } else {
                        stats.files_indexed += 1;
                        stats.symbols_extracted += file_stats.symbols;
                        stats.callers_extracted += file_stats.callers;
                        stats.imports_extracted += file_stats.imports;
                    }
                }
                Err(e) => {
                    eprintln!("[asgrep] failed to index {rel_str}: {e}");
                    stats.files_failed += 1;
                }
            }
        }

        stats.walk_errors = walk_errors;
        if !walk_errors {
            let indexed = self.store.all_file_paths()?;
            for path in indexed {
                if !seen_paths.contains(&path) {
                    self.store.remove_file(&path)?;
                    stats.files_removed += 1;
                }
            }
        }

        if self.options.use_tantivy
            || crate::tantivy_index::should_use_tantivy(stats.files_indexed, false)
        {
            self.rebuild_tantivy_sidecar()?;
        }

        if self.options.embed_semantic {
            self.rebuild_semantic_ivf_sidecar()?;
        }

        Ok(stats)
    }

    fn rebuild_semantic_ivf_sidecar(&self) -> Result<()> {
        let chunks = self.store.all_semantic_chunks(None)?;
        crate::semantic_ann::rebuild_semantic_ivf_sidecar(
            self.store(),
            &chunks,
            self.options.ann_threshold,
        )
    }

    fn rebuild_tantivy_sidecar(&self) -> Result<()> {
        let lines = self.store.all_indexed_lines()?;
        let sidecar = crate::tantivy_index::TantivySidecar::open_for_index(
            &self.options.root,
            self.options.index_path.as_deref(),
        )?;
        sidecar.rebuild_from_lines(&lines)
    }

    pub fn reindex_all(&mut self) -> Result<IndexStats> {
        let prev = self.options.force_reindex;
        self.options.force_reindex = true;
        let stats = self.index_all();
        self.options.force_reindex = prev;
        stats
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

    /// Index in-memory content (LSP `didChange` / unsaved buffers).
    pub fn index_content(&mut self, rel_path: &str, content: &str) -> Result<FileIndexStats> {
        let now = SystemTime::now();
        let (mtime_secs, mtime_nanos) = system_time_to_parts(now);
        self.index_content_at(rel_path, content, Path::new(rel_path), mtime_secs, mtime_nanos)
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

        if !self.options.force_reindex {
            if let Some(stored_hash) = self.store.file_hash(rel_path)? {
                if stored_hash == hash {
                    return Ok(FileIndexStats {
                        skipped: true,
                        ..Default::default()
                    });
                }
            }
        }

        let language = detect_language(lang_path, Some(content));
        if let Some(ref lang_filter) = self.options.lang_filter {
            match language {
                Some(lang) if lang.as_str() == lang_filter.as_str() => {}
                _ => {
                    if self.store.file_hash(rel_path)?.is_some() {
                        self.store.remove_file(rel_path)?;
                    }
                    return Ok(FileIndexStats::default());
                }
            }
        }

        let split = split_content_lines(content);
        let lines = &split.lines;

        let (symbols, callers, imports) = if let Some(lang) = language {
            let extraction = self.parsers.parse(lang, content).map_err(|e| {
                crate::StoreError::Other(format!(
                    "failed to parse {rel_path} as {}: {e}",
                    lang.as_str()
                ))
            })?;
            let symbols: Vec<SymbolRow> = extraction
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
                .collect();
            let callers: Vec<CallerRow> = extraction
                .calls
                .iter()
                .map(|c| CallerRow {
                    caller: c.caller.clone(),
                    callee: c.callee.clone(),
                    line_no: c.line,
                    byte_start: c.byte_start,
                    byte_end: c.byte_end,
                })
                .collect();
            let imports: Vec<ImportRow> = extraction
                .imports
                .iter()
                .map(|i| ImportRow {
                    module_path: i.module_path.clone(),
                    line_no: i.line,
                })
                .collect();
            (symbols, callers, imports)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };

        let sym_count = symbols.len();
        let caller_count = callers.len();
        let import_count = imports.len();

        let semantic_chunks = if self.options.embed_semantic {
            crate::semantic_chunk::build_semantic_chunks(&symbols, &callers, &lines)
        } else {
            Vec::new()
        };

        self.store.upsert_file(
            rel_path,
            language.map(|l| l.as_str()),
            mtime_secs,
            mtime_nanos,
            &hash,
            lines,
            split.eol,
            &symbols,
            &callers,
            &imports,
            &semantic_chunks,
            self.options.embed_semantic,
            self.options.embed_backend.to_preference(),
        )?;

        Ok(FileIndexStats {
            symbols: sym_count,
            callers: caller_count,
            imports: import_count,
            skipped: false,
        })
    }

    fn should_skip_file(&self, path: &Path) -> bool {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                return true;
            }
        }

        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_lowercase();
            !matches!(
                ext.as_str(),
                "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "pyi" | "go"
                    | "java" | "cs" | "rb"
                    | "toml" | "md" | "txt" | "json" | "yaml" | "yml"
            )
        } else {
            true
        }
    }
}

#[derive(Debug, Clone)]
struct SplitLines {
    lines: Vec<(u32, String)>,
    eol: &'static str,
}

fn split_content_lines(content: &str) -> SplitLines {
    if content.is_empty() {
        return SplitLines {
            lines: vec![(1, String::new())],
            eol: "lf",
        };
    }
    let eol = if content.contains("\r\n") {
        "crlf"
    } else {
        "lf"
    };
    let lines = content
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            let stripped = line.strip_suffix('\r').unwrap_or(line);
            ((i + 1) as u32, stripped.to_string())
        })
        .collect();
    SplitLines { lines, eol }
}

fn hash_content(content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(content.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn system_time_to_parts(time: SystemTime) -> (i64, u32) {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    (duration.as_secs() as i64, duration.subsec_nanos())
}

fn should_skip_dir(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        matches!(
            name,
            ".git" | ".asgrep" | "target" | "node_modules" | "dist" | "build" | ".cargo"
        )
    } else {
        false
    }
}

fn load_ignore_patterns(root: &Path) -> Vec<String> {
    let mut patterns = vec![
        "target/".to_string(),
        "node_modules/".to_string(),
        ".git/".to_string(),
        ".asgrep/".to_string(),
    ];

    for name in [".gitignore", ".asgrepignore"] {
        let path = root.join(name);
        if let Ok(content) = fs::read_to_string(path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                patterns.push(line.to_string());
            }
        }
    }

    patterns
}

fn is_ignored(rel: &Path, patterns: &[String]) -> bool {
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let mut ignored = false;
    for pattern in patterns {
        let pat = pattern.trim();
        if pat.is_empty() {
            continue;
        }
        let (negate, glob_pat) = if let Some(rest) = pat.strip_prefix('!') {
            (true, rest.trim())
        } else {
            (false, pat)
        };
        if glob_matches(glob_pat, &rel_str) {
            ignored = !negate;
        }
    }
    ignored
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let pat = pattern.trim_end_matches('/');
    if pat.contains("**/") {
        if let Some(rest) = pat.split("**/").nth(1) {
            return glob_matches(rest, text)
                || text.split('/').any(|seg| glob_matches(rest, seg));
        }
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        return text.ends_with(suffix) || text.split('/').any(|seg| seg.ends_with(suffix));
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        return text.starts_with(prefix)
            || text.split('/').any(|seg| seg.starts_with(prefix));
    }
    text == pat || text.starts_with(&format!("{pat}/"))
}

/// Load additional ignore patterns from .asgrepignore
pub fn load_asgrepignore(root: &Path) -> Vec<String> {
    let path = root.join(".asgrepignore");
    if let Ok(content) = fs::read_to_string(path) {
        content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod glob_tests {
    use std::path::Path;

    use super::{glob_matches, is_ignored, split_content_lines};

    #[test]
    fn star_suffix_matches_extension() {
        assert!(glob_matches("*.pyc", "foo/bar.pyc"));
        assert!(!glob_matches("*.pyc", "foo/bar.py"));
    }

    #[test]
    fn double_star_prefix() {
        assert!(glob_matches("**/*.log", "deep/nested/app.log"));
    }

    #[test]
    fn gitignore_negation_unignores_matching_files() {
        let patterns = vec!["*.log".into(), "!important.log".into()];
        assert!(is_ignored(Path::new("app.log"), &patterns));
        assert!(!is_ignored(Path::new("important.log"), &patterns));
    }

    #[test]
    fn crlf_lines_strip_carriage_return_and_record_eol() {
        let split = split_content_lines("a\r\nb\r\n");
        assert_eq!(split.eol, "crlf");
        assert_eq!(split.lines, vec![(1, "a".into()), (2, "b".into()), (3, "".into())]);
    }
}