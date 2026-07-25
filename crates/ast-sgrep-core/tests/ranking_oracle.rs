/// e2hc.19(e): Wire tests/fixtures/ranking/cases.json into the test suite.
/// The fixture existed but no repository consumer loaded it, so the expected
/// ranks protected no invariant. This test deserializes the cases, indexes the
/// sample corpus, runs each query, and asserts the must_include constraints.
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use ast_sgrep_testkit::index_sample;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RankingCases {
    cases: Vec<RankingCase>,
}

#[derive(Debug, Deserialize)]
struct RankingCase {
    name: String,
    query: String,
    top_k: u32,
    must_include: Vec<MustInclude>,
}

#[derive(Debug, Deserialize)]
struct MustInclude {
    kind: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    callee: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    excerpt_contains: Option<String>,
    max_rank: usize,
}

fn hit_kind_from_str(s: &str) -> HitKind {
    match s {
        "def" => HitKind::Def,
        "caller" => HitKind::Caller,
        "embed" => HitKind::Embed,
        "graph" => HitKind::Graph,
        "anchor" => HitKind::Anchor,
        "pattern" => HitKind::Pattern,
        _ => HitKind::Asgrep,
    }
}

fn hit_matches(hit: &ast_sgrep_core::SearchHit, req: &MustInclude) -> bool {
    if hit.kind != hit_kind_from_str(&req.kind) {
        return false;
    }
    if let Some(ref sym) = req.symbol {
        if hit.symbol.as_deref() != Some(sym) {
            return false;
        }
    }
    if let Some(ref callee) = req.callee {
        if hit.callee.as_deref() != Some(callee) {
            return false;
        }
    }
    if let Some(ref file) = req.file {
        if !hit.file.ends_with(file) {
            return false;
        }
    }
    if let Some(ref needle) = req.excerpt_contains {
        if !hit.excerpt.contains(needle) {
            return false;
        }
    }
    true
}

#[test]
fn ranking_oracle_cases_json() {
    let cases_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ranking/cases.json");
    let json = std::fs::read_to_string(&cases_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", cases_path.display()));
    let cases: RankingCases =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("parse cases.json: {e}"));

    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let searcher = Searcher::new(SearchOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        limit: 32,
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher");

    let mut failures = Vec::new();
    for case in &cases.cases {
        let resp = searcher.search(&case.query).expect("search");
        let hits = &resp.hits;
        for req in &case.must_include {
            let found = hits.iter().take(req.max_rank).any(|h| hit_matches(h, req));
            if !found {
                failures.push(format!(
                    "case '{}' must_include kind={} symbol={:?} callee={:?} file={:?} max_rank={} not satisfied; hits: {}",
                    case.name,
                    req.kind,
                    req.symbol,
                    req.callee,
                    req.file,
                    req.max_rank,
                    hits.iter().take(8).map(|h| format!("{:?}({},{:?})", h.kind, h.file, h.symbol)).collect::<Vec<_>>().join(", ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "ranking oracle failures:\n{}", failures.join("\n"));
}
