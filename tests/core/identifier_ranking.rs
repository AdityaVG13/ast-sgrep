//! Exact identifiers rank the definition; conceptual queries prefer code over docs.
use ast_sgrep_core::query::ParsedQuery;
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use serde::Deserialize;
use std::fs;
use std::path::Path;
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
        "src/plugins.rs",
        r#"
/// Compact output interned paths for agent envelopes.
pub fn intern_paths(path: &str) -> &str {
    path
}
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

#[test]
fn snapshot_unique_keeps_concept_def_first() {
    let (temp, _searcher) = indexed_corpus();
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 12,
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher");
    searcher.hold_read_snapshot().expect("hold snapshot");
    searcher.warm_search_path().expect("warm");
    for (query, file, symbol) in [
        ("credential renewal", "auth.rs", "auth_refresh"),
        (
            "throttle inbound clients",
            "throttle.rs",
            "rate_limit_client",
        ),
        (
            "debounce noisy updates",
            "debounce.rs",
            "coalesce_watch_events",
        ),
        (
            "retry after transient failure",
            "retry.rs",
            "backoff_attempt",
        ),
    ] {
        let response = searcher.search(query).expect("search");
        require_first(
            query,
            &response.hits,
            file,
            symbol,
            "Pi unique hybrid on a held warmed snapshot must keep the concept def first.",
        );
    }
}

#[test]
fn compact_output_path_interning_ranks_intern_paths() {
    let (_temp, searcher) = indexed_corpus();
    let query = "compact output path interning";
    let response = searcher.search(query).expect("search");
    require_first(
        query,
        &response.hits,
        "plugins.rs",
        "intern_paths",
        "conceptual NL must retrieve intern_paths without a hardcoded concept-def allowlist.",
    );
}

#[derive(Deserialize)]
struct InventPathGold {
    queries: Vec<InventPathQuery>,
}

#[derive(Deserialize)]
struct InventPathQuery {
    name: String,
    query: String,
    k: usize,
    relevant: Vec<InventPathRelevant>,
}

#[derive(Deserialize)]
struct InventPathRelevant {
    file: String,
    symbol: Option<String>,
}

#[test]
fn invent_path_gold_has_no_loss() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../../benchmarks/fixtures/invent_path");
    let gold_path = manifest.join("../../benchmarks/gold/invent_path.json");
    let gold: InventPathGold = serde_json::from_str(
        &fs::read_to_string(&gold_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", gold_path.display())),
    )
    .expect("parse invent_path gold");
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: root.clone(),
        force_reindex: true,
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root,
        limit: 20,
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .with_response_stamp(false);
    searcher.hold_read_snapshot().expect("hold snapshot");
    searcher.warm_search_path().expect("warm");

    let mut misses = Vec::new();
    for case in &gold.queries {
        let response = searcher.search(&case.query).expect("search");
        let hits = &response.hits;
        let found = case.relevant.iter().all(|rel| {
            hits.iter().take(case.k).any(|hit| {
                hit.file.replace('\\', "/").ends_with(&rel.file)
                    && rel
                        .symbol
                        .as_ref()
                        .is_none_or(|s| hit.symbol.as_deref() == Some(s.as_str()))
            })
        });
        if !found {
            misses.push(format!("{} {:?}\n{}", case.name, case.query, autopsy(hits)));
        }
    }
    assert!(
        misses.is_empty(),
        "invent-path gold loss ({} of {}):\n{}",
        misses.len(),
        gold.queries.len(),
        misses.join("\n\n")
    );
}

#[test]
#[ignore = "indexes this repo; run explicitly for self-gold no-loss"]
fn self_gold_has_no_loss() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.join("../..");
    let gold_path = manifest.join("../../benchmarks/gold/self.json");
    let gold: InventPathGold = serde_json::from_str(
        &fs::read_to_string(&gold_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", gold_path.display())),
    )
    .expect("parse self gold");
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        limit: 20,
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher")
    .with_response_stamp(false);
    searcher.hold_read_snapshot().expect("hold snapshot");
    searcher.warm_search_path().expect("warm");

    let mut misses = Vec::new();
    for case in &gold.queries {
        let response = searcher.search(&case.query).expect("search");
        let hits = &response.hits;
        let found = case.relevant.iter().all(|rel| {
            hits.iter().take(case.k).any(|hit| {
                hit.file.replace('\\', "/").ends_with(&rel.file)
                    && rel
                        .symbol
                        .as_ref()
                        .is_none_or(|s| hit.symbol.as_deref() == Some(s.as_str()))
            })
        });
        if !found {
            misses.push(format!("{} {:?}\n{}", case.name, case.query, autopsy(hits)));
        }
    }
    assert!(
        misses.is_empty(),
        "self gold loss ({} of {}):\n{}",
        misses.len(),
        gold.queries.len(),
        misses.join("\n\n")
    );
}

