//! Index open, dry-run, and status helpers.

use crate::cli_args::{usage_error, Cli};
use crate::machine::print_machine_json;
use anyhow::Context;
use ast_sgrep_core::{
    index_db_path, EmbedBackend, IndexOptions, IndexStats, Indexer, SearchOptions,
};
use std::path::{Path, PathBuf};

pub(crate) fn effective_root(cli: &Cli, fallback: &Path) -> PathBuf {
    cli.root.clone().unwrap_or_else(|| fallback.to_path_buf())
}

pub(crate) fn resolve_root_index(cli: &Cli, root: &Path) -> (PathBuf, Option<PathBuf>) {
    (effective_root(cli, root), cli.index_path.clone())
}

pub(crate) fn ensure_unambiguous_root(root: &std::path::Path, cli: &Cli) -> anyhow::Result<()> {
    if cli.root.is_some() && root != Path::new(".") {
        return Err(usage_error(
            "ROOT is ambiguous: use either --root ROOT or a positional ROOT, not both",
        ));
    }
    Ok(())
}

pub(crate) fn ensure_existing_root(root: &Path, cli: &Cli) -> anyhow::Result<PathBuf> {
    ensure_unambiguous_root(root, cli)?;
    let root = effective_root(cli, root);
    if !root.is_dir() {
        anyhow::bail!(
            "project root does not exist or is not a directory: {}",
            root.display()
        );
    }
    Ok(root)
}

fn index_db_display(root: &Path, index_path: Option<&Path>) -> PathBuf {
    index_db_path(root, index_path)
}

pub(crate) fn ensure_nonempty_index(root: &Path, file_count: usize) -> anyhow::Result<()> {
    if file_count == 0 {
        anyhow::bail!(
            "index is empty for {}; run: asgrep index {} --json",
            root.display(),
            root.display()
        );
    }
    Ok(())
}

pub(crate) fn open_indexer(root: &Path, cli: &Cli) -> anyhow::Result<Indexer> {
    ensure_existing_root(root, cli)?;
    let opts = index_options(root, cli);
    let db = index_db_display(&opts.root, opts.index_path.as_deref());
    Indexer::new(opts).with_context(|| {
        format!(
            "failed to open index at {} (root {})",
            db.display(),
            root.display()
        )
    })
}

pub(crate) fn index_options(root: &Path, cli: &Cli) -> IndexOptions {
    let (root, index_path) = resolve_root_index(cli, root);
    let t = cli.active_tuning();
    IndexOptions {
        root,
        index_path,
        lang_filter: cli.lang.clone(),
        respect_gitignore: true,
        use_tantivy: t.tantivy,
        embed_semantic: !t.no_embed,
        embed_backend: EmbedBackend::from_flags(
            t.cloud_embed,
            t.ollama_embed,
            t.neural_embed,
            t.semantic_only,
        ),
        force_reindex: false,
        ann_threshold: t.ann_threshold,
    }
}

pub(crate) fn with_index<T: serde::Serialize>(
    command: &str,
    root: &Path,
    cli: &Cli,
    op: impl FnOnce(&mut Indexer) -> anyhow::Result<T>,
    human: impl FnOnce(&T),
) -> anyhow::Result<()> {
    let mut indexer = open_indexer(root, cli)?;
    let v = op(&mut indexer)?;
    print_json_or(cli.json, command, &v, || human(&v))
}

