use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use ast_sgrep_core::{
    format_hit_line, EmbedBackend, IndexOptions, IndexStats, Indexer, SearchOptions, SearchResponse,
    Searcher,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "asgrep", version, about = "Polyglot hybrid code search")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Search query (when no subcommand is given)
    #[arg(value_name = "QUERY")]
    query: Option<String>,

    /// Project root directory
    #[arg(long, global = true)]
    root: Option<PathBuf>,

    /// Maximum number of results
    #[arg(long, global = true, env = "ASGREP_LIMIT")]
    limit: Option<usize>,

    /// Emit JSON output
    #[arg(long, global = true)]
    json: bool,

    /// Custom index database path
    #[arg(long, global = true, env = "ASGREP_INDEX_PATH")]
    index_path: Option<PathBuf>,

    /// Filter by language (rust, typescript, javascript, python, go)
    #[arg(long, global = true)]
    lang: Option<String>,

    /// Disable semantic embedding at index and search time
    #[arg(long, global = true, env = "ASGREP_NO_EMBED")]
    no_embed: bool,

    /// Use cloud API for neural embeddings (ASGREP_EMBED_API_KEY)
    #[arg(long, global = true, env = "ASGREP_CLOUD_EMBED")]
    cloud_embed: bool,

    /// Use Ollama for local neural embeddings (ASGREP_OLLAMA_URL)
    #[arg(long, global = true, env = "ASGREP_OLLAMA_EMBED")]
    ollama_embed: bool,

    /// Force offline code-aware semantic embeddings only (no cloud/Ollama)
    #[arg(long, global = true, env = "ASGREP_SEMANTIC_ONLY")]
    semantic_only: bool,

    /// Build/use lexical FTS sidecar for large-repo search (`--tantivy`)
    #[arg(long, global = true, env = "ASGREP_TANTIVY")]
    tantivy: bool,

    /// Semantic IVF ANN threshold in symbol chunks (default 2000, env: ASGREP_ANN_THRESHOLD)
    #[arg(long, global = true, env = "ASGREP_ANN_THRESHOLD")]
    ann_threshold: Option<usize>,

    /// JSON output format: native, github, gitlab, agent
    #[arg(long, global = true, value_name = "FORMAT")]
    format: Option<String>,

    /// Root path positional for search
    #[arg(value_name = "ROOT", default_value = ".")]
    search_root: PathBuf,
}

#[derive(Subcommand)]
enum Commands {
    /// Build or update the search index
    Index {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Show index status
    Status {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Force full reindex (bypass hash/mtime skip)
    Reindex {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Run search benchmarks
    Bench {
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, default_value = "process_request")]
        query: String,
        #[arg(long, default_value = "100")]
        iterations: u32,
        /// Run a named benchmark suite (e.g. `default`) instead of a single query
        #[arg(long)]
        suite: Option<String>,
    },
    /// Watch the repo and incrementally re-index on file changes
    Watch {
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Debounce window in milliseconds before re-indexing
        #[arg(long, default_value = "300")]
        debounce_ms: u64,
    },
    /// Semantic-only search (embed pass, synonym / NL queries)
    Semantic {
        /// Natural-language or synonym query
        query: String,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
}

/// Entry point for `asgrep` / `ast-sgrep` binaries.
pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Index { ref root }) => {
            let opts = index_options(root, &cli);
            let mut indexer = Indexer::new(opts).context("failed to open index")?;
            let stats = indexer.index_all().context("indexing failed")?;
            print_json_or(cli.json, &stats, || print_index_stats(&stats))?;
        }
        Some(Commands::Status { ref root }) => {
            let opts = index_options(root, &cli);
            let indexer = Indexer::new(opts).context("failed to open index")?;
            let status = indexer.store().status().context("failed to read status")?;
            print_json_or(cli.json, &status, || print_status(&status))?;
        }
        Some(Commands::Reindex { ref root }) => {
            let opts = index_options(root, &cli);
            let mut indexer = Indexer::new(opts).context("failed to open index")?;
            let stats = indexer.reindex_all().context("reindex failed")?;
            print_json_or(cli.json, &stats, || print_index_stats(&stats))?;
        }
        Some(Commands::Bench {
            ref root,
            ref query,
            iterations,
            ref suite,
        }) => {
            if let Some(suite_name) = suite {
                run_bench_suite(root, &cli, suite_name, iterations)?;
            } else {
                run_bench(root, &cli, query, iterations)?;
            }
        }
        Some(Commands::Watch {
            ref root,
            debounce_ms,
        }) => {
            run_watch(root, &cli, debounce_ms)?;
        }
        Some(Commands::Semantic { ref query, ref root }) => {
            run_search(root, &cli, query, true)?;
        }
        None => {
            let root = effective_root(&cli, &cli.search_root);
            let query = cli
                .query
                .as_deref()
                .context("search query required (e.g. asgrep \"auth refresh\")")?;
            run_search(&root, &cli, query, false)?;
        }
    }

    Ok(())
}

