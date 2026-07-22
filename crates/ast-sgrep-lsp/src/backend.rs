use crate::support::{
    apply_text_edit, call_hierarchy_endpoint, extract_identifier_at, innermost_symbol,
    line_at_index, line_range, line_range_ext, location_value, utf16_char_to_byte,
    workspace_symbol, AsgrepSettings,
};
pub use crate::support::{path_to_file_uri as path_to_uri, uri_to_rel_path};
use crate::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams, TextDocumentContentChangeEvent,
    TextDocumentPositionParams, SYMBOL_KIND_FUNCTION,
};
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
pub struct LspBackend {
    root: PathBuf,
    index_path: Option<PathBuf>,
    settings: AsgrepSettings,
    index_ready: Arc<AtomicBool>,
    background_index_started: bool,
    index_lock: Arc<Mutex<()>>,
}
fn first_cmd_arg(p: &ExecuteCommandParams) -> &str {
    p.arguments.first().and_then(|v| v.as_str()).unwrap_or("")
}
impl LspBackend {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: crate::support::canonicalize_workspace_root(root),
            index_path: None,
            settings: AsgrepSettings::default(),
            index_ready: Arc::new(AtomicBool::new(false)),
            background_index_started: false,
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
    fn record_index_result<T>(&self, result: anyhow::Result<T>) -> anyhow::Result<T> {
        self.index_ready.store(result.is_ok(), Ordering::SeqCst);
        result
    }
    fn with_index_lock<T>(&self, f: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _g = self
            .index_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        f()
    }
    fn with_locked_indexer<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&mut Indexer) -> anyhow::Result<T>,
    {
        self.record_index_result(self.with_index_lock(|| {
            let mut indexer = Indexer::new(self.index_options())?;
            f(&mut indexer)
        }))
    }
    fn with_store<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&ast_sgrep_core::IndexStore) -> anyhow::Result<T>,
    {
        self.with_index_lock(|| f(Indexer::new(self.index_options())?.store()))
    }
    fn with_locked_searcher<F, T>(&self, limit: usize, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&Searcher) -> anyhow::Result<T>,
    {
        self.with_index_lock(|| f(&Searcher::new(self.search_options(limit))?))
    }
    fn hit_locations(&self, s: &Searcher, query: &str) -> anyhow::Result<Vec<Value>> {
        Ok(s.search(query)?
            .hits
            .iter()
            .map(|h| location_value(&self.root, &h.file, h.line_start, h.line_end))
            .collect())
    }
    pub fn start_background_index(&mut self) {
        if self.background_index_started {
            return;
        }
        self.background_index_started = true;
        self.index_ready.store(false, Ordering::SeqCst);
        let opts = self.index_options();
        let ready = Arc::clone(&self.index_ready);
        let lock = Arc::clone(&self.index_lock);
        std::thread::spawn(move || {
            let Ok(_g) = lock.lock() else {
                return;
            };
            let ok = Indexer::new(opts)
                .and_then(|mut i| i.index_all().map(|_| ()))
                .is_ok();
            ready.store(ok, Ordering::SeqCst);
            if !ok {
                crate::server::log("background index failed");
            }
        });
    }
    pub fn ensure_index(&self) -> anyhow::Result<()> {
        self.with_locked_indexer(|i| {
            i.index_all()?;
            Ok(())
        })?;
        self.index_ready.store(true, Ordering::SeqCst);
        Ok(())
    }
    pub fn reindex_file(&self, rel: &str) -> anyhow::Result<()> {
        self.with_locked_indexer(|i| {
            let abs = self.root.join(rel);
            if abs.is_file() {
                i.index_file(&abs, rel)?;
            }
            Ok(())
        })
    }
    pub fn index_content(&self, rel: &str, content: &str) -> anyhow::Result<()> {
        self.with_locked_indexer(|i| {
            i.index_content(rel, content)?;
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
            for c in changes {
                content = if c.range.is_some() {
                    apply_text_edit(&content, c)
                } else {
                    c.text.clone()
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
                "workspaceSymbolProvider": true, "definitionProvider": true, "experimental": { "asgrepSearchProvider": true },
                "referencesProvider": true, "documentSymbolProvider": true, "callHierarchyProvider": true,
                "executeCommandProvider": { "commands": ["asgrep.search", "asgrep.search.semantic", "asgrep.reindex", "asgrep.callers", "asgrep.defs"] }
            }, "serverInfo": { "name": "asgrep-lsp", "version": env!("CARGO_PKG_VERSION") }
        })
    }
    pub fn workspace_symbols(&self, query: &str) -> anyhow::Result<Value> {
        if query.is_empty() {
            return Ok(json!([]));
        }
        self.with_locked_searcher(50, |s| {
            Ok(Value::Array(
                s.search(query)?
                    .hits
                    .into_iter()
                    .filter_map(|h| workspace_symbol(&self.root, &h.file, &h))
                    .collect(),
            ))
        })
    }
    pub fn document_symbols(&self, params: &DocumentSymbolParams) -> anyhow::Result<Value> {
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        self.with_store(|store| {
            Ok(Value::Array(store.symbols_in_file(&rel)?.iter().map(|sym| {
                let kind = match sym.kind.as_str() {
                    "method" => crate::types::SYMBOL_KIND_METHOD, "class" => crate::types::SYMBOL_KIND_CLASS,
                    "interface" => crate::types::SYMBOL_KIND_INTERFACE, "enum" => crate::types::SYMBOL_KIND_ENUM,
                    "type" => crate::types::SYMBOL_KIND_STRUCT, _ => SYMBOL_KIND_FUNCTION, };
                let end = store.line_content(&rel, sym.line_end).ok().flatten(); json!({
                    "name": sym.name, "kind": kind, "range": line_range_ext(sym.line_start, sym.line_end, end.as_deref()),
                    "selectionRange": line_range(sym.line_start, sym.line_start), "detail": sym.kind
                })
            }).collect()))
        })
    }
    pub fn goto_definition(&self, params: &TextDocumentPositionParams) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(params)?;
        self.with_locked_searcher(16, |s| {
            let locs = self.hit_locations(s, &format!("defs:{symbol}"))?;
            Ok(match locs.len() {
                0 => Value::Null,
                1 => locs.into_iter().next().unwrap_or(Value::Null),
                _ => Value::Array(locs),
            })
        })
    }
    pub fn find_references(&self, params: &crate::types::ReferenceParams) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(&params.at)?;
        self.with_locked_searcher(128, |s| {
            let mut locs = self.hit_locations(s, &format!("callers:{symbol}"))?;
            if params
                .context
                .as_ref()
                .map(|c| c.include_declaration)
                .unwrap_or(true)
            {
                locs.extend(self.hit_locations(s, &format!("defs:{symbol}"))?);
            }
            Ok(Value::Array(locs))
        })
    }
    pub fn prepare_call_hierarchy(
        &self,
        params: &TextDocumentPositionParams,
    ) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(params)?;
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        let line = params.position.line + 1;
        let range = line_range(line, line);
        Ok(json!([CallHierarchyItem {
            name: symbol,
            kind: SYMBOL_KIND_FUNCTION,
            uri: path_to_uri(&self.root.join(&rel)),
            range: range.clone(),
            selection_range: range,
            detail: Some("ast-sgrep".into()),
        }]))
    }
    pub fn incoming_calls(&self, item: &CallHierarchyItem) -> anyhow::Result<Value> {
        self.with_store(|store| Ok(Value::Array(store.incoming_calls(&item.name)?.iter().map(|(file, line, caller, _)| { json!({ "from": call_hierarchy_endpoint(&self.root, file, *line, caller), "fromRanges": [line_range(*line, *line)] }) }).collect())))
    }
    pub fn outgoing_calls(&self, item: &CallHierarchyItem) -> anyhow::Result<Value> {
        self.with_store(|store| {
            let from = item.range.start.line + 1;
            Ok(Value::Array(store.outgoing_calls(&item.name)?.iter().map(|(file, line, _, callee)| { json!({ "to": call_hierarchy_endpoint(&self.root, file, *line, callee), "fromRanges": [line_range(from, from)] }) }).collect()))
        })
    }
    pub fn search(&self, query: &str, semantic: bool, limit: usize) -> anyhow::Result<Value> {
        self.with_locked_searcher(limit, |s| {
            Ok(serde_json::to_value(if semantic {
                s.search_semantic(query)?
            } else {
                s.search(query)?
            })?)
        })
    }
    pub fn execute_command(&self, params: &ExecuteCommandParams) -> anyhow::Result<Value> {
        match params.command.as_str() {
            "asgrep.reindex" => {
                self.ensure_index()?;
                Ok(json!({ "status": "reindexed" }))
            }
            "asgrep.search" => self.search(first_cmd_arg(params), false, 32),
            "asgrep.search.semantic" => self.search(first_cmd_arg(params), true, 32),
            "asgrep.callers" => {
                self.search(&format!("callers:{}", first_cmd_arg(params)), false, 32)
            }
            "asgrep.defs" => self.search(&format!("defs:{}", first_cmd_arg(params)), false, 32),
            other => Err(anyhow::anyhow!("unknown command: {other}")),
        }
    }
    pub fn symbol_at_position(
        &self,
        params: &TextDocumentPositionParams,
    ) -> anyhow::Result<String> {
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        let line_no = params.position.line + 1;
        self.with_store(|store| {
            let line = store
                .line_content(&rel, line_no)?
                .or_else(|| {
                    std::fs::read_to_string(self.root.join(&rel))
                        .ok()
                        .and_then(|s| line_at_index(&s, params.position.line as usize))
                })
                .unwrap_or_default();
            let byte = utf16_char_to_byte(&line, params.position.character);
            if let Some(id) = extract_identifier_at(&line, byte) {
                return Ok(id);
            }
            if let Ok(syms) = store.symbols_in_file(&rel) {
                if let Some(sym) = innermost_symbol(&syms, line_no, byte) {
                    return Ok(sym.name.clone());
                }
            }
            Err(anyhow::anyhow!("no symbol at cursor"))
        })
    }
}
