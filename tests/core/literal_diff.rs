//! Bounded `literal:` file-presence differential vs pinned ripgrep.
//!
//! This gate compares only the checked-in, indexed 13-language fixture. It
//! does not claim full ripgrep identity over unindexed or arbitrary files.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_lang::Language;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const NEEDLE: &str = "return";
const PINNED_RG_VERSION: &str = "15.1.0";

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/lang/fixtures/extract")
        .canonicalize()
        .expect("13-language extraction fixture")
}

fn competitor_bin() -> Option<PathBuf> {
    let raw = std::env::var_os("ASGREP_DIFF_RG")?;
    let path = PathBuf::from(raw);
    assert!(
        path.is_absolute(),
        "ASGREP_DIFF_RG must be absolute: {}",
        path.display()
    );
    Some(path)
}

fn assert_pinned_competitor(bin: &Path) {
    let output = Command::new(bin)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| panic!("run rg --version: {error}"));
    assert!(
        output.status.success(),
        "rg --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(1)
        .map(str::to_owned);
    assert_eq!(
        version.as_deref(),
        Some(PINNED_RG_VERSION),
        "literal keep-gate requires pinned ripgrep {PINNED_RG_VERSION}"
    );
}

fn rg_file_set(bin: &Path, root: &Path) -> BTreeSet<String> {
    let output = Command::new(bin)
        .args([
            "--no-config",
            "--files-with-matches",
            "--fixed-strings",
            "--color=never",
            NEEDLE,
        ])
        .arg(root)
        .output()
        .unwrap_or_else(|error| panic!("run rg literal differential: {error}"));
    // grep convention: exit 0 = matches, exit 1 = valid zero-match result.
    let no_matches = output.status.code() == Some(1);
    assert!(
        output.status.success() || no_matches,
        "rg failed: {}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(Path::new)
        .map(|path| {
            path.strip_prefix(root)
                .unwrap_or_else(|_| panic!("rg returned path outside fixture: {}", path.display()))
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn literal_file_set_matches_pinned_rg_when_configured() {
    let Some(bin) = competitor_bin() else {
        eprintln!(
            "not-run: set ASGREP_DIFF_RG to pinned ripgrep {PINNED_RG_VERSION}; not claiming file-set equality (DISC-lexical-not-rg)"
        );
        return;
    };
    assert!(
        bin.is_file(),
        "ASGREP_DIFF_RG must be a file: {}",
        bin.display()
    );
    assert_pinned_competitor(&bin);

    let root = fixture_root();
    let temp = tempfile::tempdir().expect("temporary index directory");
    let index_path = temp.path().join("literal-diff.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .expect("open literal differential indexer");
    let stats = indexer.index_all().expect("index language fixture");
    assert_eq!(
        stats.files_indexed,
        Language::all().len(),
        "fixture must exercise every indexed AST language"
    );

    let indexed_files: BTreeSet<_> = indexer
        .store()
        .all_file_paths()
        .expect("read indexed fixture paths")
        .into_iter()
        .collect();
    assert_eq!(indexed_files.len(), Language::all().len());

    let searcher = Searcher::new(SearchOptions {
        root: root.clone(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 256,
        ..SearchOptions::default()
    })
    .expect("open literal differential searcher");
    let asgrep_files: BTreeSet<_> = searcher
        .search(&format!("literal:{NEEDLE}"))
        .expect("run literal differential search")
        .hits
        .into_iter()
        .map(|hit| hit.file)
        .collect();
    let rg_files = rg_file_set(&bin, &root);

    assert!(
        !rg_files.is_empty(),
        "fixture must not produce empty equality"
    );
    assert!(
        rg_files.is_subset(&indexed_files),
        "rg fixture matches must all be indexed-language files: {rg_files:?}"
    );
    assert_eq!(
        asgrep_files, rg_files,
        "literal file-presence mismatch on the 13-language fixture"
    );
}
