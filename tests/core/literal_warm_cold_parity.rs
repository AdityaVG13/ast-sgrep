//! One-shot `literal:` must not pay the RAM-corpus load; cold and warmed
//! searchers must agree (hit-for-hit on the literal lane, file-sets on
//! hybrid — see C2/C3 for the exact contracts).
//!
//! Flamegraph (2026-09-20, 3/3 captures): ~95% of one-shot `literal:SearchHit`
//! wall sat under `literal_pass` -> `IndexStore::line_corpus` — a full `lines`
//! table materialization plus per-line token indexing. The corpus pays off
//! across queries in a warmed session (codemode-serve); a single query must
//! take the trigram/SQL path instead.
//!
//! Contracts:
//! C1 no-implicit-load — a cold `Searcher::search("literal:...")` leaves no
//!    resident corpus behind; only `warm_search_path` loads one.
//! C2 warm/cold parity — for under-cap queries, cold (trigram) and warmed
//!    (corpus) literal hits agree on file, line, score bits, and excerpt.
//!    (Over-cap candidate sets may legitimately differ: posting-order cap vs
//!    scan-order cap. All fixtures here stay under cap.)
//! C3 hybrid file-set parity — the hybrid prefilter fallback (cold) admits
//!    the same files as the corpus distinct-files scan (warm) for under-cap
//!    unprefixed queries. Scores/counts/excerpts are deliberately NOT
//!    compared: the warm token lookup returns each file's first line as a
//!    cheap stub (`hits_from_file_ids`), which merges differently downstream
//!    than the cold fallback's actual first matching line (e.g. 6 warm hits
//!    vs 3 cold for one 3-file query — the warm extras are non-matching
//!    first-line stubs). That warm-path presentation quirk is pre-existing
//!    behavior, out of scope for this change; the funnel's load-bearing
//!    contract is the admitted file set, pinned here.
//!    (Over-budget symbol queries are excluded: the warmed in-memory symbol
//!    table and the cold SQL LIKE fallback select different over-cap sets —
//!    likewise pre-existing and untouched.)
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

// 20 files x 28 defs x 2 lines + 4 marker lines = 1124 indexed lines: above
// BMH_LINE_THRESHOLD (1000) so the cold path is the trigram scan, not
// literal_sql (except the short-needle query, which pins literal_sql).
const FILLER_FILES: usize = 20;
const FILLER_DEFS: usize = 28;

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn setup() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    for f in 0..FILLER_FILES {
        let mut body = String::new();
        for i in 0..FILLER_DEFS {
            body.push_str(&format!(
                "def fill_{f}_{i}(value):\n    return value * {i} + {f}\n"
            ));
        }
        if f == 0 {
            body.push_str("ALPHA_ZZQUUX_MARKER_PAYLOAD sentinel\n");
        }
        if f < 3 {
            body.push_str("beta_shared_rare_token payload\n");
        }
        write_src(root, &format!("src/mod_{f}.py"), &body);
    }
    let index_path = root.join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    temp
}

fn searcher_for(root: &std::path::Path) -> Searcher {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(root.join("index.db")),
        limit: 50,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap()
}

fn hit_key(hit: &ast_sgrep_core::SearchHit) -> (String, u32, u64, String) {
    (
        hit.file.clone(),
        hit.line_start,
        hit.score.to_bits(),
        hit.excerpt.clone(),
    )
}

#[test]
fn c1_cold_literal_search_leaves_no_resident_corpus() {
    let temp = setup();
    let searcher = searcher_for(temp.path());
    assert!(
        !searcher.store().has_resident_line_corpus(),
        "fresh searcher must start without a resident corpus"
    );
    let resp = searcher.search("literal:ZZQUUX").unwrap();
    assert!(
        !resp.hits.is_empty(),
        "fixture must contain the needle; got no hits"
    );
    assert!(
        !searcher.store().has_resident_line_corpus(),
        "one-shot literal must take the trigram/SQL path, not load the RAM corpus"
    );
    searcher.warm_search_path().unwrap();
    assert!(
        searcher.store().has_resident_line_corpus(),
        "explicit warm must still residently load the corpus for the session"
    );
}

#[test]
fn c2_cold_trigram_and_warmed_corpus_literal_agree() {
    let temp = setup();
    let root = temp.path();
    let cold = searcher_for(root);
    let warmed = searcher_for(root);
    warmed.warm_search_path().unwrap();
    assert!(
        warmed.store().has_resident_line_corpus(),
        "warmed searcher must actually hold the corpus for this comparison"
    );
    for query in [
        "literal:ZZQUUX",
        "literal:beta_shared_rare_token",
        "literal:fill_7_13",
        "literal:ZZ", // short needle: cold takes literal_sql, warm takes corpus
        "word:sentinel",
        "literal:valeur_absente",
    ] {
        let cold_hits: Vec<_> = cold
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        let warm_hits: Vec<_> = warmed
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        if query != "literal:valeur_absente" {
            assert!(
                !cold_hits.is_empty(),
                "fixture must contain {query}; both-empty agreement proves nothing"
            );
        }
        assert_eq!(cold_hits, warm_hits, "cold/warm hit divergence for {query}");
    }
}

#[test]
fn c3_cold_and_warmed_hybrid_identifier_agree() {
    let temp = setup();
    let root = temp.path();
    let cold = searcher_for(root);
    let warmed = searcher_for(root);
    warmed.warm_search_path().unwrap();
    for query in ["ZZQUUX", "beta_shared_rare_token"] {
        let cold_files: std::collections::BTreeSet<_> = cold
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(|hit| hit.file.clone())
            .collect();
        assert!(
            !cold_files.is_empty(),
            "fixture must answer hybrid {query}; both-empty agreement proves nothing"
        );
        let warm_files: std::collections::BTreeSet<_> = warmed
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(|hit| hit.file.clone())
            .collect();
        assert_eq!(
            cold_files, warm_files,
            "cold/warm hybrid file-set divergence for {query}"
        );
    }
}
