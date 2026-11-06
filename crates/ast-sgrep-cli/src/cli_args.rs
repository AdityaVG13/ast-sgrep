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
pub(crate) struct ReadArgs {
    /// File refs to read: `path`, `path#L12`, or `path#L12-L40` (relative to root).
    #[arg(value_name = "REF", required = true)]
    pub(crate) refs: Vec<String>,
    /// Context lines added around each requested span.
    #[arg(long, default_value_t = 0, help = "Context lines around each window")]
    pub(crate) context_lines: usize,
    /// Max characters per window (clamped to 1..=100000).
    #[arg(long, default_value_t = 100_000, help = "Max characters per window")]
    pub(crate) max_chars: usize,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct OutlineArgs {
    /// File to outline (relative to root, or absolute inside root).
    /// Required positionals precede the optional ROOT positional (clap order).
    pub(crate) path: PathBuf,
    #[command(flatten)]
    pub(crate) root: RootArg,
}

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
pub(crate) struct ChainArgs {
    #[command(flatten)]
    pub(crate) query: QueryRootArg,
    #[arg(long, default_value_t = ast_sgrep_core::chain::DEFAULT_CHAIN_WORK as u64,
          value_parser = clap::value_parser!(u64).range(1..=ast_sgrep_core::chain::MAX_CHAIN_WORK as u64),
          help = "Traversal attempt budget, independent of --limit (not a SQL/time bound)")]
    pub(crate) max_work: u64,
}

#[derive(Args, Clone, Debug)]
pub(crate) struct SymbolRootArg {
    #[arg(help = "Symbol to look up (module substring for imports)")]
    pub(crate) symbol: String,
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
        hide = true,
        help = "Removed: the static engine is the only embedding backend"
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
        hide = true,
        help = "Removed: cross-encoder reranking was retired with the neural backends"
    )]
    pub(crate) rerank: bool,
    #[arg(
        long,
        env = "ASGREP_RERANK_TOP_K",
        default_value_t = 20,
        hide = true,
        help = "Removed: see --rerank"
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
    #[arg(
        long,
        value_name = "GLOB",
        help = "Restrict search hits to a repository-relative file glob"
    )]
    pub(crate) file_filter: Option<String>,
    /// m38g: a whole-response token budget that picks per-result detail,
    /// instead of truncating every excerpt to the same ceiling.
    #[arg(
        long,
        value_parser = parse_budget_tokens,
        help = "Whole-response token budget; picks per-result detail (compact format)"
    )]
    pub(crate) budget_tokens: Option<usize>,
    #[arg(
        long,
        value_name = "MODE",
        value_parser = parse_preview_mode,
        help = "Snippet size for machine output: none|short|full (default short for compact/agent)"
    )]
    pub(crate) preview: Option<PreviewMode>,
    /// Boolean-listing output mode. Text output prints matching paths
    /// (sorted, deduped, one per line) instead of hit rows; --json envelopes
    /// add a top-level `files` array with the same set. Zero matches stays
    /// exit 0 (ok:true, empty) per the exit contract.
    #[arg(
        long,
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Print matching file paths (sorted, deduped) instead of hits; --json adds a `files` array"
    )]
    pub(crate) files_with_matches: bool,
    /// Per-file match counts (grep -c): text output prints `path:count`
    /// lines instead of hits; --json envelopes add a `counts` array.
    /// Zero-match files stay absent (the hit set is the source of truth).
    #[arg(
        long,
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Print per-file match counts (path:count) instead of hits; --json adds a `counts` array"
    )]
    pub(crate) count: bool,
}

/// Snippet size for machine-readable search output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum PreviewMode {
    None,
    #[default]
    Short,
    Full,
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

