//! Full LSP backend backed by ast-sgrep index.

use std::path::{Path, PathBuf};

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use serde_json::{json, Value};

use crate::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams, Position, Range,
    SYMBOL_KIND_FUNCTION, SYMBOL_KIND_METHOD, TextDocumentPositionParams,
};

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

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn index_options(&self) -> IndexOptions {
        IndexOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            lang_filter: None,
            respect_gitignore: true,
            use_tantivy: false,
            embed_lines: false,
            force_reindex: false,
        }
    }

    fn search_options(&self, limit: usize) -> SearchOptions {
        SearchOptions {
            root: self.root.clone(),
            index_path: self.index_path.clone(),
            limit,
            lang_filter: None,
            use_embed: false,
            use_tantivy: false,
            use_cloud_embed: false,
        }
    }

    pub fn ensure_index(&self) -> anyhow::Result<()> {
        let mut indexer = Indexer::new(self.index_options())?;
        indexer.index_all()?;
        Ok(())
    }

    pub fn reindex_file(&self, rel_path: &str) -> anyhow::Result<()> {
        let abs = self.root.join(rel_path);
        if !abs.is_file() {
            return Ok(());
        }
        let mut indexer = Indexer::new(self.index_options())?;
        indexer.index_file(&abs, rel_path)?;
        Ok(())
    }

    pub fn initialize_result(&self) -> Value {
        json!({
            "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 0, "save": { "includeText": false } },
                "workspaceSymbolProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "documentSymbolProvider": true,
                "callHierarchyProvider": true,
                "executeCommandProvider": {
                    "commands": ["asgrep.search", "asgrep.reindex", "asgrep.callers", "asgrep.defs"]
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
        let searcher = Searcher::new(self.search_options(50))?;
        let response = searcher.search(query)?;
        Ok(Value::Array(
            response
                .hits
                .into_iter()
                .filter_map(|hit| symbol_information(&self.root, &hit.file, &hit))
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
                json!({
                    "name": sym.name,
                    "kind": kind,
                    "range": line_range(sym.line_start, sym.line_end),
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

        if let Some(symbols) = store.symbols_in_file(&rel).ok() {
            for sym in &symbols {
                if line_no >= sym.line_start && line_no <= sym.line_end {
                    return Ok(sym.name.clone());
                }
            }
        }

        let content = store
            .line_content(&rel, line_no)?
            .or_else(|| std::fs::read_to_string(self.root.join(&rel)).ok())
            .and_then(|s| s.lines().nth(params.position.line as usize).map(|l| l.to_string()))
            .unwrap_or_default();

        extract_identifier_at(&content, utf16_char_to_byte(&content, params.position.character))
            .ok_or_else(|| anyhow::anyhow!("no symbol at cursor"))
    }
}

/// Convert LSP UTF-16 code unit offset to byte offset within a line.
pub fn utf16_char_to_byte(line: &str, utf16_offset: u32) -> usize {
    let mut utf16_count = 0u32;
    for (byte_idx, ch) in line.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_idx;
        }
        utf16_count += ch.len_utf16() as u32;
    }
    line.len()
}

fn symbol_information(root: &Path, file: &str, hit: &ast_sgrep_core::SearchHit) -> Option<Value> {
    let name = hit
        .symbol
        .clone()
        .or_else(|| hit.callee.clone())
        .unwrap_or_else(|| hit.excerpt.chars().take(60).collect());
    Some(json!({
        "name": name,
        "kind": SYMBOL_KIND_FUNCTION,
        "location": location_value(root, file, hit.line_start, hit.line_end)
    }))
}

fn location_value(root: &Path, file: &str, line_start: u32, line_end: u32) -> Value {
    json!({
        "uri": path_to_uri(&root.join(file)),
        "range": line_range(line_start, line_end)
    })
}

fn line_range(line_start: u32, line_end: u32) -> Range {
    Range {
        start: Position {
            line: line_start.saturating_sub(1),
            character: 0,
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: 0,
        },
    }
}

pub fn path_to_uri(path: &Path) -> String {
    let path_str = path.to_string_lossy().replace('\\', "/");
    if path_str.starts_with('/') {
        format!("file://{path_str}")
    } else {
        format!("file:///{path_str}")
    }
}

pub fn uri_to_rel_path(uri: &str, root: &Path) -> anyhow::Result<String> {
    let path = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("file:///"))
        .unwrap_or(uri);
    let path = PathBuf::from(path);
    let rel = path.strip_prefix(root).unwrap_or(&path);
    Ok(rel.to_string_lossy().replace('\\', "/"))
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
    fn extracts_identifier_at_cursor() {
        let line = "    process_request(\"x\");";
        assert_eq!(
            extract_identifier_at(line, 6),
            Some("process_request".to_string())
        );
    }

    #[test]
    fn uri_roundtrip() {
        let root = PathBuf::from("/proj");
        let uri = path_to_uri(&root.join("src/main.rs"));
        let rel = uri_to_rel_path(&uri, &root).unwrap();
        assert_eq!(rel, "src/main.rs");
    }
}
