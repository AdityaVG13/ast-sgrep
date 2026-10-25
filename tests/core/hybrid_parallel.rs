//! Intent: the forked embed/fanout channel (sibling read-only store) must
//! return byte-identical hybrid results to the serial channel path, and the
//! fork must actually fire unless `ASGREP_SERIAL_CHANNELS=1` forces serial.
//!
//! The serial arm and the parallel arm run on separate `Searcher`s over the
//! same embedded fixture index, so pass-level caches cannot mask a
//! divergence. Parity is asserted on the full hit vector (file, span, kind,
//! score), not just file sets: RRF order and scores are the contract.

use ast_sgrep_core::search::debug_channel_fork_count;
use ast_sgrep_core::{Indexer, Searcher};
use ast_sgrep_testkit::{core_index_options, core_search_options, write_core_fixture};

const FILES: &[(&str, &str)] = &[
    (
        "src/session.rs",
        r#"
pub struct SessionState { token: String, refreshed_at: u64 }

pub fn refresh_session_token(state: &mut SessionState, now: u64) -> String {
    state.token = format!("tok-{now}");
    state.refreshed_at = now;
    state.token.clone()
}

pub fn session_write_durable(state: &SessionState, journal: &mut Vec<String>) {
    journal.push(format!("{}@{}", state.token, state.refreshed_at));
}
"#,
    ),
    (
        "src/auth.rs",
        r#"
pub fn auth_credential_renewal(user: &str) -> String {
    format!("renewed-{user}")
}

pub fn validate_refresh_flow(token: &str) -> bool {
    token.starts_with("tok-")
}
"#,
    ),
    (
        "src/store.rs",
        r#"
pub fn persist_journal_rows(journal: &[String]) -> usize {
    journal.len()
}
"#,
    ),
];

#[test]
fn forked_embed_channel_matches_serial_hits() {
    let fixture = write_core_fixture(FILES);
    let mut index_options = core_index_options(&fixture.root, &fixture.db);
    index_options.embed_semantic = true;
    let mut indexer = Indexer::new(index_options).expect("indexer new");
    indexer.index_all().expect("index_all with embeddings");

    let query = "durable session write with token refresh";

    let run_arm = |serial: bool| -> Vec<String> {
        if serial {
            std::env::set_var("ASGREP_SERIAL_CHANNELS", "1");
        } else {
            std::env::remove_var("ASGREP_SERIAL_CHANNELS");
        }
        let mut options = core_search_options(&fixture.root, &fixture.db, 20);
        options.use_embed = true;
        let searcher = Searcher::new(options).expect("searcher new");
        let response = searcher.search(query).expect("hybrid search");
        response
            .hits
            .iter()
            .map(|hit| {
                format!(
                    "{:?}|{}|{}|{}|{:?}",
                    hit.kind, hit.file, hit.line_start, hit.line_end, hit.score
                )
            })
            .collect()
    };

    let before_forks = debug_channel_fork_count();
    let serial_hits = run_arm(true);
    let after_serial_forks = debug_channel_fork_count();
    let parallel_hits = run_arm(false);
    let after_parallel_forks = debug_channel_fork_count();

    assert!(
        !serial_hits.is_empty(),
        "fixture query must produce hits or the parity assertion is vacuous"
    );
    assert_eq!(
        serial_hits, parallel_hits,
        "forked embed/fanout channel diverged from the serial channel path"
    );
    assert_eq!(
        before_forks, after_serial_forks,
        "ASGREP_SERIAL_CHANNELS=1 must not fork"
    );
    assert!(
        after_parallel_forks > before_forks,
        "parallel arm must actually fork the embed channel"
    );
}