pub(crate) fn run_index_dry_run(command: &str, root: &Path, cli: &Cli) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let mut files = 0usize;
    let mut skipped = 0usize;
    // Intentional product set for dry-run "source-like" counts — broader than
    // INDEXABLE_EXTENSIONS (which also indexes md/json/toml/yml). Do not silently
    // unify without affirming dry-run semantics in machine_contracts / agent docs.
    fn walk(dir: &Path, files: &mut usize, skipped: &mut usize) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if matches!(name, ".git" | "node_modules" | "target" | ".asgrep") {
                    continue;
                }
                walk(&path, files, skipped);
            } else if ft.is_file() {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if matches!(
                    ext,
                    "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "kt" | "kts"
                        | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "rb" | "php"
                ) {
                    *files += 1;
                } else {
                    *skipped += 1;
                }
            }
        }
    }
    walk(&root, &mut files, &mut skipped);
    if !cli.json {
        eprintln!(
            "asgrep: dry-run scanned {files} candidate files under {}",
            root.display()
        );
    }
    let payload = serde_json::json!({
        "dry_run": true,
        "root": root,
        "files_would_index": files,
        "files_skipped": skipped,
        "mutates_index": false,
        "cancel_semantics": "SIGINT during a real index leaves the previous index if build-then-swap succeeds; dry-run never writes"
    });
    if cli.json {
        print_machine_json(command, payload)
    } else {
        println!(
            "dry-run {command}: would consider {files} files ({skipped} skipped) under {}",
            root.display()
        );
        Ok(())
    }
}

pub(crate) fn print_index_stats(stats: &IndexStats) {
    println!(
        "Indexed {} files ({} skipped, {} removed)\nExtracted {} symbols, {} callers, {} imports",
        stats.files_indexed,
        stats.files_skipped,
        stats.files_removed,
        stats.symbols_extracted,
        stats.callers_extracted,
        stats.imports_extracted
    );
    if stats.walk_errors {
        eprintln!("Warning: directory walk errors left the index unpruned; stale paths may remain until a clean reindex");
    }
}

pub(crate) fn print_status(s: &ast_sgrep_core::IndexStatus) {
    println!(
        "Root: {}\nIndex: {}\nFiles: {}\nLines: {}\nSymbols: {}\nCallers: {}\nImports: {}\nSemantic chunks: {}",
        s.root, s.index_path, s.file_count, s.line_count, s.symbol_count, s.caller_count,
        s.import_count, s.semantic_chunk_count
    );
    if let Some(ref b) = s.embed_backend {
        println!("Embed backend: {b}");
    }
    if let Some(d) = s.embed_dim {
        println!("Embed dim: {d}");
    }
    let ivf = if s.semantic_ivf_present {
        "present"
    } else {
        "not built (below ANN threshold or not indexed)"
    };
    println!("Semantic IVF sidecar: {ivf}");
}

fn print_json_or<T: serde::Serialize>(
    json: bool,
    command: &str,
    value: &T,
    human: impl FnOnce(),
) -> anyhow::Result<()> {
    if json {
        print_machine_json(command, value)?;
    } else {
        human();
    }
    Ok(())
}

pub(crate) fn print_status_command(cli: &Cli, root: &Path) -> anyhow::Result<()> {
    let st = open_indexer(root, cli)?
        .store()
        .status()
        .context("failed to read status")?;
    print_json_or(cli.json, "status", &st, || print_status(&st))
}

pub(crate) fn open_searcher(root: &Path, cli: &Cli) -> anyhow::Result<ast_sgrep_core::Searcher> {
    let root = ensure_existing_root(root, cli)?;
    let opts = search_options(&root, cli);
    let db = index_db_display(&opts.root, opts.index_path.as_deref());
    let searcher = ast_sgrep_core::Searcher::new(opts).with_context(|| {
        format!(
            "failed to open index at {} (root {})",
            db.display(),
            root.display()
        )
    })?;
    ensure_nonempty_index(&root, searcher.store().status()?.file_count)?;
    Ok(searcher)
}

pub(crate) fn search_options(root: &Path, cli: &Cli) -> SearchOptions {
    let (root, index_path) = resolve_root_index(cli, root);
    let t = cli.active_tuning();
    SearchOptions {
        root,
        index_path,
        limit: cli.limit.unwrap_or_else(SearchOptions::default_limit),
        lang_filter: cli.lang.clone(),
        use_embed: !t.no_embed,
        use_tantivy: t.tantivy,
        use_cloud_embed: t.cloud_embed,
        use_ollama_embed: t.ollama_embed,
        use_neural_embed: t.neural_embed,
        use_semantic_only: t.semantic_only,
        ann_threshold: t.ann_threshold,
        ann_probes: t.ann_probes,
        use_rerank: t.rerank,
        rerank_top_k: t.rerank_top_k,
        ..SearchOptions::default()
    }
}
