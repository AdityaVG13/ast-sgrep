//! CodeMode invalidation fixtures (feature `codemode`).
//!
//! # Contract
//!
//! - One canonical copy of the file-local helpers quadruplicated across the
//!   `tests/codemode/invalidation_*` suites (freshness/delta/parity/drills).
//! - Shape: token-addressable single-`def` python files over an explicit
//!   out-of-root db (an in-root db would pollute the file_count census),
//!   sessions pinning limit 8 + default format.
//!   [`crate::indexed_codemode_repo`] fixes a two-file rust shape and
//!   [`crate::indexed_repo_files`] stores the db in-root with limit 5 +
//!   `AgentCapsule`, so neither covers this surface.
//! - Builders panic (never `Result`) on IO/index failure, matching suite
//!   convention: a broken fixture is a test failure, not a fallible op.
//! - Read wrappers pin limit 8 explicitly; projections compare by basename or
//!   suffix so sessions rooted at distinct tempdirs stay comparable.

use ast_sgrep_codemode::{CodeModeSession, SessionConfig};
use ast_sgrep_core::{IndexOptions, Indexer};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// INTENT: the seed token [`seeded_py_repo`] defines in `alpha.py`. Suites
/// keep their own `ALPHA` query const; the two must match, and any drift
/// fails loudly (the seed lookup returns empty).
pub const SEEDED_ALPHA_TOKEN: &str = "snorkel_alpha_unique";

/// INTENT: limit-8 session over an explicit out-of-root db: the indexed
/// counterpart of [`crate::session_at_indexed`] for suites whose census must
/// exclude the database file. Pins limit 8 + default format + `use_embed:
/// false`. Pure constructor.
pub fn session_at_limit8(root: &Path, index_db: &Path) -> CodeModeSession {
    CodeModeSession::new(SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_db.to_path_buf()),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    })
}

/// INTENT: the writable starting point every codemode invalidation test builds
/// from — a root seeded with `alpha.py`, an explicit out-of-root db, and an
/// indexed limit-8 session. Returns `(root, index_dir, session)`; the caller
/// keeps both [`TempDir`]s alive. Panics on IO or index failure.
pub fn seeded_py_repo() -> (TempDir, TempDir, CodeModeSession) {
    let root = TempDir::new().expect("root");
    let index_dir = TempDir::new().expect("index dir");
    write_py(root.path(), "alpha.py", SEEDED_ALPHA_TOKEN);
    let mut session = session_at_limit8(root.path(), &index_dir.path().join("index.db"));
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], true);
    (root, index_dir, session)
}

/// INTENT: single-`def` python fixture (`def {token}(): return 1`) keyed by a
/// unique token so every delta is token-addressable. Panics on IO failure.
pub fn write_py(root: &Path, name: &str, token: &str) {
    fs::write(root.join(name), format!("def {token}():\n    return 1\n")).expect("write fixture");
}

/// INTENT: one-call targeted refresh (`index_repo` with `paths`). No testkit
/// writer shorthand existed; every suite re-declared this call.
pub fn targeted_refresh(session: &mut CodeModeSession, paths: &[&str]) -> Value {
    session
        .call("index_repo", json!({"paths": paths}))
        .expect("targeted refresh")
}

/// INTENT: `find` read wrapper pinning limit 8. No testkit read shorthand
/// exists.
pub fn find_limit8(session: &mut CodeModeSession, query: &str) -> Value {
    session
        .call("find", json!({"query": query, "limit": 8}))
        .expect("find")
}

/// INTENT: `search` read wrapper pinning limit 8. No testkit read shorthand
/// exists.
pub fn search_limit8(session: &mut CodeModeSession, query: String) -> Value {
    session
        .call("search", json!({"query": query, "limit": 8}))
        .expect("search")
}

/// INTENT: `index_status` file_count census probe. The strictest of the suite
/// copies: `u64` with no narrowing cast (one copy cast to `usize`).
pub fn status_file_count(session: &mut CodeModeSession) -> u64 {
    session
        .call("index_status", json!({}))
        .expect("index status")["file_count"]
        .as_u64()
        .expect("file_count u64")
}

/// INTENT: `index_status` writer-epoch probe (the status path, complementing
/// the direct on-disk probe). No testkit epoch reader fits this shape.
pub fn status_writer_generation(session: &mut CodeModeSession) -> u64 {
    session
        .call("index_status", json!({}))
        .expect("index status")["writer_generation"]
        .as_u64()
        .expect("writer_generation u64")
}

/// INTENT: raw hit-file projection. [`crate::hit_keys`] requires
/// line_start/kind fields these suites never assert, so the invalidation
/// projections below layer on this file-only base. Pure projection.
pub fn hit_files(value: &Value) -> Vec<String> {
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

/// INTENT: exact file-set assertions (spillover / duplication kills). Layers
/// on [`hit_files`] — one copy inlined the projection, the other delegated;
/// the behavior is identical and the delegation is the single source.
pub fn hit_file_set(value: &Value) -> BTreeSet<String> {
    hit_files(value).into_iter().collect()
}

/// INTENT: suffix hit predicate for cross-tempdir assertions. Layers on
/// [`hit_files`]; behavior-identical to the set-based copy (dedup cannot move
/// an `any` predicate). Pure projection.
pub fn hits_file(value: &Value, name: &str) -> bool {
    hit_files(value).iter().any(|file| file.ends_with(name))
}

/// INTENT: basename hit set comparable across sessions rooted at distinct
/// tempdirs. Pure projection.
pub fn hit_name_set(value: &Value) -> BTreeSet<String> {
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

/// INTENT: byte-stable hit-set encoding for exact serve/parity assertions.
/// Pure projection.
pub fn hit_bytes(value: &Value) -> Vec<u8> {
    hit_name_set(value)
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

/// INTENT: hit-count projection for exact serve / count-parity legs. Pure
/// projection.
pub fn hit_count(value: &Value) -> usize {
    value["hits"].as_array().map(Vec::len).unwrap_or(0)
}

/// INTENT: out-of-band writer simulation — a second [`Indexer`] on the
/// canonicalized root plus update + flush, as a watcher/CLI peer would. No
/// testkit peer-writer helper existed.
pub fn external_reindex(root: &Path, index_db: &Path, rel: &str) {
    let canon = root.canonicalize().expect("canonical root");
    let mut indexer = Indexer::new(IndexOptions {
        root: canon.clone(),
        index_path: Some(index_db.to_path_buf()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("external indexer");
    indexer
        .update_paths(&[canon.join(rel)])
        .expect("external update");
    indexer.flush_deferred_rebuilds().expect("external flush");
}

/// INTENT: out-of-root db path convention paired with [`seeded_py_repo`].
/// Named for the index dir (not the root): [`crate::index_db_path`] already
/// resolves a db path from a corpus root, which is the other shape.
pub fn index_dir_db_path(index_dir: &TempDir) -> PathBuf {
    index_dir.path().join("index.db")
}
