//! Rarest-trigram df shortcut: equivalence, fail-safety, freshness (br-umh).
//!
//! Contracts:
//! C1 equivalence — over a ≥BMH-threshold index, trigram-path hit sets equal a
//!    LIKE/GLOB contains-oracle for substring needles (file granularity).
//! C2 decoy resistance — a foreign temp virtual table squatting on the
//!    df-vocab name MUST NOT be trusted; search falls back to the full-phrase
//!    scan and stays correct (guards the Empty short cut against poisoned
//!    document frequencies).
//! C3 freshness — foreign raw-SQL row deletion/addition flips results even
//!    when the df memo holds the old generation (absence is never memoized;
//!    MATCH always reads the live index).
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

const FILLER_FILES: usize = 45;
const FILLER_DEFS: usize = 28; // x2 lines each -> 2520 indexed lines >= BMH threshold

fn write_src(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

/// Index above the BMH_LINE_THRESHOLD (1000 lines) with planted markers: one
/// file holding a unique rare token, three files sharing another.
fn setup() -> (TempDir, Searcher) {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    for f in 0..FILLER_FILES {
        let mut body = String::new();
        for i in 0..FILLER_DEFS {
            body.push_str(&format!(
                "def fill_{f}_{i}(value):\n    return value * {i} + {f}\n"
            ));
        }
        if f == 0 {
            body.push_str("ALPHA_ZZQUUX_MARKER_PAYLOAD sentinel\n");
        }
        if f < 3 {
            body.push_str("beta_shared_rare_token payload\n");
        }
        write_src(root, &format!("src/mod_{f}.py"), &body);
    }
    let index_path = root.join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(index_path),
        limit: 50,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    (temp, searcher)
}

fn hit_files(searcher: &Searcher, query: &str) -> Vec<String> {
    let response = searcher.search(query).unwrap();
    let mut files: Vec<String> = response.hits.iter().map(|h| h.file.clone()).collect();
    files.sort();
    files.dedup();
    files
}

fn contains_oracle(root: &std::path::Path, needle: &str, case_insensitive: bool) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let hay = if case_insensitive {
                text.to_lowercase()
            } else {
                text
            };
            let needle_owned = if case_insensitive {
                needle.to_lowercase()
            } else {
                needle.to_string()
            };
            if hay.contains(&needle_owned) {
                files.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    files.sort();
    files
}

#[test]
fn c1_shortcut_matches_contains_oracle() {
    let (temp, _searcher) = setup();
    // Case-insensitive surface exercises the fold-identity fast path.
    let ci_searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(temp.path().join("index.db")),
        limit: 50,
        case_insensitive: true,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let cases = [
        ("literal:zzquux", "zzquux"),
        ("literal:ZZQUUX_Marker", "zzquux_marker"),
        ("literal:beta_shared_rare_token", "beta_shared_rare_token"),
        ("literal:fill_7_13", "fill_7_13"),
        ("literal:valeur_absente", "valeur_absente"),
    ];
    for (query, oracle_needle) in cases {
        let got = hit_files(&ci_searcher, query);
        let want = contains_oracle(temp.path(), oracle_needle, true);
        assert_eq!(got, want, "file-set mismatch for {query}");
    }
}

#[test]
fn c2_decoy_vocab_table_is_not_trusted() {
    let (_temp, _searcher) = setup();
    // Case-insensitive surface so the planted marker survives the Rust
    // reverify when the scan sees real rows; any residual emptiness can then
    // only come from poisoned document frequencies.
    let searcher = Searcher::new(SearchOptions {
        root: _temp.path().to_path_buf(),
        index_path: Some(_temp.path().join("index.db")),
        limit: 50,
        case_insensitive: true,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    // Adversary 1: a SCHEMA-COMPATIBLE plain temp table squatting on the
    // df-vocab name with FORGED document frequencies. It claims a trigram
    // that does not exist in the real index ("qqq") is ultra-rare (df=1)
    // while every real needle trigram looks plausibly rare (df=40): trusting
    // the forger picks the phantom -> MATCH scans nothing -> silent empty.
    searcher
        .store()
        .connection()
        .execute_batch(
            "CREATE TABLE temp.asgrep_trigram_vocab(term TEXT PRIMARY KEY, doc INTEGER, cnt INTEGER);\
             INSERT INTO temp.asgrep_trigram_vocab VALUES\
             ('qqq', 1, 1)\
             ,('zzq', 40, 40),('zqu', 40, 40),('quu', 40, 40)\
             ,('uux', 40, 40),('ux_', 40, 40),('x_m', 40, 40)\
             ,('_ma', 40, 40),('mar', 40, 40),('ark', 40, 40)\
             ,('rke', 40, 40),('ker', 40, 40);",
        )
        .unwrap();
    let got = hit_files(&searcher, "literal:zzquux_marker");
    assert_eq!(got, vec!["src/mod_0.py".to_string()]);
}

#[test]
fn c2b_post_warm_forge_must_not_answer_silence() {
    let (_temp, _searcher) = setup();
    let searcher = Searcher::new(SearchOptions {
        root: _temp.path().to_path_buf(),
        index_path: Some(_temp.path().join("index.db")),
        limit: 50,
        case_insensitive: true,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    // Warm the df memo at the current generation (vocab ensured, entries cached).
    let got = hit_files(&searcher, "literal:beta_shared_rare_token");
    assert_eq!(got.len(), 3, "precondition: marker visible before forgery");
    // Forge AFTER warm-up: same connection, same index generation, so neither
    // the gen check nor the ensure-time drop runs. Two baits: a phantom
    // ultra-rare trigram ("qqq" is not in the real index), and — the actual
    // silence vector — a REQUIRED needle trigram claimed ABSENT (df=0),
    // which turns the Empty short cut into silent empty results.
    searcher
        .store()
        .connection()
        .execute_batch(
            "DROP TABLE temp.asgrep_trigram_vocab;\
             CREATE TABLE temp.asgrep_trigram_vocab(term TEXT PRIMARY KEY, doc INTEGER, cnt INTEGER);\
             INSERT INTO temp.asgrep_trigram_vocab VALUES\
             ('qqq', 1, 1),('zzq', 0, 0)\
             ,('zqu', 40, 40),('quu', 40, 40)\
             ,('uux', 40, 40),('ux_', 40, 40),('x_m', 40, 40)\
             ,('_ma', 40, 40),('mar', 40, 40),('ark', 40, 40)\
             ,('rke', 40, 40),('ker', 40, 40)\
             ,('_sh', 40, 40),('sha', 40, 40),('har', 40, 40)\
             ,('are', 40, 40),('red', 40, 40),('ed_', 40, 40)\
             ,('d_r', 40, 40),('et_', 40, 40);\
             ",
        )
        .unwrap();
    let got = hit_files(&searcher, "literal:zzquux_marker");
    assert_eq!(
        got,
        vec!["src/mod_0.py".to_string()],
        "forged document frequencies must not change search output"
    );
}

#[test]
fn c3_foreign_mutation_flips_results_despite_warm_memo() {
    let (temp, searcher) = setup();
    // Warm the df memo at the current generation.
    assert_eq!(
        hit_files(&searcher, "literal:beta_shared_rare_token"),
        vec![
            "src/mod_0.py".to_string(),
            "src/mod_1.py".to_string(),
            "src/mod_2.py".to_string()
        ]
    );
    // Foreign raw-SQL delete: external-content trigram requires manual rowid
    // deletes; meta counters are left untouched (stale memo generation).
    {
        let conn = rusqlite::Connection::open(temp.path().join("index.db")).unwrap();
        conn.execute_batch(
            "DELETE FROM lines_trigram WHERE rowid IN \
             (SELECT rowid FROM lines WHERE file_id = (SELECT id FROM files WHERE path='src/mod_1.py')); \
             DELETE FROM lines_fts WHERE file_id = (SELECT id FROM files WHERE path='src/mod_1.py'); \
             DELETE FROM lines_code_fts WHERE file_id = (SELECT id FROM files WHERE path='src/mod_1.py'); \
             DELETE FROM lines WHERE file_id = (SELECT id FROM files WHERE path='src/mod_1.py');",
        )
        .unwrap();
    }
    let got = hit_files(&searcher, "literal:beta_shared_rare_token");
    assert_eq!(
        got,
        vec!["src/mod_0.py".to_string(), "src/mod_2.py".to_string()],
        "foreign deletion must flip results despite warm memo"
    );
    // Foreign raw-SQL addition of a new rare-token line.
    {
        let conn = rusqlite::Connection::open(temp.path().join("index.db")).unwrap();
        conn.execute_batch(
            "INSERT INTO lines(file_id, line_no, content) \
             VALUES((SELECT id FROM files WHERE path='src/mod_9.py'), 999, 'fresh_zzquux_addition');\
             INSERT INTO lines_trigram(rowid, content) \
             VALUES((SELECT rowid FROM lines WHERE file_id=(SELECT id FROM files WHERE path='src/mod_9.py') AND line_no=999), 'fresh_zzquux_addition');",
        )
        .unwrap();
    }
    let got = hit_files(&searcher, "literal:fresh_zzquux");
    assert_eq!(
        got.first().map(String::as_str),
        Some("src/mod_9.py"),
        "foreign addition must appear despite warm memo"
    );
}
