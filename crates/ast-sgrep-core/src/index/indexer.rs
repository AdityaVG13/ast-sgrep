use std::collections::HashSet;

use ast_sgrep_lang::ParserRegistry;
use walkdir::WalkDir;

use super::types::{IndexOptions, IndexStats};
use crate::skip::{should_skip_dir, should_skip_file};
use crate::store::IndexStore;
use crate::Result;

/// Indexes source files into the SQLite store.
pub struct Indexer {
    store: IndexStore,
    pub(super) parsers: ParserRegistry,
    pub(super) options: IndexOptions,
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
}