fn effective_root(cli: &Cli, fallback: &std::path::Path) -> PathBuf {
    cli.root.clone().unwrap_or_else(|| fallback.to_path_buf())
}

fn run_search(root: &std::path::Path, cli: &Cli, query: &str, semantic: bool) -> anyhow::Result<()> {
    let searcher = Searcher::new(SearchOptions {
        root: effective_root(cli, root),
        ..search_options(root, cli)
    })
    .context("failed to open index")?;
    let response = if semantic {
        searcher.search_semantic(query).context("semantic search failed")?
    } else {
        searcher.search(query).context("search failed")?
    };
    print_search_response(cli, &response, semantic)
}

fn print_search_response(
    cli: &Cli,
    response: &SearchResponse,
    semantic: bool,
) -> anyhow::Result<()> {
    if cli.json {
        let default_format = if semantic {
            ast_sgrep_plugins::OutputFormat::Agent
        } else {
            ast_sgrep_plugins::OutputFormat::Native
        };
        let format = cli
            .format
            .as_deref()
            .and_then(ast_sgrep_plugins::OutputFormat::parse)
            .unwrap_or(default_format);
        let value = ast_sgrep_plugins::format_response(response, format);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for hit in &response.hits {
            println!("{}", format_hit_line(hit));
        }
    }
    Ok(())
}

fn print_json_or<T: serde::Serialize>(
    json: bool,
    value: &T,
    human: impl FnOnce(),
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human();
    }
    Ok(())
}

fn index_options(root: &std::path::Path, cli: &Cli) -> IndexOptions {
    IndexOptions {
        root: effective_root(cli, root),
        index_path: cli.index_path.clone(),
        lang_filter: cli.lang.clone(),
        respect_gitignore: true,
        use_tantivy: cli.tantivy,
        embed_semantic: !cli.no_embed,
        embed_backend: EmbedBackend::from_flags(cli.cloud_embed, cli.ollama_embed, cli.semantic_only),
        force_reindex: false,
        ann_threshold: cli.ann_threshold,
    }
}

fn search_options(root: &std::path::Path, cli: &Cli) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        index_path: cli.index_path.clone(),
        limit: cli.limit.unwrap_or_else(SearchOptions::default_limit),
        lang_filter: cli.lang.clone(),
        use_embed: !cli.no_embed,
        use_tantivy: cli.tantivy,
        use_cloud_embed: cli.cloud_embed,
        use_ollama_embed: cli.ollama_embed,
        use_semantic_only: cli.semantic_only,
        ann_threshold: cli.ann_threshold,
    }
}

