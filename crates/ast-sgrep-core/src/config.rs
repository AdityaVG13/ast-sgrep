//! Single `ASGREP_*` environment registry (F4).
//!
//! Every ad-hoc `std::env::var("ASGREP_*")` read that affects search/index
//! behavior parses HERE, once, via [`CoreConfig::from_env`].
//! [`crate::search::Searcher`] and [`crate::index::Indexer`] resolve the
//! config at construction and hand it to their call sites; nothing below construction re-reads process env for these knobs.
//! Parsing is byte-identical to the historical per-site reads (same names,
//! same defaults, same clamps) — only relocated.
//!
//! Process-global instrumentation already centralized in
//! [`crate::env_flag`] (profile stages, memo kill-switch cache) stays there;
//! the [`CoreConfig::memo_off`] / [`CoreConfig::nocache_groups`] mirror lets
//! construction-owned call sites read the frozen value instead of the cache.

use std::path::PathBuf;

/// One resolution of every search/index `ASGREP_*` knob.
#[derive(Debug, Clone)]
pub struct CoreConfig {
    /// `ASGREP_NO_EMBED` unset (boolish false) → embed lane on.
    pub use_embed: bool,
    /// `ASGREP_TANTIVY` boolish → tantivy lane on.
    pub use_tantivy: bool,
    /// `ASGREP_SEMANTIC_ONLY` boolish → skip lexical/symbol candidates.
    pub use_semantic_only: bool,
    /// `ASGREP_ANN_THRESHOLD` parsed usize.
    pub ann_threshold: Option<usize>,
    /// `ASGREP_ANN_PROBES` parsed usize.
    pub ann_probes: Option<usize>,
    /// `ASGREP_LIMIT` parsed then clamped (fallback 16).
    pub default_limit: usize,
    /// `ASGREP_SERIAL_CHANNELS=1` forces the serial channel order.
    pub parallel_channels: bool,
    /// `ASGREP_QEMBED_EXPANDED=1` feeds the repo-PPMI expanded form.
    pub qembed_expanded: bool,
    /// `ASGREP_LEDGER_PATH` telemetry sink.
    pub ledger_path: Option<PathBuf>,
    /// Parsed `ASGREP_NOCACHE` groups (`all` or comma list).
    pub nocache_groups: Vec<String>,
    /// `ASGREP_DISABLE_AST_GREP` in {`1`, `true`, `TRUE`} disables it.
    /// (Exact historical match set — NOT the boolish family.)
    pub external_ast_grep_enabled: bool,
    /// `ASGREP_WALK_THREADS` parsed usize ≥ 1 (fallback 4).
    pub walk_threads: usize,
    /// `ASGREP_ALLOW_AST_GREP` boolish.
    pub allow_ast_grep: bool,
    /// `ASGREP_AST_GREP` absolute-path candidate (validated at use).
    pub ast_grep_path: Option<PathBuf>,
    /// Retired embedding-backend request (`ASGREP_NEURAL_EMBED` /
    /// `ASGREP_EMBED_BACKEND`); `Some(message)` must fail closed.
    pub retired_embed_error: Option<String>,
    /// `ASGREP_WRITE_READ_BUNDLE` boolish (sql-arm bundle control).
    pub write_read_bundle: bool,
}

/// Shared contract behind `Searcher::core_config` / `Indexer::core_config`
/// and their `with_core_config` twins (W1). Both types resolve env knobs
/// ONCE at construction and expose them through this interface; the inherent
/// methods on each type keep their call surfaces and delegate here so the
/// contract (and its docs) lives in exactly one place and the twins cannot
/// drift.
pub(crate) trait CoreConfigHolder: Sized {
    /// Env knobs resolved once at construction. Call sites read from here,
    /// never from process env.
    fn core_config_ref(&self) -> &CoreConfig;
    /// Setter behind [`with_core_config`](CoreConfigHolder::with_core_config).
    fn core_config_mut(&mut self) -> &mut CoreConfig;
    /// Override the construction-resolved config (lets harnesses pin knobs
    /// without touching process env). Consulted lazily where read per query.
    fn with_core_config(mut self, config: CoreConfig) -> Self {
        *self.core_config_mut() = config;
        self
    }
}

