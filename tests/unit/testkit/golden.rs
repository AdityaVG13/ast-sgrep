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
    assert_eq!(canonicalize_text("a  \r\nb\t\r\n\r\n"), "a\nb\n");
}

#[test]
fn updating_goldens_default_false() {
    assert!(!updating_goldens() || std::env::var("ASGREP_UPDATE_GOLDENS").is_ok());
}
