//! LSP JSON conversions for locations and workspace symbols.

use std::path::Path;

use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::SearchHit;
use serde_json::{json, Value};

use crate::types::{
    Position, Range, SYMBOL_KIND_FUNCTION, SYMBOL_KIND_METHOD, SYMBOL_KIND_STRING,
};
use crate::uri::path_to_file_uri;

pub fn workspace_symbol(root: &Path, file: &str, hit: &SearchHit) -> Option<Value> {
    let name = hit
        .symbol
        .clone()
        .or_else(|| hit.callee.clone())
        .unwrap_or_else(|| hit.excerpt.chars().take(60).collect());

    let kind = match hit.kind {
        HitKind::Embed => SYMBOL_KIND_STRING,
        HitKind::Def => SYMBOL_KIND_FUNCTION,
        HitKind::Caller | HitKind::Graph => SYMBOL_KIND_METHOD,
        _ => SYMBOL_KIND_FUNCTION,
    };

    let detail = match hit.kind {
        HitKind::Embed => format!("semantic · score {:.2}", hit.score),
        other => format!("{} · score {:.2}", other.as_str(), hit.score),
    };

    let excerpt: String = hit.excerpt.chars().take(120).collect();

    Some(json!({
        "name": name,
        "kind": kind,
        "location": location_value(root, file, hit.line_start, hit.line_end),
        "containerName": file,
        "detail": detail,
        "data": {
            "asgrepKind": hit.kind.as_str(),
            "score": hit.score,
            "excerpt": excerpt,
            "semantic": hit.kind == HitKind::Embed,
        }
    }))
}

pub fn location_value(root: &Path, file: &str, line_start: u32, line_end: u32) -> Value {
    json!({
        "uri": path_to_file_uri(&root.join(file)),
        "range": line_range(line_start, line_end)
    })
}

pub fn line_range(line_start: u32, line_end: u32) -> Range {
    line_range_ext(line_start, line_end, None)
}

pub fn line_range_ext(line_start: u32, line_end: u32, end_line_text: Option<&str>) -> Range {
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

pub fn call_hierarchy_endpoint(root: &Path, file: &str, line: u32, name: &str) -> Value {
    json!({
        "name": name,
        "kind": SYMBOL_KIND_FUNCTION,
        "uri": path_to_file_uri(&root.join(file)),
        "range": line_range(line, line),
        "selectionRange": line_range(line, line)
    })
}

fn line_utf16_len(line: &str) -> u32 {
    line.chars().map(|c| c.len_utf16() as u32).sum()
}