/// H-AUDIT-52-6 e2e cell (br-uhf): rule 5 (`GENERIC_ENTRYPOINT_PENALTY`) may
/// only fire when the query expansions lane is active, and it targets
/// Caller/Graph hits — so a py/js module-level caller (`<module>`, the common
/// shape per `extract.rs`) must rank below a fn caller of the same callee
/// under a conceptual NL query. If conceptual queries stop surfacing caller
/// hits entirely, rule 5 is dead code and this cell must fail.
fn indexed_module_caller_corpus() -> (TempDir, Searcher) {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/orders.py",
        r#"
def validate_order(order_id):
    return order_id is not None


validate_order("a-1")
"#,
    );
    write_src(
        temp.path(),
        "src/pipeline.py",
        r#"
def run_pipeline(batch):
    for order_id in batch:
        validate_order(order_id)
"#,
    );
    write_src(
        temp.path(),
        "src/notify.js",
        r#"
function validate_order(orderId) {
  return orderId != null;
}

validate_order("a-1");
"#,
    );
    write_src(
        temp.path(),
        "src/flow.js",
        r#"
function run_flow(batch) {
  for (const id of batch) {
    validate_order(id);
  }
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

fn caller_rank(
    hits: &[ast_sgrep_core::SearchHit],
    file_suffix: &str,
    caller: &str,
) -> Option<usize> {
    rank_of(hits, |hit| {
        hit.kind == ast_sgrep_core::HitKind::Caller
            && hit.file.replace('\\', "/").ends_with(file_suffix)
            && hit.caller.as_deref() == Some(caller)
    })
}

#[test]
fn module_caller_ranks_below_fn_caller_on_conceptual_nl() {
    let (_temp, searcher) = indexed_module_caller_corpus();
    let query = "how do I validate an order";
    let response = searcher.search(query).expect("search");
    let cases = [
        (
            "src/orders.py",
            "<module>",
            "src/pipeline.py",
            "run_pipeline",
            "py",
        ),
        ("src/notify.js", "<module>", "src/flow.js", "run_flow", "js"),
    ];
    let mut failures = Vec::new();
    for (module_file, module_caller, fn_file, fn_caller, lang) in cases {
        let module_rank = caller_rank(&response.hits, module_file, module_caller);
        let fn_rank = caller_rank(&response.hits, fn_file, fn_caller);
        let (Some(module_rank), Some(fn_rank)) = (module_rank, fn_rank) else {
            failures.push(format!(
                "{lang}: caller hits missing under conceptual NL (module={module_rank:?} fn={fn_rank:?}) — rule 5 unreachable\n{}",
                autopsy(&response.hits)
            ));
            continue;
        };
        if module_rank <= fn_rank {
            failures.push(format!(
                "{lang}: <module> caller ranked #{module_rank}, must rank BELOW the fn caller at #{fn_rank}\n{}",
                autopsy(&response.hits)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "H-AUDIT-52-6 e2e violations ({} of {}):\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n\n")
    );
}

/// A single-token SCREAMING_SNAKE query must reach the source file holding the
/// exact token. Without a whole-word leg the fragmented terms only matched an
/// unrelated hyphenated flag name in comments.
#[test]
fn natural_mode_exact_token_reaches_source_file() {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "kernels/lmhead.cu",
        "#ifdef VT_LMHEAD_FP8
__global__ void vt_lmhead_fp8_kernel() {}
#endif
",
    );
    write_src(
        temp.path(),
        "docs/flags.md",
        "# VT-MATMUL-FP8-BLOCK-CUDA is a different flag than VT_LMHEAD_FP8.
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
        limit: 8,
        ..SearchOptions::default()
    })
    .expect("searcher");
    let response = searcher.search("VT_LMHEAD_FP8").expect("search");
    assert!(
        response
            .hits
            .iter()
            .any(|h| h.file.replace(char::from(92), "/").ends_with("lmhead.cu")),
        "exact-token query must surface the .cu source file; hits:\n{}",
        autopsy(&response.hits)
    );
}

#[test]
fn flooded_common_term_does_not_starve_rare_term_defs() {
    // INTENT: hybrid symbol admission must not spend its whole row budget on
    // one flooded query term ("run" matches 40 defs in an early path) before
    // a rare term's only def ("next" in a later path) is even scored.
    // Selection that truncates in path order starves the rare-term def;
    // admission must be term-fair. Mirrors the self-gold
    // conceptual-follow-up-derivation loss on this repo's own index.
    let temp = TempDir::new().unwrap();
    let mut flood = String::new();
    for i in 0..40 {
        flood.push_str(&format!("pub fn run_step_{i:02}() {{}}\n"));
    }
    write_src(temp.path(), "src/a_flood.rs", &flood);
    write_src(
        temp.path(),
        "src/z_planner.rs",
        r#"
/// Derive the follow-up command suggested by the top search result.
pub fn plan_suggested_next() {}
pub fn follow_ups_for_hit() {}
"#,
    );
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let assert_admitted = |searcher: &Searcher, lane: &str| {
        let response = searcher
            .search("derive the next command to run from the best result")
            .expect("search");
        let admitted = response.hits.iter().any(|hit| {
            hit.kind == ast_sgrep_core::HitKind::Def
                && hit.symbol.as_deref() == Some("plan_suggested_next")
        });
        assert!(
            admitted,
            "{lane} lane starved the rare-term def; hits:\n{}",
            autopsy(&response.hits)
        );
    };
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 20,
        use_embed: false,
        ..SearchOptions::default()
    };
    let cold = Searcher::new(options.clone()).expect("cold searcher");
    assert_admitted(&cold, "cold");
    let warm = Searcher::new(options).expect("warm searcher");
    warm.hold_read_snapshot().expect("hold snapshot");
    warm.warm_search_path().expect("warm");
    assert_admitted(&warm, "warm");
}

#[test]
fn fragment_chunks_do_not_crowd_out_the_parent_symbol() {
    // INTENT: the embed lane must always represent a function by its whole
    // body, not only by extracted pattern-node child fragments. When child
    // nodes exist, fragment-only chunking makes every non-call statement
    // invisible to conceptual ranking: the function is represented by
    // scraps like `Vec::new()` while the statements that carry its meaning
    // never reach the semantic index. Mirrors the self-gold
    // conceptual-follow-up-derivation loss, where `plan_suggested_next`'s
    // body never became a chunk. The discriminative content below sits in
    // a plain array literal (not a call), decoys own the query's generic
    // vocabulary, and the target's name shares no token with the query.
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/z_workflow.rs",
        r#"
pub fn advance_workflow() {
    if workflow_is_idle() {
        return;
    }
    let steps = ["gather follow up commands", "execute them next"];
    emit_steps(steps.len());
}

fn workflow_is_idle() -> bool { true }
fn emit_steps(_count: usize) {}
"#,
    );
    for i in 0..8 {
        write_src(
            temp.path(),
            &format!("src/run_{i}.rs"),
            &format!(
                "pub fn run_task_{i}() {{\n    let started = clock_now();\n    if started < {i} {{\n        log_line(\"early\");\n    }}\n    execute_shell_command(\"task {i}\");\n    record_elapsed(started, {i});\n}}\nfn execute_shell_command(_cmd: &str) {{}}\nfn clock_now() -> u32 {{ 0 }}\nfn log_line(_m: &str) {{}}\nfn record_elapsed(_s: u32, _i: u32) {{}}\n"
            ),
        );
    }
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 20,
        use_embed: true,
        ..SearchOptions::default()
    })
    .expect("searcher");
    let response = searcher
        .search_semantic("collect follow up commands to execute next")
        .expect("semantic search");
    // Asserted property = the INTENT above: whole-body representation must
    // carry the parent above every real-body decoy (pre-fix it sat below
    // all of them, represented only by fragments). The eight empty
    // `execute_shell_command` stubs still outrank it — their name-only
    // chunk text is a concentrated match for the query's generic tokens
    // under the borrowed Phase-1 weights. That short-text purity effect is
    // a representation-quality limitation owned by Phase 3/4, recorded in
    // docs/progress/surface-deferrals.md; asserting past it here would
    // demand score tuning, not representation.
    let pos = |pred: &dyn Fn(&str) -> bool| {
        response
            .hits
            .iter()
            .position(|hit| hit.symbol.as_deref().is_some_and(pred))
    };
    let parent = pos(&|s| s == "advance_workflow");
    let first_decoy = pos(&|s| s.starts_with("run_task_"));
    assert!(
        parent.is_some() && first_decoy.is_some() && parent < first_decoy,
        "semantic lane starved the parent body behind its fragments \
         (parent={parent:?}, first_decoy={first_decoy:?}); hits:\n{}",
        autopsy(&response.hits)
    );
}

