//! Clap argument structs and value parsers for the CLI.

use clap::{Args, Parser, Subcommand};
use std::fmt;
use std::path::PathBuf;

use crate::agent;
use crate::eval;

pub(crate) const MAX_OUTPUT_RESULTS: usize = ast_sgrep_core::MAX_OUTPUT_RESULTS;
pub(crate) const MAX_EXCERPT_LINES: usize = ast_sgrep_core::MAX_EXCERPT_LINES;
pub(crate) const DEFAULT_SNIPPET_TOKENS: usize = 96;
pub(crate) const DEFAULT_RESPONSE_SNIPPET_TOKENS: usize = 768;
pub(crate) const MAX_SNIPPET_TOKENS: usize = 4_096;
pub(crate) const MAX_RESPONSE_SNIPPET_TOKENS: usize = 65_536;

#[derive(Args, Clone, Debug)]
pub(crate) struct RootArg {
    /// Project root (canonical form). Prefer this over --root when both would conflict.
    #[arg(default_value = ".", help = "Project root directory")]
    pub(crate) root: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct QueryRootArg {
    /// Search query
    #[arg(help = "Search query string")]
    pub(crate) query: String,
    #[arg(default_value = ".", help = "Project root directory")]
    pub(crate) root: PathBuf,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CallPathArgs {
    #[arg(help = "Starting caller symbol")]
    pub(crate) source: String,
    #[arg(help = "Target callee symbol")]
    pub(crate) sink: String,
    #[arg(default_value = ".", help = "Project root directory")]
    pub(crate) root: PathBuf,
    #[arg(
        long,
        default_value_t = 8,
        value_parser = clap::value_parser!(u32).range(1..=64),
        help = "Maximum caller-to-callee hops (1..=64)"
    )]
    pub(crate) max_depth: u32,
    #[arg(
        long,
        default_value_t = 10_000,
        value_parser = clap::value_parser!(u64).range(1..=100_000),
        help = "Maximum distinct symbols visited (1..=100000)"
    )]
    pub(crate) max_nodes: u64,
    #[arg(
        long,
        default_value_t = 50_000,
        value_parser = clap::value_parser!(u64).range(1..=500_000),
        help = "Maximum call edges inspected (1..=500000)"
    )]
    pub(crate) max_edges: u64,
}

