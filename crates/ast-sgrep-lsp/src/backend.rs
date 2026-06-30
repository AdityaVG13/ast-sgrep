//! Full LSP backend backed by ast-sgrep index.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use ast_sgrep_core::store::SymbolRow;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use serde_json::{json, Value};

use crate::settings::AsgrepSettings;
use crate::uri::{self};

pub use uri::{path_to_file_uri as path_to_uri, uri_to_rel_path};

use crate::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams, Position, Range,
    TextDocumentContentChangeEvent, SYMBOL_KIND_FUNCTION, SYMBOL_KIND_METHOD,
    SYMBOL_KIND_STRING, TextDocumentPositionParams,
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
        let _guard = self
            .index_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        let mut indexer = Indexer::new(self.index_options())?;
        indexer.index_all()?;
        Ok(())
    }

    pub fn reindex_file(&self, rel_path: &str) -> anyhow::Result<()> {
        let _guard = self
            .index_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        let abs = self.root.join(rel_path);
        if abs.is_file() {
            let mut indexer = Indexer::new(self.index_options())?;
            indexer.index_file(&abs, rel_path)?;
        }
        Ok(())
    }

    pub fn index_content(&self, rel_path: &str, content: &str) -> anyhow::Result<()> {
        let _guard = self
            .index_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        let mut indexer = Indexer::new(self.index_options())?;
        indexer.index_content(rel_path, content)?;
        Ok(())
    }

    pub fn apply_document_changes(
        &self,
        uri: &str,
        changes: &[TextDocumentContentChangeEvent],
    ) -> anyhow::Result<()> {
        let _guard = self
            .index_lock
            .lock()
            .map_err(|e| anyhow::anyhow!("index lock poisoned: {e}"))?;
        let rel = uri_to_rel_path(uri, &self.root)?;
        let mut content = {
            let indexer = Indexer::new(self.index_options())?;
            indexer
                .store()
                .file_text(&rel)?
                .or_else(|| std::fs::read_to_string(self.root.join(&rel)).ok())
                .unwrap_or_default()
        };

        for change in changes {
            if change.range.is_some() {
                content = apply_text_edit(&content, change);
            } else {
                content = change.text.clone();
            }
        }

        let mut indexer = Indexer::new(self.index_options())?;
        indexer.index_content(&rel, &content)?;
        Ok(())
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
        let indexer = Indexer::new(self.index_options())?;
        let store = indexer.store();
        let symbols = store.symbols_in_file(&rel)?;
        let items: Vec<Value> = symbols
            .iter()
            .map(|sym| {
                let kind = if sym.kind.contains("method") {
                    SYMBOL_KIND_METHOD
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
        Ok(if locations.len() == 1 {
            locations.into_iter().next().unwrap_or(Value::Null)
        } else if locations.is_empty() {
            Value::Null
        } else {
            Value::Array(locations)
        })
    }

    pub fn find_references(&self, params: &crate::types::ReferenceParams) -> anyhow::Result<Value> {
        let symbol = self.symbol_at_position(&TextDocumentPositionParams {
            text_document: params.text_document.clone(),
            position: params.position.clone(),
        })?;
        let searcher = Searcher::new(self.search_options(128))?;
        let mut locations = Vec::new();

        let callers = searcher.search(&format!("callers:{symbol}"))?;
        for hit in callers.hits {
            locations.push(location_value(&self.root, &hit.file, hit.line_start, hit.line_end));
        }

        if params
            .context
            .as_ref()
            .map(|c| c.include_declaration)
            .unwrap_or(true)
        {
            let defs = searcher.search(&format!("defs:{symbol}"))?;
            for hit in defs.hits {
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
        let uri = path_to_uri(&self.root.join(&rel));
        let item = CallHierarchyItem {
            name: symbol.clone(),
            kind: SYMBOL_KIND_FUNCTION,
            uri,
            range: line_range(params.position.line + 1, params.position.line + 1),
            selection_range: line_range(params.position.line + 1, params.position.line + 1),
            detail: Some("ast-sgrep".to_string()),
        };
        Ok(json!([item]))
    }

    pub fn incoming_calls(&self, item: &CallHierarchyItem) -> anyhow::Result<Value> {
        let indexer = Indexer::new(self.index_options())?;
        let calls = indexer.store().incoming_calls(&item.name)?;
        let result: Vec<Value> = calls
            .iter()
            .map(|(file, line, caller, _callee)| {
                json!({
                    "from": {
                        "name": caller,
                        "kind": SYMBOL_KIND_FUNCTION,
                        "uri": path_to_uri(&self.root.join(file)),
                        "range": line_range(*line, *line),
                        "selectionRange": line_range(*line, *line)
                    },
                    "fromRanges": [line_range(*line, *line)]
                })
            })
            .collect();
        Ok(Value::Array(result))
    }

    pub fn outgoing_calls(&self, item: &CallHierarchyItem) -> anyhow::Result<Value> {
        let indexer = Indexer::new(self.index_options())?;
        let calls = indexer.store().outgoing_calls(&item.name)?;
        let result: Vec<Value> = calls
            .iter()
            .map(|(file, line, _caller, callee)| {
                json!({
                    "to": {
                        "name": callee,
                        "kind": SYMBOL_KIND_FUNCTION,
                        "uri": path_to_uri(&self.root.join(file)),
                        "range": line_range(*line, *line),
                        "selectionRange": line_range(*line, *line)
                    },
                    "fromRanges": [line_range(item.range.start.line + 1, item.range.start.line + 1)]
                })
            })
            .collect();
        Ok(Value::Array(result))
    }

    pub fn execute_command(&self, params: &ExecuteCommandParams) -> anyhow::Result<Value> {
        let searcher = Searcher::new(self.search_options(32))?;
        match params.command.as_str() {
            "asgrep.reindex" => {
                self.ensure_index()?;
                Ok(json!({ "status": "reindexed" }))
            }
            "asgrep.search" => {
                let query = params
                    .arguments
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let response = searcher.search(query)?;
                Ok(serde_json::to_value(response)?)
            }
            "asgrep.search.semantic" => {
                let query = params
                    .arguments
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let response = searcher.search_semantic(query)?;
                Ok(serde_json::to_value(response)?)
            }
            "asgrep.callers" => {
                let sym = params
                    .arguments
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let response = searcher.search(&format!("callers:{sym}"))?;
                Ok(serde_json::to_value(response)?)
            }
            "asgrep.defs" => {
                let sym = params
                    .arguments
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let response = searcher.search(&format!("defs:{sym}"))?;
                Ok(serde_json::to_value(response)?)
            }
            other => Err(anyhow::anyhow!("unknown command: {other}")),
        }
    }

    /// Resolve identifier at cursor using index line content + symbol table.
    pub fn symbol_at_position(&self, params: &TextDocumentPositionParams) -> anyhow::Result<String> {
        let rel = uri_to_rel_path(&params.text_document.uri, &self.root)?;
        let line_no = params.position.line + 1;
        let indexer = Indexer::new(self.index_options())?;
        let store = indexer.store();

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
    }
}

/// Convert LSP UTF-16 code unit offset to byte offset within a line.
pub fn utf16_char_to_byte(line: &str, utf16_offset: u32) -> usize {
    let mut utf16 = 0u32;
    for (byte_idx, ch) in line.char_indices() {
        let units = ch.len_utf16() as u32;
        if utf16_offset < utf16 + units {
            return byte_idx;
        }
        utf16 += units;
    }
    line.len()
}

fn workspace_symbol(root: &Path, file: &str, hit: &ast_sgrep_core::SearchHit) -> Option<Value> {
    let name = hit
        .symbol
        .clone()
        .or_else(|| hit.callee.clone())
        .unwrap_or_else(|| hit.excerpt.chars().take(60).collect());

    let kind = match hit.kind {
        ast_sgrep_core::search::HitKind::Embed => SYMBOL_KIND_STRING,
        ast_sgrep_core::search::HitKind::Def => SYMBOL_KIND_FUNCTION,
        ast_sgrep_core::search::HitKind::Caller | ast_sgrep_core::search::HitKind::Graph => {
            SYMBOL_KIND_METHOD
        }
        _ => SYMBOL_KIND_FUNCTION,
    };

    let detail = match hit.kind {
        ast_sgrep_core::search::HitKind::Embed => {
            format!("semantic · score {:.2}", hit.score)
        }
        other => format!("{} · score {:.2}", other.as_str(), hit.score),
    };

    let excerpt: String = hit.excerpt.chars().take(120).collect();
    let container = file.to_string();

    Some(json!({
        "name": name,
        "kind": kind,
        "location": location_value(root, file, hit.line_start, hit.line_end),
        "containerName": container,
        "detail": detail,
        "data": {
            "asgrepKind": hit.kind.as_str(),
            "score": hit.score,
            "excerpt": excerpt,
            "semantic": hit.kind == ast_sgrep_core::search::HitKind::Embed,
        }
    }))
}

fn location_value(root: &Path, file: &str, line_start: u32, line_end: u32) -> Value {
    json!({
        "uri": path_to_uri(&root.join(file)),
        "range": line_range(line_start, line_end)
    })
}

fn line_utf16_len(line: &str) -> u32 {
    line.chars().map(|c| c.len_utf16() as u32).sum()
}

fn line_range(line_start: u32, line_end: u32) -> Range {
    line_range_ext(line_start, line_end, None)
}

fn line_range_ext(line_start: u32, line_end: u32, end_line_text: Option<&str>) -> Range {
    let end_char = end_line_text.map(line_utf16_len).unwrap_or(0);
    Range {
        start: Position {
            line: line_start.saturating_sub(1),
            character: 0,
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: end_char,
        },
    }
}

fn line_at_index(content: &str, line_index: usize) -> Option<String> {
    content.split('\n').nth(line_index).map(|l| l.to_string())
}

fn innermost_symbol<'a>(
    symbols: &'a [SymbolRow],
    line_no: u32,
    byte_in_line: usize,
) -> Option<&'a SymbolRow> {
    symbols
        .iter()
        .filter(|sym| line_no >= sym.line_start && line_no <= sym.line_end)
        .min_by(|a, b| {
            symbol_tightness(a, line_no, byte_in_line).cmp(&symbol_tightness(b, line_no, byte_in_line))
        })
}

fn symbol_tightness(sym: &SymbolRow, _line_no: u32, byte_in_line: usize) -> (u32, usize) {
    let line_span = sym.line_end - sym.line_start;
    if sym.line_start == sym.line_end && sym.byte_end > sym.byte_start {
        if byte_in_line >= sym.byte_start && byte_in_line <= sym.byte_end {
            return (0, sym.byte_end - sym.byte_start);
        }
    }
    (line_span, sym.byte_end.saturating_sub(sym.byte_start))
}

fn apply_text_edit(content: &str, change: &TextDocumentContentChangeEvent) -> String {
    let range = match &change.range {
        Some(r) => r,
        None => return change.text.clone(),
    };
    let start = lsp_position_to_byte_offset(content, &range.start);
    let end = if let Some(len) = change.range_length {
        utf16_span_to_byte_end(content, &range.start, len)
    } else {
        lsp_position_to_byte_offset(content, &range.end)
    };
    if start > end || end > content.len() {
        return content.to_string();
    }
    let mut out = String::with_capacity(content.len().saturating_add(change.text.len()));
    out.push_str(&content[..start]);
    out.push_str(&change.text);
    out.push_str(&content[end..]);
    out
}

fn utf16_span_to_byte_end(content: &str, start: &Position, utf16_len: u32) -> usize {
    let start_byte = lsp_position_to_byte_offset(content, start);
    let tail = &content[start_byte..];
    let mut utf16 = 0u32;
    for (byte_idx, ch) in tail.char_indices() {
        utf16 += ch.len_utf16() as u32;
        if utf16 >= utf16_len {
            return start_byte + byte_idx + ch.len_utf8();
        }
    }
    content.len()
}

fn lsp_position_to_byte_offset(content: &str, pos: &Position) -> usize {
    let mut line_no = 0u32;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let line_body = line.strip_suffix('\n').unwrap_or(line);
        if line_no == pos.line {
            return offset + utf16_char_to_byte(line_body, pos.character);
        }
        offset += line.len();
        line_no += 1;
    }
    content.len()
}

fn extract_identifier_at(line: &str, byte_offset: usize) -> Option<String> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    if chars.is_empty() {
        return None;
    }
    let mut idx = 0;
    while idx < chars.len() && chars[idx].0 < byte_offset {
        idx += 1;
    }
    if idx >= chars.len() {
        idx = chars.len().saturating_sub(1);
    }
    if !is_ident_char(chars[idx].1) {
        if idx > 0 {
            idx -= 1;
        }
    }
    if !is_ident_char(chars[idx].1) {
        return None;
    }
    let mut lo = idx;
    let mut hi = idx;
    while lo > 0 && is_ident_char(chars[lo - 1].1) {
        lo -= 1;
    }
    while hi + 1 < chars.len() && is_ident_char(chars[hi + 1].1) {
        hi += 1;
    }
    let start_byte = chars[lo].0;
    let end_byte = if hi + 1 < chars.len() {
        chars[hi + 1].0
    } else {
        line.len()
    };
    let ident = line.get(start_byte..end_byte)?.trim();
    if ident.is_empty() {
        None
    } else {
        Some(ident.to_string())
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offset_handles_multibyte() {
        let line = "fn café() {}";
        assert!(utf16_char_to_byte(line, 0) < utf16_char_to_byte(line, 5));
    }

    #[test]
    fn utf16_offset_handles_emoji_interior() {
        let line = "🙂abc";
        let emoji_byte = utf16_char_to_byte(line, 0);
        let interior = utf16_char_to_byte(line, 1);
        assert_eq!(emoji_byte, interior);
        assert_eq!(utf16_char_to_byte(line, 2), "🙂".len());
    }

    #[test]
    fn apply_incremental_text_edit() {
        let content = "fn main() {\n    old();\n}\n";
        let change = TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 1, character: 4 },
                end: Position { line: 1, character: 7 },
            }),
            range_length: None,
            text: "new".to_string(),
        };
        let edited = apply_text_edit(content, &change);
        assert!(edited.contains("new();"));
        assert!(!edited.contains("old();"));
    }

    #[test]
    fn apply_edit_honors_range_length() {
        let content = "fn main() {\n    old();\n}\n";
        let change = TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position { line: 1, character: 4 },
                end: Position { line: 1, character: 4 },
            }),
            range_length: Some(3),
            text: "new".to_string(),
        };
        let edited = apply_text_edit(content, &change);
        assert!(edited.contains("new();"));
    }

    #[test]
    fn extracts_identifier_at_cursor() {
        let line = "    process_request(\"x\");";
        assert_eq!(
            extract_identifier_at(line, 6),
            Some("process_request".to_string())
        );
    }

    #[test]
    fn uri_roundtrip() {
        let root = std::fs::canonicalize("/tmp").unwrap_or_else(|_| PathBuf::from("/tmp"));
        let uri = path_to_uri(&root.join("src/main.rs"));
        let rel = uri_to_rel_path(&uri, &root).unwrap();
        assert_eq!(rel, "src/main.rs");
    }

    #[test]
    fn uri_rejects_path_traversal() {
        let root = std::fs::canonicalize("/tmp").unwrap_or_else(|_| PathBuf::from("/tmp"));
        let evil = format!("file://{}/../etc/passwd", root.display());
        assert!(uri_to_rel_path(&evil, &root).is_err());
    }
}
