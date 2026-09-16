//! br-23f: finish.rs ranking must be a total order — MCP cross-process byte-stability.
//!
//! Contract (crates/ast-sgrep-mcp/src/lib.rs: "Search envelopes are
//! deterministic for the same query and index generation"): two hits tying on
//! score, coverage, file, and line_start but distinct in line_end/symbol must
//! serialize identically no matter what order the upstream channel fed them
//! in. cmp_ranked_ends_at_line_start historically stopped at line_start and
//! relied on input order for such pairs; that input comes from a randomly
//! seeded HashMap in lexical_from_fts, so tied pairs could flip between
//! processes. This test drives finish_response twice with the tied pair in
//! opposite orders and demands byte-identical JSON both times.
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::{finish_response, HitKind, HitSignal, SearchHit, SearchOptions};

fn tied_hit(symbol: &str, line_end: u32) -> SearchHit {
    SearchHit {
        kind: HitKind::Caller,
        file: "src/app.rs".into(),
        line_start: 81,
        line_end,
        symbol: Some(symbol.into()),
        caller: Some("run_pipeline".into()),
        callee: Some("refresh_token".into()),
        language: Some("rust".into()),
        score: 2.0,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Caller],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: "run_pipeline(); refresh_token();".into(),
        byte_span: None,
    }
}

fn tie_pair() -> Vec<SearchHit> {
    // Two DISTINCT callers on the same source line: identical score,
    // coverage (single term, equal excerpts), file, line_start — differing
    // only in symbol/line_end/callee.
    vec![tied_hit("caller_one", 81), tied_hit("caller_two", 82)]
}

fn finished_json(hits: Vec<SearchHit>) -> String {
    let parsed = ParsedQuery::literal("refresh_token");
    let options = SearchOptions::default();
    let response = finish_response(&parsed, &options, hits, false);
    serde_json::to_string(&response).unwrap()
}

#[test]
fn tied_hits_serialize_identically_regardless_of_input_order() {
    let forward = finished_json(tie_pair());
    let mut reversed = tie_pair();
    reversed.reverse();
    let backward = finished_json(reversed);
    assert_eq!(
        forward, backward,
        "same query+index generation must produce byte-identical output \
         regardless of upstream channel order (br-23f)"
    );
}

// ---------------------------------------------------------------------------
// PASS 56 (P48-R1 remediation): per-process emission-order determinism.
//
// Two pipeline points historically emitted rows in randomly-seeded
// HashMap iteration order: `hits_from_matches` (lexical) and the fused-row
// loop of `apply_weighted_rrf`. br-23f hardened the finish comparator, which
// masks the order end-to-end today — but the nondeterministic decision still
// existed at the source, one refactor away from leaking into the byte-stable
// envelope contract. These tests pin the contract at BOTH levels:
//  - `weighted_rrf_emits_fused_rows_in_deterministic_order` asserts the
//    fused emission order directly (RED pre-fix: every call re-rolled the
//    HashMap seed, so 24 consecutive runs cannot agree);
//  - `search_responses_are_byte_identical_across_fresh_searchers` drives the
//    full pipeline (fresh Searcher = fresh HashMap seeds) per cell, ≥5x.
// ---------------------------------------------------------------------------

use ast_sgrep_core::fusion::apply_weighted_rrf;
use ast_sgrep_core::intent::{weights_for, QueryIntent};

fn channel_hit(file: &str, line: u32, kind: HitKind) -> SearchHit {
    SearchHit {
        kind,
        file: file.into(),
        line_start: line,
        line_end: line,
        symbol: Some("run_pipeline".into()),
        caller: None,
        callee: Some("refresh_token".into()),
        language: Some("rust".into()),
        score: 2.0,
        signal: HitSignal::Exact,
        contributors: vec![kind],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: "run_pipeline(); refresh_token();".into(),
        byte_span: None,
    }
}

fn fused_keys(hits: &[SearchHit]) -> Vec<(String, u32)> {
    hits.iter()
        .map(|hit| (hit.file.clone(), hit.line_start))
        .collect()
}

#[test]
fn weighted_rrf_emits_fused_rows_in_deterministic_order() {
    let weights = weights_for(QueryIntent::Symbol);
    // 8 distinct result loci, each confirmed by two channels → 8 fused rows.
    let build = || -> Vec<SearchHit> {
        (0..8)
            .flat_map(|i| {
                let file = format!("src/mod{i}.rs");
                [
                    channel_hit(&file, 10, HitKind::Asgrep),
                    channel_hit(&file, 10, HitKind::Embed),
                ]
            })
            .collect()
    };

    let mut reference = build();
    apply_weighted_rrf(&mut reference, &weights);
    let reference_keys = fused_keys(&reference);
    assert_eq!(reference_keys.len(), 8, "one fused row per locus");

    for run in 0..24 {
        let mut hits = build();
        apply_weighted_rrf(&mut hits, &weights);
        let keys = fused_keys(&hits);
        assert_eq!(
            keys, reference_keys,
            "run {run}: fused emission order must not depend on HashMap seed"
        );
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "run {run}: emission must be key-sorted");
    }
}

/// End-to-end loop pin (the pass-54 "10x loop" cell class): identical
/// invocations over one CLI-built index must return byte-identical hit
/// sequences. Every iteration constructs a FRESH Searcher so every internal
/// HashMap re-rolls its per-process seed.
mod loop_pins {
    use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher};
    use std::fs;
    use tempfile::TempDir;

    const CELL: &str = "def calc(a, b):\n    return a + b\n\n\n\
def work():\n    total = calc(1, 2)\n    padded = calc( 1, 2 )\n    return total\n\n\n\
def again():\n    return calc( 1, 2 )\n";

    fn indexed_fixture() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("fixture");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("cells.py"), CELL).unwrap();
        let index_path = temp.path().join("index.db");
        let mut indexer = Indexer::new(IndexOptions {
            root: root.clone(),
            index_path: Some(index_path.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap();
        let stats = indexer.index_all().unwrap();
        assert!(stats.files_indexed >= 1, "fixture must index");
        (temp, root, index_path)
    }

    fn fresh_searcher(root: &std::path::Path, db: &std::path::Path) -> Searcher {
        let store = IndexStore::open(root, Some(db)).unwrap();
        Searcher::with_store(
            store,
            SearchOptions {
                root: root.to_path_buf(),
                use_embed: false,
                ..SearchOptions::default()
            },
        )
    }

    fn hit_identity(
        response: &ast_sgrep_core::SearchResponse,
    ) -> Vec<(String, u32, u32, u64, String, String)> {
        response
            .hits
            .iter()
            .map(|hit| {
                (
                    hit.file.clone(),
                    hit.line_start,
                    hit.line_end,
                    hit.score.to_bits(),
                    format!("{:?}", hit.kind),
                    hit.excerpt.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn search_responses_are_byte_identical_across_fresh_searchers() {
        let (_temp, root, index_path) = indexed_fixture();
        for query in [
            "pattern:calc(1, 2)",
            "pattern:calc",
            "literal:calc(1, 2)",
            "calc(1, 2)",
        ] {
            let mut reference: Option<Vec<_>> = None;
            for run in 0..5 {
                let searcher = fresh_searcher(&root, &index_path);
                let response = searcher.search(query).expect("search must succeed");
                let identity = hit_identity(&response);
                if let Some(prev) = &reference {
                    assert_eq!(
                        prev, &identity,
                        "query {query:?} run {run}: identical invocation diverged \
                         across fresh searchers"
                    );
                }
                reference = Some(identity);
            }
        }
    }
}
