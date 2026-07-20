use std::path::{Path, PathBuf}; use std::sync::{Arc, Mutex};

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher}; use serde_json::Value;

use crate::settings::AsgrepSettings;

/// Minimal search-capable LSP backend (index + search only).
pub struct LspBackend {
    root: PathBuf, index_path: Option<PathBuf>, settings: AsgrepSettings, index_lock: Arc<Mutex<()>>,
}

impl LspBackend {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: root.canonicalize().unwrap_or(root), index_path: None, settings: AsgrepSettings::default(), index_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn apply_settings(&mut self, settings: AsgrepSettings) {
        if let Some(ref p) = settings.index_path { self.index_path = Some(PathBuf::from(p)); } self.settings = settings;
    }

    pub fn root(&self) -> &Path { &self.root }

    pub fn set_index_path(&mut self, path: PathBuf) { self.index_path = Some(path); }

    fn index_options(&self) -> IndexOptions {
        let mut opts = IndexOptions {
            root: self.root.clone(), index_path: self.index_path.clone(), ..IndexOptions::default()
        }; self.settings.apply_to_index_options(&mut opts); opts
    }

    fn search_options(&self, limit: usize) -> SearchOptions {
        let mut opts = SearchOptions {
            root: self.root.clone(), index_path: self.index_path.clone(), limit, ..SearchOptions::default()
        }; self.settings.apply_to_search_options(&mut opts); opts
    }

    pub fn ensure_index(&self) -> anyhow::Result<()> {
        let _g = self
            .index_lock .lock() .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        Indexer::new(self.index_options())?.index_all()?; Ok(())
    }

    pub fn search(&self, query: &str, semantic: bool, limit: usize) -> anyhow::Result<Value> {
        let _g = self
            .index_lock .lock() .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        let searcher = Searcher::new(self.search_options(limit))?; Ok(serde_json::to_value(if semantic {
            searcher.search_semantic(query)?
        } else {
            searcher.search(query)?
        })?)
    }
}