impl CoreConfig {
    /// Resolve every knob from process env. THE single read point —
    /// [`crate::search::Searcher::new`]/[`crate::search::Searcher::with_store`]
    /// and [`crate::index::Indexer::new`] call this once at construction.
    pub fn from_env() -> Self {
        Self {
            use_embed: !crate::env_flag::env_flag("ASGREP_NO_EMBED"),
            use_tantivy: crate::env_flag::env_flag("ASGREP_TANTIVY"),
            use_semantic_only: crate::env_flag::env_flag("ASGREP_SEMANTIC_ONLY"),
            ann_threshold: std::env::var("ASGREP_ANN_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok()),
            ann_probes: std::env::var("ASGREP_ANN_PROBES")
                .ok()
                .and_then(|v| v.parse().ok()),
            default_limit: crate::limits::clamp_output_limit(
                std::env::var("ASGREP_LIMIT")
                    .ok()
                    .and_then(|v| v.parse().ok()),
                16,
            ),
            parallel_channels: std::env::var_os("ASGREP_SERIAL_CHANNELS").as_deref()
                != Some(std::ffi::OsStr::new("1")),
            qembed_expanded: std::env::var("ASGREP_QEMBED_EXPANDED").ok().as_deref() == Some("1"),
            ledger_path: std::env::var_os("ASGREP_LEDGER_PATH").map(PathBuf::from),
            nocache_groups: parse_nocache_groups(std::env::var("ASGREP_NOCACHE").ok()),
            external_ast_grep_enabled: !matches!(
                std::env::var("ASGREP_DISABLE_AST_GREP").as_deref(),
                Ok("1") | Ok("true") | Ok("TRUE")
            ),
            walk_threads: std::env::var("ASGREP_WALK_THREADS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|&n| n >= 1)
                .unwrap_or(4),
            allow_ast_grep: crate::env_flag::env_flag("ASGREP_ALLOW_AST_GREP"),
            ast_grep_path: std::env::var("ASGREP_AST_GREP").ok().map(PathBuf::from),
            retired_embed_error: retired_embed_error(),
            write_read_bundle: crate::env_flag::env_flag("ASGREP_WRITE_READ_BUNDLE"),
        }
    }

    /// Measurement kill-switch for process memos. Same rule as
    /// [`crate::env_flag::memo_off`], frozen at construction.
    pub fn memo_off(&self, group: &str) -> bool {
        self.nocache_groups
            .iter()
            .any(|value| value == "all" || value == group)
    }
}

impl Default for CoreConfig {
    /// Compiled defaults — reads NO env (stable value-type constructor).
    fn default() -> Self {
        Self {
            use_embed: true,
            use_tantivy: false,
            use_semantic_only: false,
            ann_threshold: None,
            ann_probes: None,
            default_limit: crate::limits::clamp_output_limit(None, 16),
            parallel_channels: true,
            qembed_expanded: false,
            ledger_path: None,
            nocache_groups: Vec::new(),
            external_ast_grep_enabled: true,
            walk_threads: 4,
            allow_ast_grep: false,
            ast_grep_path: None,
            retired_embed_error: None,
            write_read_bundle: false,
        }
    }
}

/// Parse an `ASGREP_NOCACHE` value into lowercase groups. Shared by
/// [`CoreConfig::from_env`] and the cached [`crate::env_flag::memo_off`]
/// so the rule lives in exactly one place.
pub(crate) fn parse_nocache_groups(value: Option<String>) -> Vec<String> {
    value
        .map(|value| {
            value
                .split(',')
                .map(|part| part.trim().to_ascii_lowercase())
                .filter(|part| !part.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Error text when the environment asks for a retired embedding backend.
/// Single implementation behind both [`CoreConfig::retired_embed_error`]
/// and [`crate::env_flag::retired_embed_request`].
pub(crate) fn retired_embed_error() -> Option<String> {
    if crate::env_flag::env_flag("ASGREP_NEURAL_EMBED") {
        return Some(
            "ASGREP_NEURAL_EMBED was removed: asgrep has one embedding engine (compiled-in static); unset it and re-run `asgrep index` once to migrate old indexes"
                .to_string(),
        );
    }
    if let Ok(backend) = std::env::var("ASGREP_EMBED_BACKEND") {
        let backend = backend.trim();
        if !backend.is_empty() && !backend.eq_ignore_ascii_case("static") {
            return Some(format!(
                "ASGREP_EMBED_BACKEND={backend} was removed: asgrep has one embedding engine (compiled-in static); unset it and re-run `asgrep index` once to migrate old indexes"
            ));
        }
    }
    None
}
