use ast_sgrep_core::pattern::search_pattern;
use ast_sgrep_core::store::UpsertFileInput;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer};
use ast_sgrep_lang::PatternNode;
use std::fs;
use std::time::Instant;

const PATTERNS: &[(&str, &str)] = &[
    ("fn gitignore_matched", "gitignore_matched"),
    ("fn parse_low", "parse_low"),
    ("struct WalkBuilder", "WalkBuilder"),
    ("fn search_slice", "search_slice"),
    ("struct RegexMatcherBuilder", "RegexMatcherBuilder"),
    ("struct StandardBuilder", "StandardBuilder"),
    ("struct JSONBuilder", "JSONBuilder"),
    ("struct GlobBuilder", "GlobBuilder"),
    ("DecompressionMatcherBuilder", "DecompressionMatcherBuilder"),
    ("struct TypesBuilder", "TypesBuilder"),
    ("fn run", "run"),
    ("struct OverrideBuilder", "OverrideBuilder"),
    ("fn open_mmap", "open_mmap"),
    ("fn multi_line_with_matcher", "multi_line_with_matcher"),
    ("def full_dispatch_request", "full_dispatch_request"),
    ("class Blueprint", "Blueprint"),
    (
        "class SecureCookieSessionInterface",
        "SecureCookieSessionInterface",
    ),
    ("class DispatchingJinjaLoader", "DispatchingJinjaLoader"),
    ("class FlaskGroup", "FlaskGroup"),
    ("def from_pyfile", "from_pyfile"),
    ("class AppContext", "AppContext"),
    ("class DefaultJSONProvider", "DefaultJSONProvider"),
    ("request_started", "request_started"),
    ("class MethodView", "MethodView"),
    ("def get_flashed_messages", "get_flashed_messages"),
    ("class Request", "Request"),
    ("class App", "App"),
    ("def setupmethod", "setupmethod"),
    ("class TaggedJSONSerializer", "TaggedJSONSerializer"),
];

#[test]
fn all_29_bakeoff_patterns_resolve_from_the_native_index() {
    assert_eq!(PATTERNS.len(), 29);
    let temp = tempfile::tempdir().unwrap();
    let rust = r#"
fn gitignore_matched() {}
fn parse_low() {}
struct WalkBuilder;
fn search_slice() {}
struct RegexMatcherBuilder;
struct StandardBuilder;
struct JSONBuilder;
struct GlobBuilder;
struct DecompressionMatcherBuilder;
struct TypesBuilder;
fn run() {}
struct OverrideBuilder;
fn open_mmap() {}
fn multi_line_with_matcher() {}
"#;
    let python = r#"
def full_dispatch_request(): pass
class Blueprint: pass
class SecureCookieSessionInterface: pass
class DispatchingJinjaLoader: pass
class FlaskGroup: pass
def from_pyfile(): pass
class AppContext: pass
class DefaultJSONProvider: pass
request_started = object()
class MethodView: pass
def get_flashed_messages(): pass
class Request: pass
class App: pass
def setupmethod(): pass
class TaggedJSONSerializer: pass
"#;
    fs::write(temp.path().join("suite.rs"), rust).unwrap();
    fs::write(temp.path().join("suite.py"), python).unwrap();
    let indexer = Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    let mut indexer = indexer;
    indexer.index_all().unwrap();

    for (pattern, expected) in PATTERNS {
        let hits = search_pattern(pattern, indexer.store(), temp.path(), None).unwrap();
        assert!(
            hits.iter().any(|hit| hit.excerpt.contains(expected)),
            "native suite missed {pattern}: {hits:?}"
        );
    }
    let app_hits = search_pattern("class App", indexer.store(), temp.path(), None).unwrap();
    assert!(app_hits
        .iter()
        .any(|hit| hit.excerpt.contains("class App:")));
    assert!(app_hits
        .iter()
        .all(|hit| !hit.excerpt.contains("class AppContext")));
}

#[test]
#[ignore = "23k-file release latency probe"]
fn indexed_pattern_p50_is_below_50ms_at_23k_files() {
    let temp = tempfile::tempdir().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    store.begin_bulk_tx().unwrap();
    for index in 0..23_000 {
        let path = format!("file_{index:05}.rs");
        let content = format!("struct Noise{index};");
        let lines = [(1, content.clone())];
        let nodes = [PatternNode {
            signature: format!("struct Noise{index}"),
            line_start: 1,
            line_end: 1,
            excerpt: content,
        }];
        store
            .upsert_file(UpsertFileInput {
                rel_path: &path,
                language: Some("rust"),
                mtime_secs: 1,
                mtime_nanos: 0,
                content_hash: &format!("hash-{index}"),
                lines: &lines,
                eol: "\n",
                symbols: &[],
                callers: &[],
                imports: &[],
                pattern_nodes: &nodes,
                semantic_chunks: &[],
                embed_semantic: false,
                embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
            })
            .unwrap();
    }
    let target_lines = [(1, "struct RegexMatcherBuilder;".to_string())];
    let target_nodes = [PatternNode {
        signature: "struct RegexMatcherBuilder".into(),
        line_start: 1,
        line_end: 1,
        excerpt: "struct RegexMatcherBuilder;".into(),
    }];
    store
        .upsert_file(UpsertFileInput {
            rel_path: "target.rs",
            language: Some("rust"),
            mtime_secs: 1,
            mtime_nanos: 0,
            content_hash: "target",
            lines: &target_lines,
            eol: "\n",
            symbols: &[],
            callers: &[],
            imports: &[],
            pattern_nodes: &target_nodes,
            semantic_chunks: &[],
            embed_semantic: false,
            embed_backend: ast_sgrep_embed::EmbedPreference::Semantic,
        })
        .unwrap();
    store.commit_bulk_tx().unwrap();

    let mut samples = (0..101)
        .map(|_| {
            let started = Instant::now();
            let hits = search_pattern(
                "struct RegexMatcherBuilder",
                &store,
                temp.path(),
                Some("rust"),
            )
            .unwrap();
            assert_eq!(hits.len(), 1);
            started.elapsed().as_nanos()
        })
        .collect::<Vec<_>>();
    samples.sort_unstable();
    let p50_ns = samples[samples.len() / 2];
    eprintln!("pattern_23k_p50_ns={p50_ns}");
    assert!(p50_ns < 50_000_000, "p50 {}ms", p50_ns as f64 / 1e6);
}
