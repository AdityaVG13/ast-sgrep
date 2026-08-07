//! Production safety guards for the **test harness**.
//!
//! Focus: block accidental prod cloud endpoints / live secrets, and make REAL_*
//! opt-in gates fail loudly instead of soft-skipping into a hidden green.
//!
//! This is **not** the product SSRF allowlist ([`ast_sgrep_embed::embed_url_is_allowed`]).
//! Product code may call allowlisted public APIs; tests must not do so unless a
//! future harness deliberately points at a non-blocklisted endpoint under an
//! explicit opt-in.
//!
//! # Opt-in env
//!
//! | Variable | Role |
//! |----------|------|
//! | `ASGREP_REAL_NETWORK_TESTS=1` | Master switch for non-loopback test network |
//!
//! Boolish spellings match product: `1` / `true` / `yes` / `on` (case-insensitive).
//!
//! # Related product env (inventory only; enforced elsewhere)
//!
//! | Variable | Role |
//! |----------|------|
//! | `ASGREP_NO_EMBED=1` | Disable semantic embed in product search/index paths |
//! | `ASGREP_NO_OLLAMA=1` | Suppress Ollama config from env |
//! | `ASGREP_OLLAMA_EMBED` / `ASGREP_OLLAMA_URL` / `ASGREP_OLLAMA_MODEL` | Opt into Ollama embed |
//! | `ASGREP_CLOUD_EMBED` / `ASGREP_EMBED_API_KEY` / `ASGREP_EMBED_API_URL` / `ASGREP_EMBED_MODEL` | Cloud embed |
//! | `ASGREP_EMBED_URL_ALLOWLIST` / `ASGREP_EMBED_ALLOW_INSECURE_HTTP` | Product HTTP host policy |
//! | `ASGREP_EMBED_FALLBACK` / `ASGREP_NEURAL_FALLBACK` | Acknowledge silent hashed fallback |
//! | `ASGREP_REAL_PI_FIXTURE` | Path to archived Pi corpus (`#[ignore]` + hard expect) |
//!
//! # How zero-run green is avoided
//!
//! - Default: `real_network_tests_enabled()` is **false** — no ambient network.
//! - Live tests should be `#[ignore = "set ASGREP_REAL_NETWORK_TESTS=1"]` (or a
//!   more specific REAL_* flag) and call [`require_real_ready`] at the start.
//! - If a REAL_* gate **is** set but the service/prerequisite is missing,
//!   [`classify_real_gate`] / [`require_real_ready`] return
//!   [`SafetyError::RealServiceUnavailable`] — **never** a soft success.
//! - Production hosts and `sk_live_*`-style keys are rejected even when REAL
//!   network is on ([`assert_test_url_allowed`], [`assert_api_key_not_live`],
//!   [`assert_real_network_env_safe`]).
//!
//! Do **not** write:
//! ```ignore
//! if !real_network_tests_enabled() { return; } // HIDDEN GREEN
//! ```
//! Prefer `#[ignore]` + hard fail when requested-but-unavailable.

use ast_sgrep_core::env_flag::{env_flag, is_boolish_true};
use std::fmt;

/// Master opt-in for non-loopback test network traffic.
pub const REAL_NETWORK_TESTS_ENV: &str = "ASGREP_REAL_NETWORK_TESTS";

/// Production / paid public API hosts that tests must never hit.
///
/// Includes default product cloud hosts (`api.openai.com`, `api.azure.com`) so
/// a mis-set `ASGREP_EMBED_API_URL` cannot become a live billable call from CI.
pub const PRODUCTION_HOST_BLOCKLIST: &[&str] = &[
    "api.openai.com",
    "api.azure.com",
    "api.anthropic.com",
    "generativelanguage.googleapis.com",
    "api.cohere.ai",
    "api.cohere.com",
    "api.voyageai.com",
    "api.pinecone.io",
];