/// Search-subcommand arguments: one QUERY positional OR N `--pattern` flags
/// (multi-pattern ingress). The two forms are mutually exclusive and enforced
/// at dispatch; clap keeps QUERY optional only when --pattern is present.
#[derive(Args, Clone, Debug)]
pub(crate) struct QueryCmd {
    /// Search query string
    #[arg(
        value_name = "QUERY",
        required_unless_present = "pattern",
        help = "Search query string (omit when batching --pattern flags)"
    )]
    pub(crate) query: Option<String>,
    #[command(flatten)]
    pub(crate) root: RootArg,
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
    #[arg(
        long,
        value_name = "PATTERN",
        action = clap::ArgAction::Append,
        help = "Structural search pattern; repeat to batch N patterns into ONE envelope (one index open; --limit applies per pattern; hits are tagged with their pattern). Batch usage: asgrep search --pattern P1 --pattern P2 --root ROOT (a positional after --pattern is the QUERY slot and stays mutually exclusive)"
    )]
    pub(crate) pattern: Vec<String>,
}

/// Keyword/semantic subcommand arguments: single query, no batch surface.
#[derive(Args, Clone, Debug)]
pub(crate) struct LexicalQueryCmd {
    #[command(flatten)]
    pub(crate) query: QueryRootArg,
    #[command(flatten)]
    pub(crate) tuning: SearchTuning,
}

#[derive(Parser, Clone)]
#[command(
    name = "asgrep",
    version,
    about = "Polyglot hybrid code search",
    after_help = "Agent triad: asgrep capabilities --json | robot-docs guide | doctor --robot-triage\nSibling binaries: asgrep-mcp (MCP stdio), asgrep-lsp (LSP)\nAlias: ast-sgrep\nJSON: -j/--json (stdout data); diagnostics on stderr\nWrites: asgrep codemod apply requires --yes (alias --force); prefer --dry-run first\nExit: 0=ok 1=usage 2=fail\nRoot: positional ROOT is canonical; --root is an alias (conflict = usage error)"
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
    #[arg(
        short = 'j',
        long,
        global = true,
        help = "Emit machine JSON envelope on stdout"
    )]
    pub(crate) json: bool,
    #[arg(
        long,
        alias = "force",
        global = true,
        help = "Confirm dangerous writes; required to apply a codemod (omit with --dry-run)"
    )]
    pub(crate) yes: bool,
    /// Print the agent handbook and exit
    #[arg(long, global = true, help = "Print robot-docs guide and exit")]
    pub(crate) robot_help: bool,
    #[arg(
        long,
        global = true,
        env = "ASGREP_INDEX_PATH",
        value_parser = parse_cwd_absolute_index_path,
        help = "Override index database path (relative paths resolve against the shell cwd)"
    )]
    pub(crate) index_path: Option<PathBuf>,
    #[arg(
        long,
        global = true,
        help = "Language filter: stored id or file extension (ts, hpp, py, rs, h, …)"
    )]
    pub(crate) lang: Option<String>,
    /// `fast-unsafe` can corrupt the index on power loss, so it must be
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
        help = "Search default: do not auto-index or refresh (explicit; use --auto-index to opt in)"
    )]
    pub(crate) no_auto_index: bool,
    #[arg(
        long = "auto-index",
        global = true,
        env = "ASGREP_AUTO_INDEX",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::builder::BoolishValueParser::new(),
        help = "Opt in to indexing an empty checkout or refreshing a stale index on search"
    )]
    pub(crate) auto_index: bool,
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

