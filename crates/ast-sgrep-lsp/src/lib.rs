//! LSP server backend for ast-sgrep.

use std::path::PathBuf;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use serde_json::{json, Value};

pub struct LspBackend {
    root: PathBuf,
    index_path: Option<PathBuf>,
}

impl LspBackend {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            index_path: None,
        }
    }

    pub fn ensure_index(&self) -> anyhow::Result<()> {
        let mut indexer = Indexer::new(IndexOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            lang_filter: None,
            respect_gitignore: true,
            use_tantivy: false,
        })?;
        indexer.index_all()?;
        Ok(())
    }

    pub fn workspace_symbols(&self, query: &str) -> anyhow::Result<Value> {
        if query.is_empty() {
            return Ok(json!([]));
        }
        let searcher = Searcher::new(SearchOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            limit: 50,
            lang_filter: None,
            use_embed: false,
            use_tantivy: false,
            use_cloud_embed: false,
        })?;
        let response = searcher.search(query)?;
        let symbols: Vec<Value> = response
            .hits
            .into_iter()
            .map(|hit| {
                json!({
                    "name": hit.symbol.or(hit.callee).unwrap_or_else(|| hit.excerpt.chars().take(40).collect::<String>()),
                    "kind": 12,
                    "location": {
                        "uri": path_to_uri(&self.root.join(&hit.file)),
                        "range": {
                            "start": { "line": hit.line_start.saturating_sub(1), "character": 0 },
                            "end": { "line": hit.line_end.saturating_sub(1), "character": 0 }
                        }
                    }
                })
            })
            .collect();
        Ok(Value::Array(symbols))
    }

    pub fn goto_definition(&self, symbol: &str) -> anyhow::Result<Value> {
        let searcher = Searcher::new(SearchOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            limit: 8,
            lang_filter: None,
            use_embed: false,
            use_tantivy: false,
            use_cloud_embed: false,
        })?;
        let response = searcher.search(&format!("defs:{symbol}"))?;
        let locations: Vec<Value> = response
            .hits
            .into_iter()
            .map(|hit| {
                json!({
                    "uri": path_to_uri(&self.root.join(&hit.file)),
                    "range": {
                        "start": { "line": hit.line_start.saturating_sub(1), "character": 0 },
                        "end": { "line": hit.line_end.saturating_sub(1), "character": 0 }
                    }
                })
            })
            .collect();
        Ok(if locations.len() == 1 {
            locations[0].clone()
        } else {
            json!(locations)
        })
    }

    pub fn find_references(&self, symbol: &str) -> anyhow::Result<Value> {
        let searcher = Searcher::new(SearchOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            limit: 64,
            lang_filter: None,
            use_embed: false,
            use_tantivy: false,
            use_cloud_embed: false,
        })?;
        let response = searcher.search(&format!("callers:{symbol}"))?;
        let refs: Vec<Value> = response
            .hits
            .into_iter()
            .map(|hit| {
                json!({
                    "uri": path_to_uri(&self.root.join(&hit.file)),
                    "range": {
                        "start": { "line": hit.line_start.saturating_sub(1), "character": 0 },
                        "end": { "line": hit.line_end.saturating_sub(1), "character": 0 }
                    }
                })
            })
            .collect();
        Ok(Value::Array(refs))
    }
}

fn path_to_uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
}
