use std::path::PathBuf;

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
    #[arg(long, global = true)]
    index_path: Option<PathBuf>,

    /// Filter by language (rust, typescript, javascript, python, go)
    #[arg(long, global = true)]
    lang: Option<String>,

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
    /// Force full reindex
    Reindex {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
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
        None => {
            let query = cli
                .query
                .context("search query required (e.g. asgrep \"auth refresh\")")?;
            let root = cli.root.clone().unwrap_or(cli.search_root);
            let search_opts = SearchOptions {
                root,
                index_path: cli.index_path,
                limit: cli.limit.unwrap_or_else(SearchOptions::default_limit),
                lang_filter: cli.lang,
            };
            let searcher = Searcher::new(search_opts).context("failed to open index")?;
            let response = searcher.search(&query).context("search failed")?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                for hit in &response.hits {
                    println!("{}", format_hit_line(hit));
                }
            }
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
    }
}

fn print_index_stats(stats: &IndexStats) {
    println!("Indexed {} files ({} skipped, {} removed)", stats.files_indexed, stats.files_skipped, stats.files_removed);
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
}
