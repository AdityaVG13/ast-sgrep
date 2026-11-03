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
    if env_flag("ASGREP_NEURAL_EMBED") {
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

/// Measurement kill-switch for process memos (`ASGREP_NOCACHE`).
///
/// `all` or a comma-separated group list (`response,warm,corpus,codemode,render`).
/// Not a product flag: unique-query mill does not count cache hits. Parsed once
/// because each measured arm is its own process.
pub fn memo_off(group: &str) -> bool {
    use std::sync::OnceLock;
    static GROUPS: OnceLock<Vec<String>> = OnceLock::new();
    let groups = GROUPS.get_or_init(|| {
        std::env::var("ASGREP_NOCACHE")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(|part| part.trim().to_ascii_lowercase())
                    .filter(|part| !part.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    });
    groups.iter().any(|value| value == "all" || value == group)
}
