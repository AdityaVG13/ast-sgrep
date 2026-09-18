//! I4 end-to-end freshness drills for `ast-sgrep-codemode`.
//!
//! Where I1 pins the freshness *contract*, I2 proves each *delta class*, and
//! I3 asserts *relations* between refresh paths, I4 runs FULL drills through
//! the session's own tools (`find`, `search`, `defs`, `index_repo`,
//! `index_status`, `edit`), each in four phases:
//!
//! 1. SERVE: baseline tool results are captured exactly.
//! 2. CHANGE: a real repo mutation is applied on disk.
//! 3. DETECT: staleness is visible (pinned stale serves / unchanged epoch).
//! 4. REFRESH + SERVE: the refresh path runs, and SERVE proves the exact new
//!    tool results (hit sets, counts, bytes, census).
//!
//! One drill per change kind (add / modify / delete / rename / edit-tool /
//! force-rebuild), one render-cache freshness drill, and one chained
//! multi-change drill linking every kind in sequence.
//!
//! Discriminants are asserted with `matches!` / counts / bytes of normalized
//! hit sets only; no error-message text is matched.

use ast_sgrep_codemode::{CodeModeSession, SessionConfig};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const GAMMA: &str = "snorkel_gamma_unique";
const DELTA: &str = "snorkel_delta_unique";

fn write_py(root: &Path, name: &str, token: &str) {
    fs::write(
        root.join(name),
        format!("def {token}():\n    return 1\n"),
    )
    .expect("write fixture");
}

fn session_for(root: &Path, index_db: &Path) -> CodeModeSession {
    CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_db.to_path_buf()),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    })
}

/// Fresh root + index DB with `alpha.py` (`ALPHA`) indexed.
fn setup() -> (TempDir, TempDir, CodeModeSession) {
    let root = TempDir::new().expect("root");
    let index_dir = TempDir::new().expect("index dir");
    write_py(root.path(), "alpha.py", ALPHA);
    let mut session = session_for(root.path(), &index_dir.path().join("index.db"));
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], true);
    (root, index_dir, session)
}

fn hit_name_set(value: &Value) -> BTreeSet<String> {
    value["hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|hit| {
            hit.get("file")
                .and_then(|file| file.as_str())
                .map(|file| file.rsplit('/').next().unwrap_or(file).to_string())
        })
        .collect()
}