#[test]
fn rare_term_def_outranks_flooded_term_def() {
    // INTENT: a def matching a RARE query token is stronger evidence than a
    // def matching a flooded one. Coverage scoring that counts matching
    // terms without regard to how many symbols each term matches lets the
    // flooded term's defs (here `process`, 41 symbols) tie or beat the one
    // def carrying the query's discriminative token (`quiesce`, 1 symbol),
    // even when both match the same number of terms. Mirrors the self-gold
    // conceptual-follow-up-derivation loss, where `run`/`command` defs
    // outscored the `follow_up`/`next` defs. Vocabulary here is unrelated
    // to the gold queries.
    let temp = TempDir::new().unwrap();
    let mut flood = String::from("pub fn process_pipeline_main() { let _ = 1; }\n");
    for i in 0..40 {
        flood.push_str(&format!("pub fn process_step_{i:02}() {{ let _ = 1; }}\n"));
    }
    write_src(temp.path(), "src/a_process.rs", &flood);
    write_src(
        temp.path(),
        "src/z_quiesce.rs",
        "pub fn quiesce_pipeline() { let _ = 1; }\n",
    );
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let assert_order = |searcher: &Searcher, lane: &str| {
        let response = searcher
            .search("process the quiesce pipeline")
            .expect("search");
        let pos = |symbol: &str| {
            response
                .hits
                .iter()
                .position(|hit| hit.symbol.as_deref() == Some(symbol))
        };
        let (rare, common) = (pos("quiesce_pipeline"), pos("process_pipeline_main"));
        assert!(
            rare.is_some() && common.is_some() && rare < common,
            "{lane} lane ranked the flooded-term def above the rare-term def \
             (rare={rare:?}, common={common:?}); hits:\n{}",
            autopsy(&response.hits)
        );
    };
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 20,
        use_embed: false,
        ..SearchOptions::default()
    };
    let cold = Searcher::new(options.clone()).expect("cold searcher");
    assert_order(&cold, "cold");
    let warm = Searcher::new(options).expect("warm searcher");
    warm.hold_read_snapshot().expect("hold snapshot");
    warm.warm_search_path().expect("warm");
    assert_order(&warm, "warm");
}

