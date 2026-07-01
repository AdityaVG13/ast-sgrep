//! Built-in benchmark suites for `asgrep bench` and CI regression gates.

#[derive(Debug, Clone, Copy)]
pub struct BenchCase {
    pub name: &'static str,
    pub query: &'static str,
    pub min_hits: usize,
}

/// Default queries exercised on every fixture repo in `asgrep bench --suite default`.
pub const DEFAULT_SUITE: &[BenchCase] = &[
    BenchCase {
        name: "literal_symbol",
        query: "process_request",
        min_hits: 1,
    },
    BenchCase {
        name: "defs_prefix",
        query: "defs:auth_refresh",
        min_hits: 1,
    },
    BenchCase {
        name: "callers_prefix",
        query: "callers:process_request",
        min_hits: 1,
    },
    BenchCase {
        name: "nl_auth_refresh",
        query: "how does auth refresh work",
        min_hits: 1,
    },
    BenchCase {
        name: "synonym_credential_renewal",
        query: "credential renewal",
        min_hits: 1,
    },
];

pub fn suite_by_name(name: &str) -> Option<&'static [BenchCase]> {
    match name {
        "default" => Some(DEFAULT_SUITE),
        _ => None,
    }
}

pub fn list_suite_names() -> &'static [&'static str] {
    &["default"]
}