/// Byte-stable encoding of a hit set for exact serve assertions.
fn hit_bytes(value: &Value) -> Vec<u8> {
    hit_name_set(value)
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn hit_count(value: &Value) -> usize {
    value["hits"].as_array().map(Vec::len).unwrap_or(0)
}

fn find(session: &mut CodeModeSession, query: &str) -> Value {
    session
        .call("find", json!({"query": query, "limit": 8}))
        .expect("find")
}

fn search(session: &mut CodeModeSession, query: String) -> Value {
    session
        .call("search", json!({"query": query, "limit": 8}))
        .expect("search")
}

fn defs(session: &mut CodeModeSession, symbol: &str) -> Value {
    session
        .call("defs", json!({"symbol": symbol, "limit": 8}))
        .expect("defs")
}

fn refresh(session: &mut CodeModeSession, paths: &[&str]) -> Value {
    session
        .call("index_repo", json!({"paths": paths}))
        .expect("targeted refresh")
}

fn file_count(session: &mut CodeModeSession) -> u64 {
    session
        .call("index_status", json!({}))
        .expect("index status")["file_count"]
        .as_u64()
        .expect("file_count u64")
}

fn writer_generation(session: &mut CodeModeSession) -> u64 {
    session
        .call("index_status", json!({}))
        .expect("index status")["writer_generation"]
        .as_u64()
        .expect("writer_generation u64")
}

#[test]
fn drill_add_change_detect_refresh_serve() {
    let (root, _index, mut session) = setup();

    // SERVE: baseline resolves exactly the seeded file; the new symbol is
    // unknown and the new token unfindable.
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    assert!(hit_name_set(&defs(&mut session, BETA)).is_empty());
    let gen_before = writer_generation(&mut session);

    // CHANGE: real repo mutation on disk.
    write_py(root.path(), "beta.py", BETA);

    // DETECT: no writer ran, so the epoch is unchanged and serves are stale
    // (the new file is invisible to every read tool).
    assert_eq!(writer_generation(&mut session), gen_before);
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    assert!(hit_name_set(&search(&mut session, format!("word:{BETA}"))).is_empty());
    assert!(hit_name_set(&defs(&mut session, BETA)).is_empty());

    // REFRESH: targeted refresh of the added path.
    let refreshed = refresh(&mut session, &["beta.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_indexed"], 1);
    assert_ne!(writer_generation(&mut session), gen_before);

    // SERVE: exact new results — new token only in the new file, old intact.
    assert_eq!(
        hit_bytes(&find(&mut session, BETA)),
        b"beta.py".as_slice()
    );
    assert_eq!(hit_count(&find(&mut session, BETA)), 1);
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{BETA}"))),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&defs(&mut session, BETA)),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert_eq!(file_count(&mut session), 2);
}

#[test]
fn drill_modify_change_detect_refresh_serve() {
    let (root, _index, mut session) = setup();

    // SERVE: baseline pins the old token.
    let baseline = hit_bytes(&find(&mut session, ALPHA));
    assert_eq!(baseline, b"alpha.py".as_slice());
    let gen_before = writer_generation(&mut session);

    // CHANGE: rewrite the file with a new token.
    write_py(root.path(), "alpha.py", BETA);

    // DETECT: pinned index still serves the OLD token and misses the new one;
    // the epoch has not moved.
    assert_eq!(writer_generation(&mut session), gen_before);
    assert_eq!(hit_bytes(&find(&mut session, ALPHA)), baseline);
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());

    // REFRESH: targeted refresh of the rewritten path.
    let refreshed = refresh(&mut session, &["alpha.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_indexed"], 1);
    assert_ne!(writer_generation(&mut session), gen_before);

    // SERVE: exact swap — new token present, old token gone, via all tools.
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"alpha.py".as_slice());
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{BETA}"))),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(hit_name_set(&search(&mut session, format!("word:{ALPHA}"))).is_empty());
    assert_eq!(
        hit_name_set(&defs(&mut session, BETA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(hit_name_set(&defs(&mut session, ALPHA)).is_empty());
    assert_eq!(file_count(&mut session), 1);
}

#[test]
fn drill_delete_change_detect_refresh_serve() {
    let (root, _index, mut session) = setup();

    // SERVE: baseline resolves the seeded file through every read tool.
    assert_eq!(hit_count(&find(&mut session, ALPHA)), 1);
    assert_eq!(hit_count(&search(&mut session, format!("word:{ALPHA}"))), 1);
    assert_eq!(hit_count(&defs(&mut session, ALPHA)), 1);
    let gen_before = writer_generation(&mut session);

    // CHANGE: remove the file from the repo.
    fs::remove_file(root.path().join("alpha.py")).expect("delete fixture");

    // DETECT: the pinned index still serves the deleted file's token.
    assert_eq!(writer_generation(&mut session), gen_before);
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );

    // REFRESH: targeted refresh of the removed path.
    let refreshed = refresh(&mut session, &["alpha.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_removed"], 1);
    assert_ne!(writer_generation(&mut session), gen_before);

    // SERVE: every trace evicted — no read tool surfaces the token.
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    assert!(hit_name_set(&search(&mut session, format!("word:{ALPHA}"))).is_empty());
    assert!(hit_name_set(&defs(&mut session, ALPHA)).is_empty());
    assert_eq!(hit_bytes(&find(&mut session, ALPHA)), b"".as_slice());
    assert_eq!(file_count(&mut session), 0);
}

#[test]
fn drill_rename_change_detect_refresh_serve() {
    let (root, _index, mut session) = setup();

    // SERVE: baseline pins the token at the old path.
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );
    let gen_before = writer_generation(&mut session);

    // CHANGE: rename on disk.
    fs::rename(root.path().join("alpha.py"), root.path().join("beta.py"))
        .expect("rename fixture");

    // DETECT: pinned serves still point at the old path; the new path's
    // content is unreachable by path-agnostic token lookup only in the sense
    // that the served file set is stale.
    assert_eq!(writer_generation(&mut session), gen_before);
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );

    // REFRESH: both sides of the rename.
    let refreshed = refresh(&mut session, &["alpha.py", "beta.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_removed"], 1);
    assert_eq!(refreshed["stats"]["files_indexed"], 1);
    assert_ne!(writer_generation(&mut session), gen_before);

    // SERVE: the token moved exactly, with no duplication.
    assert_eq!(hit_bytes(&find(&mut session, ALPHA)), b"beta.py".as_slice());
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{ALPHA}"))),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&defs(&mut session, ALPHA)),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(file_count(&mut session), 1);
}

