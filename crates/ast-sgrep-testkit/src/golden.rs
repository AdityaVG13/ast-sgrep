//! Shared golden-file compare/update for ast-sgrep tests.
//!
//! # Env
//!
//! `ASGREP_UPDATE_GOLDENS` — truthy values `1`, `true`, `yes`, `on`
//! (case-insensitive). When set, mismatches rewrite the golden. When unset,
//! goldens are never written. Reject `UPDATE_GOLDENS` / `INSTA_UPDATE`.
//!
//! # Paths
//!
//! Default root is workspace `tests/golden/` (walk up from cwd to the
//! workspace `Cargo.toml`). Override with `ASGREP_GOLDEN_DIR` or
//! [`assert_golden_at`]. Crate-local fixtures stay valid via `_at` helpers.
//! Mismatches write `{golden}.actual` (gitignored `*.actual`).
//!
//! Trailing whitespace: [`canonicalize_text`] maps `\r\n` → `\n` and trims
//! trailing spaces/tabs per line. UTF-8 is required (`&str`).

use ast_sgrep_core::chain::{ChainEdge, ChainNode, ChainResponse};
use ast_sgrep_lang::{ExtractionResult, SymbolKind};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_DIFF_HUNKS: usize = 12;

/// Truthy `ASGREP_UPDATE_GOLDENS` (`1` / `true` / `yes` / `on`).
pub fn updating_goldens() -> bool {
    match std::env::var("ASGREP_UPDATE_GOLDENS") {
        Ok(raw) => matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

/// UTF-8 text with `\r\n` → `\n` and trailing per-line whitespace stripped.
pub fn canonicalize_text(input: &str) -> String {
    let unified = input.replace("\r\n", "\n");
    let mut lines: Vec<&str> = unified.lines().map(|line| line.trim_end()).collect();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Compare `actual` to workspace `tests/golden/{name}`.
pub fn assert_golden(name: &str, actual: &str) {
    assert_golden_at(&default_golden_path(name), actual);
}

/// Compare pretty JSON to workspace `tests/golden/{name}`.
pub fn assert_golden_json(name: &str, actual: &Value) {
    assert_golden_json_at(&default_golden_path(name), actual);
}

/// Text compare against an explicit golden path.
pub fn assert_golden_at(path: &Path, actual: &str) {
    let actual = canonicalize_text(actual);
    compare_or_update(path, &actual, false);
}

/// JSON compare against an explicit golden path (Value equality, pretty write).
pub fn assert_golden_json_at(path: &Path, actual: &Value) {
    let pretty = format!("{}\n", pretty_json(actual));
    compare_or_update(path, &pretty, true);
}

/// Sort chain `seeds` / `nodes` / `edges` so insertion order cannot flake.
///
/// Node key: `file`, `symbol`, `line_start`, `depth`.
/// Edge key: `from_file`, `from_symbol`, `to_file`, `to_symbol`, `label`, `depth`.
pub fn canonicalize_chain_response(mut response: ChainResponse) -> ChainResponse {
    response.seeds.sort_by(|a, b| node_sort_key(a).cmp(&node_sort_key(b)));
    response.nodes.sort_by(|a, b| node_sort_key(a).cmp(&node_sort_key(b)));
    response.edges.sort_by(|a, b| edge_sort_key(a).cmp(&edge_sort_key(b)));
    response
}

/// Sort extraction dumps so parser HashMap order cannot flake.
///
/// Symbols: `(name, kind, byte_start)`. Imports: `(module_path, line)`.
/// Calls: `(caller, callee, line, byte_start)`. Pattern nodes: `(signature, line_start, excerpt)`.
pub fn canonicalize_extraction(mut result: ExtractionResult) -> ExtractionResult {
    result.symbols.sort_by(|a, b| {
        (a.name.as_str(), kind_sort_key(a.kind), a.byte_start).cmp(&(
            b.name.as_str(),
            kind_sort_key(b.kind),
            b.byte_start,
        ))
    });
    result.imports.sort_by(|a, b| {
        a.module_path
            .cmp(&b.module_path)
            .then(a.line.cmp(&b.line))
    });
    result.calls.sort_by(|a, b| {
        (a.caller.as_str(), a.callee.as_str(), a.line, a.byte_start).cmp(&(
            b.caller.as_str(),
            b.callee.as_str(),
            b.line,
            b.byte_start,
        ))
    });
    result.pattern_nodes.sort_by(|a, b| {
        (a.signature.as_str(), a.line_start, a.excerpt.as_str()).cmp(&(
            b.signature.as_str(),
            b.line_start,
            b.excerpt.as_str(),
        ))
    });
    result
}

fn kind_sort_key(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Type => "type",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum => "enum",
        SymbolKind::Doc => "doc",
    }
}

fn node_sort_key(node: &ChainNode) -> (String, String, u32, u32) {
    (
        node.file.clone(),
        node.symbol.clone().unwrap_or_default(),
        node.line_start,
        node.depth,
    )
}

fn edge_sort_key(edge: &ChainEdge) -> (String, String, String, String, String, u32) {
    (
        edge.from_file.clone(),
        edge.from_symbol.clone().unwrap_or_default(),
        edge.to_file.clone(),
        edge.to_symbol.clone().unwrap_or_default(),
        format!("{:?}", edge.label),
        edge.depth,
    )
}

fn default_golden_path(name: &str) -> PathBuf {
    if let Ok(root) = std::env::var("ASGREP_GOLDEN_DIR") {
        return PathBuf::from(root).join(name);
    }
    workspace_root()
        .join("tests")
        .join("golden")
        .join(name)
}

fn workspace_root() -> PathBuf {
    let mut cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if cur.join("Cargo.toml").is_file() && cur.join("crates").is_dir() {
            return cur;
        }
        if !cur.pop() {
            break;
        }
    }
    PathBuf::from(".")
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("json pretty")
}

fn compare_or_update(path: &Path, actual: &str, json: bool) {
    if updating_goldens() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden parent");
        }
        fs::write(path, actual).unwrap_or_else(|err| {
            panic!("failed to write golden {}: {err}", path.display());
        });
        return;
    }

    let expected_raw = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "missing golden {}\n{err}\nCreate it with ASGREP_UPDATE_GOLDENS=1 (not UPDATE_GOLDENS / INSTA_UPDATE).",
            path.display()
        );
    });

    let matched = if json {
        let expected_val: Value =
            serde_json::from_str(&expected_raw).expect("golden JSON parses");
        let actual_val: Value = serde_json::from_str(actual).expect("actual JSON parses");
        expected_val == actual_val
    } else {
        canonicalize_text(&expected_raw) == actual
    };

    if matched {
        return;
    }

    let actual_path = actual_sidecar(path);
    fs::write(&actual_path, actual).unwrap_or_else(|err| {
        panic!("failed to write {}: {err}", actual_path.display());
    });
    let expected_display = if json {
        format!("{}\n", pretty_json(&serde_json::from_str(&expected_raw).unwrap()))
    } else {
        canonicalize_text(&expected_raw)
    };
    panic!(
        "golden mismatch\n  golden: {}\n  actual: {}\n  update: ASGREP_UPDATE_GOLDENS=1\n{}",
        path.display(),
        actual_path.display(),
        unified_diff(&expected_display, actual, MAX_DIFF_HUNKS)
    );
}