#[derive(Subcommand, Clone)]
pub(crate) enum Commands {
    /// Build or incrementally refresh an index
    #[command(about = "Build or incrementally refresh an index")]
    Index(IndexCmd),
    /// Show index and embedding status
    #[command(about = "Show index and embedding status")]
    Status(RootArg),
    /// List a file's indexed symbols (structure outline)
    #[command(about = "List a file's indexed symbols (structure outline)")]
    Outline(OutlineArgs),
    /// Read bounded source windows for file refs (MCP `code_read` parity)
    #[command(about = "Read bounded source windows for file refs (path, path#L12, path#L12-L40)")]
    Read(ReadArgs),
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
        #[arg(long, default_value = "100", value_parser = clap::value_parser!(u32).range(1..))]
        iterations: u32,
        #[arg(long)]
        suite: Option<String>,
        #[arg(long, default_value = "sample")]
        fixture: String,
        #[arg(long)]
        queries_file: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        skip_index: bool,
        /// Report a warmed-searcher block alongside the cold block, under
        /// `warmed:` history labels (the sticky-session product path).
        #[arg(long, default_value_t = false)]
        warmed: bool,
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
    Keyword(LexicalQueryCmd),
    /// Run embedding-only search
    #[command(about = "Embedding-only semantic search")]
    Semantic(LexicalQueryCmd),
    /// Expand a bounded symbol/caller/import graph
    #[command(about = "Expand a bounded symbol/caller/import graph")]
    Chain(ChainArgs),
    /// Find a bounded directed call path; this does not track values
    #[command(about = "Find a bounded call path (call graph only, not value flow)")]
    CallPath(CallPathArgs),
    /// Find where a symbol is defined
    #[command(about = "Find definitions of a symbol (defs: channel)")]
    Defs(SymbolRootArg),
    /// Find call sites of a symbol
    #[command(about = "Find call sites of a symbol (callers: channel)")]
    Callers(SymbolRootArg),
    /// Find files importing a module
    #[command(about = "Find files importing a module (imports: channel)")]
    Imports(SymbolRootArg),
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
        #[arg(long, env = "ASGREP_NO_EMBED", action = clap::ArgAction::SetTrue,
            value_parser = clap::builder::BoolishValueParser::new(),
            help = "Disable semantic embeddings")]
        no_embed: bool,
    },
    /// Sticky NDJSON session worker — the agent front door (one warm
    /// Searcher, ambient freshness, session memory for a whole program).
    #[command(alias = "codemode-serve", about = "Sticky NDJSON Code Mode worker")]
    Serve {
        #[arg(long, env = "ASGREP_NO_EMBED", action = clap::ArgAction::SetTrue,
            value_parser = clap::builder::BoolishValueParser::new(),
            help = "Disable semantic embeddings")]
        no_embed: bool,
    },
    /// Write stdio MCP config for supported agents (no daemon).
    #[command(about = "Write stdio MCP config for supported agents (no daemon)")]
    Install(crate::install::InstallArgs),
}

impl Commands {
    /// Canonical clap command name (kebab-case; aliases excluded).
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Commands::Index(_) => "index",
            Commands::Status(_) => "status",
            Commands::Outline(_) => "outline",
            Commands::Read(_) => "read",
            Commands::Reindex(_) => "reindex",
            Commands::Codemod(_) => "codemod",
            Commands::Search(_) => "search",
            Commands::Bench { .. } => "bench",
            Commands::Watch { .. } => "watch",
            Commands::Keyword(_) => "keyword",
            Commands::Semantic(_) => "semantic",
            Commands::Chain(_) => "chain",
            Commands::CallPath(_) => "call-path",
            Commands::Defs(_) => "defs",
            Commands::Callers(_) => "callers",
            Commands::Imports(_) => "imports",
            Commands::Capabilities(_) => "capabilities",
            Commands::Version(_) => "version",
            Commands::RobotDocs(_) => "robot-docs",
            Commands::Doctor { .. } => "doctor",
            Commands::Eval(_) => "eval",
            Commands::CodemodeBatch { .. } => "codemode-batch",
            Commands::Serve { .. } => "serve",
            Commands::Install(_) => "install",
        }
    }

    /// Commands sharing the search-tuning plumbing (`--format`,
    /// `--file-filter`, ...): [`SEARCH_FAMILY`](crate::SEARCH_FAMILY) plus the
    /// symbol+ROOT commands defs/callers/imports. Single source for the
    /// flag-scope usage errors in [`validate_cli`](crate::validate_cli) and the
    /// argv-recovery search-channel gates.
    pub(crate) fn is_search_tuning_command(&self) -> bool {
        matches!(
            self,
            Commands::Search(_)
                | Commands::Keyword(_)
                | Commands::Semantic(_)
                | Commands::Defs(_)
                | Commands::Callers(_)
                | Commands::Imports(_)
        )
    }

    /// Canonical names sharing the tuning plumbing, in error-display order.
    /// Keep in sync with [`Commands::is_search_tuning_command`].
    pub(crate) fn search_tuning_command_names() -> &'static [&'static str] {
        &[
            "search",
            "keyword",
            "semantic",
            "defs",
            "callers",
            "imports",
        ]
    }

    /// `search, keyword, semantic, defs, callers, or imports` — derived from
    /// [`Commands::search_tuning_command_names`] so the usage errors can never
    /// drift from the predicate.
    pub(crate) fn search_tuning_commands_display() -> String {
        let names = Self::search_tuning_command_names();
        let (last, rest) = names.split_last().expect("tuning set is non-empty");
        if rest.is_empty() {
            return (*last).to_owned();
        }
        format!("{}, or {last}", rest.join(", "))
    }

    /// Every canonical command name (every [`Commands::name`] arm, same
    /// order). Recovery tables consult this instead of hand lists.
    pub(crate) fn all_names() -> &'static [&'static str] {
        &[
            "index",
            "status",
            "outline",
            "read",
            "reindex",
            "codemod",
            "search",
            "bench",
            "watch",
            "keyword",
            "semantic",
            "chain",
            "call-path",
            "defs",
            "callers",
            "imports",
            "capabilities",
            "version",
            "robot-docs",
            "doctor",
            "eval",
            "codemode-batch",
            "serve",
            "install",
        ]
    }

    /// Search-channel aliases, in error-display order. Single source for
    /// [`Commands::is_search_channel_name`] and the agent typo-recovery
    /// tables so the pair cannot drift from either side.
    pub(crate) fn search_aliases() -> &'static [&'static str] {
        &["find", "query"]
    }

    /// Search-channel names for argv recovery: the tuning set plus the
    /// `find`/`query` search aliases.
    pub(crate) fn is_search_channel_name(name: &str) -> bool {
        Self::search_tuning_command_names().contains(&name)
            || Self::search_aliases().contains(&name)
    }

    /// Commands that always emit machine JSON (used by
    /// [`raw_machine_output_requested`](crate::machine::raw_machine_output_requested)).
    pub(crate) fn is_always_machine_command_name(name: &str) -> bool {
        matches!(name, "capabilities" | "doctor" | "codemode-batch")
    }
}