/// Search/index tuning flags — scoped to search/index/bench, not global (vdqo).
#[derive(Args, Clone, Debug, Default)]
pub(crate) struct SearchTuning {
    #[arg(
        long,
        env = "ASGREP_NO_EMBED",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Disable semantic embeddings"
    )]
    pub(crate) no_embed: bool,
    #[arg(
        long,
        env = "ASGREP_NEURAL_EMBED",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Prefer local neural embeddings (needs neural-embed feature)"
    )]
    pub(crate) neural_embed: bool,
    #[arg(
        long,
        env = "ASGREP_SEMANTIC_ONLY",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Force semantic-only search channel"
    )]
    pub(crate) semantic_only: bool,
    #[arg(
        long,
        env = "ASGREP_TANTIVY",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Enable Tantivy lexical sidecar"
    )]
    pub(crate) tantivy: bool,
    #[arg(
        long,
        env = "ASGREP_ANN_THRESHOLD",
        help = "Build IVF ANN when chunk count exceeds this"
    )]
    pub(crate) ann_threshold: Option<usize>,
    #[arg(
        long,
        env = "ASGREP_ANN_PROBES",
        help = "IVF clusters to probe (0 = adaptive)"
    )]
    pub(crate) ann_probes: Option<usize>,
    #[arg(
        long,
        env = "ASGREP_RERANK",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Rerank top candidates with local cross-encoder"
    )]
    pub(crate) rerank: bool,
    #[arg(
        long,
        env = "ASGREP_RERANK_TOP_K",
        default_value_t = 20,
        help = "Candidates considered by --rerank"
    )]
    pub(crate) rerank_top_k: usize,
    #[arg(
        long,
        value_name = "FORMAT",
        value_parser = parse_output_format,
        help = "Output format (implies --json): native|agent|agent-capsule|compact|github|gitlab"
    )]
    pub(crate) format: Option<String>,
    #[arg(
        long,
        default_value = "0",
        value_name = "N",
        value_parser = parse_excerpt_lines,
        help = "Excerpt context lines around each hit"
    )]
    pub(crate) excerpt_lines: usize,
    #[arg(
        long,
        default_value_t = DEFAULT_SNIPPET_TOKENS,
        value_parser = parse_snippet_tokens,
        help = "Per-result compact snippet token budget"
    )]
    pub(crate) snippet_tokens: usize,
    #[arg(
        long,
        default_value_t = DEFAULT_RESPONSE_SNIPPET_TOKENS,
        value_parser = parse_response_snippet_tokens,
        help = "Response-wide compact snippet token budget"
    )]
    pub(crate) response_snippet_tokens: usize,
    /// m38g: a whole-response token budget that picks per-result detail,
    /// instead of truncating every excerpt to the same ceiling.
    #[arg(
        long,
        value_parser = parse_budget_tokens,
        help = "Whole-response token budget; picks per-result detail (compact format)"
    )]
    pub(crate) budget_tokens: Option<usize>,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct IndexCmd {
    #[command(flatten)]
    pub(crate) root: RootArg,
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
    #[arg(long, help = "Report planned index work without writing")]
    pub(crate) dry_run: bool,
    #[arg(
        long = "path",
        value_name = "PATH",
        action = clap::ArgAction::Append,
        help = "Update one changed file path (repeatable; index only, max 1024)"
    )]
    pub(crate) paths: Vec<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Optional SCIP JSON overlay; missing or malformed degrades, never fails the index"
    )]
    pub(crate) scip: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct ReindexCmd {
    #[command(flatten)]
    pub(crate) root: RootArg,
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
    #[arg(long, help = "Report planned index work without writing")]
    pub(crate) dry_run: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "Optional SCIP JSON overlay; missing or malformed degrades, never fails the index"
    )]
    pub(crate) scip: Option<PathBuf>,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct CodemodCmd {
    #[command(flatten)]
    pub(crate) root: RootArg,
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
    #[arg(
        long,
        value_name = "PATTERN",
        help = "Native structural search pattern"
    )]
    pub(crate) pattern: String,
    #[arg(
        long,
        value_name = "TEMPLATE",
        help = "Replacement template using matched metavariables or $MATCH"
    )]
    pub(crate) rewrite: String,
    #[arg(long, help = "Emit an edit plan without writing source files")]
    pub(crate) dry_run: bool,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct QueryCmd {
    #[command(flatten)]
    pub(crate) query: QueryRootArg,
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
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
    pub(crate) command: Option<Commands>,
    #[arg(
        value_name = "QUERY",
        help = "Bare hybrid search query (omit subcommand)"
    )]
    pub(crate) query: Option<String>,
    #[arg(
        id = "global-root",
        long = "root",
        global = true,
        help = "Project root alias (conflicts with positional ROOT)"
    )]
    pub(crate) root: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        env = "ASGREP_LIMIT",
        value_parser = parse_output_limit,
        help = "Max results (1..=1000; 0 remapped to default)"
    )]
    pub(crate) limit: Option<usize>,
    #[arg(long, global = true, help = "Emit machine JSON envelope on stdout")]
    pub(crate) json: bool,
    /// Print the agent handbook and exit
    #[arg(long, global = true, help = "Print robot-docs guide and exit")]
    pub(crate) robot_help: bool,
    #[arg(
        long,
        global = true,
        env = "ASGREP_INDEX_PATH",
        help = "Override index database path"
    )]
    pub(crate) index_path: Option<PathBuf>,
    #[arg(long, global = true, help = "Language filter")]
    pub(crate) lang: Option<String>,
    /// 0obi: `fast-unsafe` can corrupt the index on power loss, so it must be
    /// asked for by name; it is never reached by default.
    #[arg(
        long,
        global = true,
        env = "ASGREP_DURABILITY",
        value_parser = parse_durability,
        help = "Index write durability: strict|balanced|fast-unsafe (default balanced)"
    )]
    pub(crate) durability: Option<ast_sgrep_core::Durability>,
    #[arg(
        long = "no-auto-index",
        global = true,
        env = "ASGREP_NO_AUTO_INDEX",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Fail if the index is empty instead of indexing automatically"
    )]
    pub(crate) no_auto_index: bool,
    /// Search-tuning for bare (no-subcommand) search only — not inherited by capabilities/doctor (vdqo).
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
    #[arg(
        value_name = "ROOT",
        default_value = ".",
        help = "Bare-search project root"
    )]
    pub(crate) search_root: PathBuf,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Build or incrementally refresh an index
    #[command(about = "Build or incrementally refresh an index")]
    Index(IndexCmd),
    /// Show index and embedding status
    #[command(about = "Show index and embedding status")]
    Status(RootArg),
    /// Force a full transactional rebuild
    #[command(about = "Force a full transactional rebuild")]
    Reindex(ReindexCmd),
    /// Plan or apply an indexed structural rewrite in process
    #[command(about = "Plan or apply an indexed structural rewrite in process")]
    Codemod(CodemodCmd),
    /// Search explicitly; aliases: find, query
    #[command(
        about = "Hybrid search (aliases: find, query)",
        alias = "find",
        alias = "query"
    )]
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
    /// Find a bounded directed call path; this does not track values
    #[command(about = "Find a bounded call path (call graph only, not value flow)")]
    CallPath(CallPathArgs),
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
    /// Run many Code Mode tool calls in one warm process (Pi parallel coalescing).
    #[command(about = "Run many Code Mode tool calls in one warm process")]
    CodemodeBatch {
        /// Path to a JSON BatchRequest file, or `-` for stdin.
        #[arg(long)]
        requests: PathBuf,
    },
    /// Sticky NDJSON Code Mode worker (one warm Searcher for a whole program).
    #[command(about = "Sticky NDJSON Code Mode worker")]
    CodemodeServe,
}

#[derive(Parser)]
pub(crate) struct VersionArgs {
    #[arg(long)]
    pub(crate) json: bool,
}

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

