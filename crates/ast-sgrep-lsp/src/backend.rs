//! Full LSP backend backed by ast-sgrep index.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use serde_json::{json, Value};

use crate::convert::{
    call_hierarchy_endpoint, line_range, line_range_ext, location_value, workspace_symbol,
};
use crate::settings::AsgrepSettings;
use crate::symbols::{innermost_symbol, line_at_index};
use crate::text_edit::{apply_text_edit, extract_identifier_at, utf16_char_to_byte};
use crate::uri::{self};

pub use uri::{path_to_file_uri as path_to_uri, uri_to_rel_path};

use crate::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams,
    TextDocumentContentChangeEvent, SYMBOL_KIND_FUNCTION, TextDocumentPositionParams,
};

pub struct LspBackend {
    root: PathBuf,
    index_path: Option<PathBuf>,
    settings: AsgrepSettings,
    index_ready: Arc<AtomicBool>,
    index_thread: Option<JoinHandle<()>>,
    index_lock: Arc<Mutex<()>>,
}

impl LspBackend {
    pub fn new(root: PathBuf) -> Self {
        let root = crate::uri::canonicalize_workspace_root(root);
        Self {
            root,
            index_path: None,
            settings: AsgrepSettings::default(),
            index_ready: Arc::new(AtomicBool::new(false)),
            index_thread: None,
            index_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn apply_settings(&mut self, settings: AsgrepSettings) {
        if let Some(ref p) = settings.index_path {
            self.index_path = Some(PathBuf::from(p));
        }
        self.settings = settings;
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn set_index_path(&mut self, path: PathBuf) {
        self.index_path = Some(path);
    }

    pub fn is_index_ready(&self) -> bool {
        self.index_ready.load(Ordering::SeqCst)
    }

    fn index_options(&self) -> IndexOptions {
        let mut opts = IndexOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            ..IndexOptions::default()
        };
        self.settings.apply_to_index_options(&mut opts);
        opts
    }

    fn search_options(&self, limit: usize) -> SearchOptions {
        let mut opts = SearchOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            limit,
            ..SearchOptions::default()
        };
        self.settings.apply_to_search_options(&mut opts);
        opts
    }

    fn index_guard(&self) -> anyhow::Result<MutexGuard<'_, ()>> {
        self.index_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))
    }

    fn with_locked_indexer<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Indexer) -> anyhow::Result<T>,
    {
        let _guard = self.index_guard()?;
        let mut indexer = Indexer::new(self.index_options())?;
        f(&mut indexer)
    }

    fn with_store<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&ast_sgrep_core::IndexStore) -> anyhow::Result<T>,
    {
        let indexer = Indexer::new(self.index_options())?;
        f(indexer.store())
    }

    /// Start full-workspace indexing on a background thread (non-blocking).
    pub fn start_background_index(&mut self) {
        if self.index_thread.is_some() {
            return;
        }
        let opts = self.index_options();
        let ready = Arc::clone(&self.index_ready);
        let lock = Arc::clone(&self.index_lock);
        self.index_thread = Some(std::thread::spawn(move || {
            let _guard = match lock.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let ok = Indexer::new(opts)
                .and_then(|mut indexer| indexer.index_all().map(|_| ()))
                .is_ok();
            if !ok {
                crate::server::log("background index failed");
            }
            if ok {
                ready.store(true, Ordering::SeqCst);
            }
        }));
    }

    pub fn ensure_index(&self) -> anyhow::Result<()> {
        self.with_locked_indexer(|indexer| {
            indexer.index_all()?;
            Ok(())
        })
    }

    pub fn reindex_file(&self, rel_path: &str) -> anyhow::Result<()> {
        self.with_locked_indexer(|indexer| {
            let abs = self.root.join(rel_path);
            if abs.is_file() {
                indexer.index_file(&abs, rel_path)?;
            }
            Ok(())
        })
    }

    pub fn index_content(&self, rel_path: &str, content: &str) -> anyhow::Result<()> {
        self.with_locked_indexer(|indexer| {
            indexer.index_content(rel_path, content)?;
            Ok(())
        })
    }

    pub fn apply_document_changes(
        &self,
        uri: &str,
        changes: &[TextDocumentContentChangeEvent],
    ) -> anyhow::Result<()> {
        self.with_locked_indexer(|indexer| {
            let rel = uri_to_rel_path(uri, &self.root)?;
            let mut content = indexer
                .store()
                .file_text(&rel)?
                .or_else(|| std::fs::read_to_string(self.root.join(&rel)).ok())
                .unwrap_or_default();

            for change in changes {
                content = if change.range.is_some() {
                    apply_text_edit(&content, change)
                } else {
                    change.text.clone()
                };
            }

            indexer.index_content(&rel, &content)?;
            Ok(())
        })
    }

    pub fn initialize_result(&self) -> Value {
        json!({
            "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 2, "save": { "includeText": false } },
                "workspaceSymbolProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "documentSymbolProvider": true,
                "callHierarchyProvider": true,
                "executeCommandProvider": {
                    "commands": [
                        "asgrep.search",
                        "asgrep.search.semantic",
                        "asgrep.reindex",
                        "asgrep.callers",
                        "asgrep.defs"
                    ]
                }
            },
            "serverInfo": {
                "name": "asgrep-lsp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })
    }

    pub fn workspace_symbols(&self, query: &str) -> anyhow::Result<Value> {
        if query.is_empty() {
            return Ok(json!([]));
        }
        if self.index_thread.is_some() && !self.is_index_ready() {
            return Ok(json!([]));
        }
        let searcher = Searcher::new(self.search_options(50))?;
        let response = searcher.search(query)?;
        Ok(Value::Array(
            response
                .hits
                .into_iter()
                .filter_map(|hit| workspace_symbol(&self.root, &hit.file, &hit))
                .collect(),
        ))
    }

    pub fn document_symbols(&self, params: &DocumentSymbolParams) -> anyhow::Result<Value> {
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        self.with_store(|store| {
            let symbols = store.symbols_in_file(&rel)?;
            let items: Vec<Value> = symbols
                .iter()
                .map(|sym| {
                    let kind = if sym.kind.contains("method") {
                        crate::types::SYMBOL_KIND_METHOD
                    } else {
                        SYMBOL_KIND_FUNCTION
                    };
                    let end_line = store.line_content(&rel, sym.line_end).ok().flatten();
                    json!({
                        "name": sym.name,
                        "kind": kind,
                        "range": line_range_ext(sym.line_start, sym.line_end, end_line.as_deref()),
                        "selectionRange": line_range(sym.line_start, sym.line_start),
                        "detail": sym.kind
                    })
                })
                .collect();
            Ok(Value::Array(items))
        })
    }

    pub fn goto_definition(&self, params: &TextDocumentPositionParams) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(params)?;
        let searcher = Searcher::new(self.search_options(16))?;
        let response = searcher.search(&format!("defs:{symbol}"))?;
        let locations: Vec<Value> = response
            .hits
            .iter()
            .map(|hit| location_value(&self.root, &hit.file, hit.line_start, hit.line_end))
            .collect();
        Ok(match locations.len() {
            0 => Value::Null,
            1 => locations.into_iter().next().unwrap_or(Value::Null),
            _ => Value::Array(locations),
        })
    }

    pub fn find_references(&self, params: &crate::types::ReferenceParams) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(&TextDocumentPositionParams {
            text_document: params.text_document.clone(),
            position: params.position.clone(),
        })?;
        let searcher = Searcher::new(self.search_options(128))?;
        let mut locations = Vec::new();

        for hit in searcher.search(&format!("callers:{symbol}"))?.hits {
            locations.push(location_value(&self.root, &hit.file, hit.line_start, hit.line_end));
        }

        if params
            .context
            .as_ref()
            .map(|c| c.include_declaration)
            .unwrap_or(true)
        {
            for hit in searcher.search(&format!("defs:{symbol}"))?.hits {
                locations.push(location_value(&self.root, &hit.file, hit.line_start, hit.line_end));
            }
        }

        Ok(Value::Array(locations))
    }

    pub fn prepare_call_hierarchy(
        &self,
        params: &TextDocumentPositionParams,
    ) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(params)?;
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        let item = CallHierarchyItem {
            name: symbol.clone(),
            kind: SYMBOL_KIND_FUNCTION,
            uri: path_to_uri(&self.root.join(&rel)),
            range: line_range(params.position.line + 1, params.position.line + 1),
            selection_range: line_range(params.position.line + 1, params.position.line + 1),
            detail: Some("ast-sgrep".to_string()),
        };
        Ok(json!([item]))
    }

    pub fn incoming_calls(&self, item: &CallHierarchyItem) -> anyhow::Result<Value> {
        self.with_store(|store| {
            let calls = store.incoming_calls(&item.name)?;
            let result: Vec<Value> = calls
                .iter()
                .map(|(file, line, caller, _callee)| {
                    json!({
                        "from": call_hierarchy_endpoint(&self.root, file, *line, caller),
                        "fromRanges": [line_range(*line, *line)]
                    })
                })
                .collect();
            Ok(Value::Array(result))
        })
    }

    pub fn outgoing_calls(&self, item: &CallHierarchyItem) -> anyhow::Result<Value> {
        self.with_store(|store| {
            let calls = store.outgoing_calls(&item.name)?;
            let from_line = item.range.start.line + 1;
            let result: Vec<Value> = calls
                .iter()
                .map(|(file, line, _caller, callee)| {
                    json!({
                        "to": call_hierarchy_endpoint(&self.root, file, *line, callee),
                        "fromRanges": [line_range(from_line, from_line)]
                    })
                })
                .collect();
            Ok(Value::Array(result))
        })
    }

    pub fn execute_command(&self, params: &ExecuteCommandParams) -> anyhow::Result<Value> {
        let searcher = Searcher::new(self.search_options(32))?;
        match params.command.as_str() {
            "asgrep.reindex" => {
                self.ensure_index()?;
                Ok(json!({ "status": "reindexed" }))
            }
            "asgrep.search" => {
                let query = params.arguments.first().and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::to_value(searcher.search(query)?)?)
            }
            "asgrep.search.semantic" => {
                let query = params.arguments.first().and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::to_value(searcher.search_semantic(query)?)?)
            }
            "asgrep.callers" => {
                let sym = params.arguments.first().and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::to_value(searcher.search(&format!("callers:{sym}"))?)?)
            }
            "asgrep.defs" => {
                let sym = params.arguments.first().and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::to_value(searcher.search(&format!("defs:{sym}"))?)?)
            }
            other => Err(anyhow::anyhow!("unknown command: {other}")),
        }
    }

    pub fn symbol_at_position(&self, params: &TextDocumentPositionParams) -> anyhow::Result<String> {
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        let line_no = params.position.line + 1;
        self.with_store(|store| {
            let line_content = store
                .line_content(&rel, line_no)?
                .or_else(|| {
                    std::fs::read_to_string(self.root.join(&rel))
                        .ok()
                        .and_then(|s| line_at_index(&s, params.position.line as usize))
                })
                .unwrap_or_default();

            let byte_in_line = utf16_char_to_byte(&line_content, params.position.character);

            if let Ok(symbols) = store.symbols_in_file(&rel) {
                if let Some(sym) = innermost_symbol(&symbols, line_no, byte_in_line) {
                    return Ok(sym.name.clone());
                }
            }

            extract_identifier_at(&line_content, byte_in_line)
                .ok_or_else(|| anyhow::anyhow!("no symbol at cursor"))
        })
    }
}
