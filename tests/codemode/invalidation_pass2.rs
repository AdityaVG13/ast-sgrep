//! I2 delta-discriminating oracles for `ast-sgrep-codemode` session freshness.
//!
//! Where I1 pins the freshness *contract* (stale-until-stamp, cache drops on
//! writer change), I2 proves each *repo delta class* through the session's own
//! refresh path (`index_repo` with `paths`) and read tools (`find`, `search`,
//! `defs`, `index_status`):
//!
//! * ADD: a new file + targeted refresh surfaces exactly its tokens/symbols.
//! * MODIFY: a rewritten file + targeted refresh swaps old tokens for new.
//! * DELETE: a removed file + targeted refresh evicts every trace of it.
//! * RENAME: old path + new path refresh moves hits without duplication.
//! * CATALOG: `index_status` file counts track each delta.
//! * NO-STALE: no pre-refresh render or hit survives its delta's refresh.
//!
//! Discriminants are asserted with `matches!` / counts / typed JSON accessors
//! only; no error-message text is matched.

use ast_sgrep_codemode::CodeModeSession;
use ast_sgrep_codemode::SessionConfig;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

const ALPHA: &str = "snorkel_alpha_unique";
const BETA: &str = "snorkel_beta_unique";
const NEWDEF: &str = "snorkel_newdef_unique";
const OLDDEF: &str = "snorkel_olddef_unique";

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

/// Fresh root + index DB, one file (`alpha.py`, `ALPHA`) indexed.
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