/// 0obi: an unrecognized durability value is a hard error, never a silent
/// downgrade to a weaker profile.
fn parse_durability(raw: &str) -> Result<ast_sgrep_core::Durability, String> {
    ast_sgrep_core::Durability::parse(raw).ok_or_else(|| {
        format!("unknown durability '{raw}' (expected strict, balanced, or fast-unsafe)")
    })
}

/// m38g: bounded like the other token knobs so a hostile value cannot make the
/// renderer allocate without limit.
fn parse_budget_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_RESPONSE_SNIPPET_TOKENS, "--budget-tokens")
}

fn parse_output_limit(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_OUTPUT_RESULTS, "--limit")
}

fn parse_excerpt_lines(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_EXCERPT_LINES, "--excerpt-lines")
}

fn parse_output_format(raw: &str) -> Result<String, String> {
    const FORMATS: &[&str] = &[
        "native",
        "agent",
        "agent-capsule",
        "compact",
        "github",
        "gitlab",
    ];
    let lower = raw.to_ascii_lowercase();
    if ast_sgrep_plugins::OutputFormat::parse(&lower).is_some() {
        return Ok(lower);
    }
    // Common agent mistakes: think format is "json" / typo "jason" — prefer compact for LLM use.
    let suggestion = match lower.as_str() {
        "json" | "jsno" | "josn" | "jason" | "ndjson" => Some("compact"),
        "gh" | "github-actions" => Some("github"),
        "gl" => Some("gitlab"),
        "capsule" | "agent_capsule" | "agentcapsule" => Some("agent-capsule"),
        _ => FORMATS
            .iter()
            .copied()
            .filter(|cand| edit_distance(&lower, cand) <= 2)
            .min_by_key(|cand| edit_distance(&lower, cand)),
    };
    let list = FORMATS.join(", ");
    Err(match suggestion {
        Some(s) => format!(
            "invalid --format '{raw}' (did you mean '{s}'?). Try: asgrep --json --format {s} \"query\" .\nAllowed: {list}"
        ),
        None => format!(
            "invalid --format '{raw}'. Try: asgrep --json --format compact \"query\" .\nAllowed: {list}"
        ),
    })
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
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

impl Cli {
    pub(crate) fn search_machine_output(&self) -> bool {
        self.json || self.tuning.format.is_some()
    }

    /// Merge parent (pre-subcommand) tuning with subcommand-scoped tuning (vdqo).
    /// Parent flags like `asgrep --format compact search ...` remain effective.
    pub(crate) fn active_tuning(&self) -> SearchTuning {
        let mut t = self.tuning.clone();
        let overlay = match self.command.as_ref() {
            Some(Commands::Index(c)) => Some(&c.tuning),
            Some(Commands::Reindex(c)) => Some(&c.tuning),
            Some(Commands::Codemod(c)) => Some(&c.tuning),
            Some(Commands::Search(c) | Commands::Keyword(c) | Commands::Semantic(c)) => {
                Some(&c.tuning)
            }
            Some(Commands::Bench { tuning, .. }) => Some(tuning),
            _ => None,
        };
        if let Some(o) = overlay {
            t.no_embed |= o.no_embed;
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
            if o.budget_tokens.is_some() {
                t.budget_tokens = o.budget_tokens;
            }
        }
        t
    }

    pub(crate) fn machine_output_requested(&self) -> bool {
        self.search_machine_output()
            || matches!(self.command.as_ref(), Some(Commands::Codemod(c)) if c.dry_run)
            || matches!(self.command.as_ref(), Some(Commands::Capabilities(_)))
            || matches!(self.command.as_ref(), Some(Commands::Version(a)) if a.json)
            || matches!(self.command.as_ref(), Some(Commands::Doctor { .. }))
            // codemode-batch always emits a JSON envelope on success (no --json gate).
            || matches!(self.command.as_ref(), Some(Commands::CodemodeBatch { .. }))
    }

    pub(crate) fn command_name(&self) -> &'static str {
        match self.command.as_ref() {
            None => "search",
            Some(Commands::Index(_)) => "index",
            Some(Commands::Status(_)) => "status",
            Some(Commands::Reindex(_)) => "reindex",
            Some(Commands::Codemod(_)) => "codemod",
            Some(Commands::Search(_)) => "search",
            Some(Commands::Bench { .. }) => "bench",
            Some(Commands::Watch { .. }) => "watch",
            Some(Commands::Keyword(_)) => "keyword",
            Some(Commands::Semantic(_)) => "semantic",
            Some(Commands::Chain(_)) => "chain",
            Some(Commands::CallPath(_)) => "call-path",
            Some(Commands::Capabilities(_)) => "capabilities",
            Some(Commands::Version(_)) => "version",
            Some(Commands::RobotDocs(_)) => "robot-docs",
            Some(Commands::Doctor { .. }) => "doctor",
            Some(Commands::Eval(_)) => "eval",
            Some(Commands::CodemodeBatch { .. }) => "codemode-batch",
            Some(Commands::CodemodeServe) => "codemode-serve",
        }
    }
}