#[test]
fn full_structural_page_still_considers_semantic_evidence() {
    // INTENT: on a non-conceptual hybrid query, semantic evidence must be
    // considered even when structural hits already fill the result page.
    // The hashed-era thin-page condition skipped the restricted embed pass
    // whenever `hits.len() >= limit`, so a file admitted by weak structural
    // evidence (one shared token) whose *meaning* is the real match could
    // never be promoted by the learned backend — it fell off the page
    // behind decoys that merely parrot the query's tokens. The query below
    // carries a structural marker (`=>`) in prose, so it classifies
    // Structural and takes the restricted pass; the target's vocabulary is
    // synonymous, not token-identical, so only the embedding can carry it.
    // 2D ablation, frozen suite 2026-10-14: removing the condition moved
    // RepoQA MRR@10 .521 -> .533 with CoSQA byte-identical.
    let temp = TempDir::new().unwrap();
    for i in 0..16 {
        write_src(
            temp.path(),
            &format!("src/decoy_{i}.rs"),
            &format!(
                "/// merge pending filesystem notifications callback merge pending\npub fn merge_pending_notifications_{i}() {{ let _ = 1; }}\n"
            ),
        );
    }
    write_src(
        temp.path(),
        "src/z_notify.rs",
        r#"
/// Coalesce a burst of queued file-change alerts and deliver a single
/// notification to subscribers so watchers are not flooded by duplicates.
pub fn flush_pending_alerts() { let _ = 1; }
"#,
    );
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        embed_semantic: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let assert_promoted = |searcher: &Searcher, lane: &str| {
        let response = searcher
            .search("merge pending filesystem notifications => one callback")
            .expect("search");
        let target = response
            .hits
            .iter()
            .find(|hit| hit.symbol.as_deref() == Some("flush_pending_alerts"));
        let Some(target) = target else {
            panic!(
                "{lane} lane dropped the semantic match off a full structural page; hits:\n{}",
                autopsy(&response.hits)
            );
        };
        assert!(
            target
                .contributors
                .contains(&ast_sgrep_core::HitKind::Embed),
            "{lane} lane never considered semantic evidence for the target on a \
             full structural page (contributors: {:?}); hits:\n{}",
            target.contributors,
            autopsy(&response.hits)
        );
    };
    let options = SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 4,
        use_embed: true,
        ..SearchOptions::default()
    };
    let cold = Searcher::new(options.clone()).expect("cold searcher");
    assert_promoted(&cold, "cold");
    let warm = Searcher::new(options).expect("warm searcher");
    warm.hold_read_snapshot().expect("hold snapshot");
    warm.warm_search_path().expect("warm");
    assert_promoted(&warm, "warm");
}
