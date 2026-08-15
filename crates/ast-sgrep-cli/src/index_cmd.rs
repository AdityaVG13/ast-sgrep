//! Index open, dry-run, and status helpers.

use crate::cli_args::{usage_error, Cli};
use crate::machine::print_machine_json;
use anyhow::Context;
use ast_sgrep_core::scip::{load_scip_index, ScipLoad, SCIP_CHANNEL};
use ast_sgrep_core::search::DegradedChannel;
use ast_sgrep_core::skip::should_skip_dir;
use ast_sgrep_core::{
    canonicalize_affected_path, index_db_path, EmbedBackend, IndexOptions, IndexStats, Indexer,
    SearchOptions, MAX_INCREMENTAL_PATHS,
};
use std::collections::HashSet;
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
        embed_backend: EmbedBackend::from_flags(t.neural_embed, t.semantic_only),
        force_reindex: false,
        ann_threshold: t.ann_threshold,
        // 0obi: explicit flag wins; otherwise the safe default.
        durability: cli.durability.unwrap_or_default(),
    }
}

pub(crate) fn with_index<T: serde::Serialize>(
    command: &str,
    root: &Path,
    cli: &Cli,
    force_reindex: bool,
    op: impl FnOnce(&mut Indexer) -> anyhow::Result<T>,
    human: impl FnOnce(&T),
) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let mut options = index_options(&root, cli);
    options.force_reindex = force_reindex;
    let db = index_db_display(&options.root, options.index_path.as_deref());
    let mut indexer = Indexer::new(options).with_context(|| {
        format!(
            "failed to open index at {} (root {})",
            db.display(),
            root.display()
        )
    })?;
    let v = op(&mut indexer)?;
    print_json_or(cli.json, command, &v, || human(&v))
}

#[derive(Debug, Clone, serde::Serialize)]
struct IndexCommandResult {
    #[serde(flatten)]
    stats: IndexStats,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    degraded_channels: Vec<DegradedChannel>,
}

pub(crate) fn ingest_scip(
    indexer: &Indexer,
    scip: Option<&Path>,
) -> anyhow::Result<Vec<DegradedChannel>> {
    let Some(path) = scip else {
        return Ok(Vec::new());
    };
    match load_scip_index(path) {
        ScipLoad::Loaded(index) => {
            indexer.store().apply_scip(&index)?;
            Ok(Vec::new())
        }
        ScipLoad::Degraded { reason } => Ok(vec![DegradedChannel {
            channel: SCIP_CHANNEL.to_owned(),
            reason,
        }]),
    }
}

fn index_command_result(
    indexer: &Indexer,
    stats: IndexStats,
    scip: Option<&Path>,
) -> anyhow::Result<IndexCommandResult> {
    Ok(IndexCommandResult {
        stats,
        degraded_channels: ingest_scip(indexer, scip)?,
    })
}

pub(crate) fn run_full_index(
    command: &str,
    root: &Path,
    cli: &Cli,
    force_reindex: bool,
    scip: Option<&Path>,
) -> anyhow::Result<()> {
    with_index(
        command,
        root,
        cli,
        force_reindex,
        |indexer| {
            if !cli.json {
                let verb = if force_reindex {
                    "reindexing"
                } else {
                    "indexing"
                };
                eprintln!("asgrep: {verb} {} ...", root.display());
            }
            let stats = if force_reindex {
                indexer.reindex_all().context("reindex failed")?
            } else {
                indexer.index_all().context("indexing failed")?
            };
            index_command_result(indexer, stats, scip)
        },
        print_index_result,
    )
}

