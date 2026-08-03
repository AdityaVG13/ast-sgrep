mod agent;
mod bench;
mod eval;
mod machine;
mod search_cmd;
pub mod supervisor;
mod watch;
use anyhow::Context;
use ast_sgrep_core::{
    EmbedBackend, IndexOptions, IndexStats, Indexer, SearchOptions, Searcher,
};
use clap::{Args, Parser, Subcommand};
use machine::{
    print_machine_failure, raw_command_name, raw_machine_output_requested, MACHINE_SCHEMA_VERSION,
};
use std::fmt;
use std::path::{Path, PathBuf};

pub(crate) use machine::print_machine_json;
#[derive(Args)]
struct RootArg {
    #[arg(default_value = ".")]
    root: PathBuf,
}
#[derive(Args)]
struct QueryRootArg {
    query: String,
    #[arg(default_value = ".")]
    root: PathBuf,
}
#[derive(Parser)]
#[command(
    name = "asgrep",
    version,
    about = "Polyglot hybrid code search",
    after_help = "Agent: asgrep capabilities --json | robot-docs guide | doctor --robot-triage\nExit: 0=ok 1=usage 2=fail"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    #[arg(value_name = "QUERY")]
    query: Option<String>,
    #[arg(long, global = true)]
    root: Option<PathBuf>,
    #[arg(long, global = true, env = "ASGREP_LIMIT", value_parser = parse_output_limit)]
    limit: Option<usize>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, env = "ASGREP_INDEX_PATH")]
    index_path: Option<PathBuf>,
    #[arg(long, global = true)]
    lang: Option<String>,
    #[arg(long, global = true, env = "ASGREP_NO_EMBED")]
    no_embed: bool,
    #[arg(long, global = true, env = "ASGREP_CLOUD_EMBED")]
    cloud_embed: bool,
    #[arg(long, global = true, env = "ASGREP_OLLAMA_EMBED")]
    ollama_embed: bool,
    /// Use local neural embeddings (fastembed/ONNX; needs `neural-embed` feature)
    #[arg(long, global = true, env = "ASGREP_NEURAL_EMBED")]
    neural_embed: bool,
    #[arg(long, global = true, env = "ASGREP_SEMANTIC_ONLY")]
    semantic_only: bool,
    #[arg(long, global = true, env = "ASGREP_TANTIVY")]
    tantivy: bool,
    #[arg(long, global = true, env = "ASGREP_ANN_THRESHOLD")]
    ann_threshold: Option<usize>,
    /// IVF clusters to probe (0 = adaptive √k; ≥ n_clusters = exact)
    #[arg(long, global = true, env = "ASGREP_ANN_PROBES")]
    ann_probes: Option<usize>,
    /// Rerank fused top candidates with local ONNX cross-encoder (`rerank` feature)
    #[arg(long, global = true, env = "ASGREP_RERANK", action = clap::ArgAction::Set, default_value_t = false, num_args = 0..=1, default_missing_value = "true", value_parser = clap::builder::BoolishValueParser::new())]
    rerank: bool,
    #[arg(long, global = true, env = "ASGREP_RERANK_TOP_K", default_value_t = 20)]
    rerank_top_k: usize,
    #[arg(long, global = true, value_name = "FORMAT")]
    format: Option<String>,
    #[arg(long, global = true, default_value = "0", value_name = "N", value_parser = parse_excerpt_lines)]
    excerpt_lines: usize,
    #[arg(value_name = "ROOT", default_value = ".")]
    search_root: PathBuf,
}
#[derive(Subcommand)]
enum Commands {
    Index(RootArg),
    Status(RootArg),
    Reindex(RootArg),
    Bench {
        #[command(flatten)]
        root: RootArg,
        #[arg(long, default_value = "process_request")]
        query: String,
        #[arg(long, default_value = "100")]
        iterations: u32,
        #[arg(long)]
        suite: Option<String>,
        #[arg(long, default_value = "sample")]
        fixture: String,
        #[arg(long)]
        queries_file: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        skip_index: bool,
    },
    Watch {
        #[command(flatten)]
        root: RootArg,
        #[arg(long, default_value = "300")]
        debounce_ms: u64,
    },
    Semantic(QueryRootArg),
    Chain(QueryRootArg),
    Capabilities(agent::CapabilitiesArgs),
    Version(VersionArgs),
    RobotDocs(agent::RobotDocsArgs),
    Doctor {
        #[command(flatten)]
        root: RootArg,
        #[command(flatten)]
        args: agent::DoctorArgs,
    },
    Eval(eval::EvalArgs),
}
#[derive(Parser)]
struct VersionArgs {
    #[arg(long)]
    json: bool,
}
const MAX_OUTPUT_RESULTS: usize = 1_000;
const MAX_EXCERPT_LINES: usize = 100;
pub(crate) const MAX_QUERIES_FILE_LINES: usize = 1000;
#[derive(Debug)]
pub(crate) struct UsageError(String);
impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for UsageError {}
pub(crate) fn usage_error(message: impl Into<String>) -> anyhow::Error {
    UsageError(message.into()).into()
}
fn parse_bounded_usize(raw: &str, maximum: usize, name: &str) -> Result<usize, String> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a non-negative integer"))?;
    if value > maximum {
        return Err(format!("{name} must not exceed {maximum}"));
    }
    Ok(value)
}
fn parse_output_limit(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_OUTPUT_RESULTS, "--limit")
}
fn parse_excerpt_lines(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_EXCERPT_LINES, "--excerpt-lines")
}
pub fn main() -> anyhow::Result<()> {
    #[cfg(not(unix))]
    {
        run_process()
    }
    #[cfg(unix)]
    {
        if supervisor::is_worker() {
            if supervisor::worker_authenticate() {
                supervisor::worker_start();
                run_process()
            } else {
                supervisor::supervise()
            }
        } else {
            supervisor::supervise()
        }
    }
}
fn run_process() -> ! {
    let raw_args: Vec<_> = std::env::args_os().collect();
    let cli = match Cli::try_parse_from(&raw_args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = if error.use_stderr() { 1 } else { 0 };
            if exit_code == 1 && raw_machine_output_requested(&raw_args) {
                print_machine_failure(
                    raw_command_name(&raw_args),
                    "usage",
                    exit_code,
                    &error.to_string(),
                );
            }
            let _ = error.print();
            std::process::exit(exit_code);
        }
    };
    match run_cli(&cli) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            let usage = error.downcast_ref::<UsageError>().is_some();
            let exit_code = if usage { 1 } else { 2 };
            if cli.machine_output_requested() {
                print_machine_failure(
                    cli.command_name(),
                    if usage { "usage" } else { "operational" },
                    exit_code,
                    &format!("{error:#}"),
                );
            }
            eprintln!("{error:#}");
            if !cli.machine_output_requested() {
                agent::print_agent_help_footer();
            }
            std::process::exit(exit_code);
        }
    }
}
impl Cli {
    fn machine_output_requested(&self) -> bool {
        self.json
            || matches!(self.command.as_ref(), Some(Commands::Capabilities(a)) if a.json)
            || matches!(self.command.as_ref(), Some(Commands::Version(a)) if a.json)
            || matches!(self.command.as_ref(), Some(Commands::Doctor { args, .. }) if args.json || args.robot_triage)
    }
    fn command_name(&self) -> &'static str {
        match self.command.as_ref() {
            None => "search",
            Some(Commands::Index(_)) => "index",
            Some(Commands::Status(_)) => "status",
            Some(Commands::Reindex(_)) => "reindex",
            Some(Commands::Bench { .. }) => "bench",
            Some(Commands::Watch { .. }) => "watch",
            Some(Commands::Semantic(_)) => "semantic",
            Some(Commands::Chain(_)) => "chain",
            Some(Commands::Capabilities(_)) => "capabilities",
            Some(Commands::Version(_)) => "version",
            Some(Commands::RobotDocs(_)) => "robot-docs",
            Some(Commands::Doctor { .. }) => "doctor",
            Some(Commands::Eval(_)) => "eval",
        }
    }
}
fn run_cli(cli: &Cli) -> anyhow::Result<()> {
    match cli.command.as_ref() {
        Some(c) => run_command(cli, c),
        None => run_default_search(cli),
    }
}
fn run_command(cli: &Cli, command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Index(r) => with_index(
            "index",
            &r.root,
            cli,
            |i| i.index_all().context("indexing failed"),
            print_index_stats,
        ),
        Commands::Status(r) => {
            let st = open_indexer(&r.root, cli)?
                .store()
                .status()
                .context("failed to read status")?;
            print_json_or(cli.json, "status", &st, || print_status(&st))
        }
        Commands::Reindex(r) => with_index(
            "reindex",
            &r.root,
            cli,
            |i| i.reindex_all().context("reindex failed"),
            print_index_stats,
        ),
        Commands::Bench {
            root,
            query,
            iterations,
            suite,
            fixture,
            queries_file,
            skip_index,
        } => bench::run_bench_command(
            &root.root,
            cli,
            query,
            *iterations,
            suite.as_deref(),
            fixture,
            queries_file.as_deref(),
            *skip_index,
        ),
        Commands::Watch { root, debounce_ms } => watch::run_watch(&root.root, cli, *debounce_ms),
        Commands::Semantic(q) => search_cmd::run_search(&q.root, cli, &q.query, true),
        Commands::Chain(q) => search_cmd::run_chain(&q.root, cli, &q.query),
        Commands::Capabilities(args) => agent::run_capabilities(cli, args),
        Commands::Version(args) => run_version(cli, args),
        Commands::RobotDocs(args) => agent::run_robot_docs(cli, args),
        Commands::Doctor { root, args } => agent::run_doctor(cli, &root.root, args),
        Commands::Eval(args) => eval::run_eval(cli, args),
    }
}
fn run_version(cli: &Cli, args: &VersionArgs) -> anyhow::Result<()> {
    if cli.json || args.json {
        print_machine_json(
            "version",
            serde_json::json!({"version": env!("CARGO_PKG_VERSION"), "machine_schema_version": MACHINE_SCHEMA_VERSION}),
        )
    } else {
        println!("asgrep {}", env!("CARGO_PKG_VERSION"));
        Ok(())
    }
}
fn with_index<T: serde::Serialize>(
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
fn run_default_search(cli: &Cli) -> anyhow::Result<()> {
    let query = cli.query.as_deref().ok_or_else(|| usage_error("search query required (e.g. asgrep \"auth refresh\") or use a subcommand: asgrep capabilities --json"))?;
    if let Some(sub) = agent::query_looks_like_subcommand_typo(query) {
        return Err(usage_error(format!("unknown subcommand '{query}'; did you mean: asgrep {sub} ... ? Try: asgrep capabilities --json")));
    }
    search_cmd::run_search(&cli.search_root, cli, query, false)
}
pub(crate) fn effective_root(cli: &Cli, fallback: &Path) -> PathBuf {
    cli.root.clone().unwrap_or_else(|| fallback.to_path_buf())
}
pub(crate) fn resolve_root_index(cli: &Cli, root: &Path) -> (PathBuf, Option<PathBuf>) {
    (effective_root(cli, root), cli.index_path.clone())
}
pub(crate) fn open_indexer(root: &Path, cli: &Cli) -> anyhow::Result<Indexer> {
    Indexer::new(index_options(root, cli)).context("failed to open index")
}
pub(crate) fn open_searcher(root: &Path, cli: &Cli) -> anyhow::Result<Searcher> {
    Searcher::new(search_options(root, cli)).context("failed to open index")
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
pub(crate) fn index_options(root: &Path, cli: &Cli) -> IndexOptions {
    let (root, index_path) = resolve_root_index(cli, root);
    IndexOptions {
        root,
        index_path,
        lang_filter: cli.lang.clone(),
        respect_gitignore: true,
        use_tantivy: cli.tantivy,
        embed_semantic: !cli.no_embed,
        embed_backend: EmbedBackend::from_flags(
            cli.cloud_embed,
            cli.ollama_embed,
            cli.neural_embed,
            cli.semantic_only,
        ),
        force_reindex: false,
        ann_threshold: cli.ann_threshold,
    }
}
pub(crate) fn search_options(root: &Path, cli: &Cli) -> SearchOptions {
    let (root, index_path) = resolve_root_index(cli, root);
    SearchOptions {
        root,
        index_path,
        limit: cli.limit.unwrap_or_else(SearchOptions::default_limit),
        lang_filter: cli.lang.clone(),
        use_embed: !cli.no_embed,
        use_tantivy: cli.tantivy,
        use_cloud_embed: cli.cloud_embed,
        use_ollama_embed: cli.ollama_embed,
        use_neural_embed: cli.neural_embed,
        use_semantic_only: cli.semantic_only,
        ann_threshold: cli.ann_threshold,
        ann_probes: cli.ann_probes,
        use_rerank: cli.rerank,
        rerank_top_k: cli.rerank_top_k,
        ..SearchOptions::default()
    }
}
#[allow(clippy::too_many_arguments)]
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
fn print_status(s: &ast_sgrep_core::IndexStatus) {
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
