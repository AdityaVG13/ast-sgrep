use ast_sgrep_testkit::{
    asgrep_bin, core_search_hit_keys, json_hit_keys, lsp_search_hit_keys, CliSession,
    HitKey,
};
use serde_json::Value;

fn sorted_keys(mut keys: Vec<HitKey>) -> Vec<HitKey> {
    keys.sort();
    keys
}

fn embed_keys(keys: &[HitKey]) -> Vec<HitKey> {
    keys.iter().filter(|k| k.kind == "embed").cloned().collect()
}

/// x1p5: multi-mode surface equivalence (CLI / core / LSP HitKeys) with
/// `--no-embed`. Equal-score ties may differ in emission order across surfaces;
/// compare sorted rich HitKeys (file, line, kind, symbol, callee, caller).
#[test]
fn surface_equivalence_multi_mode_hit_keys() {
    const LIMIT: usize = 10;
    let session = CliSession::sample(asgrep_bin());
    let cases: &[(&str, &[&str])] = &[
        ("process_request", &["--limit", "10", "--no-embed"]),
        ("defs:process_request", &["--limit", "10", "--no-embed"]),
        ("callers:process_request", &["--limit", "10", "--no-embed"]),
        ("imports:lib", &["--limit", "10", "--no-embed"]),
        ("pattern:process_request($$$A)", &["--limit", "10", "--no-embed"]),
        (
            "how does auth refresh work",
            &["--limit", "10", "--no-embed"],
        ),
    ];
    for &(query, extra) in cases {
        let cli = sorted_keys(json_hit_keys(&session.search_json(query, extra)));
        let core = sorted_keys(core_search_hit_keys(
            &session.root,
            &session.index_path,
            query,
            LIMIT,
            /* use_embed */ false,
        ));
        let lsp = sorted_keys(lsp_search_hit_keys(
            &session.root,
            &session.index_path,
            query,
            LIMIT,
            /* use_embed */ false,
        ));
        assert!(
            !core.is_empty() || query.starts_with("imports:") || query.starts_with("pattern:"),
            "fixture query {query:?} must produce core hits (or be a known sparse mode)"
        );
        assert_eq!(cli, core, "CLI JSON diverged from core for query {query:?}");
        assert_eq!(
            lsp, core,
            "LSP search diverged from core for query {query:?}"
        );
    }
}

/// lbx1.13: embed-kind hit-key parity across surfaces with embed ON (hashed).
///
/// `--no-embed` parity alone does not close this bead. Same corpus/index;
/// search with embed on (CLI default, core use_embed=true, LSP no_embed=false):
/// - sorted embed hit-keys agree across CLI / core / LSP
/// - full sorted key sets also agree (hybrid fusion identity)
/// - non-empty embed-kind keys where the semantic channel fires (no soft-skip)
///
/// Since ba3fa030 the semantic channel is a fallback: it stays silent when
/// structural evidence answers. Only the zero-overlap query carries the
/// non-empty requirement; the structurally answered queries still pin
/// cross-surface parity on identical (embed-silent) output.
#[test]
fn surface_equivalence_embed_on_hit_keys() {
    const LIMIT: usize = 32;
    let session = CliSession::sample(asgrep_bin());

    // (query, expect_embed): `credential renewal` is zero-overlap (embed
    // fires); the auth_refresh queries are structurally answered (embed
    // correctly silent). Hashed backend -- no network.
    let cases: &[(&str, bool)] = &[
        ("credential renewal", true),
        ("how does auth refresh work", false),
        ("auth_refresh", false),
    ];

    for &(query, expect_embed) in cases {
        // CLI: production default is embed-on (do NOT pass --no-embed).
        let cli_json = session.search_json(query, &["--limit", "32"]);
        let cli = sorted_keys(json_hit_keys(&cli_json));
        let core = sorted_keys(core_search_hit_keys(
            &session.root,
            &session.index_path,
            query,
            LIMIT,
            /* use_embed */ true,
        ));
        let lsp = sorted_keys(lsp_search_hit_keys(
            &session.root,
            &session.index_path,
            query,
            LIMIT,
            /* use_embed */ true,
        ));

        assert!(
            !core.is_empty(),
            "embed-on core search must return hits for {query:?}"
        );

        let cli_embed = embed_keys(&cli);
        let core_embed = embed_keys(&core);
        let lsp_embed = embed_keys(&lsp);

        // Hard fail: empty embed channel where the semantic fallback fires
        // is a bug, not a soft-skip (mock-free e2e gap lbx1.13 negative).
        // Queries the structural evidence answers skip this: embed silence
        // there is intended fallback behavior, and the parity asserts below
        // still bind all three surfaces to identical output.
        if expect_embed {
            assert!(
                !core_embed.is_empty(),
                "embed-on core must emit kind=embed hits for {query:?}; keys={core:?}"
            );
            assert!(
                !cli_embed.is_empty(),
                "embed-on CLI must emit kind=embed hits for {query:?}; keys={cli:?}"
            );
            assert!(
                !lsp_embed.is_empty(),
                "embed-on LSP must emit kind=embed hits for {query:?}; keys={lsp:?}"
            );
        }

        assert_eq!(
            cli_embed, core_embed,
            "embed-kind keys: CLI vs core for {query:?}"
        );
        assert_eq!(
            lsp_embed, core_embed,
            "embed-kind keys: LSP vs core for {query:?}"
        );

        // Full hybrid key identity (embed + non-embed contributors).
        assert_eq!(cli, core, "full hit keys: CLI vs core for {query:?}");
        assert_eq!(lsp, core, "full hit keys: LSP vs core for {query:?}");
    }
}

