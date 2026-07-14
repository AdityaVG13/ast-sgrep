mod agent;
mod eval;
pub mod supervisor;
use anyhow::Context;
use ast_sgrep_core::{
    chain::{expand_chain, ChainConfig},
    format_hit_line, index_db_path, EmbedBackend, IndexOptions, IndexStats, IndexStore, Indexer,
    SearchOptions, SearchResponse, Searcher,
};
use clap::{Parser, Subcommand};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;
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
    #[arg(
        long, global = true, env = "ASGREP_RERANK", action = clap::ArgAction::Set,
        default_value_t = false, num_args = 0..=1, default_missing_value = "true",
        value_parser = clap::builder::BoolishValueParser::new(),
    )]
    rerank: bool,
    #[arg(long, global = true, env = "ASGREP_RERANK_TOP_K", default_value_t = 20)]
    rerank_top_k: usize,
    #[arg(long, global = true, value_name = "FORMAT")]
    format: Option<String>,
    #[arg(
        long, global = true, default_value = "0", value_name = "N",
        value_parser = parse_excerpt_lines,
    )]
    excerpt_lines: usize,
    #[arg(value_name = "ROOT", default_value = ".")]
    search_root: PathBuf,
}
#[derive(Subcommand)]
enum Commands {
    Index {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Status {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Reindex {
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Bench {
        #[arg(default_value = ".")]
        root: PathBuf,
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
        #[arg(default_value = ".")]
        root: PathBuf,
        #[arg(long, default_value = "300")]
        debounce_ms: u64,
    },
    Semantic {
        query: String,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Chain {
        query: String,
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    Capabilities(agent::CapabilitiesArgs),
    Version(VersionArgs),
    RobotDocs(agent::RobotDocsArgs),
    Doctor {
        #[arg(default_value = ".")]
        root: PathBuf,
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

const MACHINE_SCHEMA_VERSION: &str = "1.0.0";
const MAX_OUTPUT_RESULTS: usize = 1_000;
const MAX_EXCERPT_LINES: usize = 100;
const MAX_ERROR_MESSAGE_CHARS: usize = 4_096;

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

pub fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run_cli(&cli)
}
impl Cli {
    fn machine_output_requested(&self) -> bool {
        self.json
            || matches!(
                self.command.as_ref(),
                Some(Commands::Capabilities(args)) if args.json
            )
            || matches!(
                self.command.as_ref(),
                Some(Commands::Version(args)) if args.json
            )
            || matches!(
                self.command.as_ref(),
                Some(Commands::Doctor { args, .. }) if args.json || args.robot_triage
            )
    }

    fn command_name(&self) -> &'static str {
        match self.command.as_ref() {
            None => "search",
            Some(Commands::Index { .. }) => "index",
            Some(Commands::Status { .. }) => "status",
            Some(Commands::Reindex { .. }) => "reindex",
            Some(Commands::Bench { .. }) => "bench",
            Some(Commands::Watch { .. }) => "watch",
            Some(Commands::Semantic { .. }) => "semantic",
            Some(Commands::Chain { .. }) => "chain",
            Some(Commands::Capabilities(_)) => "capabilities",
            Some(Commands::Version(_)) => "version",
            Some(Commands::RobotDocs(_)) => "robot-docs",
            Some(Commands::Doctor { .. }) => "doctor",
            Some(Commands::Eval(_)) => "eval",
        }
    }
}

fn raw_machine_output_requested(args: &[std::ffi::OsString]) -> bool {
    args.iter()
        .any(|arg| arg == "--json" || arg == "--robot-triage")
}

fn raw_command_name(args: &[std::ffi::OsString]) -> &'static str {
    const COMMANDS: &[&str] = &[
        "index",
        "status",
        "reindex",
        "bench",
        "watch",
        "semantic",
        "chain",
        "capabilities",
        "version",
        "robot-docs",
        "doctor",
        "eval",
    ];
    args.iter()
        .filter_map(|arg| arg.to_str())
        .find_map(|arg| COMMANDS.iter().copied().find(|command| arg == *command))
        .unwrap_or("search")
}

fn bounded_error_message(message: &str) -> String {
    let mut chars = message.chars();
    let bounded: String = chars.by_ref().take(MAX_ERROR_MESSAGE_CHARS).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn machine_value(command: &str, value: impl serde::Serialize) -> anyhow::Result<serde_json::Value> {
    let mut value = serde_json::to_value(value)?;
    let object = match &mut value {
        serde_json::Value::Object(object) => object,
        _ => {
            return Ok(serde_json::json!({
                "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep",
                "command": command, "ok": true, "data": value,
            }))
        }
    };
    if command == "status" {
        object
            .entry("embed_backend")
            .or_insert(serde_json::Value::Null);
        object.entry("embed_dim").or_insert(serde_json::Value::Null);
    }
    object.insert("schema_version".into(), MACHINE_SCHEMA_VERSION.into());
    object.insert("tool".into(), "asgrep".into());
    object.insert("command".into(), command.into());
    object.insert("ok".into(), true.into());
    Ok(value)
}

pub(crate) fn print_machine_json(
    command: &str,
    value: impl serde::Serialize,
) -> anyhow::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&machine_value(command, value)?)?
    );
    Ok(())
}

fn print_machine_failure(command: &str, kind: &str, exit_code: i32, message: &str) {
    let value = serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION,
        "tool": "asgrep",
        "command": command,
        "ok": false,
        "exit_code": exit_code,
        "error": {"kind": kind, "message": bounded_error_message(message)},
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("failure envelope serializes")
    );
}

fn run_cli(cli: &Cli) -> anyhow::Result<()> {
    match cli.command.as_ref() {
        Some(c) => run_command(cli, c),
        None => run_default_search(cli),
    }
}
fn run_command(cli: &Cli, command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Index { root } => with_index(
            "index",
            root,
            cli,
            |i| i.index_all().context("indexing failed"),
            print_index_stats,
        ),
        Commands::Status { root } => {
            let st = Indexer::new(index_options(root, cli))
                .context("failed to open index")?
                .store()
                .status()
                .context("failed to read status")?;
            print_json_or(cli.json, "status", &st, || print_status(&st))
        }
        Commands::Reindex { root } => with_index(
            "reindex",
            root,
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
        } => run_bench_command(
            root,
            cli,
            query,
            *iterations,
            suite.as_deref(),
            fixture,
            queries_file.as_deref(),
            *skip_index,
        ),
        Commands::Watch { root, debounce_ms } => run_watch(root, cli, *debounce_ms),
        Commands::Semantic { query, root } => run_search(root, cli, query, true),
        Commands::Chain { query, root } => run_chain(root, cli, query),
        Commands::Capabilities(args) => agent::run_capabilities(cli, args),
        Commands::Version(args) => run_version(cli, args),
        Commands::RobotDocs(args) => agent::run_robot_docs(cli, args),
        Commands::Doctor { root, args } => agent::run_doctor(cli, root, args),
        Commands::Eval(args) => eval::run_eval(cli, args),
    }
}
fn run_version(cli: &Cli, args: &VersionArgs) -> anyhow::Result<()> {
    if cli.json || args.json {
        print_machine_json(
            "version",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "machine_schema_version": MACHINE_SCHEMA_VERSION,
            }),
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
    let mut indexer = Indexer::new(index_options(root, cli)).context("failed to open index")?;
    let v = op(&mut indexer)?;
    print_json_or(cli.json, command, &v, || human(&v))
}
fn run_default_search(cli: &Cli) -> anyhow::Result<()> {
    let root = effective_root(cli, &cli.search_root);
    let query = cli.query.as_deref().ok_or_else(|| usage_error(
        "search query required (e.g. asgrep \"auth refresh\") or use a subcommand: asgrep capabilities --json",
    ))?;
    if let Some(sub) = agent::query_looks_like_subcommand_typo(query) {
        return Err(usage_error(format!(
            "unknown subcommand '{query}'; did you mean: asgrep {sub} ... ? Try: asgrep capabilities --json"
        )));
    }
    run_search(&root, cli, query, false)
}
pub(crate) fn effective_root(cli: &Cli, fallback: &Path) -> PathBuf {
    cli.root.clone().unwrap_or_else(|| fallback.to_path_buf())
}
fn run_chain(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let root = effective_root(cli, root);
    let store =
        IndexStore::open(&root, cli.index_path.as_deref()).context("failed to open index")?;
    let config = ChainConfig {
        limit: cli.limit.unwrap_or(ChainConfig::default().limit),
        top_n: 1,
        ..ChainConfig::default()
    };
    let r = expand_chain(&store, query, &config).context("chain search failed")?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    println!(
        "chain {:?}: {} nodes, {} edges (max depth {})",
        r.query, r.node_count, r.edge_count, r.max_depth
    );
    println!("nodes:");
    for n in &r.nodes {
        println!(
            "  depth {} score {:.4} {}:{}-{} {}",
            n.depth,
            n.score,
            n.file,
            n.line_start,
            n.line_end,
            n.symbol.as_deref().unwrap_or("<file>")
        );
    }
    println!("edges:");
    for e in &r.edges {
        println!(
            "  depth {} {:?}: {}::{} -> {}::{}",
            e.depth,
            e.label,
            e.from_file,
            e.from_symbol.as_deref().unwrap_or("<file>"),
            e.to_file,
            e.to_symbol.as_deref().unwrap_or("<file>")
        );
    }
    Ok(())
}
fn run_search(root: &Path, cli: &Cli, query: &str, semantic: bool) -> anyhow::Result<()> {
    let searcher = Searcher::new(SearchOptions {
        root: effective_root(cli, root),
        ..search_options(root, cli)
    })
    .context("failed to open index")?;
    let response = if semantic {
        searcher
            .search_semantic(query)
            .context("semantic search failed")?
    } else {
        searcher.search(query).context("search failed")?
    };
    if cli.json {
        let default = if semantic {
            ast_sgrep_plugins::OutputFormat::Agent
        } else {
            ast_sgrep_plugins::OutputFormat::Native
        };
        let format = match cli.format.as_deref() {
            Some(raw) => ast_sgrep_plugins::OutputFormat::parse(raw)
                .ok_or_else(|| usage_error(format!(
                    "unknown output format {raw:?}; expected native, agent, agent-capsule, github, or gitlab"
                )))?,
            None => default,
        };
        let value = ast_sgrep_plugins::format_response_with(&response, format, cli.excerpt_lines);
        print_machine_json(if semantic { "semantic" } else { "search" }, value)?;
    } else {
        for hit in &response.hits {
            println!("{}", format_hit_line(hit));
        }
    }
    Ok(())
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
    IndexOptions {
        root: effective_root(cli, root),
        index_path: cli.index_path.clone(),
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
fn search_options(root: &Path, cli: &Cli) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        index_path: cli.index_path.clone(),
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
fn run_bench_command(
    root: &Path,
    cli: &Cli,
    query: &str,
    iterations: u32,
    suite: Option<&str>,
    fixture: &str,
    queries_file: Option<&Path>,
    skip_index: bool,
) -> anyhow::Result<()> {
    if let Some(path) = queries_file {
        return run_bench_batch(root, cli, path, iterations, skip_index);
    }
    match suite {
        Some(name) => run_bench_suite(root, cli, name, fixture, iterations, skip_index),
        None => run_bench(root, cli, query, iterations, skip_index),
    }
}
fn maybe_index(root: &Path, cli: &Cli, skip: bool) -> anyhow::Result<(Option<IndexStats>, f64)> {
    if skip {
        return Ok((None, 0.0));
    }
    let mut indexer = Indexer::new(index_options(root, cli)).context("failed to open index")?;
    let t0 = Instant::now();
    Ok((
        Some(indexer.index_all()?),
        t0.elapsed().as_secs_f64() * 1000.0,
    ))
}
fn bench_searcher(root: &Path, cli: &Cli, skip_index: bool) -> anyhow::Result<Searcher> {
    let db = index_db_path(root, cli.index_path.as_deref());
    if skip_index && !db.exists() {
        anyhow::bail!(
            "failed to open existing index at {} (run `asgrep index` first)",
            db.display()
        );
    }
    Searcher::new(search_options(root, cli)).context("failed to open index")
}
fn do_search(s: &Searcher, q: &str, semantic: bool) -> anyhow::Result<SearchResponse> {
    if semantic {
        Ok(s.search_semantic(q)?)
    } else {
        Ok(s.search(q)?)
    }
}
fn add_index_json(obj: &mut serde_json::Value, stats: Option<&IndexStats>, index_ms: f64) {
    if let Some(s) = stats {
        obj["files_indexed"] = serde_json::json!(s.files_indexed);
        obj["index_ms"] = serde_json::json!(index_ms);
    } else {
        obj["index_skipped"] = serde_json::json!(true);
        obj["index_ms"] = serde_json::json!(0.0);
        obj["files_indexed"] = serde_json::Value::Null;
    }
}
fn print_index_skipped(stats: Option<&IndexStats>, index_ms: Option<f64>) {
    match (stats, index_ms) {
        (Some(s), Some(ms)) => println!("Indexed {} files in {ms:.2}ms", s.files_indexed),
        (Some(s), None) => println!("Indexed {} files", s.files_indexed),
        _ => println!("Index skipped (using existing index)"),
    }
}
fn run_bench_suite(
    root: &Path,
    cli: &Cli,
    suite_name: &str,
    fixture_name: &str,
    iterations: u32,
    skip_index: bool,
) -> anyhow::Result<()> {
    use ast_sgrep_core::bench_suite;
    let fix = bench_suite::fixture_by_name(fixture_name).with_context(|| {
        format!(
            "unknown fixture {fixture_name:?}; available: {}",
            bench_suite::list_fixture_names().join(", ")
        )
    })?;
    let selected = if suite_name.is_empty() {
        fix.suite
    } else {
        suite_name
    };
    let cases = bench_suite::suite_by_name(selected).with_context(|| {
        format!(
            "unknown suite {suite_name:?}; available: {}",
            bench_suite::list_suite_names().join(", ")
        )
    })?;
    let bench_root = if root.as_os_str() == "." {
        fix.root.to_path_buf()
    } else {
        root.to_path_buf()
    };
    let (stats, _) = maybe_index(&bench_root, cli, skip_index)?;
    let searcher = bench_searcher(&bench_root, cli, skip_index)?;
    let results: Vec<serde_json::Value> = cases
        .iter()
        .map(|case| {
            let mut total = 0.0;
            let mut hits = 0usize;
            for _ in 0..iterations {
                let t0 = Instant::now();
                let r = searcher.search(case.query)?;
                total += t0.elapsed().as_secs_f64() * 1000.0;
                hits = r.hits.len();
            }
            let avg = total / f64::from(iterations);
            let ag_pat = ast_sgrep_core::pattern::ast_grep_pattern_for_query(case.query);
            let ag_ms = ag_pat.as_ref().and_then(|p| {
                ast_sgrep_core::pattern::bench_ast_grep(p, &bench_root, iterations.min(3))
            });
            Ok(serde_json::json!({
                "name": case.name, "query": case.query, "avg_search_ms": avg, "hits": hits,
                "min_hits": case.min_hits, "ok": hits >= case.min_hits,
                "ast_grep_pattern": ag_pat, "avg_ast_grep_ms": ag_ms,
                "speedup_vs_ast_grep": ag_ms.map(|ag| ag / avg),
            }))
        })
        .collect::<anyhow::Result<_>>()?;

    if cli.json {
        let mut obj = serde_json::json!({
            "fixture": fixture_name, "suite": suite_name, "iterations": iterations, "cases": results,
        });
        if let Some(s) = &stats {
            obj["files_indexed"] = serde_json::json!(s.files_indexed);
        } else {
            obj["index_skipped"] = serde_json::json!(true);
            obj["index_ms"] = serde_json::json!(0.0);
            obj["files_indexed"] = serde_json::Value::Null;
        }
        println!("{obj}");
    } else {
        println!("Benchmark fixture: {fixture_name}, suite: {suite_name}");
        print_index_skipped(stats.as_ref(), None);
        for row in &results {
            let st = if row["ok"].as_bool().unwrap_or(false) {
                "ok"
            } else {
                "FAIL"
            };
            println!(
                "  {}: {:.2}ms avg, {} hits {st}",
                row["name"].as_str().unwrap_or("?"),
                row["avg_search_ms"].as_f64().unwrap_or(0.0),
                row["hits"].as_u64().unwrap_or(0)
            );
            if let (Some(p), Some(ms)) = (
                row["ast_grep_pattern"].as_str(),
                row["avg_ast_grep_ms"].as_f64(),
            ) {
                println!("    ast-grep ({p}): {ms:.2}ms");
                if let Some(sp) = row["speedup_vs_ast_grep"].as_f64() {
                    println!("    speedup vs ast-grep: {sp:.1}x");
                }
            }
        }
    }
    if results.iter().any(|r| r["ok"] == false) {
        anyhow::bail!("benchmark suite had cases below min_hits threshold");
    }
    Ok(())
}
fn run_watch(root: &Path, cli: &Cli, debounce_ms: u64) -> anyhow::Result<()> {
    use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use std::collections::HashSet;
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
    eprintln!(
        "[asgrep] watching {} (debounce {debounce_ms}ms)",
        root.display()
    );
    let mut indexer = Indexer::new(opts)?;
    let initial = indexer.index_all()?;
    eprintln!(
        "[asgrep] initial index: {} files indexed, {} skipped",
        initial.files_indexed, initial.files_skipped
    );
    let debounce = Duration::from_millis(debounce_ms);
    let mut pending = HashSet::new();
    let mut full = false;
    loop {
        match rx.recv_timeout(debounce) {
            Ok(Ok(ev)) => match ev.kind {
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
                    pending.extend(ev.paths)
                }
                EventKind::Other | EventKind::Any => full = true,
                _ => {}
            },
            Ok(Err(e)) => eprintln!("[asgrep] watch error: {e}"),
            Err(RecvTimeoutError::Timeout) if full => {
                let s = indexer.index_all()?;
                eprintln!(
                    "[asgrep] full rescan: {} updated, {} skipped, {} removed",
                    s.files_indexed, s.files_skipped, s.files_removed
                );
                full = false;
                pending.clear();
            }
            Err(RecvTimeoutError::Timeout) if !pending.is_empty() => {
                let paths: Vec<_> = pending.drain().collect();
                let t0 = Instant::now();
                let s = indexer.update_paths(&paths)?;
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                if s.files_indexed + s.files_removed + s.files_failed > 0 {
                    eprintln!(
                        "[asgrep] updated {} file(s) ({} removed, {} skipped) in {ms:.3}ms",
                        s.files_indexed, s.files_removed, s.files_skipped
                    );
                }
            }
            Err(RecvTimeoutError::Timeout) if indexer.deferred_rebuilds_pending() => {
                let t0 = Instant::now();
                indexer.flush_deferred_rebuilds()?;
                eprintln!(
                    "[asgrep] deferred rebuilds done in {:.1}ms",
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
fn run_bench(
    root: &Path,
    cli: &Cli,
    query: &str,
    iterations: u32,
    skip_index: bool,
) -> anyhow::Result<()> {
    let (stats_opt, index_ms) = maybe_index(root, cli, skip_index)?;
    let searcher = bench_searcher(root, cli, skip_index)?;
    let mut times = Vec::with_capacity(iterations as usize);
    let mut hits = 0usize;
    for _ in 0..iterations {
        let t0 = Instant::now();
        let r = do_search(&searcher, query, cli.semantic_only)?;
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
        hits = r.hits.len();
    }
    let avg = times.iter().sum::<f64>() / f64::from(iterations);
    let first = times.first().copied().unwrap_or_default();
    let warm = if times.len() > 1 {
        times[1..].iter().sum::<f64>() / (times.len() - 1) as f64
    } else {
        first
    };
    let ag_iters = iterations.min(3);
    let ag_pat = ast_sgrep_core::pattern::ast_grep_pattern_for_query(query);
    let ag_ms = ag_pat
        .as_ref()
        .and_then(|p| ast_sgrep_core::pattern::bench_ast_grep(p, root, ag_iters));
    let speedup = ag_ms.map(|ag| ag / avg);

    if cli.json {
        let mut obj = serde_json::json!({
            "query": query, "iterations": iterations, "avg_search_ms": avg,
            "first_search_ms": first, "warm_search_ms": warm,
            "cold_overhead_ms": first - warm, "hits": hits,
            "ast_grep_pattern": ag_pat, "ast_grep_iterations": ag_iters,
            "avg_ast_grep_ms": ag_ms, "speedup_vs_ast_grep": speedup,
        });
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        println!("{obj}");
    } else {
        println!("Benchmark (v1.0 targets: search <20ms, 0% false callers)");
        print_index_skipped(stats_opt.as_ref(), Some(index_ms));
        println!("Query: {query}");
        println!("Avg search: {avg:.2}ms over {iterations} iterations ({hits} hits)");
        if let (Some(p), Some(ms)) = (&ag_pat, ag_ms) {
            println!("Avg ast-grep (pattern: {p}): {ms:.2}ms over {ag_iters} iterations");
            if let Some(sp) = speedup {
                println!("Speedup vs ast-grep: {sp:.1}x");
            }
        }
    }
    Ok(())
}
const MAX_QUERIES_FILE_LINES: usize = 1000;
fn run_bench_batch(
    root: &Path,
    cli: &Cli,
    queries_path: &Path,
    iterations: u32,
    skip_index: bool,
) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(queries_path)
        .with_context(|| format!("failed to read queries file {}", queries_path.display()))?;
    let queries: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if queries.is_empty() {
        anyhow::bail!("queries file is empty or contains only blank lines");
    }
    if queries.len() > MAX_QUERIES_FILE_LINES {
        anyhow::bail!(
            "queries file has {} lines; maximum is {MAX_QUERIES_FILE_LINES}",
            queries.len()
        );
    }
    let (stats_opt, index_ms) = maybe_index(root, cli, skip_index)?;
    let searcher = bench_searcher(root, cli, skip_index)?;
    let mut results = Vec::with_capacity(queries.len());
    for query in &queries {
        let mut samples = Vec::with_capacity(iterations as usize);
        let mut last = None;
        for _ in 0..iterations {
            let t0 = Instant::now();
            let r = do_search(&searcher, query, cli.semantic_only)?;
            samples.push(t0.elapsed().as_secs_f64() * 1000.0);
            last = Some(r);
        }
        samples.sort_by(f64::total_cmp);
        let avg = samples.iter().sum::<f64>() / f64::from(iterations);
        let p50 = if samples.is_empty() {
            0.0
        } else {
            samples[(samples.len() - 1) / 2]
        };
        let (hits, top_10) = match &last {
            Some(r) => {
                let mut hs: Vec<_> = r.hits.iter().collect();
                hs.sort_by(|a, b| {
                    b.score
                        .total_cmp(&a.score)
                        .then_with(|| a.file.cmp(&b.file))
                        .then_with(|| a.line_start.cmp(&b.line_start))
                });
                hs.truncate(10);
                let top = hs.iter().map(|h| {
                    serde_json::json!({"file": h.file, "line_start": h.line_start, "symbol": h.symbol})
                }).collect::<Vec<_>>();
                (r.hits.len(), top)
            }
            None => (0, vec![]),
        };
        results.push(serde_json::json!({
            "query": query, "avg_search_ms": avg, "p50_search_ms": p50, "hits": hits, "top_10": top_10,
        }));
    }
    if cli.json {
        let mut obj = serde_json::json!({"iterations": iterations, "queries": results});
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        println!("{obj}");
    } else {
        println!(
            "Batch benchmark: {} queries over {} iterations each",
            queries.len(),
            iterations
        );
        print_index_skipped(stats_opt.as_ref(), Some(index_ms));
        for r in &results {
            println!(
                "  {}: avg={:.2}ms p50={:.2}ms hits={}",
                r["query"].as_str().unwrap_or("?"),
                r["avg_search_ms"].as_f64().unwrap_or(0.0),
                r["p50_search_ms"].as_f64().unwrap_or(0.0),
                r["hits"].as_u64().unwrap_or(0)
            );
        }
    }
    Ok(())
}
fn print_index_stats(stats: &IndexStats) {
    println!(
        "Indexed {} files ({} skipped, {} removed)\nExtracted {} symbols, {} callers, {} imports",
        stats.files_indexed,
        stats.files_skipped,
        stats.files_removed,
        stats.symbols_extracted,
        stats.callers_extracted,
        stats.imports_extracted
    );
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
    println!(
        "Semantic IVF sidecar: {}",
        if s.semantic_ivf_present {
            "present"
        } else {
            "not built (below ANN threshold or not indexed)"
        }
    );
}
