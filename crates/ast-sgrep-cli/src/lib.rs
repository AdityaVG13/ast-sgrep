#![forbid(unsafe_code)]

mod agent;
mod eval;
pub mod supervisor;
use anyhow::Context;
use ast_sgrep_core::{
    chain::{expand_chain, ChainConfig},
    format_hit_line, index_db_path, EmbedBackend, IndexOptions, IndexStats, IndexStore, Indexer,
    SearchOptions, SearchResponse, Searcher,
};
use clap::{Args, Parser, Subcommand};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;
#[derive(Args, Clone, Debug)]
struct RootArg {
    /// Project root (canonical form). Prefer this over --root when both would conflict.
    #[arg(default_value = ".", help = "Project root directory")]
    root: PathBuf,
}
#[derive(Args, Clone, Debug)]
struct QueryRootArg {
    /// Search query
    #[arg(help = "Search query string")]
    query: String,
    #[arg(default_value = ".", help = "Project root directory")]
    root: PathBuf,
}
/// Search/index tuning flags — scoped to search/index/bench, not global (vdqo).
#[derive(Args, Clone, Debug, Default)]
pub(crate) struct SearchTuning {
    #[arg(long, env = "ASGREP_NO_EMBED", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Disable semantic embeddings")]
    pub(crate) no_embed: bool,
    #[arg(long, env = "ASGREP_CLOUD_EMBED", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Prefer cloud embeddings")]
    pub(crate) cloud_embed: bool,
    #[arg(long, env = "ASGREP_OLLAMA_EMBED", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Prefer Ollama embeddings")]
    pub(crate) ollama_embed: bool,
    #[arg(long, env = "ASGREP_NEURAL_EMBED", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Prefer local neural embeddings (needs neural-embed feature)")]
    pub(crate) neural_embed: bool,
    #[arg(long, env = "ASGREP_SEMANTIC_ONLY", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Force semantic-only search channel")]
    pub(crate) semantic_only: bool,
    #[arg(long, env = "ASGREP_TANTIVY", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Enable Tantivy lexical sidecar")]
    pub(crate) tantivy: bool,
    #[arg(long, env = "ASGREP_ANN_THRESHOLD", help = "Build IVF ANN when chunk count exceeds this")]
    pub(crate) ann_threshold: Option<usize>,
    #[arg(long, env = "ASGREP_ANN_PROBES", help = "IVF clusters to probe (0 = adaptive)")]
    pub(crate) ann_probes: Option<usize>,
    #[arg(long, env = "ASGREP_RERANK", action = clap::ArgAction::SetTrue, value_parser = clap::builder::BoolishValueParser::new(), help = "Rerank top candidates with local cross-encoder")]
    pub(crate) rerank: bool,
    #[arg(long, env = "ASGREP_RERANK_TOP_K", default_value_t = 20, help = "Candidates considered by --rerank")]
    pub(crate) rerank_top_k: usize,
    #[arg(long, value_name = "FORMAT", value_parser = parse_output_format, help = "Output format (implies --json): native|agent|agent-capsule|compact|github|gitlab")]
    pub(crate) format: Option<String>,
    #[arg(long, default_value = "0", value_name = "N", value_parser = parse_excerpt_lines, help = "Excerpt context lines around each hit")]
    pub(crate) excerpt_lines: usize,
    #[arg(long, default_value_t = DEFAULT_SNIPPET_TOKENS, value_parser = parse_snippet_tokens, help = "Per-result compact snippet token budget")]
    pub(crate) snippet_tokens: usize,
    #[arg(long, default_value_t = DEFAULT_RESPONSE_SNIPPET_TOKENS, value_parser = parse_response_snippet_tokens, help = "Response-wide compact snippet token budget")]
    pub(crate) response_snippet_tokens: usize,
}
#[derive(Args, Clone, Debug)]
struct IndexCmd {
    #[command(flatten)]
    root: RootArg,
    #[command(flatten)]
    tuning: SearchTuning,
    #[arg(long, help = "Report planned index work without writing")]
    dry_run: bool,
}
#[derive(Args, Clone, Debug)]
struct QueryCmd {
    #[command(flatten)]
    query: QueryRootArg,
    #[command(flatten)]
    tuning: SearchTuning,
}
#[derive(Parser)]
#[command(
    name = "asgrep",
    version,
    about = "Polyglot hybrid code search",
    after_help = "Agent triad: asgrep capabilities --json | robot-docs guide | doctor --robot-triage\nSibling binaries: asgrep-mcp (MCP stdio), asgrep-lsp (LSP)\nAlias: ast-sgrep\nExit: 0=ok 1=usage 2=fail\nRoot: positional ROOT is canonical; --root is an alias (conflict = usage error)"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
    #[arg(value_name = "QUERY", help = "Bare hybrid search query (omit subcommand)")]
    query: Option<String>,
    #[arg(id = "global-root", long = "root", global = true, help = "Project root alias (conflicts with positional ROOT)")]
    root: Option<PathBuf>,
    #[arg(long, global = true, env = "ASGREP_LIMIT", value_parser = parse_output_limit, help = "Max results (1..=1000; 0 remapped to default)")]
    limit: Option<usize>,
    #[arg(long, global = true, help = "Emit machine JSON envelope on stdout")]
    json: bool,
    /// Print the agent handbook and exit
    #[arg(long, global = true, help = "Print robot-docs guide and exit")]
    robot_help: bool,
    #[arg(long, global = true, env = "ASGREP_INDEX_PATH", help = "Override index database path")]
    index_path: Option<PathBuf>,
    #[arg(long, global = true, help = "Language filter")]
    lang: Option<String>,
    /// Search-tuning for bare (no-subcommand) search only — not inherited by capabilities/doctor (vdqo).
    #[command(flatten)]
    tuning: SearchTuning,
    #[arg(value_name = "ROOT", default_value = ".", help = "Bare-search project root")]
    search_root: PathBuf,
}
#[derive(Subcommand)]
enum Commands {
    /// Build or incrementally refresh an index
    #[command(about = "Build or incrementally refresh an index")]
    Index(IndexCmd),
    /// Show index and embedding status
    #[command(about = "Show index and embedding status")]
    Status(RootArg),
    /// Clear and rebuild an index
    #[command(about = "Clear and rebuild an index")]
    Reindex(IndexCmd),
    /// Search explicitly; aliases: find, query
    #[command(about = "Hybrid search (aliases: find, query)", alias = "find", alias = "query")]
    Search(QueryCmd),
    /// Run fixed performance and identity suites
    #[command(about = "Run fixed performance and identity suites")]
    Bench {
        #[command(flatten)]
        root: RootArg,
        #[command(flatten)]
        tuning: SearchTuning,
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
    /// Watch files and update the index incrementally
    #[command(about = "Watch files and update the index incrementally")]
    Watch {
        #[command(flatten)]
        root: RootArg,
        #[arg(long, default_value = "300", help = "Debounce window in milliseconds")]
        debounce_ms: u64,
    },
    /// Run lexical-only search
    #[command(about = "Lexical-only (FTS/trigram) search")]
    Keyword(QueryCmd),
    /// Run embedding-only search
    #[command(about = "Embedding-only semantic search")]
    Semantic(QueryCmd),
    /// Expand a bounded symbol/caller/import graph
    #[command(about = "Expand a bounded symbol/caller/import graph")]
    Chain(QueryRootArg),
    /// Print the machine-readable CLI contract
    #[command(about = "Print the machine-readable CLI contract (JSON)")]
    Capabilities(agent::CapabilitiesArgs),
    /// Print package and machine schema versions
    #[command(about = "Print package and machine schema versions")]
    Version(VersionArgs),
    /// Print the agent handbook
    #[command(about = "Print the agent handbook (robot-docs guide)")]
    RobotDocs(agent::RobotDocsArgs),
    /// Diagnose index health and return recovery commands
    #[command(about = "Diagnose index health and return recovery commands")]
    Doctor {
        #[command(flatten)]
        root: RootArg,
        #[command(flatten)]
        args: agent::DoctorArgs,
    },
    /// Evaluate retrieval against a gold fixture
    #[command(about = "Evaluate retrieval against a gold fixture")]
    Eval(eval::EvalArgs),
}
#[derive(Parser)]
struct VersionArgs {
    #[arg(long)]
    json: bool,
}
const MACHINE_SCHEMA_VERSION: &str = "1.0.0";
const MAX_OUTPUT_RESULTS: usize = ast_sgrep_core::MAX_OUTPUT_RESULTS;
const MAX_EXCERPT_LINES: usize = ast_sgrep_core::MAX_EXCERPT_LINES;
const DEFAULT_SNIPPET_TOKENS: usize = 96;
const DEFAULT_RESPONSE_SNIPPET_TOKENS: usize = 768;
const MAX_SNIPPET_TOKENS: usize = 4_096;
const MAX_RESPONSE_SNIPPET_TOKENS: usize = 65_536;
const MAX_ERROR_MESSAGE_CHARS: usize = 4_096;
const MAX_QUERIES_FILE_LINES: usize = 1000;
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
fn parse_output_format(raw: &str) -> Result<String, String> {
    ast_sgrep_plugins::OutputFormat::parse(raw)
        .map(|_| raw.to_ascii_lowercase())
        .ok_or_else(|| {
            "format must be one of: native, agent, agent-capsule, compact, github, gitlab".into()
        })
}
fn parse_snippet_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_SNIPPET_TOKENS, "--snippet-tokens")
}
fn parse_response_snippet_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(
        raw,
        MAX_RESPONSE_SNIPPET_TOKENS,
        "--response-snippet-tokens",
    )
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
            } else {
                let _ = error.print();
            }
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
            } else {
                eprintln!("{error:#}");
                agent::print_agent_help_footer();
            }
            std::process::exit(exit_code);
        }
    }
}
impl Cli {
    fn search_machine_output(&self) -> bool {
        self.json || self.tuning.format.is_some()
    }
    /// Merge parent (pre-subcommand) tuning with subcommand-scoped tuning (vdqo).
    /// Parent flags like `asgrep --format compact search ...` remain effective.
    pub(crate) fn active_tuning(&self) -> SearchTuning {
        let mut t = self.tuning.clone();
        let overlay = match self.command.as_ref() {
            Some(Commands::Index(c) | Commands::Reindex(c)) => Some(&c.tuning),
            Some(Commands::Search(c) | Commands::Keyword(c) | Commands::Semantic(c)) => {
                Some(&c.tuning)
            }
            Some(Commands::Bench { tuning, .. }) => Some(tuning),
            _ => None,
        };
        if let Some(o) = overlay {
            t.no_embed |= o.no_embed;
            t.cloud_embed |= o.cloud_embed;
            t.ollama_embed |= o.ollama_embed;
            t.neural_embed |= o.neural_embed;
            t.semantic_only |= o.semantic_only;
            t.tantivy |= o.tantivy;
            t.rerank |= o.rerank;
            if o.ann_threshold.is_some() {
                t.ann_threshold = o.ann_threshold;
            }
            if o.ann_probes.is_some() {
                t.ann_probes = o.ann_probes;
            }
            if o.format.is_some() {
                t.format = o.format.clone();
            }
            // Prefer explicitly scoped non-default numeric budgets; keep parent pre-subcommand values otherwise.
            if o.rerank_top_k != 20 {
                t.rerank_top_k = o.rerank_top_k;
            }
            if o.excerpt_lines != 0 {
                t.excerpt_lines = o.excerpt_lines;
            }
            if o.snippet_tokens != DEFAULT_SNIPPET_TOKENS {
                t.snippet_tokens = o.snippet_tokens;
            }
            if o.response_snippet_tokens != DEFAULT_RESPONSE_SNIPPET_TOKENS {
                t.response_snippet_tokens = o.response_snippet_tokens;
            }
        }
        t
    }
    fn machine_output_requested(&self) -> bool {
        self.search_machine_output()
            || matches!(self.command.as_ref(), Some(Commands::Capabilities(_)))
            || matches!(self.command.as_ref(), Some(Commands::Version(a)) if a.json)
            || matches!(self.command.as_ref(), Some(Commands::Doctor { .. }))
    }
    fn command_name(&self) -> &'static str {
        match self.command.as_ref() {
            None => "search",
            Some(Commands::Index(_)) => "index",
            Some(Commands::Status(_)) => "status",
            Some(Commands::Reindex(_)) => "reindex",
            Some(Commands::Search(_)) => "search",
            Some(Commands::Bench { .. }) => "bench",
            Some(Commands::Watch { .. }) => "watch",
            Some(Commands::Keyword(_)) => "keyword",
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
fn raw_machine_output_requested(args: &[std::ffi::OsString]) -> bool {
    args.iter().any(|a| {
        a == "--json"
            || a == "--robot-triage"
            || a == "--format"
            || a.to_str().is_some_and(|raw| raw.starts_with("--format="))
    }) || args.iter().any(|a| a == "capabilities" || a == "doctor")
}
fn raw_command_name(args: &[std::ffi::OsString]) -> &'static str {
    const C: &[&str] = &[
        "index",
        "status",
        "reindex",
        "search",
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
        .filter_map(|a| a.to_str())
        .find_map(|a| C.iter().copied().find(|c| a == *c))
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
        serde_json::Value::Object(o) => o,
        _ => {
            return Ok(serde_json::json!({
                "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep",
                "command": command, "ok": true, "exit_code": 0, "data": value
            }));
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
    object.insert("exit_code".into(), 0.into());
    Ok(value)
}
pub(crate) fn print_machine_json(
    command: &str,
    value: impl serde::Serialize,
) -> anyhow::Result<()> {
    print_machine_json_with_style(command, value, false, true, 0)
}
/// Machine envelope with explicit ok/exit_code (doctor unhealthy path).
pub(crate) fn print_machine_json_status(
    command: &str,
    value: impl serde::Serialize,
    ok: bool,
    exit_code: i32,
) -> anyhow::Result<()> {
    print_machine_json_with_style(command, value, false, ok, exit_code)
}
fn print_machine_json_with_style(
    command: &str,
    value: impl serde::Serialize,
    compact: bool,
    ok: bool,
    exit_code: i32,
) -> anyhow::Result<()> {
    let mut value = machine_value(command, value)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("ok".into(), ok.into());
        object.insert("exit_code".into(), exit_code.into());
    }
    if compact {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}
fn print_machine_failure(command: &str, kind: &str, exit_code: i32, message: &str) {
    let value = serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep", "command": command,
        "ok": false, "exit_code": exit_code,
        "error": {"kind": kind, "message": bounded_error_message(message)}
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("failure envelope serializes")
    );
}
fn run_cli(cli: &Cli) -> anyhow::Result<()> {
    if cli.robot_help {
        agent::print_robot_guide();
        return Ok(());
    }
    if cli.active_tuning().format.is_some()
        && !matches!(
            cli.command.as_ref(),
            None
                | Some(
                    Commands::Search(_)
                        | Commands::Keyword(_)
                        | Commands::Semantic(_)
                        | Commands::Index(_)
                        | Commands::Reindex(_)
                        | Commands::Bench { .. }
                )
        )
    {
        return Err(usage_error(
            "--format applies only to search, keyword, or semantic commands",
        ));
    }
    match cli.command.as_ref() {
        Some(c) => run_command(cli, c),
        None => run_default_search(cli),
    }
}
fn run_command(cli: &Cli, command: &Commands) -> anyhow::Result<()> {
    match command {
        Commands::Index(c) => {
            if c.dry_run {
                return run_index_dry_run("index", &c.root.root, cli);
            }
            with_index(
                "index",
                &c.root.root,
                cli,
                |i| {
                    if !cli.json {
                        eprintln!("asgrep: indexing {} ...", c.root.root.display());
                    }
                    i.index_all().context("indexing failed")
                },
                print_index_stats,
            )
        }
        Commands::Status(r) => {
            let st = open_indexer(&r.root, cli)?
                .store()
                .status()
                .context("failed to read status")?;
            print_json_or(cli.json, "status", &st, || print_status(&st))
        }
        Commands::Reindex(c) => {
            if c.dry_run {
                return run_index_dry_run("reindex", &c.root.root, cli);
            }
            with_index(
                "reindex",
                &c.root.root,
                cli,
                |i| {
                    if !cli.json {
                        eprintln!("asgrep: reindexing {} ...", c.root.root.display());
                    }
                    i.reindex_all().context("reindex failed")
                },
                print_index_stats,
            )
        }
        Commands::Search(q) => run_search(&q.query.root, cli, &q.query.query, false),
        Commands::Bench {
            root,
            query,
            iterations,
            suite,
            fixture,
            queries_file,
            skip_index,
            ..
        } => run_bench_command(
            &root.root,
            cli,
            query,
            *iterations,
            suite.as_deref(),
            fixture,
            queries_file.as_deref(),
            *skip_index,
        ),
        Commands::Watch { root, debounce_ms } => run_watch(&root.root, cli, *debounce_ms),
        Commands::Keyword(q) => run_keyword_search(&q.query.root, cli, &q.query.query),
        Commands::Semantic(q) => run_search(&q.query.root, cli, &q.query.query, true),
        Commands::Chain(q) => run_chain(&q.root, cli, &q.query),
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
    run_search(&cli.search_root, cli, query, false)
}
pub(crate) fn effective_root(cli: &Cli, fallback: &Path) -> PathBuf {
    cli.root.clone().unwrap_or_else(|| fallback.to_path_buf())
}
pub(crate) fn resolve_root_index(cli: &Cli, root: &Path) -> (PathBuf, Option<PathBuf>) {
    (effective_root(cli, root), cli.index_path.clone())
}
fn ensure_unambiguous_root(root: &Path, cli: &Cli) -> anyhow::Result<()> {
    if cli.root.is_some() && root != Path::new(".") {
        return Err(usage_error(
            "ROOT is ambiguous: use either --root ROOT or a positional ROOT, not both",
        ));
    }
    Ok(())
}
fn ensure_existing_root(root: &Path, cli: &Cli) -> anyhow::Result<PathBuf> {
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
fn run_index_dry_run(command: &str, root: &Path, cli: &Cli) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let mut files = 0usize;
    let mut skipped = 0usize;
    fn walk(dir: &Path, files: &mut usize, skipped: &mut usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
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
fn index_db_display(root: &Path, index_path: Option<&Path>) -> PathBuf {
    index_db_path(root, index_path).unwrap_or_else(|_| root.join(".asgrep/index.db"))
}
fn ensure_nonempty_index(root: &Path, file_count: usize) -> anyhow::Result<()> {
    if file_count == 0 {
        anyhow::bail!(
            "index is empty for {}; run: asgrep index {} --json",
            root.display(),
            root.display()
        );
    }
    Ok(())
}
fn resolve_output_format(
    raw: Option<&str>,
    default: ast_sgrep_plugins::OutputFormat,
) -> anyhow::Result<ast_sgrep_plugins::OutputFormat> {
    match raw {
        Some(raw) => ast_sgrep_plugins::OutputFormat::parse(raw).ok_or_else(|| {
            usage_error(format!(
                "unknown output format {raw:?}; expected native, agent, agent-capsule, compact, github, or gitlab"
            ))
        }),
        None => Ok(default),
    }
}
fn open_indexer(root: &Path, cli: &Cli) -> anyhow::Result<Indexer> {
    ensure_existing_root(root, cli)?;
    {
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
}
fn open_searcher(root: &Path, cli: &Cli) -> anyhow::Result<Searcher> {
    let root = ensure_existing_root(root, cli)?;
    let opts = search_options(&root, cli);
    let db = index_db_display(&opts.root, opts.index_path.as_deref());
    let searcher = Searcher::new(opts).with_context(|| {
        format!(
            "failed to open index at {} (root {})",
            db.display(),
            root.display()
        )
    })?;
    ensure_nonempty_index(&root, searcher.store().status()?.file_count)?;
    Ok(searcher)
}
fn run_chain(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let root = ensure_existing_root(root, cli)?;
    let (_, index_path) = resolve_root_index(cli, &root);
    let store = IndexStore::open(&root, index_path.as_deref()).context("failed to open index")?;
    ensure_nonempty_index(&root, store.status()?.file_count)?;
    let config = ChainConfig {
        limit: cli.limit.unwrap_or(ChainConfig::default().limit),
        top_n: 1,
        ..ChainConfig::default()
    };
    let r = expand_chain(&store, query, &config).context("chain search failed")?;
    if cli.json {
        return print_machine_json("chain", &r);
    }
    println!(
        "chain {:?}: {} nodes, {} edges (max depth {})",
        r.query, r.node_count, r.edge_count, r.max_depth
    );
    println!("nodes:");
    for n in &r.nodes {
        let sym = n.symbol.as_deref().unwrap_or("<file>");
        println!(
            "  depth {} score {:.4} {}:{}-{} {sym}",
            n.depth, n.score, n.file, n.line_start, n.line_end
        );
    }
    println!("edges:");
    for e in &r.edges {
        let from = e.from_symbol.as_deref().unwrap_or("<file>");
        let to = e.to_symbol.as_deref().unwrap_or("<file>");
        println!(
            "  depth {} {:?}: {}::{from} -> {}::{to}",
            e.depth, e.label, e.from_file, e.to_file
        );
    }
    Ok(())
}
fn run_keyword_search(root: &Path, cli: &Cli, query: &str) -> anyhow::Result<()> {
    let response = open_searcher(root, cli)?
        .search_lexical(query)
        .context("keyword search failed")?;
    if !cli.search_machine_output() {
        for hit in &response.hits {
            println!("{}", format_hit_line(hit));
        }
        return Ok(());
    }
    let format = resolve_output_format(
        cli.active_tuning().format.as_deref(),
        ast_sgrep_plugins::OutputFormat::Native,
    )?;
    print_search_response("keyword", &response, format, cli)
}

fn run_search(root: &Path, cli: &Cli, query: &str, semantic: bool) -> anyhow::Result<()> {
    let ctx = if semantic || cli.active_tuning().semantic_only {
        "semantic search failed"
    } else {
        "search failed"
    };
    let response =
        do_search_with_cli(&open_searcher(root, cli)?, query, semantic, cli).context(ctx)?;
    if !cli.search_machine_output() {
        for hit in &response.hits {
            println!("{}", format_hit_line(hit));
        }
        return Ok(());
    }
    let tuning = cli.active_tuning();
    let default = if semantic || tuning.semantic_only {
        ast_sgrep_plugins::OutputFormat::Agent
    } else {
        ast_sgrep_plugins::OutputFormat::Native
    };
    let format = resolve_output_format(tuning.format.as_deref(), default)?;
    print_search_response(
        if semantic || tuning.semantic_only {
            "semantic"
        } else {
            "search"
        },
        &response,
        format,
        cli,
    )
}
fn print_search_response(
    command: &str,
    response: &ast_sgrep_core::SearchResponse,
    format: ast_sgrep_plugins::OutputFormat,
    cli: &Cli,
) -> anyhow::Result<()> {
    let value = ast_sgrep_plugins::format_response_with_budget(
        response,
        format,
        cli.active_tuning().excerpt_lines,
        ast_sgrep_plugins::CompactBudget {
            per_result_tokens: cli.active_tuning().snippet_tokens,
            response_tokens: cli.active_tuning().response_snippet_tokens,
        },
    );
    print_machine_json_with_style(
        command,
        value,
        format == ast_sgrep_plugins::OutputFormat::Compact,
        true,
        0,
    )
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
    let mut indexer = open_indexer(root, cli)?;
    let t0 = Instant::now();
    Ok((
        Some(indexer.index_all()?),
        t0.elapsed().as_secs_f64() * 1000.0,
    ))
}
fn bench_searcher(root: &Path, cli: &Cli, skip_index: bool) -> anyhow::Result<Searcher> {
    let (resolved, index_path) = resolve_root_index(cli, root);
    let db = index_db_path(&resolved, index_path.as_deref())?;
    if skip_index && !db.exists() {
        anyhow::bail!(
            "failed to open existing index at {} (run `asgrep index` first)",
            db.display()
        );
    }
    open_searcher(root, cli)
}
fn do_search(s: &Searcher, q: &str, semantic: bool) -> anyhow::Result<SearchResponse> {
    if semantic {
        Ok(s.search_semantic(q)?)
    } else {
        Ok(s.search(q)?)
    }
}
fn do_search_with_cli(
    s: &Searcher,
    q: &str,
    semantic: bool,
    cli: &Cli,
) -> anyhow::Result<SearchResponse> {
    // `--semantic-only` / ASGREP_SEMANTIC_ONLY forces the semantic channel (ziij).
    do_search(s, q, semantic || cli.active_tuning().semantic_only)
}
fn timed_searches(
    s: &Searcher,
    q: &str,
    semantic: bool,
    iterations: u32,
) -> anyhow::Result<(Vec<f64>, Option<SearchResponse>)> {
    let mut times = Vec::with_capacity(iterations as usize);
    let mut last = None;
    for _ in 0..iterations {
        let t0 = Instant::now();
        last = Some(do_search(s, q, semantic)?);
        times.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    Ok((times, last))
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
    let results: Vec<serde_json::Value> = cases.iter().map(|case| {
        let expected = ast_sgrep_core::bench_suite::benchmark_expectation(case)
            .ok_or_else(|| anyhow::anyhow!("benchmark case '{}' has no identity contract", case.name))?;
        let semantic_only = expected.kind == Some(ast_sgrep_core::search::HitKind::Embed);
        let (times, last) = timed_searches(&searcher, case.query, semantic_only, iterations)?;
        let hits = last.as_ref().map_or(0, |r| r.hits.len());
        let identity_ok = last.as_ref().is_some_and(|response| {
            response
                .hits
                .iter()
                .take(expected.max_rank)
                .any(|hit| expected.matches(hit))
        });
        let avg = times.iter().sum::<f64>() / f64::from(iterations.max(1));
        let ag_pat = ast_sgrep_core::pattern::ast_grep_pattern_for_query(case.query);
        let ag_ms = ag_pat.as_ref().and_then(|p| ast_sgrep_core::pattern::bench_ast_grep(p, &bench_root, iterations.min(3)));
        Ok(serde_json::json!({"name": case.name, "query": case.query, "avg_search_ms": avg, "hits": hits, "min_hits": case.min_hits,
            "identity_ok": identity_ok, "identity_max_rank": expected.max_rank,
            "ok": hits >= case.min_hits && identity_ok,
            "ast_grep_pattern": ag_pat, "avg_ast_grep_ms": ag_ms, "speedup_vs_ast_grep": ag_ms.map(|ag| ag / avg)}))
    }).collect::<anyhow::Result<_>>()?;
    if cli.json {
        let mut obj = serde_json::json!({"fixture": fixture_name, "suite": suite_name, "iterations": iterations, "cases": results});
        if let Some(s) = &stats {
            obj["files_indexed"] = serde_json::json!(s.files_indexed);
        } else {
            obj["index_skipped"] = serde_json::json!(true);
            obj["index_ms"] = serde_json::json!(0.0);
            obj["files_indexed"] = serde_json::Value::Null;
        }
        print_machine_json("bench", &obj)?;
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
        anyhow::bail!("benchmark suite failed hit-count or result-identity thresholds");
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
    let (times, last) = timed_searches(&searcher, query, cli.active_tuning().semantic_only, iterations)?;
    let hits = last.as_ref().map_or(0, |r| r.hits.len());
    let avg = times.iter().sum::<f64>() / f64::from(iterations.max(1));
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
        let mut obj = serde_json::json!({"query": query, "iterations": iterations, "avg_search_ms": avg, "first_search_ms": first, "warm_search_ms": warm, "cold_overhead_ms": first - warm, "hits": hits, "ast_grep_pattern": ag_pat, "ast_grep_iterations": ag_iters, "avg_ast_grep_ms": ag_ms, "speedup_vs_ast_grep": speedup});
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        print_machine_json("bench", &obj)?;
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
        let (mut samples, last) = timed_searches(&searcher, query, cli.active_tuning().semantic_only, iterations)?;
        samples.sort_by(f64::total_cmp);
        let avg = samples.iter().sum::<f64>() / f64::from(iterations.max(1));
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
                (r.hits.len(), hs.iter().map(|h| serde_json::json!({"file": h.file, "line_start": h.line_start, "symbol": h.symbol})).collect::<Vec<_>>())
            }
            None => (0, vec![]),
        };
        results.push(serde_json::json!({"query": query, "avg_search_ms": avg, "p50_search_ms": p50, "hits": hits, "top_10": top_10}));
    }
    if cli.json {
        let mut obj = serde_json::json!({"iterations": iterations, "queries": results});
        add_index_json(&mut obj, stats_opt.as_ref(), index_ms);
        print_machine_json("bench", &obj)?;
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
