//! Recovery CLI: auxiliary surfaces (doctor, filesystem paths, bench history,
//! install config, watch resume).
//!
//! Covers catalog `tests/catalog/recovery.md` CLI rows: doctor verdicts,
//! read-only/missing paths, bench history missing/torn rebuild, committed-prior
//! fail-open, install config recovery, and killed-watch resume. Discriminants
//! only (exit code, envelope, shapes, bytes) — never message text.

#[cfg(unix)]
use ast_sgrep_testkit::kill9;
use ast_sgrep_testkit::{
    assert_doctor_unhealthy, assert_failure_envelope, assert_success, run_in, run_index,
    run_reindex, run_search, run_status, seed_project, KillOnDrop, QUERY, SOURCE,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Exact binary path via the compile-time env cargo sets for THIS test target.
/// `env!` must expand here (it is unset inside the testkit dependency); the
/// resolved path is passed as `bin` to the testkit runners.
fn asgrep() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// INTENT: doctor verdicts track durable state — corrupt or missing databases
/// report `index_open` with healthy:false, null status, exit 2; a healed index
/// flips back to healthy:true with zero issues.
/// FACETS: corrupt-db arm, missing-db arm, healed arm.
/// KILLS: ok-true/wrong-kind, recovery-residue (doctor still red after heal).
/// COVERS: doctor_reports_corrupt_and_missing_index_as_index_open.
#[test]
fn doctor_verdicts_track_durable_state() {
    let (temp, root, index) = seed_project();
    run_index(&asgrep(), &temp, &root, &index);
    fs::write(&index, b"RECOVERY-AUX-DOCTOR-GARBAGE-0001-NOT-SQLITE").unwrap();
    let root_s = root.to_str().unwrap();

    for db in [index.clone(), temp.path().join("nonexistent.db")] {
        let output = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                db.to_str().unwrap(),
                "--json",
                "doctor",
                root_s,
            ],
            &[],
        );
        assert_doctor_unhealthy(&output, &db.display().to_string());
    }

    let healed = run_reindex(&asgrep(), &temp, &root, &index, &[]);
    assert_eq!(healed["files_indexed"], 1);
    let output = run_in(
        &asgrep(),
        temp.path(),
        &[
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "doctor",
            root_s,
        ],
        &[],
    );
    let doctor = assert_success(&output, "doctor");
    assert_eq!(
        doctor["healthy"], true,
        "doctor must flip healthy after heal: {doctor}"
    );
    assert_eq!(doctor["issues"].as_array().unwrap().len(), 0);
}

