//! Exact identifiers rank the definition; conceptual queries prefer code over docs.
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn indexed_corpus() -> (TempDir, Searcher) {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/search.rs",
        r#"
pub struct Searcher {
    root: String,
}

pub fn bench_searcher() {}
pub fn open_searcher() {}
pub fn searcher() {}

pub fn search_hybrid(_query: &str) {}
"#,
    );
    write_src(
        temp.path(),
        "src/auth.rs",
        r#"
/// Renew the credential before the current session expires.
pub fn auth_refresh() {
    let token = fetch_token();
    store_token(token);
}

fn fetch_token() -> u32 { 1 }
fn store_token(_token: u32) {}

fn main() {
    auth_refresh();
}
"#,
    );
    write_src(
        temp.path(),
        "README.md",
        r#"
Query: "credential renewal"
  → semantic pass ranks auth_refresh (zero token overlap)
"#,
    );
    write_src(
        temp.path(),
        "CHANGELOG.md",
        "hybrid search that understands intent
",
    );
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 12,
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher");
    (temp, searcher)
}

fn rank_of(
    hits: &[ast_sgrep_core::SearchHit],
    pred: impl Fn(&ast_sgrep_core::SearchHit) -> bool,
) -> Option<usize> {
    hits.iter().position(pred).map(|i| i + 1)
}

#[test]
fn identifier_spelling_keeps_user_case() {
    let parsed = ParsedQuery::parse("Searcher");
    assert_eq!(parsed.identifier_spelling(), Some("Searcher"));
    let defs = ParsedQuery::parse("defs:Searcher");
    assert_eq!(defs.identifier_spelling(), Some("Searcher"));
}

#[test]
fn searcher_query_ranks_the_type_not_helpers() {
    let (_temp, searcher) = indexed_corpus();
    let response = searcher.search("Searcher").expect("search");
    let rank = rank_of(&response.hits, |hit| {
        hit.kind == HitKind::Def && hit.symbol.as_deref() == Some("Searcher")
    });
    assert!(
        rank.is_some_and(|r| r <= 5),
        "struct Searcher should be in the top 5, got rank {rank:?} hits {:?}",
        response
            .hits
            .iter()
            .take(8)
            .map(|h| format!("{:?} {:?}", h.kind, h.symbol))
            .collect::<Vec<_>>()
    );
    assert!(
        response.query_expansions.is_empty(),
        "identifier queries must not advertise co-occurrence expansion: {:?}",
        response.query_expansions
    );
}

#[test]
fn defs_searcher_ranks_exact_case_first() {
    let (_temp, searcher) = indexed_corpus();
    let response = searcher.search("defs:Searcher").expect("defs");
    assert_eq!(
        response.hits.is_empty(),
        false,
        "defs:Searcher must return hits"
    );
    assert_eq!(
        response.hits[0].symbol.as_deref(),
        Some("Searcher"),
        "exact-case type must beat lowercase searcher helpers, hits {:?}",
        response
            .hits
            .iter()
            .map(|h| h.symbol.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn credential_renewal_ranks_code_not_readme() {
    let (_temp, searcher) = indexed_corpus();
    let response = searcher.search("credential renewal").expect("search");
    let code_rank = rank_of(&response.hits, |hit| {
        hit.symbol.as_deref() == Some("auth_refresh")
            && matches!(hit.kind, HitKind::Def | HitKind::Embed | HitKind::Pattern)
    });
    let readme_first = response
        .hits
        .first()
        .is_some_and(|hit| hit.file.ends_with("README.md"));
    assert_eq!(
        readme_first,
        false,
        "README must not eat the credential-renewal demo, hits {:?}",
        response
            .hits
            .iter()
            .take(8)
            .map(|h| format!("{:?} {} {:?}", h.kind, h.file, h.symbol))
            .collect::<Vec<_>>()
    );
    assert!(
        code_rank.is_some_and(|r| r <= 3),
        "auth_refresh should be in the top 3, got {code_rank:?} hits {:?}",
        response
            .hits
            .iter()
            .take(8)
            .map(|h| format!("{:?} {} {:?}", h.kind, h.file, h.symbol))
            .collect::<Vec<_>>()
    );
    let first_is_main = response.hits.first().is_some_and(|hit| {
        hit.caller.as_deref() == Some("main") || hit.symbol.as_deref() == Some("main")
    });
    assert_eq!(
        first_is_main,
        false,
        "entrypoint callers must not outrank auth_refresh, hits {:?}",
        response
            .hits
            .iter()
            .take(8)
            .map(|h| format!(
                "{:?} {} {:?} caller={:?}",
                h.kind, h.file, h.symbol, h.caller
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn auth_refresh_identifier_ranks_the_definition() {
    let (_temp, searcher) = indexed_corpus();
    let response = searcher.search("auth_refresh").expect("search");
    assert_eq!(
        response.hits.is_empty(),
        false,
        "auth_refresh must return hits"
    );
    assert_eq!(
        response.hits[0].symbol.as_deref(),
        Some("auth_refresh"),
        "exact identifier must beat substring refresh helpers, hits {:?}",
        response
            .hits
            .iter()
            .take(8)
            .map(|h| format!("{:?} {} {:?}", h.kind, h.file, h.symbol))
            .collect::<Vec<_>>()
    );
}

#[test]
fn hybrid_nl_ranks_search_hybrid_definition() {
    let (_temp, searcher) = indexed_corpus();
    let response = searcher
        .search("how does hybrid search work")
        .expect("search");
    let rank = rank_of(&response.hits, |hit| {
        hit.file.ends_with("search.rs")
            && (hit.symbol.as_deref() == Some("search_hybrid")
                || hit.symbol.as_deref() == Some("Searcher"))
    });
    assert!(
        rank.is_some_and(|r| r <= 8),
        "search.rs definition should be in the top 8, got {rank:?} hits {:?}",
        response
            .hits
            .iter()
            .take(8)
            .map(|h| format!("{:?} {} {:?}", h.kind, h.file, h.symbol))
            .collect::<Vec<_>>()
    );
}
