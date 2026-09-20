//! H-CONF-024 depth budget (br-g7j): the extraction walk is bounded and the
//! bound is LOUD. A file whose walk hits [`ast_sgrep_lang::
//! MAX_EXTRACTION_DEPTH`] is flagged in the index, the flag surfaces in the
//! index stats envelope, and the cached pattern lane refuses to serve flagged
//! files as authoritative — the native walk answers instead (a bare cap alone
//! fails open: the cached lane trusts index completeness, so silently
//! dropped deep rows would become silent missing hits).
use ast_sgrep_core::{IndexOptions, SearchOptions, Searcher};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_src(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, body).unwrap();
}

fn nested_mods(inner: &str, levels: usize) -> String {
    let mut out = String::new();
    for i in 0..levels {
        out.push_str(&format!("mod level_{i} {{\n"));
    }
    out.push_str(inner);
    out.push('\n');
    for _ in 0..levels {
        out.push_str("}\n");
    }
    out
}

/// deep.rs (600-deep paren chain) and nested300.rs (marker past the budget)
/// must BOTH be flagged; shallow.rs (marker + 20-deep marker) must NOT.
fn indexed_depth_corpus() -> (TempDir, ast_sgrep_core::IndexStats, Searcher) {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/deep.rs",
        &format!(
            "static DEEP: i32 = {}1{};\n",
            "(".repeat(600),
            ")".repeat(600)
        ),
    );
    write_src(
        temp.path(),
        "src/nested300.rs",
        &nested_mods("fn deep_marker() -> u32 { 1 }", 300),
    );
    write_src(
        temp.path(),
        "src/shallow.rs",
        &format!(
            "fn shallow_marker() -> u32 {{ 1 }}\n\n{}",
            nested_mods("fn mid_marker() -> u32 { 2 }", 20)
        ),
    );
    let stats = ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 12,
        ..SearchOptions::default()
    })
    .expect("searcher");
    (temp, stats, searcher)
}

fn first_hit_file(hits: &[ast_sgrep_core::SearchHit], needle: &str) -> Option<String> {
    hits.iter()
        .map(|hit| hit.file.replace('\\', "/"))
        .find(|file| file.contains(needle))
}

#[test]
fn depth_budget_flags_exactly_the_files_beyond_the_bound() {
    let (_temp, stats, _searcher) = indexed_depth_corpus();
    assert_eq!(
        stats.files_depth_truncated, 2,
        "deep.rs (600-deep parens) and nested300.rs (marker past the budget) must both be flagged; shallow.rs must not. Absent or wrong flagging = the budget is silent (H-CONF-024 fail-open) or mis-scoped"
    );
}

#[test]
fn truncated_files_answers_come_from_the_native_walk_not_the_index() {
    let (_temp, _stats, searcher) = indexed_depth_corpus();
    // deep_marker's signature rows were skipped by the budget; the cached
    // exact-ident lane would answer ok:true-empty (a silent miss). The
    // truncation refusal must route the query to the native walk, which
    // finds the definition.
    let response = searcher.search("pattern:deep_marker").expect("search");
    let file = first_hit_file(&response.hits, "nested300.rs");
    assert_eq!(
        file.as_deref(),
        Some("src/nested300.rs"),
        "pattern:deep_marker must be answered by the native walk over the truncated file; empty = the cached lane served its incomplete rows as authoritative (H-CONF-024 fail-open)\n{response:#?}"
    );
}

#[test]
fn files_within_the_budget_keep_their_index_lane() {
    let (_temp, _stats, searcher) = indexed_depth_corpus();
    // shallow.rs was not truncated, so its markers must still resolve.
    for (pattern, needle) in [
        ("pattern:shallow_marker", "shallow.rs"),
        ("pattern:mid_marker", "shallow.rs"),
    ] {
        let response = searcher.search(pattern).expect("search");
        let file = first_hit_file(&response.hits, needle);
        assert_eq!(
            file.as_deref(),
            Some("src/shallow.rs"),
            "{pattern} must resolve; the budget must not drop symbols within the bound\n{response:#?}"
        );
    }
}

/// B7-F2 (Sept 14 wave): with a depth-truncated file in scope, the cached
/// lane must not serve (its rows miss beyond-budget occurrences), so the
/// native walk answers every shape it can decide. A COMPLETE template is
/// walk-decidable: the deep occurrence itself must resolve.
#[test]
fn complete_templates_resolve_via_the_walk_when_truncation_is_in_scope() {
    let (_temp, _stats, searcher) = indexed_depth_corpus();
    let response = searcher
        .search("pattern:fn deep_marker() -> u32 { 1 }")
        .expect("complete decl template must be answered, not refused");
    let file = first_hit_file(&response.hits, "nested300.rs");
    assert_eq!(
        file.as_deref(),
        Some("src/nested300.rs"),
        "the deep fn template must resolve through the native walk with truncation in scope; empty = the walk did not decide the shape (B7-F2)\n{response:#?}"
    );
}

/// SEP15-3 (R-SEPT14E-2 closure): decl signatures are budget-exempt at
/// extraction — a node beyond MAX_EXTRACTION_DEPTH records its own decl row
/// (prefix+name from direct children), only the recursion stops. A decl-exact
/// template's rows are therefore COMPLETE even with a truncated file in
/// scope, and the cached lane must serve the full hit set.
#[test]
fn python_partial_def_template_never_answers_silent_empty_under_truncation() {
    let temp = TempDir::new().unwrap();
    write_src(
        temp.path(),
        "src/shallow.py",
        "def shallow_top():\n    return 1\n\nshallow_top()\n",
    );
    let mut deep = String::new();
    for i in 0..300 {
        deep.push_str(&format!("{}def level_{i}():\n", "    ".repeat(i)));
    }
    deep.push_str(&format!("    {}def shallow_top():\n", "    ".repeat(300)));
    deep.push_str(&format!("    {}    return 2\n", "    ".repeat(300)));
    write_src(temp.path(), "src/deep.py", &deep);
    ast_sgrep_core::Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("indexer")
    .index_all()
    .expect("index");
    let searcher = Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        limit: 12,
        ..SearchOptions::default()
    })
    .expect("searcher");
    let response = searcher
        .search("pattern:def shallow_top():")
        .expect("decl-exact template must be served from its budget-exempt decl rows; a refusal here means the extraction exemption (SEP15-3) is missing");
    let shallow = first_hit_file(&response.hits, "shallow.py");
    assert_eq!(
        shallow.as_deref(),
        Some("src/shallow.py"),
        "the shallow occurrence must resolve\n{response:#?}"
    );
    let deep = first_hit_file(&response.hits, "deep.py");
    assert_eq!(
        deep.as_deref(),
        Some("src/deep.py"),
        "the beyond-budget nested def must be served from its budget-exempt decl row; missing = decl rows still skipped past the budget (R-SEPT14E-2 unfixed)\n{response:#?}"
    );
}