fn run_bench_suite(
    root: &std::path::Path,
    cli: &Cli,
    suite_name: &str,
    iterations: u32,
) -> anyhow::Result<()> {
    let cases = ast_sgrep_core::bench_suite::suite_by_name(suite_name).with_context(|| {
        format!(
            "unknown suite {suite_name:?}; available: {}",
            ast_sgrep_core::bench_suite::list_suite_names().join(", ")
        )
    })?;

    let opts = index_options(root, cli);
    let mut indexer = Indexer::new(opts.clone()).context("failed to open index")?;
    let index_start = Instant::now();
    let stats = indexer.index_all()?;
    let index_ms = index_start.elapsed().as_secs_f64() * 1000.0;

    let search_opts = search_options(root, cli);
    let searcher = Searcher::new(search_opts)?;

    let mut results = Vec::new();
    for case in cases {
        let mut total_search_ms = 0.0;
        let mut hits = 0usize;
        for _ in 0..iterations {
            let start = Instant::now();
            let response = searcher.search(case.query)?;
            total_search_ms += start.elapsed().as_secs_f64() * 1000.0;
            hits = response.hits.len();
        }
        let avg_search_ms = total_search_ms / f64::from(iterations);
        let ast_grep_pattern = ast_sgrep_core::pattern::ast_grep_pattern_for_query(case.query);
        let ast_grep_avg_ms = ast_grep_pattern.as_ref().and_then(|pattern| {
            ast_sgrep_core::pattern::bench_ast_grep(pattern, root, iterations)
        });
        let speedup_vs_ast_grep = ast_grep_avg_ms.map(|ag| ag / avg_search_ms);
        let ok = hits >= case.min_hits;
        results.push(serde_json::json!({
            "name": case.name,
            "query": case.query,
            "avg_search_ms": avg_search_ms,
            "hits": hits,
            "min_hits": case.min_hits,
            "ok": ok,
            "ast_grep_pattern": ast_grep_pattern,
            "avg_ast_grep_ms": ast_grep_avg_ms,
            "speedup_vs_ast_grep": speedup_vs_ast_grep,
        }));
    }

    if cli.json {
        println!(
            "{}",
            serde_json::json!({
                "suite": suite_name,
                "files_indexed": stats.files_indexed,
                "index_ms": index_ms,
                "iterations": iterations,
                "cases": results,
            })
        );
    } else {
        println!("Benchmark suite: {suite_name}");
        println!("Indexed {} files in {index_ms:.2}ms", stats.files_indexed);
        for row in &results {
            let name = row["name"].as_str().unwrap_or("?");
            let query = row["query"].as_str().unwrap_or("?");
            let avg = row["avg_search_ms"].as_f64().unwrap_or(0.0);
            let hits = row["hits"].as_u64().unwrap_or(0);
            let ok = row["ok"].as_bool().unwrap_or(false);
            let status = if ok { "ok" } else { "FAIL" };
            println!("  {name}: {avg:.2}ms avg, {hits} hits {status}");
            if let (Some(pattern), Some(ag_ms)) = (
                row["ast_grep_pattern"].as_str(),
                row["avg_ast_grep_ms"].as_f64(),
            ) {
                println!("    ast-grep ({pattern}): {ag_ms:.2}ms");
                if let Some(speedup) = row["speedup_vs_ast_grep"].as_f64() {
                    println!("    speedup vs ast-grep: {speedup:.1}x");
                }
            }
            let _ = query;
        }
    }

    if results.iter().any(|r| r["ok"] == false) {
        anyhow::bail!("benchmark suite had cases below min_hits threshold");
    }
    Ok(())
}

