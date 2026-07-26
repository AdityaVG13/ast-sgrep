/// e2hc.19(e): Wire tests/fixtures/ranking/cases.json into the test suite.
/// The fixture existed but no repository consumer loaded it, so the expected
/// ranks protected no invariant. This test deserializes the cases, indexes the
/// sample corpus, runs each query, and asserts the must_include constraints.
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use ast_sgrep_testkit::index_sample;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RankingCases {
    fixture: String,
    cases: Vec<RankingCase>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RankingCase {
    name: String,
    query: String,
    #[serde(default)]
    mode: RetrievalMode,
    top_k: u32,
    must_include: Vec<MustInclude>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RetrievalMode {
    #[default]
    Hybrid,
    Semantic,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RequiredKind {
    Asgrep,
    Def,
    Caller,
    Graph,
    Anchor,
    Import,
    Pattern,
    Embed,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MustInclude {
    kind: RequiredKind,
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

fn required_hit_kind(kind: RequiredKind) -> HitKind {
    match kind {
        RequiredKind::Asgrep => HitKind::Asgrep,
        RequiredKind::Def => HitKind::Def,
        RequiredKind::Caller => HitKind::Caller,
        RequiredKind::Graph => HitKind::Graph,
        RequiredKind::Anchor => HitKind::Anchor,
        RequiredKind::Import => HitKind::Import,
        RequiredKind::Pattern => HitKind::Pattern,
        RequiredKind::Embed => HitKind::Embed,
    }
}

fn hit_matches(hit: &ast_sgrep_core::SearchHit, req: &MustInclude) -> bool {
    if hit.kind != required_hit_kind(req.kind) {
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

    assert_eq!(
        cases.fixture, "sample",
        "ranking fixture must target sample corpus"
    );
    let indexed = index_sample(IndexOptions {
        force_reindex: true,
        ..IndexOptions::default()
    });
    let root = indexed.indexer.store().root().to_path_buf();
    let index_path = indexed.indexer.store().db_path().to_path_buf();

    let mut failures = Vec::new();
    for case in &cases.cases {
        let top_k = usize::try_from(case.top_k).expect("top_k fits usize");
        assert!(
            !case.must_include.is_empty(),
            "case {} must contain at least one identity expectation",
            case.name
        );
        assert!(
            top_k > 0,
            "case {} must request at least one hit",
            case.name
        );
        let searcher = Searcher::new(SearchOptions {
            root: root.clone(),
            index_path: Some(index_path.clone()),
            limit: top_k,
            use_embed: true,
            ..SearchOptions::default()
        })
        .expect("searcher");
        let resp = match case.mode {
            RetrievalMode::Hybrid => searcher.search(&case.query),
            RetrievalMode::Semantic => searcher.search_semantic(&case.query),
        }
        .expect("search");
        let hits = &resp.hits;
        assert!(
            hits.len() <= top_k,
            "case {} returned {} hits beyond top_k={top_k}",
            case.name,
            hits.len()
        );
        for req in &case.must_include {
            assert!(
                req.max_rank > 0 && req.max_rank <= top_k,
                "case {} max_rank={} must be within top_k={top_k}",
                case.name,
                req.max_rank
            );
            let found = hits.iter().take(req.max_rank).any(|h| hit_matches(h, req));
            if !found {
                failures.push(format!(
                    "case '{}' must_include kind={:?} symbol={:?} callee={:?} file={:?} max_rank={} not satisfied; hits: {}",
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
    assert!(
        failures.is_empty(),
        "ranking oracle failures:\n{}",
        failures.join("\n")
    );
}
