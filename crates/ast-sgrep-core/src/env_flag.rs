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