#[test]
fn drill_edit_tool_change_refresh_serve() {
    let (_root, _index, mut session) = setup();

    // SERVE: baseline — old token present, new token absent.
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(hit_name_set(&defs(&mut session, BETA)).is_empty());
    let gen_before = writer_generation(&mut session);

    // CHANGE + REFRESH: the session `edit` tool mutates the repo AND runs the
    // writer inline — one tool call covers both phases.
    let edited = session
        .call(
            "edit",
            json!({"path": "alpha.py", "oldText": ALPHA, "newText": BETA}),
        )
        .expect("edit");
    assert_eq!(edited["ok"], true);
    assert_eq!(edited["changed"], 1);
    assert_ne!(writer_generation(&mut session), gen_before);

    // SERVE: exact swap through every read tool.
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"alpha.py".as_slice());
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{BETA}"))),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(hit_name_set(&search(&mut session, format!("word:{ALPHA}"))).is_empty());
    assert_eq!(
        hit_name_set(&defs(&mut session, BETA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(hit_name_set(&defs(&mut session, ALPHA)).is_empty());
    assert_eq!(file_count(&mut session), 1);
}

#[test]
fn drill_force_rebuild_change_detect_refresh_serve() {
    let (root, _index, mut session) = setup();

    // SERVE: baseline resolves only the seeded file.
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    let gen_before = writer_generation(&mut session);

    // CHANGE: several unrefreshed mutations at once (modify + add).
    write_py(root.path(), "alpha.py", GAMMA);
    write_py(root.path(), "beta.py", BETA);

    // DETECT: every serve is stale — old token served, new tokens invisible.
    assert_eq!(writer_generation(&mut session), gen_before);
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    assert!(hit_name_set(&find(&mut session, GAMMA)).is_empty());

    // REFRESH: full force rebuild.
    let rebuilt = session
        .call("index_repo", json!({"force": true}))
        .expect("force rebuild");
    assert_eq!(rebuilt["ok"], true);
    assert_eq!(rebuilt["force"], true);
    assert_ne!(writer_generation(&mut session), gen_before);

    // SERVE: exact final state through every read tool.
    assert_eq!(hit_bytes(&find(&mut session, GAMMA)), b"alpha.py".as_slice());
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{BETA}"))),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&defs(&mut session, GAMMA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&defs(&mut session, BETA)),
        BTreeSet::from(["beta.py".to_string()])
    );
    assert_eq!(file_count(&mut session), 2);
}

#[test]
fn drill_render_cache_freshness_across_refresh() {
    let (root, _index, mut session) = setup();
    let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});

    // SERVE: warm the render cache; it answers.
    let warm = session.call("search", args.clone()).expect("warm search");
    assert_eq!(
        hit_name_set(&warm),
        BTreeSet::from(["alpha.py".to_string()])
    );
    assert!(matches!(session.peek_cached_search(&args), Some(_)));

    // CHANGE: rewrite the file so the cached query's answer must empty out.
    write_py(root.path(), "alpha.py", BETA);

    // DETECT: the cache still answers (stale hit) since no writer ran.
    assert!(matches!(session.peek_cached_search(&args), Some(_)));

    // REFRESH: targeted refresh invalidates the stale render.
    refresh(&mut session, &["alpha.py"]);
    assert!(matches!(session.peek_cached_search(&args), None));

    // SERVE: the next search proves freshly computed emptiness (never the
    // cached hit) and repopulates the cache under the new epoch.
    let fresh = session.call("search", args.clone()).expect("post-drill search");
    assert!(hit_name_set(&fresh).is_empty());
    assert_eq!(hit_bytes(&fresh), b"".as_slice());
    assert!(matches!(session.peek_cached_search(&args), Some(_)));

    // The new token's own search serves exactly and caches too.
    let beta_args = json!({"query": format!("word:{BETA}"), "limit": 8});
    let beta = session
        .call("search", beta_args.clone())
        .expect("beta search");
    assert_eq!(
        hit_bytes(&beta),
        b"alpha.py".as_slice()
    );
    assert!(matches!(session.peek_cached_search(&beta_args), Some(_)));
}