#[derive(Parser, Clone)]
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

/// An unrecognized durability value is a hard error, never a silent
/// downgrade to a weaker profile.
fn parse_durability(raw: &str) -> Result<ast_sgrep_core::Durability, String> {
    ast_sgrep_core::Durability::parse(raw).ok_or_else(|| {
        format!("unknown durability '{raw}' (expected strict, balanced, or fast-unsafe)")
    })
}

/// A user-typed `--index-path` (or `ASGREP_INDEX_PATH`) resolves against the
/// shell CWD at the arg boundary, so every subcommand agrees on one absolute
/// path. Core's `try_index_db_path` root-joins RELATIVE paths, which is the
/// library contract — the CLI must never hand it a relative one.
fn parse_cwd_absolute_index_path(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| format!("cannot resolve relative --index-path: {error}"))
}

/// m38g: bounded like the other token knobs so a hostile value cannot make the
/// renderer allocate without limit.
fn parse_budget_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_RESPONSE_SNIPPET_TOKENS, "--budget-tokens")
}

fn parse_preview_mode(raw: &str) -> Result<PreviewMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(PreviewMode::None),
        "short" => Ok(PreviewMode::Short),
        "full" => Ok(PreviewMode::Full),
        other => Err(format!(
            "invalid --preview '{other}' (expected none|short|full)"
        )),
    }
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
            .filter(|cand| crate::agent::edit_distance(&lower, cand) <= 2)
            .min_by_key(|cand| crate::agent::edit_distance(&lower, cand)),
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
        self.json || self.active_tuning().format.is_some()
    }

    /// Merge parent (pre-subcommand) tuning with subcommand-scoped tuning (vdqo).
    /// Parent flags like `asgrep --format compact search ...` remain effective.
    pub(crate) fn should_auto_index(&self) -> bool {
        self.auto_index && !self.no_auto_index
    }

    /// Bench `--warmed`: measure the warmed product path as a second block.
    pub(crate) fn bench_warmed(&self) -> bool {
        matches!(
            self.command.as_ref(),
            Some(Commands::Bench { warmed: true, .. })
        )
    }

    pub(crate) fn active_tuning(&self) -> SearchTuning {
        let mut t = self.tuning.clone();
        let overlay = match self.command.as_ref() {
            Some(Commands::Index(c)) => Some(&c.tuning),
            Some(Commands::Reindex(c)) => Some(&c.tuning),
            Some(Commands::Codemod(c)) => Some(&c.tuning),
            Some(Commands::Search(c)) => Some(&c.tuning),
            Some(Commands::Keyword(c) | Commands::Semantic(c)) => Some(&c.tuning),
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
            if o.file_filter.is_some() {
                t.file_filter.clone_from(&o.file_filter);
            }
            if o.budget_tokens.is_some() {
                t.budget_tokens = o.budget_tokens;
            }
            if o.preview.is_some() {
                t.preview.clone_from(&o.preview);
            }
            t.files_with_matches |= o.files_with_matches;
            t.count |= o.count;
        }
        t.no_embed |= matches!(
            self.command.as_ref(),
            Some(
                Commands::CodemodeBatch { no_embed: true, .. } | Commands::Serve { no_embed: true }
            )
        );
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
        self.command
            .as_ref()
            .map_or("search", |command| command.name())
    }
}

