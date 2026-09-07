//! F74c-1 (pass 74c MEDIUM, r25 pass 75b) — the pass-73 php `::` call-signature
//! re-key changed `pattern_nodes` row semantics (`call:bar` → `call:Foo::bar`)
//! WITHOUT an `INDEX_SCHEMA_VERSION` bump, so a pre-73 index (stamped 14) is
//! indistinguishable from a current one. Because the exact `call:`/`decl:`
//! lane serves index rows AUTHORITATIVELY (`pattern.rs`:
//! "Re-walking the tree cannot add a hit the index missed") and search defaults
//! to a read-only, no-refresh open, the steady state post-upgrade was: stale
//! `call:bar` rows silently resurrect the F72a-1 wrong answers
//! (`bar($$$A)` → stale-row over-match).
//!
//! Contract pinned here:
//! 1. A store stamped at 14 (the version the pass-73 re-key shipped on) is
//!    NEVER served through a read-only open: it must fail loud with a rebuild
//!    instruction. Writable opens auto-migrate instead.
//! 2. The writable migration must DISCARD pre-rekey signature rows (they have
//!    no row-level migration path — their meaning is binary-defined) and
//!    invalidate EVERY unchanged-file fast path so the next indexing pass
//!    re-extracts every file under the current keying — including the per-file
//!    structure fingerprints (`body:<rel>` written by `commit_prepared_files`,
//!    `struct:<rel>` written by `upsert_file_material`): both route a
//!    fingerprint-unchanged file to `refresh_lines_only`, which rewrites the
//!    PLAIN content hash without reparsing, so a surviving fingerprint
//!    re-legitimizes the hash and leaves `pattern_nodes` permanently empty
//!    (F76c-1: the loud gate's own `asgrep reindex` remedy never rebuilt the
//!    rows; ident/decl search silently answered `[]` and the codemod
//!    H-CONF-023 gate flipped loud-refusal → silent ok:true zero-edit). A
//!    fresh build + search cycle then answers the F72a-1 faces exactly.
//! 3. `plan_codemod` only READS the store (`get_meta("root")`,
//!    `all_file_paths`, `pattern_nodes_matching_limited`), so it must open
//!    read-only: on a pre-current store it refuses loudly (naming the rebuild
//!    remedy) instead of silently firing the row-discarding migration as a
//!    side effect of a planning read (F76c-2).
//!
//! Failure-first: tests 1–2 were run against the pre-fix binary (version
//! stayed 14, read-only open served the stale row; later the F76c-1 seam
//! upgrade — real build re-arming the fingerprints — made test 2 fail with
//! `pattern_nodes` empty after the post-migration reindex, pre-fix). Test 3
//! failed pre-fix with an Ok plan + migrated store. Mutation-kills: removing
//! the `body:`/`struct:` wipe from the `version < 15` block fails test 2;
//! reverting `plan_codemod` to the writable open fails test 3.
use ast_sgrep_core::codemod::plan_codemod;
use ast_sgrep_core::{IndexOptions, IndexStore, SearchOptions, Searcher, INDEX_SCHEMA_VERSION};
use ast_sgrep_testkit::isolated_index_session;

/// Php fixture whose only calls are `Foo::bar(...)` scoped calls. Lines 7 and
/// 8 hold the calls; there is deliberately NO bare `bar(` call, so a fresh
/// index holds zero `call:bar` rows and the `bar($$$A)` over-match face must
/// answer empty.
const PHP_FIXTURE: &str = "<?php\nclass Foo {\n    public static function bar($x) {\n        return $x;\n    }\n}\nFoo::bar(1);\nFoo::bar(2);\n";

/// The on-disk stamp the pass-73 `::` re-key shipped on (audit fact: the re-key
/// landed while `INDEX_SCHEMA_VERSION` was still 14). Pinned literally so the
/// test keeps validating this historical migration across future bumps.
const PRE_REKEY_STAMP: i64 = 14;

