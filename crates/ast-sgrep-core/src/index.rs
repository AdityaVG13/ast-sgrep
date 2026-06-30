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
        }
    }
}

/// Statistics from an indexing run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub symbols_extracted: usize,
    pub callers_extracted: usize,
    pub imports_extracted: usize,
}

/// Indexes source files into the SQLite store.
pub struct Indexer {
    store: IndexStore,
    parsers: ParserRegistry,
    options: IndexOptions,
    ignore_patterns: Vec<String>,
}

impl Indexer {
    pub fn new(options: IndexOptions) -> Result<Self> {
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

        for entry in WalkDir::new(&self.options.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !should_skip_dir(e.path()))
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
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
                    if file_stats.0 == 0 && file_stats.1 == 0 && file_stats.2 == 0 {
                        // might be unchanged skip - still count if file exists in index
                    }
                    stats.files_indexed += 1;
                    stats.symbols_extracted += file_stats.0;
                    stats.callers_extracted += file_stats.1;
                    stats.imports_extracted += file_stats.2;
                }
                Err(_) => {
                    stats.files_skipped += 1;
                }
            }
        }

        let indexed = self.store.all_file_paths()?;
        for path in indexed {
            if !seen_paths.contains(&path) {
                self.store.remove_file(&path)?;
                stats.files_removed += 1;
            }
        }

        if self.options.use_tantivy
            || crate::tantivy_index::should_use_tantivy(stats.files_indexed, false)
        {
            self.rebuild_tantivy_sidecar()?;
        }

        Ok(stats)
    }

    fn rebuild_tantivy_sidecar(&self) -> Result<()> {
        let lines = self.store.all_indexed_lines()?;
        let sidecar = crate::tantivy_index::TantivySidecar::open(&self.options.root)?;
        sidecar.rebuild_from_lines(&lines)
    }

    pub fn reindex_all(&mut self) -> Result<IndexStats> {
        let prev = self.options.force_reindex;
        self.options.force_reindex = true;
        let stats = self.index_all();
        self.options.force_reindex = prev;
        stats
    }

    pub fn index_file(&mut self, abs_path: &Path, rel_path: &str) -> Result<(usize, usize, usize)> {
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
    pub fn index_content(&mut self, rel_path: &str, content: &str) -> Result<(usize, usize, usize)> {
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
    ) -> Result<(usize, usize, usize)> {
        let hash = hash_content(content);

        if !self.options.force_reindex {
            if let Some(stored_hash) = self.store.file_hash(rel_path)? {
                if stored_hash == hash {
                    if let Some((s, n)) = self.store.file_mtime(rel_path)? {
                        if s == mtime_secs && n == mtime_nanos {
                            return Ok((0, 0, 0));
                        }
                    }
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
                    return Ok((0, 0, 0));
                }
            }
        }

        let lines: Vec<(u32, String)> = content
            .lines()
            .enumerate()
            .map(|(i, line)| ((i + 1) as u32, line.to_string()))
            .collect();

        let (symbols, callers, imports) = if let Some(lang) = language {
            match self.parsers.parse(lang, content) {
                Ok(extraction) => {
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
                }
                Err(_) => (Vec::new(), Vec::new(), Vec::new()),
            }
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
            &lines,
            &symbols,
            &callers,
            &imports,
            &semantic_chunks,
            self.options.embed_semantic,
            self.options.embed_backend.to_preference(),
        )?;

        Ok((sym_count, caller_count, import_count))
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
    for pattern in patterns {
        let pat = pattern.trim_end_matches('/');
        if pat.contains('*') {
            if simple_glob_match(pat, &rel_str) {
                return true;
            }
        } else if rel_str == pat || rel_str.starts_with(&format!("{pat}/")) {
            return true;
        }
    }
    false
}

fn simple_glob_match(pattern: &str, text: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        return text.starts_with(prefix)
            || text
                .split('/')
                .any(|segment| segment.starts_with(prefix));
    }
    pattern == text
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