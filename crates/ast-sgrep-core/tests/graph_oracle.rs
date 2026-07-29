//! Graph query oracle: indexed defs/callers/imports/chain must be retrievable.
//!
//! Bead ast-sgrep-55hl — catches the Issue #12 class (data indexed but not
//! retrievable) by indexing a known fixture and asserting non-empty parity for
//! every retrieval mode against a known symbol set, including mixed-case queries.
use ast_sgrep_core::chain::{expand_chain, ChainConfig, EdgeLabel};
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::store::IndexStore;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;

struct SymbolCase {
    /// Canonical name as written in source / stored by the indexer.
    stored: &'static str,
    /// Query spellings that must all retrieve the same indexed fact.
    queries: &'static [&'static str],
}

const SYMBOLS: &[SymbolCase] = &[
    SymbolCase {
        stored: "refresh_token",
        queries: &["refresh_token", "Refresh_Token", "REFRESH_TOKEN"],
    },
    SymbolCase {
        stored: "RefreshToken",
        queries: &["RefreshToken", "refreshtoken", "REFRESHTOKEN"],
    },
    SymbolCase {
        stored: "parseJSON",
        queries: &["parseJSON", "parsejson", "PARSEJSON"],
    },
    SymbolCase {
        stored: "MAIN",
        queries: &["MAIN", "main", "Main"],
    },
];

fn index_oracle_fixture() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    // Rust: snake + camel + SCREAMING defs with call edges.
    fs::write(
        corpus.path().join("auth.rs"),
        r#"
use crate::Utils::Helper;

fn refresh_token() {}
fn RefreshToken() { refresh_token(); }
fn parseJSON() { RefreshToken(); }
fn MAIN() { parseJSON(); }
fn entry() {
    refresh_token();
    RefreshToken();
    parseJSON();
    MAIN();
}
"#,
    )
    .unwrap();
    // TS: mixed-case module path for imports: coverage.
    fs::write(
        corpus.path().join("app.ts"),
        "import { Bar } from './Utils';\nexport function useUtils() { return Bar; }\n",
    )
    .unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    (corpus, index_dir, index_path)
}

fn searcher_for(root: &std::path::Path, index_path: &std::path::Path) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.to_path_buf()),
        limit: 32,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap()
}

#[test]
fn graph_oracle_defs_callers_imports_chain_parity() {
    let (corpus, _index_dir, index_path) = index_oracle_fixture();
    let searcher = searcher_for(corpus.path(), &index_path);
    let store = IndexStore::open(corpus.path(), Some(&index_path)).unwrap();
    let stats = store.status().unwrap();
    assert!(stats.symbol_count >= SYMBOLS.len(), "fixture must index symbols");
    assert!(stats.caller_count > 0, "fixture must index callers");
    assert!(stats.import_count > 0, "fixture must index imports");

    let mut defs_ok = 0usize;
    let mut callers_ok = 0usize;
    let mut chain_ok = 0usize;

    for sym in SYMBOLS {
        // Indexed count for this symbol name (exact stored casing).
        let indexed_defs = store.symbols_named(sym.stored, 32).unwrap();
        assert!(
            !indexed_defs.is_empty(),
            "store must contain def for {}",
            sym.stored
        );

        for q in sym.queries {
            // Chain expand_one feeds callee strings into symbols_named; case
            // variants must resolve to the stored definition.
            let named = store.symbols_named(q, 32).unwrap();
            assert!(
                named.iter().any(|s| s.name == sym.stored),
                "symbols_named({q}) must resolve stored {}; got {:#?}",
                sym.stored,
                named.iter().map(|s| &s.name).collect::<Vec<_>>()
            );

            let defs = searcher.search(&format!("defs:{q}")).unwrap();
            let def_hits: Vec<_> = defs
                .hits
                .iter()
                .filter(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some(sym.stored))
                .collect();
            assert!(
                !def_hits.is_empty(),
                "defs:{q} must retrieve stored symbol {}; got {:#?}",
                sym.stored,
                defs.hits
            );
            defs_ok += 1;

            let callers = searcher.search(&format!("callers:{q}")).unwrap();
            let caller_hits: Vec<_> = callers
                .hits
                .iter()
                .filter(|h| h.kind == HitKind::Caller && h.callee.as_deref() == Some(sym.stored))
                .collect();
            assert!(
                !caller_hits.is_empty(),
                "callers:{q} must retrieve calls to {}; got {:#?}",
                sym.stored,
                callers.hits
            );
            assert!(
                caller_hits.iter().all(|h| h.score > 0.0),
                "callers:{q} hits must have positive score"
            );
            callers_ok += 1;
        }

        let chain = expand_chain(
            &store,
            sym.stored,
            &ChainConfig {
                top_n: 8,
                max_depth: 2,
                limit: 32,
                ..ChainConfig::default()
            },
        )
        .unwrap();
        let has_symbol = chain
            .nodes
            .iter()
            .chain(chain.seeds.iter())
            .any(|n| n.symbol.as_deref() == Some(sym.stored))
            || chain.edges.iter().any(|e| {
                e.to_symbol.as_deref() == Some(sym.stored)
                    || e.from_symbol.as_deref() == Some(sym.stored)
                    || matches!(e.label, EdgeLabel::Calls | EdgeLabel::CalledBy)
            });
        assert!(
            has_symbol || !chain.nodes.is_empty() || !chain.seeds.is_empty(),
            "chain {} must produce graph structure; nodes={:#?} edges={:#?}",
            sym.stored,
            chain.nodes,
            chain.edges
        );
        chain_ok += 1;
    }

    // imports: mixed-case module path parity (TS './Utils').
    for q in ["imports:./Utils", "imports:./utils", "imports:./UTILS"] {
        let resp = searcher.search(q).unwrap();
        assert!(
            resp.hits
                .iter()
                .any(|h| h.kind == HitKind::Import && h.symbol.as_deref() == Some("./Utils")),
            "{q} must return Import './Utils'; got {:#?}",
            resp.hits
        );
    }

    // Non-empty parity gate: at least N symbols × query variants covered.
    assert!(defs_ok >= 12, "expected >=12 defs assertions, got {defs_ok}");
    assert!(
        callers_ok >= 12,
        "expected >=12 callers assertions, got {callers_ok}"
    );
    assert_eq!(chain_ok, SYMBOLS.len());
}