fn hit_file_set(value: &Value) -> BTreeSet<String> {
    value["hits"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|hit| {
            hit.get("file")
                .and_then(|file| file.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn hits_file(value: &Value, name: &str) -> bool {
    hit_file_set(value).iter().any(|file| file.ends_with(name))
}

fn refresh(session: &mut CodeModeSession, paths: &[&str]) -> Value {
    session
        .call("index_repo", json!({"paths": paths}))
        .expect("targeted refresh")
}

fn file_count(session: &mut CodeModeSession) -> usize {
    session
        .call("index_status", json!({}))
        .expect("index status")["file_count"]
        .as_u64()
        .expect("file_count u64") as usize
}

fn writer_generation(session: &mut CodeModeSession) -> u64 {
    session
        .call("index_status", json!({}))
        .expect("index status")["writer_generation"]
        .as_u64()
        .expect("writer_generation u64")
}

#[test]
fn delta_add_find_surfaces_new_file_token_only() {
    let (root, _index, mut session) = setup();
    write_py(root.path(), "beta.py", BETA);
    let refreshed = refresh(&mut session, &["beta.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_indexed"], 1);
    assert_eq!(refreshed["stats"]["files_removed"], 0);

    // ADD delta: the new token resolves to exactly the new file.
    let found = session
        .call("find", json!({"query": BETA, "limit": 8}))
        .expect("find beta");
    assert!(hits_file(&found, "beta.py"), "{found}");
    assert!(!hits_file(&found, "alpha.py"), "{found}");
    assert_eq!(hit_file_set(&found).len(), 1, "{found}");
    // The pre-existing file's token is undisturbed by the sibling add.
    let sibling = session
        .call("find", json!({"query": ALPHA, "limit": 8}))
        .expect("find alpha");
    assert!(hits_file(&sibling, "alpha.py"), "{sibling}");
}

#[test]
fn delta_add_search_word_query_surfaces_new_file() {
    let (root, _index, mut session) = setup();
    write_py(root.path(), "beta.py", BETA);
    refresh(&mut session, &["beta.py"]);

    // ADD delta through hybrid search: exact file set, no spillover.
    let found = session
        .call("search", json!({"query": format!("word:{BETA}"), "limit": 8}))
        .expect("search beta");
    assert!(hits_file(&found, "beta.py"), "{found}");
    assert_eq!(hit_file_set(&found).len(), 1, "{found}");
}

#[test]
fn delta_add_defs_lookup_finds_new_symbol() {
    let (root, _index, mut session) = setup();
    write_py(root.path(), "newdef.py", NEWDEF);
    refresh(&mut session, &["newdef.py"]);

    // ADD delta through defs: the new definition resolves to the new file,
    // and an unknown symbol resolves to nothing (no phantom hits).
    let found = session
        .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
        .expect("defs newdef");
    assert!(hits_file(&found, "newdef.py"), "{found}");
    let phantom = session
        .call("defs", json!({"symbol": "snorkel_never_defined", "limit": 8}))
        .expect("defs phantom");
    assert!(hit_file_set(&phantom).is_empty(), "{phantom}");
}

#[test]
fn delta_modify_find_swaps_token_exactly() {
    let (root, _index, mut session) = setup();
    write_py(root.path(), "alpha.py", BETA);
    let refreshed = refresh(&mut session, &["alpha.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_indexed"], 1);
    assert_eq!(refreshed["stats"]["files_removed"], 0);

    // MODIFY delta: new token present AND old token gone — exactly the swap.
    let fresh = session
        .call("find", json!({"query": BETA, "limit": 8}))
        .expect("find beta");
    assert!(hits_file(&fresh, "alpha.py"), "{fresh}");
    let gone = session
        .call("find", json!({"query": ALPHA, "limit": 8}))
        .expect("find alpha");
    assert!(hit_file_set(&gone).is_empty(), "{gone}");
}

#[test]
fn delta_modify_defs_lookup_tracks_renamed_symbol() {
    let (root, _index, mut session) = setup();
    write_py(root.path(), "defs.py", OLDDEF);
    refresh(&mut session, &["defs.py"]);
    let before = session
        .call("defs", json!({"symbol": OLDDEF, "limit": 8}))
        .expect("defs olddef");
    assert!(hits_file(&before, "defs.py"), "{before}");

    // MODIFY delta: rename the definition; lookup follows the new name only.
    write_py(root.path(), "defs.py", NEWDEF);
    refresh(&mut session, &["defs.py"]);
    let fresh = session
        .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
        .expect("defs newdef");
    assert!(hits_file(&fresh, "defs.py"), "{fresh}");
    let gone = session
        .call("defs", json!({"symbol": OLDDEF, "limit": 8}))
        .expect("defs olddef");
    assert!(hit_file_set(&gone).is_empty(), "{gone}");
}

#[test]
fn delta_delete_find_and_search_evict_token() {
    let (root, _index, mut session) = setup();
    fs::remove_file(root.path().join("alpha.py")).expect("delete fixture");
    let refreshed = refresh(&mut session, &["alpha.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_removed"], 1);
    assert_eq!(refreshed["stats"]["files_indexed"], 0);

    // DELETE delta: no read tool may surface the removed token.
    let via_find = session
        .call("find", json!({"query": ALPHA, "limit": 8}))
        .expect("find alpha");
    assert!(hit_file_set(&via_find).is_empty(), "{via_find}");
    let via_search = session
        .call("search", json!({"query": format!("word:{ALPHA}"), "limit": 8}))
        .expect("search alpha");
    assert!(hit_file_set(&via_search).is_empty(), "{via_search}");
}

#[test]
fn delta_delete_defs_lookup_empties() {
    let (root, _index, mut session) = setup();
    write_py(root.path(), "doomed.py", NEWDEF);
    refresh(&mut session, &["doomed.py"]);
    let before = session
        .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
        .expect("defs newdef");
    assert!(hits_file(&before, "doomed.py"), "{before}");

    // DELETE delta through defs: removing the defining file empties lookup.
    fs::remove_file(root.path().join("doomed.py")).expect("delete fixture");
    let refreshed = refresh(&mut session, &["doomed.py"]);
    assert_eq!(refreshed["stats"]["files_removed"], 1);
    let gone = session
        .call("defs", json!({"symbol": NEWDEF, "limit": 8}))
        .expect("defs newdef");
    assert!(hit_file_set(&gone).is_empty(), "{gone}");
}

#[test]
fn delta_rename_refresh_moves_hits_to_new_path() {
    let (root, _index, mut session) = setup();
    fs::rename(root.path().join("alpha.py"), root.path().join("beta.py"))
        .expect("rename fixture");
    let refreshed = refresh(&mut session, &["alpha.py", "beta.py"]);
    assert_eq!(refreshed["ok"], true);
    assert_eq!(refreshed["stats"]["files_removed"], 1);
    assert_eq!(refreshed["stats"]["files_indexed"], 1);

    // RENAME delta: the token moves to the new path with no duplication.
    let moved = session
        .call("find", json!({"query": ALPHA, "limit": 8}))
        .expect("find alpha");
    assert!(hits_file(&moved, "beta.py"), "{moved}");
    assert!(!hits_file(&moved, "alpha.py"), "{moved}");
    assert_eq!(hit_file_set(&moved).len(), 1, "{moved}");
    // A rename is one removal plus one add: the file census is unchanged.
    assert_eq!(file_count(&mut session), 1);
}

#[test]
fn delta_catalog_file_count_tracks_add_and_delete() {
    let (root, _index, mut session) = setup();
    assert_eq!(file_count(&mut session), 1);
    let gen_before = writer_generation(&mut session);

    // CATALOG consistency: each delta moves the census and the epoch.
    write_py(root.path(), "beta.py", BETA);
    refresh(&mut session, &["beta.py"]);
    assert_eq!(file_count(&mut session), 2);
    assert_ne!(writer_generation(&mut session), gen_before);

    fs::remove_file(root.path().join("beta.py")).expect("delete fixture");
    refresh(&mut session, &["beta.py"]);
    assert_eq!(file_count(&mut session), 1);

    // Census matches reads: only the surviving file's token resolves.
    let survivor = session
        .call("find", json!({"query": ALPHA, "limit": 8}))
        .expect("find alpha");
    assert_eq!(hit_file_set(&survivor).len(), 1, "{survivor}");
    let evicted = session
        .call("find", json!({"query": BETA, "limit": 8}))
        .expect("find beta");
    assert!(hit_file_set(&evicted).is_empty(), "{evicted}");
}

#[test]
fn delta_delete_refresh_drops_stale_render_and_repopulates_fresh() {
    let (root, _index, mut session) = setup();
    let args = json!({"query": format!("word:{ALPHA}"), "limit": 8});
    let warm = session.call("search", args.clone()).expect("warm search");
    assert!(hits_file(&warm, "alpha.py"), "{warm}");
    assert!(matches!(session.peek_cached_search(&args), Some(_)));

    // NO-STALE: the delete's refresh invalidates the warm render; the next
    // search serves freshly computed emptiness, never the cached hit.
    fs::remove_file(root.path().join("alpha.py")).expect("delete fixture");
    refresh(&mut session, &["alpha.py"]);
    assert!(matches!(session.peek_cached_search(&args), None));
    let fresh = session
        .call("search", args.clone())
        .expect("post-delete search");
    assert!(hit_file_set(&fresh).is_empty(), "{fresh}");

    // The re-added file repopulates the cache under the new epoch.
    write_py(root.path(), "alpha.py", ALPHA);
    refresh(&mut session, &["alpha.py"]);
    assert!(matches!(session.peek_cached_search(&args), None));
    let revived = session
        .call("search", args.clone())
        .expect("post-add search");
    assert!(hits_file(&revived, "alpha.py"), "{revived}");
    assert!(matches!(session.peek_cached_search(&args), Some(_)));
}