fn actual_sidecar(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".actual");
    PathBuf::from(os)
}

fn unified_diff(expected: &str, actual: &str, max_hunks: usize) -> String {
    let exp: Vec<&str> = expected.lines().collect();
    let act: Vec<&str> = actual.lines().collect();
    let mut out = String::from("--- golden\n+++ actual\n");
    let mut hunks = 0;
    let mut i = 0;
    let mut j = 0;
    while i < exp.len() || j < act.len() {
        if i < exp.len() && j < act.len() && exp[i] == act[j] {
            i += 1;
            j += 1;
            continue;
        }
        hunks += 1;
        if hunks > max_hunks {
            out.push_str(&format!(
                "... truncated after {max_hunks} hunks ({} expected lines, {} actual)\n",
                exp.len(),
                act.len()
            ));
            break;
        }
        out.push_str(&format!("@@ expected:{i} actual:{j} @@\n"));
        let mut shown = 0;
        while shown < 8 && (i < exp.len() || j < act.len()) {
            if i < exp.len() && j < act.len() && exp[i] == act[j] {
                break;
            }
            if i < exp.len() {
                out.push_str(&format!("-{}\n", exp[i]));
                i += 1;
                shown += 1;
            }
            if j < act.len() && shown < 8 {
                out.push_str(&format!("+{}\n", act[j]));
                j += 1;
                shown += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_chain_response, canonicalize_extraction, canonicalize_text, updating_goldens,
    };
    use ast_sgrep_core::chain::{ChainEdge, ChainNode, ChainResponse, EdgeLabel};
    use ast_sgrep_lang::{CallSite, ExtractionResult, ImportSite, SymbolDef, SymbolKind};

    fn node(file: &str, symbol: &str, line: u32) -> ChainNode {
        ChainNode {
            file: file.to_string(),
            line_start: line,
            line_end: line,
            symbol: Some(symbol.to_string()),
            language: Some("rust".to_string()),
            score: 1.0,
            depth: 0,
        }
    }

    fn edge(from: &str, to: &str) -> ChainEdge {
        ChainEdge {
            from_file: from.to_string(),
            from_symbol: Some("a".to_string()),
            to_file: to.to_string(),
            to_symbol: Some("b".to_string()),
            label: EdgeLabel::Calls,
            depth: 1,
        }
    }

    #[test]
    fn chain_canonicalize_matches_across_insertion_orders() {
        let a = ChainResponse {
            query: "q".to_string(),
            seeds: vec![node("b.rs", "b", 2), node("a.rs", "a", 1)],
            nodes: vec![node("b.rs", "b", 2), node("a.rs", "a", 1)],
            edges: vec![edge("b.rs", "a.rs"), edge("a.rs", "b.rs")],
            max_depth: 2,
            decay_factor: 0.5,
            node_count: 2,
            edge_count: 2,
        };
        let b = ChainResponse {
            query: "q".to_string(),
            seeds: vec![node("a.rs", "a", 1), node("b.rs", "b", 2)],
            nodes: vec![node("a.rs", "a", 1), node("b.rs", "b", 2)],
            edges: vec![edge("a.rs", "b.rs"), edge("b.rs", "a.rs")],
            max_depth: 2,
            decay_factor: 0.5,
            node_count: 2,
            edge_count: 2,
        };
        let ca = canonicalize_chain_response(a);
        let cb = canonicalize_chain_response(b);
        assert_eq!(ca.nodes[0].file, cb.nodes[0].file);
        assert_eq!(ca.nodes[1].file, cb.nodes[1].file);
        assert_eq!(ca.edges[0].from_file, cb.edges[0].from_file);
        assert_eq!(ca.edges[1].from_file, cb.edges[1].from_file);
        assert_eq!(ca.seeds[0].file, "a.rs");
    }

    #[test]
    fn extraction_canonicalize_matches_across_insertion_orders() {
        fn symbol(name: &str, kind: SymbolKind, start: usize) -> SymbolDef {
            SymbolDef {
                name: name.to_string(),
                kind,
                line_start: 1,
                line_end: 1,
                byte_start: start,
                byte_end: start + 1,
            }
        }
        fn call(caller: &str, callee: &str, line: u32) -> CallSite {
            CallSite {
                caller: caller.to_string(),
                callee: callee.to_string(),
                line,
                byte_start: 0,
                byte_end: 1,
            }
        }
        let a = ExtractionResult {
            symbols: vec![
                symbol("b", SymbolKind::Method, 10),
                symbol("a", SymbolKind::Function, 1),
            ],
            calls: vec![call("b", "a", 2), call("a", "b", 1)],
            imports: vec![
                ImportSite {
                    module_path: "z".into(),
                    line: 1,
                },
                ImportSite {
                    module_path: "a".into(),
                    line: 2,
                },
            ],
            pattern_nodes: Vec::new(),
        };
        let b = ExtractionResult {
            symbols: vec![
                symbol("a", SymbolKind::Function, 1),
                symbol("b", SymbolKind::Method, 10),
            ],
            calls: vec![call("a", "b", 1), call("b", "a", 2)],
            imports: vec![
                ImportSite {
                    module_path: "a".into(),
                    line: 2,
                },
                ImportSite {
                    module_path: "z".into(),
                    line: 1,
                },
            ],
            pattern_nodes: Vec::new(),
        };
        let ca = canonicalize_extraction(a);
        let cb = canonicalize_extraction(b);
        assert_eq!(ca.symbols[0].name, "a");
        assert_eq!(ca.symbols[1].name, "b");
        assert_eq!(ca.imports[0].module_path, "a");
        assert_eq!(ca.calls[0].caller, "a");
        assert_eq!(ca, cb);
    }

    #[test]
    fn canonicalize_text_crlf_and_trailing_ws() {
        assert_eq!(
            canonicalize_text("a  \r\nb\t\r\n\r\n"),
            "a\nb\n"
        );
    }

    #[test]
    fn updating_goldens_default_false() {
        assert!(!updating_goldens() || std::env::var("ASGREP_UPDATE_GOLDENS").is_ok());
    }
}