/// Guard failure. Prefer propagating this over soft-skip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyError {
    /// `ASGREP_REAL_NETWORK_TESTS` (or named REAL_* gate) not set.
    RealNetworkNotEnabled {
        env: &'static str,
    },
    /// URL host is on the production blocklist (always hard-fail).
    ProductionHostBlocked {
        host: String,
        url: String,
    },
    /// API key looks like a live/production secret.
    LiveApiKeyRejected {
        kind: &'static str,
    },
    /// REAL_* was requested but the dependency is missing/unreachable.
    RealServiceUnavailable {
        service: String,
        reason: String,
    },
    /// URL could not be parsed far enough to extract a host.
    InvalidUrl {
        detail: String,
    },
}

impl fmt::Display for SafetyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RealNetworkNotEnabled { env } => {
                write!(
                    f,
                    "real network tests disabled; set {env}=1 (and do not soft-skip)"
                )
            }
            Self::ProductionHostBlocked { host, url } => {
                write!(
                    f,
                    "test harness blocked production host {host:?} for URL {url:?}; \
                     use loopback fake server or a non-production endpoint"
                )
            }
            Self::LiveApiKeyRejected { kind } => {
                write!(
                    f,
                    "test harness rejected live/production API key pattern ({kind}); \
                     use a test/fake key, never sk_live_* in tests"
                )
            }
            Self::RealServiceUnavailable { service, reason } => {
                write!(
                    f,
                    "real {service} test requested but unavailable: {reason} \
                     (hard fail — not a soft skip)"
                )
            }
            Self::InvalidUrl { detail } => write!(f, "invalid test URL: {detail}"),
        }
    }
}

impl std::error::Error for SafetyError {}

/// Disposition of a REAL_* gated test body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealGateStatus {
    /// Opt-in not set. Body must not run as a green no-op (use `#[ignore]`).
    NotRequested,
    /// Opt-in set but prerequisite missing → hard fail.
    RequestedUnavailable {
        reason: String,
    },
    /// Opt-in set and ready to exercise the real path.
    Ready,
}

/// Process-env read of [`REAL_NETWORK_TESTS_ENV`].
pub fn real_network_tests_enabled() -> bool {
    env_flag(REAL_NETWORK_TESTS_ENV)
}

/// Pure variant for unit tests (no process env).
pub fn real_network_flag_from(value: Option<&str>) -> bool {
    value.is_some_and(is_boolish_true)
}

/// True for loopback hosts safe for local fake servers / Ollama defaults.
pub fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

/// True if `host` is on [`PRODUCTION_HOST_BLOCKLIST`] (exact, case-insensitive).
pub fn is_production_blocked_host(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    PRODUCTION_HOST_BLOCKLIST
        .iter()
        .any(|blocked| *blocked == host)
}

/// Extract hostname from an `http(s)://` URL (strips userinfo and port).
pub fn host_from_url(url: &str) -> Result<String, SafetyError> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://").ok_or_else(|| SafetyError::InvalidUrl {
        detail: "missing scheme".into(),
    })?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(SafetyError::InvalidUrl {
            detail: format!("scheme {scheme:?} not allowed in tests"),
        });
    }
    let authority = rest
        .split(|c| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or("");
    if authority.is_empty() {
        return Err(SafetyError::InvalidUrl {
            detail: "missing host".into(),
        });
    }
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let host = if hostport.starts_with('[') {
        hostport
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    } else {
        hostport
            .split(':')
            .next()
            .unwrap_or(hostport)
            .to_ascii_lowercase()
    };
    if host.is_empty() {
        return Err(SafetyError::InvalidUrl {
            detail: "empty host".into(),
        });
    }
    Ok(host)
}