/// Simulate a pre-73 index: current-schema tables holding a `call:bar` row
/// keyed the OLD way (the scoped `Foo::bar(1)` call at line 7 recorded under
/// its bare callee tail), stamped `user_version = 14`. Returns the stamp.
fn seed_pre_rekey_store(corpus_root: &std::path::Path, index_path: &std::path::Path) -> i64 {
    let store = IndexStore::open(corpus_root, Some(index_path)).expect("seed writable open");
    store
        .connection()
        .execute_batch(&format!(
            "INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash)
             VALUES('probe.php', 'php', 1, 0, 'pre-rekey-hash');
             INSERT INTO pattern_nodes(file_id, signature, line_start, line_end, excerpt)
             VALUES(last_insert_rowid(), 'call:bar', 7, 7, '');
             PRAGMA user_version = {PRE_REKEY_STAMP};"
        ))
        .expect("seed stale-keyed row + pre-rekey stamp");
    let rows: i64 = store
        .connection()
        .query_row("SELECT COUNT(*) FROM pattern_nodes", [], |r| r.get(0))
        .expect("count seeded rows");
    assert_eq!(rows, 1, "seed must hold exactly the stale call:bar row");
    drop(store);
    PRE_REKEY_STAMP
}

/// Contract 1: the default search path (read-only open, no refresh) must
/// REFUSE a pre-rekey store loudly instead of serving its stale-keyed rows
/// authoritatively. Pre-fix this open succeeded and `bar($$$A)` answered from
/// the stale `call:bar` row.
#[test]
fn read_only_open_refuses_pre_rekey_stamped_store_with_rebuild_instruction() {
    let session = isolated_index_session();
    session.write("probe.php", PHP_FIXTURE);
    let stamp = seed_pre_rekey_store(&session.corpus_root, &session.index_path);
    assert_eq!(stamp, PRE_REKEY_STAMP);
    assert_ne!(
        stamp, INDEX_SCHEMA_VERSION,
        "precondition: the binary must have moved past the pre-rekey stamp"
    );

    let error = match Searcher::new(SearchOptions {
        use_embed: false,
        lang_filter: Some("php".to_string()),
        ..session.search_options()
    }) {
        Err(error) => error,
        Ok(_) => panic!(
            "a pre-rekey (stamp 14) store must not open read-only for search; \
             serving its stale-keyed rows authoritatively resurrects F72a-1"
        ),
    };
    let message = error.to_string();
    assert!(
        message.contains("reindex"),
        "refusal must carry the rebuild instruction: {message}"
    );

    let error = match IndexStore::open_readonly(&session.corpus_root, Some(&session.index_path)) {
        Err(error) => error,
        Ok(_) => panic!("raw read-only open must refuse the same store"),
    };
    assert!(
        error.to_string().contains("reindex"),
        "raw open refusal must carry the rebuild instruction: {error}"
    );
}

