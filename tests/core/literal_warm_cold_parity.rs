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
//! C3 hybrid parity — the hybrid prefilter fallback (cold) and the corpus
//!    distinct-files scan (warm) must agree hit-for-hit on under-cap
//!    unprefixed queries: same files, same representative lines, same
//!    scores. The warm token lookup locates the actual first matching line
//!    per file via bounded memchr (`hits_from_file_ids`), not a first-line
//!    stub: a stub merges differently downstream and surfaces non-matching
//!    excerpts as evidence.
//! C4 mixed-kind symbol parity — over-budget symbol queries with minority
//!    kinds must agree too: both paths sort by (kind, path, line, name) and
//!    apply the same quota buckets (functions plus reserved type/other
//!    slots), so the cold SQL window and the warmed table select the same
//!    rows. (A rowid-ordered head-cut with no quotas would crowd minority
//!    kinds out of the cold page while the warmed page reserves them slots.)
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

// 20 files x (28 fill defs + 3 qzz fns) x 2 lines + 5 classes x 2 lines +
// 4 marker lines = 1254 indexed lines: above BMH_LINE_THRESHOLD (1000) so
// the cold path is the trigram scan, not literal_sql (except the
// short-needle query, which pins literal_sql).
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
        // C4 vehicles: 60 single-kind functions plus 5 minority-kind classes
        // sharing one rare term, so quota bucketing (not a head-cut) decides
        // the over-budget symbol page.
        for k in 0..3 {
            body.push_str(&format!("def qzz_f{f}_{k}(value):\n    return value\n"));
        }
        if f < 5 {
            body.push_str(&format!("class qzz_C{f}:\n    pass\n"));
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
        let cold_hits: Vec<_> = cold
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        assert!(
            !cold_hits.is_empty(),
            "fixture must answer hybrid {query}; both-empty agreement proves nothing"
        );
        let warm_hits: Vec<_> = warmed
            .search(query)
            .unwrap()
            .hits
            .iter()
            .map(hit_key)
            .collect();
        assert_eq!(
            cold_hits, warm_hits,
            "cold/warm hybrid divergence for {query}"
        );
    }
}

#[test]
fn c4_cold_and_warmed_mixed_kind_symbol_query_agree() {
    let temp = setup();
    let root = temp.path();
    let cold = searcher_for(root);
    let warmed = searcher_for(root);
    warmed.warm_search_path().unwrap();
    // 60 functions + 5 classes share `qzz` (over the 50-row symbol budget):
    // both paths must select the same quota-balanced rows.
    let cold_hits: Vec<_> = cold
        .search("qzz")
        .unwrap()
        .hits
        .iter()
        .map(hit_key)
        .collect();
    assert!(
        !cold_hits.is_empty(),
        "fixture must answer hybrid qzz; both-empty agreement proves nothing"
    );
    let warm_hits: Vec<_> = warmed
        .search("qzz")
        .unwrap()
        .hits
        .iter()
        .map(hit_key)
        .collect();
    assert_eq!(
        cold_hits, warm_hits,
        "cold/warm mixed-kind symbol divergence for qzz"
    );
}