/// Pure URL policy for tests.
///
/// 1. Production blocklist → always [`SafetyError::ProductionHostBlocked`].
/// 2. Loopback → allowed (local fake HTTP / Ollama).
/// 3. Any other host → requires `real_network == true`.
pub fn check_test_url(url: &str, real_network: bool) -> Result<(), SafetyError> {
    let host = host_from_url(url)?;
    if is_production_blocked_host(&host) {
        return Err(SafetyError::ProductionHostBlocked {
            host,
            url: url.trim().to_string(),
        });
    }
    if is_loopback_host(&host) {
        return Ok(());
    }
    if !real_network {
        return Err(SafetyError::RealNetworkNotEnabled {
            env: REAL_NETWORK_TESTS_ENV,
        });
    }
    Ok(())
}

/// Env-aware URL check (reads [`REAL_NETWORK_TESTS_ENV`]).
pub fn assert_test_url_allowed(url: &str) -> Result<(), SafetyError> {
    check_test_url(url, real_network_tests_enabled())
}

/// Reject Stripe-style live secret keys and similar `*_live_*` prefixes.
///
/// Empty keys are allowed (caller may treat missing key separately).
pub fn assert_api_key_not_live(key: &str) -> Result<(), SafetyError> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(());
    }
    let lower = key.to_ascii_lowercase();
    // Common live-secret prefixes (Stripe and lookalikes).
    const LIVE_PREFIXES: &[(&str, &str)] = &[
        ("sk_live_", "sk_live_"),
        ("rk_live_", "rk_live_"),
        ("pk_live_", "pk_live_"),
        ("whsec_live_", "whsec_live_"),
    ];
    for (prefix, kind) in LIVE_PREFIXES {
        if lower.starts_with(prefix) {
            return Err(SafetyError::LiveApiKeyRejected { kind });
        }
    }
    // Generic token shape: <letters>_live_<rest>
    if let Some(idx) = lower.find("_live_") {
        if idx > 0 && lower[..idx].bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(SafetyError::LiveApiKeyRejected {
                kind: "*_live_*",
            });
        }
    }
    Ok(())
}

/// Classify a REAL_* gate given whether the dependency is available.
///
/// Pure: does not soft-skip. Callers map [`RealGateStatus::NotRequested`] to
/// `#[ignore]` (or fail if the body was reached without opt-in) and map
/// [`RealGateStatus::RequestedUnavailable`] to a hard failure.
pub fn classify_real_gate(requested: bool, available: bool, unavailable_reason: &str) -> RealGateStatus {
    if !requested {
        return RealGateStatus::NotRequested;
    }
    if available {
        RealGateStatus::Ready
    } else {
        RealGateStatus::RequestedUnavailable {
            reason: unavailable_reason.to_string(),
        }
    }
}

/// Classify using the master network opt-in env flag.
pub fn classify_real_network_gate(available: bool, unavailable_reason: &str) -> RealGateStatus {
    classify_real_gate(
        real_network_tests_enabled(),
        available,
        unavailable_reason,
    )
}

/// Classify using a named env flag (e.g. a future `ASGREP_REAL_OLLAMA=1`).
pub fn classify_real_env_gate(
    env_name: &str,
    available: bool,
    unavailable_reason: &str,
) -> RealGateStatus {
    classify_real_gate(env_flag(env_name), available, unavailable_reason)
}

/// Convert [`RealGateStatus`] into a hard Result (never Ok when unavailable).
///
/// `service` is a short label for error messages (e.g. `"ollama"`, `"network"`).
pub fn require_real_ready(status: RealGateStatus, service: &str) -> Result<(), SafetyError> {
    match status {
        RealGateStatus::Ready => Ok(()),
        RealGateStatus::NotRequested => Err(SafetyError::RealNetworkNotEnabled {
            env: REAL_NETWORK_TESTS_ENV,
        }),
        RealGateStatus::RequestedUnavailable { reason } => {
            Err(SafetyError::RealServiceUnavailable {
                service: service.to_string(),
                reason,
            })
        }
    }
}