/// x1p5: both-error table — core and CLI agree on failure for invalid inputs.
#[test]
fn surface_equivalence_both_error_table() {
    let session = CliSession::sample(asgrep_bin());
    // Invalid regex should fail on both surfaces (not silent-empty success).
    let bad_regex = "regex:(";
    let cli = session.run(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "--no-embed",
        bad_regex,
        session.root.to_str().unwrap(),
    ]);
    let cli_failed = !cli.status.success();

    let core = ast_sgrep_core::Searcher::new(ast_sgrep_core::SearchOptions {
        root: session.root.clone(),
        index_path: Some(session.index_path.clone()),
        limit: 10,
        use_embed: false,
        ..ast_sgrep_core::SearchOptions::default()
    })
    .and_then(|s| s.search(bad_regex));
    let core_failed = core.is_err();

    assert!(
        cli_failed && core_failed,
        "both-error: invalid regex must fail on CLI and core; cli_failed={cli_failed} core={core:?}"
    );

    // Polyglot-corpus backstop: `fn $NAME($$$)` is natively answerable per
    // language, but the sample corpus holds languages with no native
    // template for the face and the walk finds no hits -- per-file
    // Ok(empty) there is unanswerability, not agreement, so the search
    // stays loud on every surface instead of composing a silent empty.
    let beyond_native_face = "pattern:fn $NAME($$$)";
    let cli_face = session.run(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "--no-embed",
        beyond_native_face,
        session.root.to_str().unwrap(),
    ]);
    let core_face = ast_sgrep_core::Searcher::new(ast_sgrep_core::SearchOptions {
        root: session.root.clone(),
        index_path: Some(session.index_path.clone()),
        limit: 10,
        use_embed: false,
        ..ast_sgrep_core::SearchOptions::default()
    })
    .and_then(|s| s.search(beyond_native_face));
    let core_face_msg = format!("{core_face:?}");
    assert!(
        !cli_face.status.success()
            && core_face.is_err()
            && core_face_msg.contains("structural fallback"),
        "both-error: beyond-native face must fail closed on CLI and core; cli={} core={core_face_msg}",
        cli_face.status
    );

    // Empty/whitespace query: both return structured empty success (not a crash).
    // Vacuous `assert!(is_empty() || true)` is forbidden -- assert real shape.
    let core_empty = core_search_hit_keys(&session.root, &session.index_path, " ", 5, false);
    assert!(
        core_empty.is_empty(),
        "whitespace-only hybrid query must yield zero hits; got {core_empty:?}"
    );
    let cli_ws = session.search_json(" ", &["--limit", "5", "--no-embed"]);
    let cli_hits = cli_ws["hits"].as_array().cloned().unwrap_or_default();
    assert!(
        cli_hits.is_empty(),
        "CLI whitespace-only query must yield zero hits; got {cli_hits:?}"
    );

    // Confirm usage error path remains observable.
    let usage = session.run_failure(&["--index-path", session.index_path.to_str().unwrap()]);
    let _: Value = serde_json::from_slice(&usage.stdout).unwrap_or(Value::Null);
    assert!(!usage.status.success());
}
