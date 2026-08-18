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

