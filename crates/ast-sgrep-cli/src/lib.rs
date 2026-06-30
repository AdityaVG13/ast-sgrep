use std::path::PathBuf;
use std::time::Instant;

use anyhow::Context;
use ast_sgrep_core::{
    format_hit_line, IndexOptions, IndexStats, Indexer, SearchOptions, Searcher,
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
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                print_index_stats(&stats);
            }
        }
        Some(Commands::Status { ref root }) => {
            let opts = index_options(root, &cli);
            let indexer = Indexer::new(opts).context("failed to open index")?;
            let status = indexer.store().status().context("failed to read status")?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                print_status(&status);
            }
        }
        Some(Commands::Reindex { ref root }) => {
            let opts = index_options(root, &cli);
            let mut indexer = Indexer::new(opts).context("failed to open index")?;
            let stats = indexer.reindex_all().context("reindex failed")?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            } else {
                print_index_stats(&stats);
            }
        }
        Some(Commands::Bench {
            ref root,
            ref query,
            iterations,
        }) => {
            run_bench(root, &cli, query, iterations)?;
        }
        Some(Commands::Semantic { ref query, ref root }) => {
            run_semantic_search(root, &cli, query)?;
        }
        None => {
            let search_opts = search_options(
                &cli.root.clone().unwrap_or_else(|| cli.search_root.clone()),
                &cli,
            );
            let query = cli
                .query
                .context("search query required (e.g. asgrep \"auth refresh\")")?;
            let root = cli.root.clone().unwrap_or(cli.search_root);
            let searcher = Searcher::new(SearchOptions {
                root,
                ..search_opts
            })
            .context("failed to open index")?;
            let response = searcher.search(&query).context("search failed")?;
            if cli.json {
                let format = cli
                    .format
                    .as_deref()
                    .and_then(ast_sgrep_plugins::OutputFormat::parse)
                    .unwrap_or(ast_sgrep_plugins::OutputFormat::Native);
                let value = ast_sgrep_plugins::format_response(&response, format);
                println!("{}", serde_json::to_string_pretty(&value)?);
            } else {
                for hit in &response.hits {
                    println!("{}", format_hit_line(hit));
                }
            }
        }
    }

    Ok(())
}

fn run_semantic_search(
    root: &std::path::Path,
    cli: &Cli,
    query: &str,
) -> anyhow::Result<()> {
    let search_opts = search_options(root, cli);
    let searcher = Searcher::new(SearchOptions {
        root: cli.root.clone().unwrap_or_else(|| root.to_path_buf()),
        ..search_opts
    })
    .context("failed to open index")?;
    let response = searcher
        .search_semantic(query)
        .context("semantic search failed")?;
    if cli.json {
        let format = cli
            .format
            .as_deref()
            .and_then(ast_sgrep_plugins::OutputFormat::parse)
            .unwrap_or(ast_sgrep_plugins::OutputFormat::Agent);
        let value = ast_sgrep_plugins::format_response(&response, format);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for hit in &response.hits {
            println!("{}", format_hit_line(hit));
        }
    }
    Ok(())
}

fn index_options(root: &std::path::Path, cli: &Cli) -> IndexOptions {
    IndexOptions {
        root: cli.root.clone().unwrap_or_else(|| root.to_path_buf()),
        index_path: cli.index_path.clone(),
        lang_filter: cli.lang.clone(),
        respect_gitignore: true,
        use_tantivy: cli.tantivy,
        embed_semantic: !cli.no_embed,
        embed_backend: embed_backend_from_cli(cli),
        force_reindex: false,
        ann_threshold: cli.ann_threshold,
    }
}

fn embed_backend_from_cli(cli: &Cli) -> ast_sgrep_core::EmbedBackend {
    if cli.cloud_embed {
        ast_sgrep_core::EmbedBackend::Cloud
    } else if cli.ollama_embed {
        ast_sgrep_core::EmbedBackend::Ollama
    } else if cli.semantic_only {
        ast_sgrep_core::EmbedBackend::Semantic
    } else {
        ast_sgrep_core::EmbedBackend::Auto
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
            })
        );
    } else {
        println!("Benchmark (v1.0 targets: search <20ms, 0% false callers)");
        println!("Indexed {} files in {index_ms:.2}ms", stats.files_indexed);
        println!("Query: {query}");
        println!("Avg search: {avg_search_ms:.2}ms over {iterations} iterations ({hits} hits)");
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