#[cfg(test)]
mod command_name_sync_tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    /// Required-arg fixtures so `try_parse_from` succeeds per command.
    fn extra_argv(name: &str) -> &'static [&'static str] {
        match name {
            "outline" => &["f.rs"],
            "read" => &["f.rs#L1"],
            "codemod" => &["--pattern", "p", "--rewrite", "r"],
            "search" | "keyword" | "semantic" | "chain" => &["q"],
            "call-path" => &["a", "b"],
            "defs" | "callers" | "imports" => &["sym"],
            "eval" => &["--gold", "g.json"],
            "codemode-batch" => &["--requests", "-"],
            "install" => &["--target", "cursor"],
            _ => &[],
        }
    }

    #[test]
    fn cli_name_tables_stay_in_sync() {
        // (a) every `Commands::name` arm, in order, equals `all_names()` (24).
        let mut parsed_names = Vec::with_capacity(Commands::all_names().len());
        let mut tuning_names = Vec::new();
        for name in Commands::all_names() {
            let mut argv = vec!["asgrep", name];
            argv.extend_from_slice(extra_argv(name));
            let cli = Cli::try_parse_from(argv)
                .unwrap_or_else(|e| panic!("parse fixture for '{name}' failed: {e}"));
            let command = cli.command.as_ref().expect("subcommand must parse");
            assert_eq!(
                command.name(),
                *name,
                "Commands::name arm drift for '{name}'"
            );
            parsed_names.push(command.name());
            if command.is_search_tuning_command() {
                tuning_names.push(command.name());
            }
        }
        assert_eq!(
            parsed_names,
            Commands::all_names(),
            "Commands::name set/order drift vs all_names()"
        );
        assert_eq!(
            Commands::all_names().len(),
            24,
            "all_names count drift (add the new command to the fixture + tables)"
        );
        let cmd = Cli::command();
        let clap_names: Vec<&str> = cmd.get_subcommands().map(|s| s.get_name()).collect();
        assert_eq!(
            clap_names,
            Commands::all_names(),
            "clap registration order drift vs all_names()"
        );

        // (b) tuning predicate set == tuning name list.
        assert_eq!(
            tuning_names,
            Commands::search_tuning_command_names(),
            "is_search_tuning_command drift vs search_tuning_command_names()"
        );

        // (c) clap alias attrs == search_aliases().
        let cmd = Cli::command();
        let search = cmd
            .get_subcommands()
            .find(|s| s.get_name() == "search")
            .expect("search subcommand registered");
        let mut aliases: Vec<&str> = search.get_aliases().collect();
        aliases.sort_unstable();
        let mut expected: Vec<&str> = Commands::search_aliases().to_vec();
        expected.sort_unstable();
        assert_eq!(
            aliases, expected,
            "search clap aliases drift vs search_aliases()"
        );
    }
}
