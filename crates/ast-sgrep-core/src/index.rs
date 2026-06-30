use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ast_sgrep_lang::{detect_language, ExtractionResult, ParserRegistry};
use blake3::Hasher;
use walkdir::WalkDir;

use crate::skip::{should_skip_dir, should_skip_file};
use crate::store::{CallerRow, ImportRow, IndexStore, SymbolRow};
use crate::text::split_content_lines;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedBackend {
    #[default]
    Auto,
    Cloud,
    Ollama,
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

    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cloud" => EmbedBackend::Cloud,
            "ollama" => EmbedBackend::Ollama,
            "semantic" | "local" => EmbedBackend::Semantic,
            _ => EmbedBackend::Auto,
        }
    }

    pub fn from_flags(cloud: bool, ollama: bool, semantic_only: bool) -> Self {
        if cloud {
            EmbedBackend::Cloud
        } else if ollama {
            EmbedBackend::Ollama
        } else if semantic_only {
            EmbedBackend::Semantic
        } else {
            EmbedBackend::Auto
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
}

impl Indexer {
    pub fn new(mut options: IndexOptions) -> Result<Self> {
        options.root = options
            .root
            .canonicalize()
            .unwrap_or(options.root.clone());
        let store = IndexStore::open(&options.root, options.index_path.as_deref())?;
        store.set_meta("root", &options.root.display().to_string())?;
        Ok(Self {
            store,
            parsers: ParserRegistry::new(),
            options,
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

            if self.options.respect_gitignore && crate::gitignore::is_ignored(&self.options.root, rel) {
                stats.files_skipped += 1;
                continue;
            }

            seen_paths.insert(rel_str.clone());

            if should_skip_file(path) {
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
            rows_from_extraction(&extraction)
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
}

fn hash_content(content: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(content.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn rows_from_extraction(
    extraction: &ExtractionResult,
) -> (Vec<SymbolRow>, Vec<CallerRow>, Vec<ImportRow>) {
    let symbols = extraction
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
    let callers = extraction
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
    let imports = extraction
        .imports
        .iter()
        .map(|i| ImportRow {
            module_path: i.module_path.clone(),
            line_no: i.line,
        })
        .collect();
    (symbols, callers, imports)
}

fn system_time_to_parts(time: SystemTime) -> (i64, u32) {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    (duration.as_secs() as i64, duration.subsec_nanos())
}