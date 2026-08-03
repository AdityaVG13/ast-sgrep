#![forbid(unsafe_code)]

mod agent;
mod bench;
mod eval;
mod machine;
mod search_cmd;
pub mod supervisor;
mod watch;

use anyhow::Context;
use ast_sgrep_core::{
    index_db_path, EmbedBackend, IndexOptions, IndexStats, Indexer, SearchOptions, Searcher,
};
use clap::{Args, Parser, Subcommand};
use machine::{
    print_machine_failure, raw_command_name, raw_machine_output_requested, MACHINE_SCHEMA_VERSION,
};
use std::fmt;
use std::path::{Path, PathBuf};

pub(crate) use machine::{print_machine_json, print_machine_json_status};

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
const MAX_OUTPUT_RESULTS: usize = ast_sgrep_core::MAX_OUTPUT_RESULTS;
const MAX_EXCERPT_LINES: usize = ast_sgrep_core::MAX_EXCERPT_LINES;
const DEFAULT_SNIPPET_TOKENS: usize = 96;
const DEFAULT_RESPONSE_SNIPPET_TOKENS: usize = 768;
const MAX_SNIPPET_TOKENS: usize = 4_096;
const MAX_RESPONSE_SNIPPET_TOKENS: usize = 65_536;
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
    pub(crate) fn search_machine_output(&self) -> bool {
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
        Commands::Search(q) => search_cmd::run_search(&q.query.root, cli, &q.query.query, false),
        Commands::Bench {
            root,
            query,
            iterations,
            suite,
            fixture,
            queries_file,
            skip_index,
            ..
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
        Commands::Keyword(q) => {
            search_cmd::run_keyword_search(&q.query.root, cli, &q.query.query)
        }
        Commands::Semantic(q) => search_cmd::run_search(&q.query.root, cli, &q.query.query, true),
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
fn ensure_unambiguous_root(root: &Path, cli: &Cli) -> anyhow::Result<()> {
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
fn run_index_dry_run(command: &str, root: &Path, cli: &Cli) -> anyhow::Result<()> {
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
fn index_db_display(root: &Path, index_path: Option<&Path>) -> PathBuf {
    index_db_path(root, index_path).unwrap_or_else(|_| root.join(".asgrep/index.db"))
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
pub(crate) fn open_searcher(root: &Path, cli: &Cli) -> anyhow::Result<Searcher> {
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