/// Contract 2: a writable open auto-migrates — stamp advances, stale signature
/// rows are discarded, stored content identity AND the per-file structure
/// fingerprints are invalidated — and the next indexing pass RE-EXTRACTS under
/// the current `::`-aware keying (it must not take the `body:`/`struct:`
/// structure-skip into `refresh_lines_only`, which would rewrite the plain
/// hash without reparsing and leave `pattern_nodes` permanently empty), after
/// which the F72a-1 faces answer exactly. A second writable open at the
/// current stamp is a no-op (no re-wipe).
#[test]
fn writable_open_migrates_then_reindex_rebuilds_scoped_call_rows_and_answers_faces() {
    let session = isolated_index_session();
    session.write("probe.php", PHP_FIXTURE);

    // Step 1 — a REAL full build with the current binary. A real db always
    // carries the per-file structure fingerprints (`body:<rel>` from
    // `commit_prepared_files`, `struct:<rel>` from `upsert_file_material`), a
    // plain content hash, and mtime identity. This arms every unchanged-file
    // fast path exactly as they are armed on every production index — the
    // F76c-1 audit fact the original hand-INSERTed seam could not see (no
    // `body:`/`struct:` meta ⇒ the structure-skip could not fire there, so the
    // seam held while every real db silently kept `pattern_nodes` empty).
    {
        let mut indexer = session.indexer(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        indexer.index_all().expect("reference full build");
    }
    {
        let store = session.open_store();
        let fingerprinted: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key IN ('body:probe.php', 'struct:probe.php')",
                [],
                |r| r.get(0),
            )
            .expect("count armed fingerprints");
        assert_eq!(
            fingerprinted, 2,
            "precondition: a real build must arm both structure fingerprints"
        );
    }

    // Step 2 — regress the store to the state a pre-rekey db presents: stale
    // `call:bar` signature keying + the old stamp. Fingerprints, plain hash,
    // and mtime identity stay exactly as the real build wrote them.
    {
        let store = session.open_store();
        store
            .connection()
            .execute_batch(
                "UPDATE pattern_nodes SET signature = 'call:bar' WHERE signature = 'call:Foo::bar';
                 PRAGMA user_version = 14;",
            )
            .expect("regress to the pre-rekey state");
        assert_eq!(
            store.on_disk_schema_version().expect("on-disk stamp"),
            PRE_REKEY_STAMP,
            "regression must present the pre-rekey stamp"
        );
        let stale: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pattern_nodes WHERE signature = 'call:bar'",
                [],
                |r| r.get(0),
            )
            .expect("count regressed rows");
        assert_eq!(stale, 2, "both scoped calls must present the stale call:bar key");
    }

    // Step 3 — writable open auto-migrates: stamp advances, stale rows are
    // discarded, content identity is prefixed, AND (F76c-1) both structure
    // fingerprints are discarded so no fast path can re-legitimize the hash.
    {
        let store = IndexStore::open(&session.corpus_root, Some(&session.index_path))
            .expect("writable open must migrate, not refuse");
        let on_disk = store.on_disk_schema_version().expect("on-disk stamp");
        assert_eq!(
            on_disk, INDEX_SCHEMA_VERSION,
            "writable open must advance the stamp to the current schema"
        );
        let stale_rows: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM pattern_nodes", [], |r| r.get(0))
            .expect("count rows after migration");
        assert_eq!(
            stale_rows, 0,
            "pre-rekey signature rows must not survive the migration"
        );
        let hash = store.file_hash("probe.php").expect("file hash").expect("row");
        assert!(
            hash.starts_with("schema15-rekey:"),
            "stored content identity must be prefixed so the hash fast path cannot skip: {hash}"
        );
        for key in ["body:probe.php", "struct:probe.php"] {
            assert!(
                store.get_meta(key).expect("read fingerprint meta").is_none(),
                "F76c-1: the migration must discard the {key} structure fingerprint; \
                 a surviving fingerprint routes the next pass to refresh_lines_only, \
                 which rewrites the plain hash WITHOUT reparsing and leaves \
                 pattern_nodes permanently empty"
            );
        }
    }

    // Step 4 — the next indexing pass must genuinely RE-EXTRACT: rows rebuilt
    // under the new keying, not a fingerprint-skip that only refreshes lines.
    {
        let mut indexer = session.indexer(IndexOptions {
            embed_semantic: false,
            ..session.index_options()
        });
        let stats = indexer.index_all().expect("reindex after migration");
        assert_eq!(
            stats.files_indexed, 1,
            "identity invalidation must force re-extraction of the fixture"
        );
    }
    {
        let store = session.open_store();
        let scoped: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pattern_nodes WHERE signature = 'call:Foo::bar'",
                [],
                |r| r.get(0),
            )
            .expect("count scoped-call rows");
        assert!(
            scoped >= 1,
            "F76c-1: the post-migration pass must rebuild signature rows under the \
             current keying (pre-fix the armed struct: fingerprint routed the file \
             to refresh_lines_only and pattern_nodes stayed empty)"
        );
        let bare: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pattern_nodes WHERE signature = 'call:bar'",
                [],
                |r| r.get(0),
            )
            .expect("count bare-callee rows");
        assert_eq!(bare, 0, "no bare bar() call exists, so no call:bar row may exist");
    }

    // Step 5 — F72a-1 faces on the rebuilt index (build + search cycle):
    // exact face answers both lines; the stale over-match face answers empty.
    {
        let searcher = session.searcher(SearchOptions {
            use_embed: false,
            lang_filter: Some("php".to_string()),
            ..session.search_options()
        });
        let exact = searcher.search("pattern:Foo::bar($A)").expect("exact face");
        let mut lines: Vec<u32> = exact.hits.iter().map(|hit| hit.line_start).collect();
        lines.sort_unstable();
        assert_eq!(lines, vec![7, 8], "Foo::bar($A) must answer both scoped calls");
        let overmatch = searcher.search("pattern:bar($$$A)").expect("over-match face");
        assert!(
            overmatch.hits.is_empty(),
            "bar($$$A) must answer empty on a fresh index: {:?}",
            overmatch.hits
        );
    }

    // Step 6 — idempotence: a second writable open at the current stamp is a
    // no-op (the version gate prevents migration re-entry), so the rebuilt
    // rows and the stamp survive.
    {
        let store = session.open_store();
        assert_eq!(
            store.on_disk_schema_version().expect("on-disk stamp"),
            INDEX_SCHEMA_VERSION,
            "second open must not re-run the migration"
        );
        let scoped: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM pattern_nodes WHERE signature = 'call:Foo::bar'",
                [],
                |r| r.get(0),
            )
            .expect("count scoped-call rows after second open");
        assert!(
            scoped >= 1,
            "second writable open must not re-wipe the rebuilt rows"
        );
    }
}