fn run_watch(root: &std::path::Path, cli: &Cli, debounce_ms: u64) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;

    let opts = index_options(root, cli);
    let root = opts.root.clone();
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .context("failed to create file watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .context("failed to watch project root")?;

    eprintln!("[asgrep] watching {} (debounce {debounce_ms}ms)", root.display());

    let mut indexer = Indexer::new(opts)?;
    let initial = indexer.index_all()?;
    eprintln!(
        "[asgrep] initial index: {} files indexed, {} skipped",
        initial.files_indexed, initial.files_skipped
    );

    let debounce = Duration::from_millis(debounce_ms);
    loop {
        match rx.recv_timeout(debounce) {
            Ok(Ok(event)) => match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                    while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}
                    let stats = indexer.index_all()?;
                    eprintln!(
                        "[asgrep] re-indexed: {} updated, {} skipped, {} removed",
                        stats.files_indexed, stats.files_skipped, stats.files_removed
                    );
                }
                _ => {}
            },
            Ok(Err(e)) => eprintln!("[asgrep] watch error: {e}"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn run_bench(
    root: &std::path::Path,
    cli: &Cli,
    query: &str,
    iterations: u32,
) -> anyhow::Result<()> {
    let opts = index_options(root, cli);
    let mut indexer = Indexer::new(opts.clone()).context("failed to open index")?;
    let index_start = Instant::now();
    let stats = indexer.index_all()?;
    let index_ms = index_start.elapsed().as_secs_f64() * 1000.0;

    let search_opts = search_options(root, cli);
    let searcher = Searcher::new(search_opts)?;
    let mut total_search_ms = 0.0;
    let mut hits = 0usize;
    for _ in 0..iterations {
        let start = Instant::now();
        let response = searcher.search(query)?;
        total_search_ms += start.elapsed().as_secs_f64() * 1000.0;
        hits = response.hits.len();
    }
    let avg_search_ms = total_search_ms / f64::from(iterations);
    let ast_grep_pattern = ast_sgrep_core::pattern::ast_grep_pattern_for_query(query);
    let ast_grep_avg_ms = ast_grep_pattern.as_ref().and_then(|pattern| {
        ast_sgrep_core::pattern::bench_ast_grep(pattern, root, iterations)
    });
    let speedup_vs_ast_grep = ast_grep_avg_ms.map(|ag| ag / avg_search_ms);

    if cli.json {
        println!(
            "{}",
            serde_json::json!({
                "files_indexed": stats.files_indexed,
                "index_ms": index_ms,
                "query": query,
                "iterations": iterations,
                "avg_search_ms": avg_search_ms,
                "hits": hits,
                "ast_grep_pattern": ast_grep_pattern,
                "avg_ast_grep_ms": ast_grep_avg_ms,
                "speedup_vs_ast_grep": speedup_vs_ast_grep,
            })
        );
    } else {
        println!("Benchmark (v1.0 targets: search <20ms, 0% false callers)");
        println!("Indexed {} files in {index_ms:.2}ms", stats.files_indexed);
        println!("Query: {query}");
        println!("Avg search: {avg_search_ms:.2}ms over {iterations} iterations ({hits} hits)");
        if let (Some(pattern), Some(ag_ms)) = (&ast_grep_pattern, ast_grep_avg_ms) {
            println!("Avg ast-grep (pattern: {pattern}): {ag_ms:.2}ms over {iterations} iterations");
            if let Some(speedup) = speedup_vs_ast_grep {
                println!("Speedup vs ast-grep: {speedup:.1}x");
            }
        }
    }
    Ok(())
}

fn print_index_stats(stats: &IndexStats) {
    println!(
        "Indexed {} files ({} skipped, {} removed)",
        stats.files_indexed, stats.files_skipped, stats.files_removed
    );
    println!(
        "Extracted {} symbols, {} callers, {} imports",
        stats.symbols_extracted, stats.callers_extracted, stats.imports_extracted
    );
}

fn print_status(status: &ast_sgrep_core::IndexStatus) {
    println!("Root: {}", status.root);
    println!("Index: {}", status.index_path);
    println!("Files: {}", status.file_count);
    println!("Lines: {}", status.line_count);
    println!("Symbols: {}", status.symbol_count);
    println!("Callers: {}", status.caller_count);
    println!("Imports: {}", status.import_count);
    println!("Semantic chunks: {}", status.semantic_chunk_count);
    if let Some(ref backend) = status.embed_backend {
        println!("Embed backend: {backend}");
    }
    if let Some(dim) = status.embed_dim {
        println!("Embed dim: {dim}");
    }
    println!(
        "Semantic IVF sidecar: {}",
        if status.semantic_ivf_present {
            "present"
        } else {
            "not built (below ANN threshold or not indexed)"
        }
    );
}
