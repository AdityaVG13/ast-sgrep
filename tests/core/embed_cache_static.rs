//! Intent: with the static (deterministic, model-free) embedder the
//! `embed_cache` table must stay empty: cached outputs are pure functions
//! of the text (`embed_texts` is the static hasher), so the 63 MB-per-repo
//! cache buys a point-read to avoid a microsecond recompute. Search output
//! must be byte-identical between an index built cache-off (default) and
//! cache-on (ASGREP_EMBED_CACHE=1 control), and re-embedding the same
//! fixture twice must produce identical output (determinism of the
//! compute path the cache would have short-circuited).

use ast_sgrep_core::{Indexer, Searcher};
use ast_sgrep_testkit::{core_index_options, core_search_options, write_core_fixture};
use std::sync::Mutex;

static ENV_GUARD: Mutex<()> = Mutex::new(());

const FILES: &[(&str, &str)] = &[
    (
        "src/session.rs",
        "/// Durable session writer with token refresh.\npub struct SessionWriter {\n    token: String,\n    durable: bool,\n}\n\nimpl SessionWriter {\n    pub fn refresh_token(&mut self, next: String) {\n        self.token = next;\n        self.durable = true;\n    }\n}\n",
    ),
    (
        "src/store.ts",
        "export function persistSession(session: SessionRecord): void {\n  writeToDisk(session);\n}\nexport interface SessionRecord {\n  token: string;\n}\n",
    ),
    (
        "src/validator.py",
        "def validate_refresh(record):\n    \"\"\"Return True when the refresh token is still durable.\"\"\"\n    return bool(record.get(\"token\"))\n",
    ),
];

fn cache_count(db: &std::path::Path) -> i64 {
    let conn =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("open db");
    conn.query_row("SELECT COUNT(*) FROM embed_cache", [], |r| r.get(0))
        .expect("count embed_cache")
}

/// Compare the semantic payload (query, hits, counts, estimates) only: the
/// `SnapshotStamp` tail embeds the wall-clock `worktree_revision`, which
/// legitimately differs whenever two index runs straddle a second boundary.
fn strip_stamp(outs: &[String]) -> Vec<String> {
    outs.iter()
        .map(|s| {
            s.split("snapshot: SnapshotStamp")
                .next()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn index_and_search(
    fixture: &ast_sgrep_testkit::CorePipelineFixture,
    queries: &[&str],
) -> Vec<String> {
    let mut options = core_index_options(&fixture.root, &fixture.db);
    options.embed_semantic = true;
    let mut indexer = Indexer::new(options).expect("indexer new");
    indexer.index_all().expect("index_all");
    let searcher = Searcher::new(core_search_options(&fixture.root, &fixture.db, 5)).expect("s");
    queries
        .iter()
        .map(|q| format!("{:?}", searcher.search(q).expect("search")))
        .collect()
}

const QUERIES: &[&str] = &[
    "durable session write with token refresh",
    "SessionWriter",
    "persistSession",
    "validator refresh record",
    "token",
];

#[test]
fn embed_cache_stays_empty_and_outputs_match_cached_control() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());

    // Arm 1 (default, cache-off): embed_cache must stay empty.
    let a = write_core_fixture(FILES);
    let out_a = index_and_search(&a, QUERIES);
    assert_eq!(
        cache_count(&a.db),
        0,
        "static embedder must not populate embed_cache (deterministic recompute)"
    );

    // Determinism: same fixture indexed again (fresh embed pass, no cache)
    // must produce identical output.
    let a2 = write_core_fixture(FILES);
    let out_a2 = index_and_search(&a2, QUERIES);
    assert_eq!(
        strip_stamp(&out_a),
        strip_stamp(&out_a2),
        "compute path must be deterministic"
    );

    // Arm 2 (control, cache forced on): cache populated AND outputs identical.
    std::env::set_var("ASGREP_EMBED_CACHE", "1");
    let b = write_core_fixture(FILES);
    let out_b = index_and_search(&b, QUERIES);
    std::env::remove_var("ASGREP_EMBED_CACHE");
    assert!(
        cache_count(&b.db) > 0,
        "control arm must prove the env switch restores the legacy cache path"
    );
    assert_eq!(
        strip_stamp(&out_a),
        strip_stamp(&out_b),
        "cache-off and cache-on indexes must return byte-identical search output"
    );
}

/// A full rebuild must reclaim legacy `embed_cache` rows from pre-gate
/// indexes: with the static cache disabled nothing repopulates the table,
/// so surviving rows are pure dead weight (63 MB at repo scale).
#[test]
fn reindex_all_purges_legacy_embed_cache_rows() {
    let _guard = ENV_GUARD.lock().unwrap_or_else(|p| p.into_inner());
    // Control arm: force the legacy cache on to manufacture stale rows.
    std::env::set_var("ASGREP_EMBED_CACHE", "1");
    let b = write_core_fixture(FILES);
    let out_control = index_and_search(&b, QUERIES);
    assert!(
        cache_count(&b.db) > 0,
        "control arm must populate embed_cache first"
    );
    std::env::remove_var("ASGREP_EMBED_CACHE");

    // Full rebuild under the default (static, cache-off) regime.
    let mut options = core_index_options(&b.root, &b.db);
    options.embed_semantic = true;
    options.force_reindex = true;
    let mut indexer = Indexer::new(options).expect("indexer new");
    indexer.reindex_all().expect("reindex_all");
    assert_eq!(
        cache_count(&b.db),
        0,
        "reindex_all must purge legacy embed_cache rows"
    );

    // Purge must not change search output.
    let out_after = index_and_search(&b, QUERIES);
    assert_eq!(
        strip_stamp(&out_control),
        strip_stamp(&out_after),
        "search output must be unchanged by the purge"
    );
}