/// Contract 3 (F76c-2): `plan_codemod` only reads the store, so it must open
/// READ-ONLY and therefore refuse a pre-current store loudly (naming the
/// rebuild remedy, symmetric with search) instead of silently firing the
/// row-discarding migration as a side effect of a planning read. Pre-fix
/// (writable open) a mere `codemod --dry-run` migrated the store to the
/// current stamp with `pattern_nodes` wiped and hashes prefixed — demoting
/// the index with no re-extract, no warning, and no marker, so read-only
/// search then served the demoted index as current.
#[test]
fn codemod_plan_refuses_pre_current_store_loudly_without_migrating() {
    let session = isolated_index_session();
    session.write("probe.php", PHP_FIXTURE);
    seed_pre_rekey_store(&session.corpus_root, &session.index_path);

    let error = match plan_codemod(
        &session.corpus_root,
        Some(&session.index_path),
        Some("php"),
        "Foo::bar($A)",
        "Foo::baz($A)",
    ) {
        Ok(plan) => panic!(
            "codemod plan on a pre-current store must refuse loudly, not silently \
             migrate + demote it; got ok plan (edit_count {})",
            plan.edit_count
        ),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("reindex"),
        "loud refusal must name the rebuild remedy: {error}"
    );

    // The store is untouched: still stamped 14, stale row intact, hash
    // unprefixed. Inspected through a raw read-only connection because any
    // writable open would itself migrate.
    let conn = rusqlite::Connection::open_with_flags(
        &session.index_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("read-only peek at the untouched store");
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("read user_version");
    assert_eq!(
        version, PRE_REKEY_STAMP,
        "planning must never migrate the store (a read-only command must not \
         fire a row-discarding migration)"
    );
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM pattern_nodes", [], |r| r.get(0))
        .expect("count signature rows");
    assert_eq!(rows, 1, "planning must not discard signature rows");
    let hash: String = conn
        .query_row(
            "SELECT content_hash FROM files WHERE path = 'probe.php'",
            [],
            |r| r.get(0),
        )
        .expect("read stored hash");
    assert_eq!(
        hash, "pre-rekey-hash",
        "planning must not prefix content hashes (the demotion marker)"
    );
}
