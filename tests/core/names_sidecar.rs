//! Intent: the names sidecar (folded symbol-name and distinct-callee
//! vocabularies, generation-gated) must serve idf dfs identical to the SQL
//! streaming fallback, must fall back soundly when the sidecar is stale,
//! corrupt, or absent, and must be refreshed by a reindex that changes
//! symbol names.
//!
//! Parity is asserted on full search output bytes with the sidecar enabled
//! vs `ASGREP_NO_NAMES_SIDECAR=1` (SQL fallback), because idf weights feed
//! def/caller ranking: any df divergence moves scores, not just file sets.

use ast_sgrep_core::names_sidecar::{NamePopulation, NamesSidecar};
use ast_sgrep_core::{Indexer, Searcher};
use ast_sgrep_testkit::{core_index_options, core_search_options, write_core_fixture};
use std::sync::Mutex;

/// Env flips are process-global; serialize the two parity tests.
static ENV_GUARD: Mutex<()> = Mutex::new(());

const FILES: &[(&str, &str)] = &[
    (
        "src/session.rs",
        r#"
pub struct SessionState { token: String }

pub fn refresh_session_token(state: &mut SessionState) -> String {
    state.token = std::mem::take(&mut state.token)
}

pub fn session_write_durable(state: &SessionState, journal: &mut Vec<String>) {
    journal.push(state.token.clone());
}
"#,
    ),
    (
        "src/auth.rs",
        r#"
pub fn validateRefreshFlow(token: &str) -> bool {
    token.starts_with("tok-")
}

pub fn cafe_loader(token: &str) -> &str {
    token
}
"#,
    ),
];

fn index_fixture() -> (ast_sgrep_testkit::CorePipelineFixture, Searcher) {
    let fixture = write_core_fixture(FILES);
    let mut options = core_index_options(&fixture.root, &fixture.db);
    options.embed_semantic = false;
    let mut indexer = Indexer::new(options).expect("indexer new");
    indexer.index_all().expect("index_all");
    let searcher =
        Searcher::new(core_search_options(&fixture.root, &fixture.db, 20)).expect("searcher new");
    (fixture, searcher)
}

fn search_output(searcher: &Searcher, query: &str) -> String {
    format!("{:?}", searcher.search(query).expect("search"))
}

#[test]
fn sidecar_parity_and_stale_fallback() {
    let _guard = ENV_GUARD.lock().unwrap();
    let (fixture, searcher) = index_fixture();

    let path = ast_sgrep_core::names_sidecar::sidecar_path(&fixture.db);
    assert!(path.exists(), "names sidecar must exist after index_all");

    // The sidecar must be fresh for the current generation.
    let sidecar = NamesSidecar::open(&fixture.db).expect("sidecar opens");
    assert!(
        sidecar.is_fresh(searcher.store().index_data_version().expect("gen")),
        "sidecar generation must match the store it was built from"
    );

    // df counts over the folded vocabularies (spot terms incl. mixed case).
    let terms: Vec<String> = ["session", "refresh", "validaterefreshflow", "caf"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (dfs, total) = sidecar.dfs(NamePopulation::Symbols, &terms);
    assert_eq!(
        total as usize > 4,
        true,
        "symbols population is non-trivial"
    );
    assert!(
        dfs[0] >= 2,
        "'session' appears in >=2 symbol names: {dfs:?}"
    );
    assert!(
        dfs[2] >= 1,
        "folded search must find validateRefreshFlow: {dfs:?}"
    );
    assert!(dfs[3] >= 1, "folded search must find cafe_loader: {dfs:?}");
    let (callee_dfs, callee_total) = sidecar.dfs(NamePopulation::DistinctCallees, &terms);
    assert!(callee_total > 0, "callees population is non-empty");

    // Behavior parity: sidecar lane vs SQL fallback lane, full output bytes.
    let queries = [
        "session token refresh",
        "validaterefreshflow",
        "cafe loader",
    ];
    let _ = std::env::remove_var("ASGREP_NO_NAMES_SIDECAR");
    let with_sidecar: Vec<String> = queries
        .iter()
        .map(|q| search_output(&searcher, q))
        .collect();
    std::env::set_var("ASGREP_NO_NAMES_SIDECAR", "1");
    let with_sql: Vec<String> = queries
        .iter()
        .map(|q| search_output(&searcher, q))
        .collect();
    std::env::remove_var("ASGREP_NO_NAMES_SIDECAR");
    for (i, (a, b)) in with_sidecar.iter().zip(with_sql.iter()).enumerate() {
        assert_eq!(
            a, b,
            "sidecar lane diverged from SQL lane on {}",
            queries[i]
        );
    }

    // Stale sidecar (generation from the future cannot be fresh): corrupt the
    // header generation and re-run — output must stay identical via fallback.
    let bytes = std::fs::read(&path).expect("read sidecar");
    let mut corrupted = bytes.clone();
    // generation lives at header offset 10..18 (i64 LE); write an impossible
    // future value so is_fresh fails closed.
    corrupted[10..18].copy_from_slice(&i64::MAX.to_le_bytes());
    std::fs::write(&path, &corrupted).expect("corrupt sidecar");
    let stale: Vec<String> = queries
        .iter()
        .map(|q| search_output(&searcher, q))
        .collect();
    for (i, (a, b)) in with_sidecar.iter().zip(stale.iter()).enumerate() {
        assert_eq!(a, b, "stale sidecar changed output on {}", queries[i]);
    }

    // Absent sidecar: same fallback contract.
    std::fs::remove_file(&path).expect("remove sidecar");
    let absent: Vec<String> = queries
        .iter()
        .map(|q| search_output(&searcher, q))
        .collect();
    for (i, (a, b)) in with_sidecar.iter().zip(absent.iter()).enumerate() {
        assert_eq!(a, b, "absent sidecar changed output on {}", queries[i]);
    }
}

#[test]
fn reindex_refreshes_sidecar() {
    let _guard = ENV_GUARD.lock().unwrap();
    let (fixture, searcher) = index_fixture();

    let before = NamesSidecar::open(&fixture.db).expect("sidecar before reindex");
    let term = "zqx".to_string();
    let (dfs_before, _) = before.dfs(NamePopulation::Symbols, &[term.clone()]);
    assert_eq!(dfs_before[0], 0, "marker symbol not indexed yet");

    // Edit: add a symbol whose name contains the marker.
    let session = fixture.root.join("src/session.rs");
    let mut text = std::fs::read_to_string(&session).expect("read fixture file");
    text.push_str("\npub fn zqx_marker_added_later() -> u8 { 7 }\n");
    std::fs::write(&session, &text).expect("edit fixture file");

    let mut options = core_index_options(&fixture.root, &fixture.db);
    options.embed_semantic = false;
    let mut indexer = Indexer::new(options).expect("indexer for reindex");
    indexer.index_all().expect("reindex");

    let after = NamesSidecar::open(&fixture.db).expect("sidecar after reindex");
    assert_ne!(
        before.generation(),
        after.generation(),
        "reindex with an added symbol must bump the recorded generation"
    );
    let (dfs_after, _) = after.dfs(NamePopulation::Symbols, &[term]);
    assert_eq!(dfs_after[0], 1, "marker symbol must be in the new sidecar");

    // Fresh sidecar must also match the store generation.
    assert!(after.is_fresh(searcher.store().index_data_version().expect("gen")));
}