#[test]
fn drill_chained_multi_change_add_modify_delete_rename() {
    let (root, _index, mut session) = setup();

    // LINK 1 — ADD: baseline serves only alpha; add beta; refresh; serve both.
    assert_eq!(
        hit_name_set(&find(&mut session, ALPHA)),
        BTreeSet::from(["alpha.py".to_string()])
    );
    let gen0 = writer_generation(&mut session);
    write_py(root.path(), "beta.py", BETA);
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    let link1 = refresh(&mut session, &["beta.py"]);
    assert_eq!(link1["ok"], true);
    assert_ne!(writer_generation(&mut session), gen0);
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );
    assert_eq!(file_count(&mut session), 2);

    // LINK 2 — MODIFY: rewrite alpha.py; stale serves old; refresh; serve swap.
    let gen1 = writer_generation(&mut session);
    write_py(root.path(), "alpha.py", GAMMA);
    assert_eq!(
        hit_bytes(&find(&mut session, ALPHA)),
        b"alpha.py".as_slice()
    );
    let link2 = refresh(&mut session, &["alpha.py"]);
    assert_eq!(link2["ok"], true);
    assert_ne!(writer_generation(&mut session), gen1);
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"alpha.py".as_slice()
    );
    assert!(hit_name_set(&find(&mut session, ALPHA)).is_empty());
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    assert_eq!(file_count(&mut session), 2);

    // LINK 3 — DELETE: remove beta.py; stale serves deleted; refresh; evicted.
    let gen2 = writer_generation(&mut session);
    fs::remove_file(root.path().join("beta.py")).expect("delete beta");
    assert_eq!(hit_bytes(&find(&mut session, BETA)), b"beta.py".as_slice());
    let link3 = refresh(&mut session, &["beta.py"]);
    assert_eq!(link3["stats"]["files_removed"], 1);
    assert_ne!(writer_generation(&mut session), gen2);
    assert!(hit_name_set(&find(&mut session, BETA)).is_empty());
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"alpha.py".as_slice()
    );
    assert_eq!(file_count(&mut session), 1);

    // LINK 4 — RENAME: move alpha.py to delta.py; stale serves old path;
    // refresh; token moved exactly.
    let gen3 = writer_generation(&mut session);
    fs::rename(
        root.path().join("alpha.py"),
        root.path().join("delta.py"),
    )
    .expect("rename fixture");
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"alpha.py".as_slice()
    );
    let link4 = refresh(&mut session, &["alpha.py", "delta.py"]);
    assert_eq!(link4["ok"], true);
    assert_eq!(link4["stats"]["files_removed"], 1);
    assert_eq!(link4["stats"]["files_indexed"], 1);
    assert_ne!(writer_generation(&mut session), gen3);
    assert_eq!(
        hit_bytes(&find(&mut session, GAMMA)),
        b"delta.py".as_slice()
    );
    assert_eq!(
        hit_name_set(&find(&mut session, GAMMA)),
        BTreeSet::from(["delta.py".to_string()])
    );

    // FINAL SERVE: exact end state — only delta.py/GAMMA resolves, via every
    // read tool; all other tokens are gone.
    assert_eq!(
        hit_name_set(&search(&mut session, format!("word:{GAMMA}"))),
        BTreeSet::from(["delta.py".to_string()])
    );
    assert_eq!(
        hit_name_set(&defs(&mut session, GAMMA)),
        BTreeSet::from(["delta.py".to_string()])
    );
    for token in [ALPHA, BETA, DELTA] {
        assert!(hit_name_set(&find(&mut session, token)).is_empty());
        assert!(hit_name_set(&defs(&mut session, token)).is_empty());
    }
    assert_eq!(file_count(&mut session), 1);
}
