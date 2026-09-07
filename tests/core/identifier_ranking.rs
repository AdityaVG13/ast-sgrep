//! Exact identifiers rank the definition; conceptual queries prefer code over docs.
use ast_sgrep_core::query::ParsedQuery;
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
        "src/throttle.rs",
        r#"
pub fn rate_limit_client(client_id: &str) -> bool {
    !client_id.is_empty()
}
"#,
    );
    write_src(
        temp.path(),
        "src/debounce.rs",
        r#"
pub fn coalesce_watch_events(pending: usize) -> bool {
    pending > 0
}
"#,
    );
    write_src(
        temp.path(),
        "src/retry.rs",
        r#"
pub fn backoff_attempt(attempt: u32) -> u64 {
    1u64 << attempt.min(16)
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
    write_src(
        temp.path(),
        "src/search/conjunction.rs",
        r#"
//! Two-channel conjunction: intersect two prefixed channels in one query.
pub fn combine(left: &str, right: &str) -> bool {
    !left.is_empty() && !right.is_empty()
}
"#,
    );
    write_src(
        temp.path(),
        "src/fusion.rs",
        r#"
pub fn channel_sensitivity() {}
pub fn learn_fusion_weights() {}
pub struct FusionChannel;
"#,
    );
    write_src(
        temp.path(),
        "src/eval.rs",
        r#"
pub fn print_single() {}
pub fn single_json() {}
pub fn run_single() {}
"#,
    );
    write_src(
        temp.path(),
        "src/search/field_weight.rs",
        r#"
pub fn combine_field_scores() {}
pub fn field_weights() {}
"#,
    );
    write_src(
        temp.path(),
        "tests/core/cascade_planner.rs",
        r#"
#[test]
fn hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages() {
    let _ = "how does hybrid search work";
}
"#,
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

fn autopsy(hits: &[ast_sgrep_core::SearchHit]) -> String {
    if hits.is_empty() {
        return "    <no hits — the engine returned nothing>".into();
    }
    hits.iter()
        .enumerate()
        .map(|(i, h)| {
            format!(
                "    #{:<2} score={:.4} kind={:?} symbol={:?} file={}",
                i + 1,
                h.score,
                h.kind,
                h.symbol,
                h.file
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn require_first(
    query: &str,
    hits: &[ast_sgrep_core::SearchHit],
    file_suffix: &str,
    symbol: &str,
    contract: &str,
) {
    let board = autopsy(hits);
    let Some(first) = hits.first() else {
        panic!(
            "\n\n===== DEAD PROGRAM: ZERO HITS =====\n\
             Query: {query:?}\n\
             Contract: {contract}\n\
             Required #1: {symbol} in *{file_suffix}\n\
             The engine produced an empty list. This is not a ranking miss; \
             search did not run as a program.\n\
             ===== END AUTOPSY =====\n"
        );
    };
    let file_ok = first.file.replace('\\', "/").ends_with(file_suffix);
    let symbol_ok = first.symbol.as_deref() == Some(symbol);
    if file_ok && symbol_ok {
        return;
    }
    panic!(
        "\n\n===== THIS IS NOT A SEARCH ENGINE =====\n\
         Query: {query:?}\n\
         Contract: {contract}\n\
         Required #1: {symbol} in *{file_suffix}\n\
         Actual   #1: {:?} {} {:?}\n\
         If this ranking looks 'close enough', it is not. First is first.\n\n\
         FULL SCOREBOARD:\n{board}\n\
         ===== END AUTOPSY =====\n",
        first.kind, first.file, first.symbol
    );
}

#[test]
fn searcher_query_ranks_the_type_not_helpers() {
    let (_temp, searcher) = indexed_corpus();
    let query = "Searcher";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "search.rs",
        "Searcher",
        "typed identifier Searcher is the type, not bench_searcher/open_searcher/searcher helpers. Rank 2 is a dead program.",
    );
    if !response.query_expansions.is_empty() {
        panic!(
            "\n\n===== DEAD PROGRAM: IDENTIFIER QUERY EXPANDED =====\n\
             Query: {query:?}\n\
             Expansions: {:?}\n\
             FULL SCOREBOARD:\n{}\n\
             ===== END AUTOPSY =====\n",
            response.query_expansions,
            autopsy(&response.hits)
        );
    }
}

#[test]
fn defs_searcher_ranks_exact_case_first() {
    let (_temp, searcher) = indexed_corpus();
    let query = "defs:Searcher";
    let response = searcher.search(query).expect("defs");
    require_first(
        query,
        &response.hits,
        "search.rs",
        "Searcher",
        "defs:Searcher must be the type definition first. Lowercase searcher helpers at #1 means the program does not understand case.",
    );
}

#[test]
fn credential_renewal_ranks_code_not_readme() {
    let (_temp, searcher) = indexed_corpus();
    let query = "credential renewal";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "auth.rs",
        "auth_refresh",
        "conceptual NL must rank auth_refresh first. README, changelog, or main at #1 means this is not a program.",
    );
}

#[test]
fn auth_refresh_identifier_ranks_the_definition() {
    let (_temp, searcher) = indexed_corpus();
    let query = "auth_refresh";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "auth.rs",
        "auth_refresh",
        "exact identifier must be the definition first. A substring helper at #1 is a dead program.",
    );
}

#[test]
fn hybrid_nl_ranks_implementation_above_tests() {
    let (_temp, searcher) = indexed_corpus();
    let query = "how does hybrid search work";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "search.rs",
        "search_hybrid",
        "conceptual NL must rank the hybrid implementation first, not a tests/ name-dump",
    );
    let test_rank = rank_of(&response.hits, |hit| {
        hit.file
            .replace('\\', "/")
            .split('/')
            .any(|seg| seg.eq_ignore_ascii_case("tests") || seg.eq_ignore_ascii_case("test"))
    });
    if let Some(test_rank) = test_rank {
        if test_rank == 1 {
            panic!(
                "\n\n===== DEAD PROGRAM: TEST FILE IS #1 =====\n\
                 Query: {query:?}\n\
                 FULL SCOREBOARD:\n{}\n\
                 ===== END AUTOPSY =====\n",
                autopsy(&response.hits)
            );
        }
    }
}

#[test]
fn defs_query_still_finds_test_function() {
    let (_temp, searcher) = indexed_corpus();
    let query = "defs:hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages";
    let response = searcher.search(query).expect("defs");
    require_first(
        query,
        &response.hits,
        "cascade_planner.rs",
        "hybrid_query_cascades_lexical_files_into_structural_and_semantic_stages",
        "identifier/defs queries must still rank the test definition first. The conceptual tests/ clamp must not fire here.",
    );
}

#[test]
fn combine_channels_ranks_conjunction_combine_first() {
    let (_temp, searcher) = indexed_corpus();
    let query = "combine two search channels in a single query";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "conjunction.rs",
        "combine",
        "two-channel AND lives in conjunction.rs::combine. fusion.rs, eval.rs, and combine_field_scores at #1 means this is not a program.",
    );
    let fusion_rank = rank_of(&response.hits, |hit| {
        hit.file.replace('\\', "/").ends_with("fusion.rs")
    });
    let eval_rank = rank_of(&response.hits, |hit| {
        hit.file.replace('\\', "/").ends_with("eval.rs")
    });
    let field_rank = rank_of(&response.hits, |hit| {
        hit.file.replace('\\', "/").ends_with("field_weight.rs")
            || hit.symbol.as_deref() == Some("combine_field_scores")
    });
    if fusion_rank == Some(1) || eval_rank == Some(1) || field_rank == Some(1) {
        panic!(
            "\n\n===== DEAD PROGRAM: WRONG PRODUCT SURFACE IS #1 =====\n\
             Query: {query:?}\n\
             fusion.rs rank={fusion_rank:?} eval.rs rank={eval_rank:?} field_weight.rs rank={field_rank:?}\n\
             FULL SCOREBOARD:\n{}\n\
             ===== END AUTOPSY =====\n",
            autopsy(&response.hits)
        );
    }
}

#[test]
fn reciprocal_rank_fusion_still_ranks_fusion_module() {
    let (_temp, searcher) = indexed_corpus();
    let query = "reciprocal rank fusion across evidence channels";
    let response = searcher.search(query).expect("search");
    let board = autopsy(&response.hits);
    let Some(first) = response.hits.first() else {
        panic!(
            "\n\n===== DEAD PROGRAM: ZERO HITS =====\nQuery: {query:?}\n===== END AUTOPSY =====\n"
        );
    };
    let file = first.file.replace('\\', "/");
    if !file.ends_with("fusion.rs") {
        panic!(
            "\n\n===== THIS IS NOT A SEARCH ENGINE =====\n\
             Query: {query:?}\n\
             Contract: RRF ranks fusion.rs first. Conjunction expansion must not steal this query.\n\
             Actual #1: {:?} {} {:?}\n\nFULL SCOREBOARD:\n{board}\n\
             ===== END AUTOPSY =====\n",
            first.kind, first.file, first.symbol
        );
    }
}

#[test]
fn throttle_inbound_ranks_rate_limit_client() {
    let (_temp, searcher) = indexed_corpus();
    let query = "throttle inbound clients";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "throttle.rs",
        "rate_limit_client",
        "cg5 invent-path: throttle paraphrase must rank rate_limit_client first.",
    );
}

#[test]
fn debounce_noisy_ranks_coalesce_watch_events() {
    let (_temp, searcher) = indexed_corpus();
    let query = "debounce noisy updates";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "debounce.rs",
        "coalesce_watch_events",
        "cg5 invent-path: debounce paraphrase must rank coalesce_watch_events first.",
    );
}

#[test]
fn retry_transient_ranks_backoff_attempt() {
    let (_temp, searcher) = indexed_corpus();
    let query = "retry after transient failure";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "retry.rs",
        "backoff_attempt",
        "cg5 invent-path: retry paraphrase must rank backoff_attempt first.",
    );
}