/// INTENT: unwritable and absent paths fail closed (exit 2 + operational) without
/// leaving damage, while a read-only db FILE in a writable home still serves.
/// FACETS: read-only home refuses fresh index (nothing created) and refuses
/// readers, then recovers fully; read-only db file serves; read-only history
/// dir refuses bench; missing project root refuses status.
/// KILLS: panic/partial-write, file/dir-split, serve-without-root mutants.
/// COVERS: fault_readonly_index_home_fails_closed_exit_2,
/// unwritable_and_missing_dirs_error_cleanly.
#[test]
fn readonly_and_missing_paths_fail_closed() {
    // Missing root: status refuses without touching anything (portable facet).
    {
        let (temp, root, index) = seed_project();
        run_index(&asgrep(), &temp, &root, &index);
        let missing_root = temp.path().join("no-such-root");
        let output = run_in(
            &asgrep(),
            temp.path(),
            &[
                "--index-path",
                index.to_str().unwrap(),
                "--no-embed",
                "--json",
                "status",
                missing_root.to_str().unwrap(),
            ],
            &[],
        );
        assert_failure_envelope(&output, "status", 2, "operational");
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Read-only home: writes AND readers refuse; writability restores fully.
        {
            let temp = TempDir::new().unwrap();
            let root = temp.path().join("proj");
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/lib.rs"), SOURCE).unwrap();
            let state = temp.path().join("state");
            fs::create_dir(&state).unwrap();
            let db = state.join("idx.db");
            let root_s = root.to_str().unwrap();
            let db_s = db.to_str().unwrap();

            fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
            let output = run_in(
                &asgrep(),
                temp.path(),
                &[
                    "--index-path",
                    db_s,
                    "--no-embed",
                    "--json",
                    "index",
                    root_s,
                ],
                &[],
            );
            assert_failure_envelope(&output, "index", 2, "operational");
            assert!(
                !db.exists(),
                "the refused index must not leave a torn database behind"
            );
            fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();

            let output = run_in(
                &asgrep(),
                temp.path(),
                &[
                    "--index-path",
                    db_s,
                    "--no-embed",
                    "--json",
                    "index",
                    root_s,
                ],
                &[],
            );
            assert_success(&output, "index");
            fs::set_permissions(&state, fs::Permissions::from_mode(0o555)).unwrap();
            let status = run_in(
                &asgrep(),
                temp.path(),
                &[
                    "--index-path",
                    db_s,
                    "--no-embed",
                    "--json",
                    "status",
                    root_s,
                ],
                &[],
            );
            assert_failure_envelope(&status, "status", 2, "operational");
            fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
            let status = run_in(
                &asgrep(),
                temp.path(),
                &[
                    "--index-path",
                    db_s,
                    "--no-embed",
                    "--json",
                    "status",
                    root_s,
                ],
                &[],
            );
            let status = assert_success(&status, "status");
            assert_eq!(status["file_count"], 1);

            // The file/dir split: a read-only db FILE in a writable home serves.
            fs::set_permissions(&db, fs::Permissions::from_mode(0o444)).unwrap();
            let status = run_in(
                &asgrep(),
                temp.path(),
                &[
                    "--index-path",
                    db_s,
                    "--no-embed",
                    "--json",
                    "status",
                    root_s,
                ],
                &[],
            );
            let status = assert_success(&status, "status");
            assert_eq!(status["file_count"], 1);
            fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
        }

        // Read-only history dir: bench refuses with the operational discriminant.
        {
            let (temp, root, index) = seed_project();
            run_index(&asgrep(), &temp, &root, &index);
            let ro = temp.path().join("ro");
            fs::create_dir(&ro).unwrap();
            fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
            let hist_dir = temp.path().join("hist").to_str().unwrap().to_owned();
            let ro_history = ro.join("hist.json").to_str().unwrap().to_owned();
            let output = run_in(
                &asgrep(),
                temp.path(),
                &[
                    "--no-embed",
                    "--index-path",
                    index.to_str().unwrap(),
                    "--json",
                    "bench",
                    "--query",
                    QUERY,
                    "--iterations",
                    "1",
                    root.to_str().unwrap(),
                ],
                &[
                    ("ASGREP_BENCH_HISTORY_PATH", ro_history.as_str()),
                    ("ASGREP_BENCH_HISTORY_DIR", hist_dir.as_str()),
                ],
            );
            assert_failure_envelope(&output, "bench", 2, "operational");
            fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// One-use bench runner: the history-path envs differ per facet, so the closure
/// shape stays local to this test rather than growing the shared harness.
fn run_bench(
    temp: &TempDir,
    root_s: &str,
    index_s: &str,
    history_s: &str,
    hist_dir_s: &str,
) -> std::process::Output {
    run_in(
        &asgrep(),
        temp.path(),
        &[
            "--no-embed",
            "--index-path",
            index_s,
            "--json",
            "bench",
            "--query",
            QUERY,
            "--iterations",
            "1",
            root_s,
        ],
        &[
            ("ASGREP_BENCH_HISTORY_PATH", history_s),
            ("ASGREP_BENCH_HISTORY_DIR", hist_dir_s),
        ],
    )
}

/// INTENT: keep-gate history degrades safely per artifact — a missing aggregate
/// (even with missing parents) is rebuilt, a torn aggregate fails loud with its
/// bytes preserved and re-establishes the IDENTICAL verdict once removed, while
/// a corrupt committed prior fails open to a fresh baseline.
/// FACETS: missing file+dirs rebuild (schema v1, label entry); torn aggregate
/// (exit 2, bytes preserved, identical verdict after removal); generous prior
/// control arm (keep) then corrupt prior (establish_baseline, bytes preserved,
/// run snapshot written).
/// KILLS: refuse-on-missing, reset-evidence, verdict-drift, fail-closed-on-prior.
/// COVERS: bench_history_missing_file_and_dir_are_rebuilt,
/// fault_torn_truncated_bench_history_fails_loud_preserves_bytes,
/// relation_corrupt_recover_verify_roundtrip_bench_history,
/// corrupt_committed_prior_fails_open_to_baseline.
#[test]
fn bench_history_fault_matrix() {
    // Missing history (parents included) is rebuilt, not refused.
    {
        let (temp, root, index) = seed_project();
        let history = temp.path().join("histdir").join("sub").join("hist.json");
        assert!(!history.parent().unwrap().exists());
        let hist_dir = temp.path().join("hist").to_str().unwrap().to_owned();
        let value = assert_success(
            &run_bench(
                &temp,
                root.to_str().unwrap(),
                index.to_str().unwrap(),
                history.to_str().unwrap(),
                &hist_dir,
            ),
            "bench",
        );
        assert_eq!(value["bench_history"]["verdict"], "establish_baseline");
        assert_eq!(value["bench_history"]["ratchet_ok"], true);
        let doc: Value = serde_json::from_str(&fs::read_to_string(&history).unwrap()).unwrap();
        assert_eq!(doc["schema_version"], "1");
        assert!(
            doc["entries"]["query:probe_target"]["avg_search_ms"]
                .as_f64()
                .is_some(),
            "rebuilt history must carry the label entry: {doc}"
        );
    }

    // Torn aggregate fails loud, preserves bytes, then re-establishes identically.
    {
        let (temp, root, index) = seed_project();
        let history = temp.path().join("hist.json");
        let hist_dir = temp.path().join("hist").to_str().unwrap().to_owned();
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let history_s = history.to_str().unwrap().to_owned();
        let fresh = assert_success(
            &run_bench(&temp, &root_s, &index_s, &history_s, &hist_dir),
            "bench",
        );
        assert_eq!(fresh["bench_history"]["verdict"], "establish_baseline");
        let mut torn = fs::read(&history).unwrap();
        torn.truncate(torn.len() / 2);
        assert!(
            serde_json::from_slice::<Value>(&torn).is_err(),
            "the harness must inject genuinely torn JSON"
        );
        fs::write(&history, &torn).unwrap();
        assert_failure_envelope(
            &run_bench(&temp, &root_s, &index_s, &history_s, &hist_dir),
            "bench",
            2,
            "operational",
        );
        assert_eq!(
            fs::read(&history).unwrap(),
            torn,
            "the torn aggregate is evidence and must not be reset"
        );
        fs::remove_file(&history).unwrap();
        let recovered = assert_success(
            &run_bench(&temp, &root_s, &index_s, &history_s, &hist_dir),
            "bench",
        );
        assert_eq!(
            recovered["bench_history"]["verdict"], fresh["bench_history"]["verdict"],
            "recovered history must re-establish the fresh-run verdict"
        );
        let doc: Value = serde_json::from_str(&fs::read_to_string(&history).unwrap()).unwrap();
        assert_eq!(doc["schema_version"], "1");
        assert!(
            doc["entries"]["query:probe_target"]["avg_search_ms"]
                .as_f64()
                .is_some(),
            "recovered history must carry the label entry: {doc}"
        );
    }

    // Committed prior: generous prior consulted (keep), corrupt prior fails open.
    {
        let (temp, root, index) = seed_project();
        let hist_dir = temp.path().join("hist");
        fs::create_dir(&hist_dir).unwrap();
        let history = temp.path().join("hist.json");
        let root_s = root.to_str().unwrap().to_owned();
        let index_s = index.to_str().unwrap().to_owned();
        let history_s = history.to_str().unwrap().to_owned();
        let hist_dir_s = hist_dir.to_str().unwrap().to_owned();
        // Use the CLI's declared paths, including its collision-resistant label
        // identity, rather than seeding an obsolete unhashed filename.
        let baseline = assert_success(
            &run_bench(&temp, &root_s, &index_s, &history_s, &hist_dir_s),
            "bench",
        );
        assert_eq!(baseline["bench_history"]["verdict"], "establish_baseline");
        let prior = PathBuf::from(
            baseline["bench_history"]["committed_prior"]
                .as_str()
                .unwrap(),
        );
        let run_path = PathBuf::from(baseline["bench_history"]["run_path"].as_str().unwrap());
        fs::write(
            &prior,
            serde_json::json!({
                "schema_version": "1",
                "label": "query:probe_target",
                "avg_search_ms": 1_000_000_000.0,
                "geomean_search_ms": null,
                "cv_pct": 0.0,
                "host": "oracle",
                "git_sha": "oracle",
                "profile": "oracle",
                "verdict": "keep",
                "updated_unix_ms": 0,
                "placeholder": false,
                "keep_eligible": true
            })
            .to_string(),
        )
        .unwrap();
        let value = assert_success(
            &run_bench(&temp, &root_s, &index_s, &history_s, &hist_dir_s),
            "bench",
        );
        assert_eq!(
            value["bench_history"]["verdict"], "keep",
            "the generous prior must be consulted (control arm): {value}"
        );
        fs::write(&prior, "{ corrupt json").unwrap();
        let value = assert_success(
            &run_bench(&temp, &root_s, &index_s, &history_s, &hist_dir_s),
            "bench",
        );
        assert_eq!(
            value["bench_history"]["verdict"], "establish_baseline",
            "a corrupt prior must fail open to a fresh baseline: {value}"
        );
        assert_eq!(
            fs::read_to_string(&prior).unwrap(),
            "{ corrupt json",
            "the corrupt prior is evidence and must not be rewritten"
        );
        let run_snapshot: Value =
            serde_json::from_str(&fs::read_to_string(run_path).unwrap()).unwrap();
        assert_eq!(run_snapshot["label"], "query:probe_target");
        assert_eq!(run_snapshot["verdict"], "establish_baseline");
    }
}

/// INTENT: install writes are create-if-absent and never clobber evidence — a
/// missing agent config is created with the managed entry, a corrupt config is
/// refused (exit 2 + operational) with its bytes kept for human repair.
/// FACETS: missing-config creation arm, corrupt-config refusal arm.
/// KILLS: overwrite-corrupt, refuse-on-missing mutants.
/// COVERS: install_config_missing_rebuilt_corrupt_refused.
#[test]
fn install_config_missing_rebuilt_corrupt_refused() {
    let temp = TempDir::new().unwrap();
    let cursor_dir = temp.path().join("cursor").to_str().unwrap().to_owned();
    let fake_bin = temp.path().join("fake-asgrep-mcp");
    fs::write(&fake_bin, "#!/bin/sh\n").unwrap();
    let fake_bin_s = fake_bin.to_str().unwrap().to_owned();
    let config = temp.path().join("cursor").join("mcp.json");

    let output = run_in(
        &asgrep(),
        temp.path(),
        &["install", "--target", "cursor", "--yes"],
        &[
            ("CURSOR_CONFIG_DIR", cursor_dir.as_str()),
            ("ASGREP_MCP_BIN", fake_bin_s.as_str()),
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "missing config must be created: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let doc: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(doc["mcpServers"]["asgrep"]["command"], fake_bin_s);

    // Legacy and current names share the same explicit replacement contract.
    for name in ["asgrep", "ast-sgrep", "zvec_grep"] {
        let authored = json!({
            "editor": {"keep": true},
            "mcpServers": {
                name: {"command": "old", "env": {"OLD": "legacy"}},
                "peer": {"command": "keep-peer", "args": ["authored argument"]}
            }
        });
        let bytes = serde_json::to_string_pretty(&authored).unwrap();
        fs::write(&config, &bytes).unwrap();
        let run = |force: bool| {
            let mut args = vec!["install", "--target", "cursor", "--yes", "--json"];
            if force {
                args.push("--force");
            }
            run_in(
                &asgrep(),
                temp.path(),
                &args,
                &[
                    ("CURSOR_CONFIG_DIR", cursor_dir.as_str()),
                    ("ASGREP_MCP_BIN", fake_bin_s.as_str()),
                ],
            )
        };
        assert_failure_envelope(&run(false), "install", 2, "operational");
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            bytes,
            "refusal must preserve {name}"
        );
        assert!(run(true).status.success());
        let merged = fs::read_to_string(&config).unwrap();
        let doc: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(doc["editor"], authored["editor"]);
        assert_eq!(doc["mcpServers"]["peer"], authored["mcpServers"]["peer"]);
        assert_eq!(doc["mcpServers"].as_object().unwrap().len(), 2);
        assert_eq!(doc["mcpServers"]["asgrep"]["command"], fake_bin_s);
        assert_eq!(doc["mcpServers"]["asgrep"]["env"], json!({}));
        assert!(run(true).status.success());
        assert_eq!(fs::read_to_string(&config).unwrap(), merged);
    }

    fs::write(&config, "{ corrupt json").unwrap();
    let output = run_in(
        &asgrep(),
        temp.path(),
        &[
            "install", "--target", "cursor", "--yes", "--force", "--json",
        ],
        &[
            ("CURSOR_CONFIG_DIR", cursor_dir.as_str()),
            ("ASGREP_MCP_BIN", fake_bin_s.as_str()),
        ],
    );
    assert_failure_envelope(&output, "install", 2, "operational");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "{ corrupt json",
        "a corrupt config must not be overwritten"
    );
}

#[test]
fn install_codex_force_replaces_server_without_touching_peers() {
    let temp = TempDir::new().unwrap();
    let command = "/new/quoted\"\\asgrep-mcp\nnext";
    let project = "[projects.\"/preserve/project\"]\ntrust_level = \"trusted\"\n";
    // Header/marker-looking string data must never become deletion boundaries.
    let preamble = "model = \"keep-model\"\nnotes = '''\n[mcp_servers.asgrep]\n# BEGIN asgrep-mcp\n# END asgrep-mcp\n'''\n";
    let entries = [
        "[mcp_servers.ast-sgrep]\ncommand = 'old'\n[mcp_servers.ast-sgrep.env]\nOLD = 'legacy'\n[mcp_servers.peer]\ncommand = 'keep-peer'\n",
        "[mcp_servers.asgrep]\ncommand = 'old'\n[mcp_servers.asgrep.env]\nOLD = 'legacy'\n[mcp_servers.peer]\ncommand = 'keep-peer'\n",
        "[mcp_servers.'ast-sgrep']\ncommand = 'old'\n[mcp_servers.'ast-sgrep'.env]\nOLD = 'legacy'\n[mcp_servers.peer]\ncommand = 'keep-peer'\n",
        "mcp_servers = { asgrep = { command = 'old', env = { OLD = 'legacy' } }, peer = { command = 'keep-peer' } }\n",
        "mcp_servers.asgrep.command = 'old'\nmcp_servers.asgrep.env.OLD = 'legacy'\nmcp_servers.peer.command = 'keep-peer'\n",
        "# BEGIN asgrep-mcp (managed by `asgrep install`; no daemon)\n[mcp_servers.asgrep]\ncommand = 'old'\nargs = []\n# END asgrep-mcp\n[mcp_servers.peer]\ncommand = 'keep-peer'\n",
    ];
    for (case, entry) in entries.iter().enumerate() {
        let home = temp.path().join(case.to_string());
        fs::create_dir(&home).unwrap();
        let config = home.join("config.toml");
        let authored = format!("{preamble}{entry}{project}");
        fs::write(&config, &authored).unwrap();
        let run = |force: bool| {
            let mut args = vec!["install", "--target", "codex", "--yes"];
            if force {
                args.push("--force");
            }
            run_in(
                &asgrep(),
                temp.path(),
                &args,
                &[
                    ("CODEX_HOME", home.to_str().unwrap()),
                    ("ASGREP_MCP_BIN", command),
                ],
            )
        };
        assert_eq!(run(false).status.code(), Some(2), "case {case}");
        assert_eq!(fs::read_to_string(&config).unwrap(), authored);
        let output = run(true);
        assert!(
            output.status.success(),
            "case {case}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let merged = fs::read_to_string(&config).unwrap();
        let doc = merged.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(
            doc["mcp_servers"]["asgrep"]["command"].as_str(),
            Some(command)
        );
        assert_eq!(
            doc["mcp_servers"]["asgrep"]["args"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        let servers = doc["mcp_servers"].as_table_like().unwrap();
        assert!(
            servers.get("ast-sgrep").is_none(),
            "legacy server must not stay live"
        );
        assert!(
            servers
                .get("asgrep")
                .unwrap()
                .as_table_like()
                .unwrap()
                .get("env")
                .is_none(),
            "superseded descendants must be removed"
        );
        assert_eq!(
            doc["mcp_servers"]["peer"]["command"].as_str(),
            Some("keep-peer")
        );
        assert!(
            merged.contains(preamble) && merged.contains(project),
            "authored peers must retain their formatting"
        );
        assert!(run(true).status.success());
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            merged,
            "repeated force must be idempotent"
        );
        for invalid in ["{ corrupt toml", "mcp_servers = 'not a table'\n"] {
            fs::write(&config, invalid).unwrap();
            assert_eq!(run(true).status.code(), Some(2));
            assert_eq!(fs::read_to_string(&config).unwrap(), invalid);
        }
    }
}

/// INTENT: a `watch` loop killed mid-run resumes through the next plain `index`
/// (no full reindex): status intact and the raced edit searchable.
/// FACETS: initial commit visible cross-process while the loop lives; raced edit
/// in the kill window; SIGKILL death by signal; plain-index resume converges.
/// KILLS: resume-requires-reindex, lost-edit mutants.
/// COVERS: fault_killed_watch_resumes_via_next_index.
#[test]
#[cfg(unix)]
fn killed_watch_resumes_via_next_index() {
    use std::io::{BufRead, BufReader};
    use std::sync::{Arc, Mutex};

    let (temp, root, index) = seed_project();
    let root_s = root.to_str().unwrap().to_owned();
    let index_s = index.to_str().unwrap().to_owned();

    // A real `watch` re-execs the sibling helper; guarantee it is the
    // current build, not a stale sibling from an earlier target state.
    let bin = asgrep();
    ast_sgrep_testkit::ensure_watch_helper(&bin);
    let watch = Command::new(&bin)
        .args([
            "--index-path",
            &index_s,
            "--no-embed",
            "watch",
            "--debounce-ms",
            "50",
            &root_s,
        ])
        .env("NO_COLOR", "1")
        .current_dir(temp.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let mut watch = KillOnDrop(Some(watch));
    let stderr = watch.child().stderr.take().expect("piped watch stderr");
    let log = Arc::new(Mutex::new(String::new()));
    let writer = Arc::clone(&log);
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if let Ok(mut held) = writer.lock() {
                held.push_str(&line);
                held.push('\n');
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = log.lock().map(|held| held.clone()).unwrap_or_default();
        if text.contains("initial index") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "watch never finished its initial index: {text}"
        );
        assert!(
            watch.child().try_wait().expect("try_wait watch").is_none(),
            "watch died before its initial index: {text}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The initial commit is durable and visible cross-process while the loop lives.
    let status = run_status(&asgrep(), &temp, &root_s, &index_s, &[]);
    assert_eq!(status["file_count"], 1);

    // The raced edit: it may or may not reach the loop before the shot.
    fs::write(
        root.join("src/lib.rs"),
        "fn probe_target() { run(1); }\nfn raced_edit() {}\n",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let pid = watch.child().id();
    assert!(
        kill9(pid),
        "the SIGKILL fault must land on a live watch loop"
    );
    let death = watch.child().wait().expect("wait watch");
    assert!(
        death.code().is_none(),
        "watch must die by signal, got {death:?}"
    );

    // Resume boundary: the next plain index converges, no reindex needed.
    run_index(&asgrep(), &temp, &root, &index);
    let status = run_status(&asgrep(), &temp, &root_s, &index_s, &[]);
    assert_eq!(status["file_count"], 1);
    let search = run_search(
        &asgrep(),
        &temp,
        &root_s,
        &index_s,
        "raced_edit",
        &["--no-embed"],
        &[],
    );
    assert!(
        search["hits"]
            .as_array()
            .is_some_and(|hits| !hits.is_empty()),
        "the resumed index must answer the raced edit: {search}"
    );
}