/// When REAL network is requested, refuse production hosts / live keys from
/// common ambient embed env vars. No-op when the master opt-in is off.
///
/// Call at the start of live network integration tests so a developer laptop
/// with production `ASGREP_EMBED_API_*` cannot silently bill or exfil.
pub fn assert_real_network_env_safe() -> Result<(), SafetyError> {
    if !real_network_tests_enabled() {
        return Ok(());
    }
    for var in ["ASGREP_EMBED_API_URL", "ASGREP_OLLAMA_URL"] {
        if let Ok(url) = std::env::var(var) {
            if url.trim().is_empty() {
                continue;
            }
            // Always re-check blocklist (loopback ok; prod host fails).
            let host = host_from_url(&url)?;
            if is_production_blocked_host(&host) {
                return Err(SafetyError::ProductionHostBlocked {
                    host,
                    url: url.trim().to_string(),
                });
            }
            // Non-loopback ambient URLs still need the opt-in (already true here).
            check_test_url(&url, true)?;
        }
    }
    if let Ok(key) = std::env::var("ASGREP_EMBED_API_KEY") {
        assert_api_key_not_live(&key)?;
    }
    Ok(())
}

/// Combined guard for a would-be live embed call from tests.
pub fn guard_live_embed_config(url: &str, api_key: Option<&str>) -> Result<(), SafetyError> {
    assert_test_url_allowed(url)?;
    if let Some(key) = api_key {
        assert_api_key_not_live(key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn real_network_opt_in_false_by_default_spellings() {
        assert!(!real_network_flag_from(None));
        assert!(!real_network_flag_from(Some("")));
        assert!(!real_network_flag_from(Some("0")));
        assert!(!real_network_flag_from(Some("false")));
        assert!(!real_network_flag_from(Some("off")));
        assert!(!real_network_flag_from(Some("maybe")));
    }

    #[test]
    fn real_network_opt_in_accepts_boolish_true() {
        for v in ["1", "true", "TRUE", "yes", "on", " Yes "] {
            assert!(real_network_flag_from(Some(v)), "{v}");
        }
    }

    #[test]
    fn production_blocklist_hits_known_public_apis() {
        assert!(is_production_blocked_host("api.openai.com"));
        assert!(is_production_blocked_host("API.OpenAI.COM"));
        assert!(is_production_blocked_host("api.azure.com"));
        assert!(is_production_blocked_host("api.anthropic.com"));
        assert!(!is_production_blocked_host("127.0.0.1"));
        assert!(!is_production_blocked_host("localhost"));
        assert!(!is_production_blocked_host("embed.test.local"));
    }

    #[test]
    fn blocklist_url_always_rejected_even_with_real_network() {
        let url = "https://api.openai.com/v1/embeddings";
        let err = check_test_url(url, true).expect_err("prod host");
        assert!(
            matches!(err, SafetyError::ProductionHostBlocked { .. }),
            "{err}"
        );
        // Without opt-in: still production-blocked (not "not enabled").
        let err = check_test_url(url, false).expect_err("prod host");
        assert!(
            matches!(err, SafetyError::ProductionHostBlocked { .. }),
            "{err}"
        );
    }

    #[test]
    fn loopback_allowed_without_real_network() {
        check_test_url("http://127.0.0.1:11434/api/embeddings", false).unwrap();
        check_test_url("http://localhost:9/", false).unwrap();
        check_test_url("http://[::1]:11434/", false).unwrap();
    }

    #[test]
    fn non_loopback_requires_real_network_opt_in() {
        let url = "https://embeddings.example.com/v1";
        let err = check_test_url(url, false).expect_err("need opt-in");
        assert!(
            matches!(
                err,
                SafetyError::RealNetworkNotEnabled {
                    env: REAL_NETWORK_TESTS_ENV
                }
            ),
            "{err}"
        );
        check_test_url(url, true).unwrap();
    }

    #[test]
    fn rejects_sk_live_style_keys() {
        assert_api_key_not_live("sk_live_abc123secret").expect_err("sk_live");
        assert_api_key_not_live("rk_live_xyz").expect_err("rk_live");
        assert_api_key_not_live("pk_live_pub").expect_err("pk_live");
        assert_api_key_not_live("acme_live_token").expect_err("generic _live_");
        // Test / placeholder keys ok.
        assert_api_key_not_live("sk_test_abc").unwrap();
        assert_api_key_not_live("sk-proj-test-not-stripe-live").unwrap();
        assert_api_key_not_live("test-key").unwrap();
        assert_api_key_not_live("").unwrap();
    }

    #[test]
    fn classify_real_gate_avoids_hidden_green() {
        assert_eq!(
            classify_real_gate(false, false, "down"),
            RealGateStatus::NotRequested
        );
        assert_eq!(
            classify_real_gate(false, true, "down"),
            RealGateStatus::NotRequested
        );
        match classify_real_gate(true, false, "connection refused") {
            RealGateStatus::RequestedUnavailable { reason } => {
                assert_eq!(reason, "connection refused");
            }
            other => panic!("expected RequestedUnavailable, got {other:?}"),
        }
        assert_eq!(
            classify_real_gate(true, true, "ignored"),
            RealGateStatus::Ready
        );
    }

    #[test]
    fn require_real_ready_hard_fails_when_requested_but_unavailable() {
        let status = classify_real_gate(true, false, "ollama not listening");
        let err = require_real_ready(status, "ollama").expect_err("must fail");
        match err {
            SafetyError::RealServiceUnavailable { service, reason } => {
                assert_eq!(service, "ollama");
                assert!(reason.contains("not listening"));
            }
            other => panic!("unexpected {other:?}"),
        }
        // Not requested → also Err (caller must not treat as soft pass).
        let err = require_real_ready(RealGateStatus::NotRequested, "network").unwrap_err();
        assert!(matches!(err, SafetyError::RealNetworkNotEnabled { .. }));
        require_real_ready(RealGateStatus::Ready, "network").unwrap();
    }

    #[test]
    fn guard_live_embed_config_combines_url_and_key() {
        // Loopback + test key, no REAL needed.
        guard_live_embed_config("http://127.0.0.1:9/v1", Some("sk_test_x")).unwrap();
        // Prod URL blocked.
        assert!(matches!(
            guard_live_embed_config(
                "https://api.openai.com/v1/embeddings",
                Some("sk_test_x")
            )
            .unwrap_err(),
            SafetyError::ProductionHostBlocked { .. }
        ));
        // Live key rejected on otherwise ok loopback.
        assert!(matches!(
            guard_live_embed_config("http://127.0.0.1:9/v1", Some("sk_live_real"))
                .unwrap_err(),
            SafetyError::LiveApiKeyRejected { .. }
        ));
    }

    #[test]
    fn invalid_urls_rejected() {
        assert!(matches!(
            host_from_url("not-a-url"),
            Err(SafetyError::InvalidUrl { .. })
        ));
        assert!(matches!(
            host_from_url("file:///etc/passwd"),
            Err(SafetyError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn real_network_env_safe_noop_when_opt_in_off() {
        let _g = env_lock();
        let prev = std::env::var_os(REAL_NETWORK_TESTS_ENV);
        std::env::remove_var(REAL_NETWORK_TESTS_ENV);
        // Even if prod URL is in ambient env, no-op when REAL not requested.
        let prev_url = std::env::var_os("ASGREP_EMBED_API_URL");
        std::env::set_var(
            "ASGREP_EMBED_API_URL",
            "https://api.openai.com/v1/embeddings",
        );
        let result = assert_real_network_env_safe();
        match prev_url {
            Some(v) => std::env::set_var("ASGREP_EMBED_API_URL", v),
            None => std::env::remove_var("ASGREP_EMBED_API_URL"),
        }
        match prev {
            Some(v) => std::env::set_var(REAL_NETWORK_TESTS_ENV, v),
            None => std::env::remove_var(REAL_NETWORK_TESTS_ENV),
        }
        result.expect("should no-op when REAL network off");
    }

    #[test]
    fn real_network_env_safe_fails_on_prod_url_when_opt_in_on() {
        let _g = env_lock();
        let prev_real = std::env::var_os(REAL_NETWORK_TESTS_ENV);
        let prev_url = std::env::var_os("ASGREP_EMBED_API_URL");
        let prev_key = std::env::var_os("ASGREP_EMBED_API_KEY");
        std::env::set_var(REAL_NETWORK_TESTS_ENV, "1");
        std::env::set_var(
            "ASGREP_EMBED_API_URL",
            "https://api.openai.com/v1/embeddings",
        );
        std::env::remove_var("ASGREP_EMBED_API_KEY");
        let result = assert_real_network_env_safe();
        match prev_url {
            Some(v) => std::env::set_var("ASGREP_EMBED_API_URL", v),
            None => std::env::remove_var("ASGREP_EMBED_API_URL"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ASGREP_EMBED_API_KEY", v),
            None => std::env::remove_var("ASGREP_EMBED_API_KEY"),
        }
        match prev_real {
            Some(v) => std::env::set_var(REAL_NETWORK_TESTS_ENV, v),
            None => std::env::remove_var(REAL_NETWORK_TESTS_ENV),
        }
        let err = result.expect_err("prod host with REAL on");
        assert!(
            matches!(err, SafetyError::ProductionHostBlocked { .. }),
            "{err}"
        );
    }

    #[test]
    fn real_network_env_safe_fails_on_live_key_when_opt_in_on() {
        let _g = env_lock();
        let prev_real = std::env::var_os(REAL_NETWORK_TESTS_ENV);
        let prev_url = std::env::var_os("ASGREP_EMBED_API_URL");
        let prev_key = std::env::var_os("ASGREP_EMBED_API_KEY");
        std::env::set_var(REAL_NETWORK_TESTS_ENV, "1");
        // Loopback URL is fine; live key is not.
        std::env::set_var("ASGREP_EMBED_API_URL", "http://127.0.0.1:9/v1");
        std::env::set_var("ASGREP_EMBED_API_KEY", "sk_live_should_never_be_in_tests");
        let result = assert_real_network_env_safe();
        match prev_url {
            Some(v) => std::env::set_var("ASGREP_EMBED_API_URL", v),
            None => std::env::remove_var("ASGREP_EMBED_API_URL"),
        }
        match prev_key {
            Some(v) => std::env::set_var("ASGREP_EMBED_API_KEY", v),
            None => std::env::remove_var("ASGREP_EMBED_API_KEY"),
        }
        match prev_real {
            Some(v) => std::env::set_var(REAL_NETWORK_TESTS_ENV, v),
            None => std::env::remove_var(REAL_NETWORK_TESTS_ENV),
        }
        let err = result.expect_err("live key");
        assert!(
            matches!(err, SafetyError::LiveApiKeyRejected { .. }),
            "{err}"
        );
    }

    #[test]
    fn process_real_network_tests_enabled_matches_env() {
        let _g = env_lock();
        let prev = std::env::var_os(REAL_NETWORK_TESTS_ENV);
        std::env::remove_var(REAL_NETWORK_TESTS_ENV);
        assert!(!real_network_tests_enabled());
        std::env::set_var(REAL_NETWORK_TESTS_ENV, "1");
        assert!(real_network_tests_enabled());
        std::env::set_var(REAL_NETWORK_TESTS_ENV, "0");
        assert!(!real_network_tests_enabled());
        match prev {
            Some(v) => std::env::set_var(REAL_NETWORK_TESTS_ENV, v),
            None => std::env::remove_var(REAL_NETWORK_TESTS_ENV),
        }
    }
}
