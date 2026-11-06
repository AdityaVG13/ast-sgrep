//! Shared environment flag parsing (boolish values).

/// True for `1` / `true` / `yes` / `on` (case-insensitive). Other values are false.
pub fn is_boolish_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Read `name` from the environment; missing or non-true-ish → false.
pub fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .as_deref()
        .is_some_and(is_boolish_true)
}

/// Error text when the environment asks for a retired embedding backend
/// (`ASGREP_NEURAL_EMBED` on, or `ASGREP_EMBED_BACKEND` naming anything but
/// the compiled-in static engine); `None` when the request is servable.
/// Shared by every host surface (CLI validates the flags too, but core must
/// refuse on its own so MCP/LSP/library callers cannot silently serve
/// static vectors under a neural request).
pub fn retired_embed_request() -> Option<String> {
    crate::config::retired_embed_error()
}

/// CLI-latency-gauntlet sub-stage timers (`ASGREP_PROFILE=1`, same gate and
/// `{"stage","ms"}` stderr contract formerly kept in a CLI profiler module,
/// since removed; core keeps its own gate). Cached single read; default-off the hot path pays
/// one branch and zero `Instant::now()` calls.
pub fn profile_stages_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ASGREP_PROFILE").is_some_and(|v| v == "1"))
}

/// Process/total clock: `Some(now)` only when sub-stage profiling, else `None`.
pub fn profile_clock() -> Option<std::time::Instant> {
    profile_stages_enabled().then(std::time::Instant::now)
}

/// Emit one stage line; no-op when profiling is off or the clock is `None`.
pub fn emit_profile_stage(stage: &str, wall: std::time::Duration) {
    if profile_stages_enabled() {
        eprintln!(
            "{{\"stage\":\"{stage}\",\"ms\":{:.3}}}",
            wall.as_secs_f64() * 1000.0
        );
    }
}

/// Emit elapsed-since-`profile_clock()`; no-op on `None`.
pub fn emit_profile_stage_since(stage: &str, since: Option<std::time::Instant>) {
    if let Some(t) = since {
        emit_profile_stage(stage, t.elapsed());
    }
}
/// Measurement kill-switch for process memos (`ASGREP_NOCACHE`).
///
/// `all` or a comma-separated group list (`response,warm,corpus,codemode,render`).
/// Not a product flag: unique-query mill does not count cache hits. Parsed once
/// because each measured arm is its own process.
pub fn memo_off(group: &str) -> bool {
    use std::sync::OnceLock;
    static GROUPS: OnceLock<Vec<String>> = OnceLock::new();
    let groups =
        GROUPS.get_or_init(|| crate::config::CoreConfig::from_env().nocache_groups);
    groups.iter().any(|value| value == "all" || value == group)
}