pub(crate) fn run_targeted_index(
    root_arg: &Path,
    cli: &Cli,
    raw_paths: &[PathBuf],
    scip: Option<&Path>,
) -> anyhow::Result<()> {
    if raw_paths.is_empty() || raw_paths.len() > MAX_INCREMENTAL_PATHS {
        return Err(usage_error(format!(
            "--path must be supplied 1..={MAX_INCREMENTAL_PATHS} times"
        )));
    }
    let root = ensure_existing_root(root_arg, cli)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve project root: {}", root.display()))?;
    let mut seen = HashSet::with_capacity(raw_paths.len());
    let mut paths = Vec::with_capacity(raw_paths.len());
    for raw in raw_paths {
        if raw.as_os_str().is_empty()
            || raw
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return Err(usage_error(format!(
                "invalid --path (empty or parent traversal): {}",
                raw.display()
            )));
        }
        let candidate = if raw.is_absolute() {
            raw.clone()
        } else {
            root.join(raw)
        };
        let canonical = canonicalize_affected_path(&candidate)
            .with_context(|| format!("failed to resolve --path: {}", candidate.display()))?;
        if !canonical.starts_with(&root) {
            return Err(usage_error(format!(
                "--path resolves outside project root: {}",
                candidate.display()
            )));
        }
        if canonical.is_dir() {
            return Err(usage_error(format!(
                "--path accepts files, not directories: {}",
                candidate.display()
            )));
        }
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
    }

    with_index(
        "index",
        root_arg,
        cli,
        false,
        |indexer| {
            let stats = indexer.update_paths(&paths)?;
            indexer.flush_deferred_rebuilds()?;
            let degraded = ingest_scip(indexer, scip)?;
            let mut value = serde_json::json!({
                "targeted": true,
                "path_count": paths.len(),
                "stats": stats,
            });
            if !degraded.is_empty() {
                value["degraded_channels"] = serde_json::to_value(degraded)?;
            }
            Ok(value)
        },
        |value| {
            let stats = &value["stats"];
            println!(
                "Updated {} paths ({} indexed, {} skipped, {} removed, {} failed)",
                value["path_count"],
                stats["files_indexed"],
                stats["files_skipped"],
                stats["files_removed"],
                stats["files_failed"],
            );
        },
    )
}

pub(crate) fn run_index_dry_run(command: &str, root: &Path, cli: &Cli) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let mut files = 0usize;
    let mut skipped = 0usize;
    let mut walk_errors = false;
    // Intentional product set for dry-run "source-like" counts — broader than
    // INDEXABLE_EXTENSIONS (which also indexes md/json/toml/yml). Do not silently
    // unify without affirming dry-run semantics in machine_contracts / agent docs.
    fn walk(dir: &Path, files: &mut usize, skipped: &mut usize, walk_errors: &mut bool) {
        let rd = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => {
                *walk_errors = true;
                return;
            }
        };
        for entry in rd {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    *walk_errors = true;
                    continue;
                }
            };
            let path = entry.path();
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => {
                    *walk_errors = true;
                    continue;
                }
            };
            if ft.is_dir() {
                if should_skip_dir(&path) {
                    continue;
                }
                walk(&path, files, skipped, walk_errors);
            } else if ft.is_file() {
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if matches!(
                    ext,
                    "rs" | "py"
                        | "js"
                        | "ts"
                        | "tsx"
                        | "jsx"
                        | "go"
                        | "java"
                        | "kt"
                        | "kts"
                        | "c"
                        | "h"
                        | "cc"
                        | "cpp"
                        | "hpp"
                        | "cs"
                        | "rb"
                        | "php"
                ) {
                    *files += 1;
                } else {
                    *skipped += 1;
                }
            }
        }
    }
    walk(&root, &mut files, &mut skipped, &mut walk_errors);
    if !cli.json {
        eprintln!(
            "asgrep: dry-run scanned {files} candidate files under {}",
            root.display()
        );
        if walk_errors {
            // Parity with print_index_stats; machine JSON carries walk_errors (d2a1.11).
            eprintln!(
                "Warning: directory walk errors left the dry-run count incomplete; permission or IO failures may hide files"
            );
        }
    }
    let payload = serde_json::json!({
        "dry_run": true,
        "root": root,
        "files_would_index": files,
        "files_skipped": skipped,
        "walk_errors": walk_errors,
        "mutates_index": false,
        "cancel_semantics": "index writes are transactional; an interrupted uncommitted write is rolled back during SQLite recovery; dry-run never writes"
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

fn print_index_result(result: &IndexCommandResult) {
    print_index_stats(&result.stats);
    for channel in &result.degraded_channels {
        eprintln!("Warning: {} degraded: {}", channel.channel, channel.reason);
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
    println!("Durability: {}", s.durability);
    println!("Writer generation: {}", s.writer_generation);
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
    let mut opts = SearchOptions {
        root,
        index_path,
        // Remap 0 / oversize here so CLI envelope `limit` matches Searcher (docs: 0 → default).
        limit: ast_sgrep_core::clamp_output_limit(cli.limit, SearchOptions::default_limit()),
        lang_filter: cli.lang.clone(),
        use_embed: !t.no_embed,
        use_tantivy: t.tantivy,
        ann_threshold: t.ann_threshold,
        ann_probes: t.ann_probes,
        use_rerank: t.rerank,
        rerank_top_k: t.rerank_top_k.clamp(1, ast_sgrep_core::MAX_OUTPUT_RESULTS),
        ..SearchOptions::default()
    };
    // Exclusive collapse: Neural > Semantic > Auto.
    opts.set_embed_backend(EmbedBackend::from_flags(t.neural_embed, t.semantic_only));
    opts
}

#[cfg(test)]
#[path = "../../../tests/unit/cli/index_cmd.rs"]
mod tests;
