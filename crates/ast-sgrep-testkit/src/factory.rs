//! Realistic multi-file corpus factories for hybrid search tests.
//!
//! Factories write **real source files** into an [`IsolatedIndexSession`] via
//! [`IsolatedIndexSession::write`], then optionally index and build a
//! [`Searcher`]. Content is intentionally multi-module (symbols, calls,
//! imports) so graph + lexical + hashed-embed paths have something to chew on
//! -- not single-line stubs.
//!
//! # Quick start
//!
//! ```ignore
//! use ast_sgrep_testkit::{
//!     factory_corpus_basic_graph, factory_default_index_options,
//!     factory_default_search_options, factory_index_and_searcher,
//!     isolated_index_session,
//! };
//!
//! let session = isolated_index_session();
//! let paths = factory_corpus_basic_graph(&session);
//! let bundle = factory_index_and_searcher(
//!     &session,
//!     factory_default_index_options(&session),
//!     factory_default_search_options(&session),
//! );
//! assert!(bundle.stats.files_indexed >= paths.len());
//! ```

use crate::isolation::IsolatedIndexSession;
use ast_sgrep_core::{
    EmbedBackend, IndexOptions, IndexStats, Indexer, SearchOptions, Searcher,
};
use std::path::Path;

/// Relative paths written by [`factory_corpus_basic_graph`].
pub const BASIC_GRAPH_FILES: &[&str] = &[
    "src/lib.rs",
    "src/auth.rs",
    "src/handlers.rs",
    "src/billing/mod.rs",
    "src/util.py",
];

/// Relative paths written by [`factory_corpus_credential_theme`].
pub const CREDENTIAL_THEME_FILES: &[&str] = &[
    "src/main.rs",
    "src/token_store.rs",
    "src/auth_refresh.py",
    "src/session_manager.py",
    "src/middleware.ts",
];

/// Indexed corpus + live [`Searcher`] produced by [`factory_index_and_searcher`].
pub struct FactoryIndexBundle {
    pub indexer: Indexer,
    pub searcher: Searcher,
    /// Stats from the index pass (`files_indexed`, `symbols_extracted`, …).
    pub stats: IndexStats,
}

/// Offline-friendly defaults: force reindex, hashed semantic embed (no network).
///
/// Sets isolation-safe `root` / `index_path` from the session. Callers may
/// override other fields with struct update syntax after calling this.
pub fn factory_default_index_options(session: &IsolatedIndexSession) -> IndexOptions {
    IndexOptions {
        force_reindex: true,
        embed_semantic: true,
        // Semantic backend is the local hashed embedder (no cloud/ollama).
        embed_backend: EmbedBackend::Semantic,
        ..session.index_options()
    }
}

/// Offline-friendly search defaults: embed on, modest limit.
pub fn factory_default_search_options(session: &IsolatedIndexSession) -> SearchOptions {
    SearchOptions {
        use_embed: true,
        limit: 16,
        ..session.search_options()
    }
}

/// Multi-module Rust + Python corpus with caller/callee chains and imports.
///
/// Layout:
/// - `src/lib.rs` -- crate root re-exporting `auth`, `handlers`, `billing`
/// - `src/auth.rs` -- `verify_token` / `Authenticator` (called by handlers)
/// - `src/handlers.rs` -- request pipeline calling into auth + billing
/// - `src/billing/mod.rs` -- nested module with charge path
/// - `src/util.py` -- Python helpers with an internal call chain
///
/// Returns the relative paths written (also available as [`BASIC_GRAPH_FILES`]).
pub fn factory_corpus_basic_graph(session: &IsolatedIndexSession) -> Vec<&'static str> {
    session.write(
        "src/lib.rs",
        r#"//! Factory basic-graph crate root.
pub mod auth;
pub mod handlers;
pub mod billing;

pub use auth::{verify_token, Authenticator};
pub use handlers::{handle_request, RequestContext};

/// Entry used by integration-style call graphs.
pub fn bootstrap() -> Result<(), String> {
    let auth = Authenticator::new("factory-secret");
    handle_request(RequestContext {
        path: "/health".into(),
        bearer: Some("tok-bootstrap".into()),
    }, &auth)
}
"#,
    );

    session.write(
        "src/auth.rs",
        r#"//! Authentication helpers for the factory graph corpus.

/// Validates a bearer token string.
pub fn verify_token(token: &str) -> bool {
    !token.is_empty() && token.starts_with("tok-")
}

/// Small auth service used by request handlers.
pub struct Authenticator {
    secret: String,
}

impl Authenticator {
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
        }
    }

    pub fn authorize(&self, token: &str) -> Result<(), String> {
        if !verify_token(token) {
            return Err("invalid token".into());
        }
        if token.contains(&self.secret) {
            return Ok(());
        }
        // Accept factory tokens that merely look well-formed.
        if token.len() >= 8 {
            Ok(())
        } else {
            Err("token too short".into())
        }
    }

    pub fn secret_fingerprint(&self) -> usize {
        self.secret.len()
    }
}
"#,
    );

    session.write(
        "src/handlers.rs",
        r#"//! HTTP-ish handlers that form a clear caller → callee chain.

use crate::auth::{verify_token, Authenticator};
use crate::billing;

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub path: String,
    pub bearer: Option<String>,
}

/// Top-level request entry: auth gate then billing side-effect.
pub fn handle_request(ctx: RequestContext, auth: &Authenticator) -> Result<(), String> {
    let token = ctx.bearer.as_deref().unwrap_or("");
    guard_auth(token, auth)?;
    if ctx.path.starts_with("/charge") {
        billing::charge_customer("cust-factory", 42)?;
    }
    Ok(())
}

fn guard_auth(token: &str, auth: &Authenticator) -> Result<(), String> {
    if !verify_token(token) {
        return Err("missing bearer".into());
    }
    auth.authorize(token)
}

/// Leaf helper so symbol search has another def to land on.
pub fn route_label(path: &str) -> String {
    format!("route:{path}")
}
"#,
    );

    session.write(
        "src/billing/mod.rs",
        r#"//! Billing submodule for nested-path graph coverage.

/// Record a charge against a customer id.
pub fn charge_customer(customer_id: &str, amount_cents: u64) -> Result<(), String> {
    if customer_id.is_empty() {
        return Err("empty customer".into());
    }
    let receipt = build_receipt(customer_id, amount_cents);
    persist_receipt(&receipt);
    Ok(())
}

fn build_receipt(customer_id: &str, amount_cents: u64) -> String {
    format!("{customer_id}:{amount_cents}")
}

fn persist_receipt(receipt: &str) {
    let _ = receipt;
}
"#,
    );

    session.write(
        "src/util.py",
        r#"""Utility helpers (Python side of the basic graph corpus)."""

from typing import Optional


def normalize_path(path: str) -> str:
    if not path:
        raise ValueError("empty path")
    return path if path.startswith("/") else f"/{path}"


def build_context(path: str, bearer: Optional[str] = None) -> dict:
    route = normalize_path(path)
    return {"path": route, "bearer": bearer}


def dispatch(path: str, bearer: Optional[str] = None) -> str:
    ctx = build_context(path, bearer)
    return f"dispatched:{ctx['path']}"
"#,
    );

    assert_files_on_disk(session, BASIC_GRAPH_FILES);
    BASIC_GRAPH_FILES.to_vec()
}

/// Credential-renewal themed multi-language corpus (Rust / Python / TypeScript).
///
/// Mirrors vocabulary from the shared sample fixture (`auth_refresh`,
/// `fetch_token`, `store_token`, credential renewal prose) so hybrid lexical +
/// hashed-semantic queries have realistic surface area. Includes a short
/// caller/callee chain across modules.
///
/// Returns the relative paths written (also available as [`CREDENTIAL_THEME_FILES`]).
pub fn factory_corpus_credential_theme(session: &IsolatedIndexSession) -> Vec<&'static str> {
    session.write(
        "src/main.rs",
        r#"//! Credential renewal entrypoints (factory theme).
mod token_store;

use token_store::{store_token, TokenStore};

fn main() {
    let _ = process_request("hello");
    auth_refresh();
}

fn process_request(input: &str) -> String {
    validate_input(input);
    format!("processed: {input}")
}

fn validate_input(input: &str) {
    if input.is_empty() {
        panic!("empty input");
    }
}

/// Renew the credential before the current session expires.
fn auth_refresh() {
    let token = fetch_token();
    store_token(token.clone());
    let mut store = TokenStore::new();
    store.rotate(token);
}

fn fetch_token() -> String {
    "tok-credential-renewal".to_string()
}
"#,
    );

    session.write(
        "src/token_store.rs",
        r#"//! In-memory token store used by auth_refresh.

pub fn store_token(token: String) {
    let _ = token;
}

pub struct TokenStore {
    current: Option<String>,
}

impl TokenStore {
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Rotate the active credential after a successful refresh.
    pub fn rotate(&mut self, token: String) {
        self.current = Some(token);
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}
"#,
    );

    session.write(
        "src/auth_refresh.py",
        r#"""Python credential renewal helpers (factory theme)."""

from session_manager import SessionManager, persist_session_token


def fetch_token() -> str:
    return "tok-credential-renewal"


def store_token(token: str) -> None:
    _ = token


def auth_refresh() -> str:
    """Renew the credential before the current session expires."""
    token = fetch_token()
    store_token(token)
    mgr = SessionManager()
    mgr.attach_token(token)
    persist_session_token(token)
    return token


def main() -> None:
    process_request("hello")
    auth_refresh()


def process_request(input: str) -> str:
    validate_input(input)
    return f"processed: {input}"


def validate_input(input: str) -> None:
    if not input:
        raise ValueError("empty input")
"#,
    );

    session.write(
        "src/session_manager.py",
        r#"""Session side of credential renewal."""


def persist_session_token(token: str) -> None:
    """Write the renewed credential into the session store."""
    _ = token


class SessionManager:
    def __init__(self) -> None:
        self.token: str | None = None

    def attach_token(self, token: str) -> None:
        self.token = token

    def has_credential(self) -> bool:
        return bool(self.token)
"#,
    );

    session.write(
        "src/middleware.ts",
        r#"// TypeScript credential middleware for multi-lang hybrid search.

export type AuthContext = {
  bearer?: string;
  path: string;
};

/** Fetch a short-lived access token for credential renewal. */
export function fetchToken(): string {
  return "tok-credential-renewal";
}

/** Persist the renewed credential before the session expires. */
export function storeToken(token: string): void {
  void token;
}

export function authRefresh(): string {
  const token = fetchToken();
  storeToken(token);
  return token;
}

export function withAuth(ctx: AuthContext): AuthContext {
  if (!ctx.bearer) {
    return { ...ctx, bearer: authRefresh() };
  }
  return ctx;
}
"#,
    );

    assert_files_on_disk(session, CREDENTIAL_THEME_FILES);
    CREDENTIAL_THEME_FILES.to_vec()
}

/// Index the session corpus and build a [`Searcher`] with the given options.
///
/// Forces isolation-safe `root` / `index_path` on both option structs (same
/// contract as [`IsolatedIndexSession::index_all`] / [`IsolatedIndexSession::searcher`]).
pub fn factory_index_and_searcher(
    session: &IsolatedIndexSession,
    index_opts: IndexOptions,
    search_opts: SearchOptions,
) -> FactoryIndexBundle {
    let mut indexer = session.indexer(index_opts);
    let stats = indexer.index_all().expect("factory index_all");
    let searcher = session.searcher(search_opts);
    FactoryIndexBundle {
        indexer,
        searcher,
        stats,
    }
}

/// Convenience: write basic-graph corpus, index, and return the search bundle.
pub fn factory_ready_basic_graph(session: &IsolatedIndexSession) -> FactoryIndexBundle {
    factory_corpus_basic_graph(session);
    factory_index_and_searcher(
        session,
        factory_default_index_options(session),
        factory_default_search_options(session),
    )
}

/// Convenience: write credential-theme corpus, index, and return the search bundle.
pub fn factory_ready_credential_theme(session: &IsolatedIndexSession) -> FactoryIndexBundle {
    factory_corpus_credential_theme(session);
    factory_index_and_searcher(
        session,
        factory_default_index_options(session),
        factory_default_search_options(session),
    )
}

fn assert_files_on_disk(session: &IsolatedIndexSession, rels: &[&str]) {
    for rel in rels {
        let path = session.corpus_root.join(rel);
        assert!(
            path.is_file(),
            "factory expected on-disk file missing: {}",
            path.display()
        );
        let meta = std::fs::metadata(&path).expect("metadata");
        assert!(
            meta.len() > 0,
            "factory wrote empty file: {}",
            path.display()
        );
    }
}

/// Count regular files under a directory (recursive). Used by tests.
pub fn count_files_under(root: &Path) -> usize {
    fn walk(dir: &Path, acc: &mut usize) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, acc);
            } else if path.is_file() {
                *acc += 1;
            }
        }
    }
    let mut n = 0;
    walk(root, &mut n);
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolation::isolated_index_session;

    #[test]
    fn basic_graph_writes_real_on_disk_files() {
        let session = isolated_index_session();
        let paths = factory_corpus_basic_graph(&session);
        assert_eq!(paths.len(), BASIC_GRAPH_FILES.len());
        assert_eq!(count_files_under(&session.corpus_root), paths.len());
        for rel in &paths {
            let body = std::fs::read_to_string(session.corpus_root.join(rel)).expect("read");
            assert!(
                body.len() > 40,
                "{rel} should be realistic source, got {} bytes",
                body.len()
            );
        }
        // Graph-ish content markers present on disk.
        let handlers =
            std::fs::read_to_string(session.corpus_root.join("src/handlers.rs")).unwrap();
        assert!(handlers.contains("handle_request"));
        assert!(handlers.contains("verify_token"));
        assert!(handlers.contains("charge_customer"));
    }

    #[test]
    fn credential_theme_writes_real_multi_lang_files() {
        let session = isolated_index_session();
        let paths = factory_corpus_credential_theme(&session);
        assert_eq!(paths.len(), CREDENTIAL_THEME_FILES.len());
        assert_eq!(count_files_under(&session.corpus_root), paths.len());
        let main_rs = std::fs::read_to_string(session.corpus_root.join("src/main.rs")).unwrap();
        assert!(main_rs.contains("auth_refresh"));
        assert!(main_rs.contains("fetch_token"));
        let py = std::fs::read_to_string(session.corpus_root.join("src/auth_refresh.py")).unwrap();
        assert!(py.contains("credential") || py.contains("Renew the credential"));
        let ts = std::fs::read_to_string(session.corpus_root.join("src/middleware.ts")).unwrap();
        assert!(ts.contains("authRefresh") || ts.contains("fetchToken"));
    }

    #[test]
    fn basic_graph_indexes_with_nonzero_symbol_and_file_counts() {
        let session = isolated_index_session();
        factory_corpus_basic_graph(&session);
        let bundle = factory_index_and_searcher(
            &session,
            factory_default_index_options(&session),
            factory_default_search_options(&session),
        );

        assert!(
            bundle.stats.files_indexed > 0,
            "files_indexed={}",
            bundle.stats.files_indexed
        );
        assert!(
            bundle.stats.symbols_extracted > 0,
            "symbols_extracted={}",
            bundle.stats.symbols_extracted
        );
        assert!(
            bundle.stats.callers_extracted > 0,
            "callers_extracted={} (expected call sites)",
            bundle.stats.callers_extracted
        );

        let status = bundle.indexer.store().status().expect("status");
        assert!(status.file_count > 0, "file_count={}", status.file_count);
        assert!(
            status.symbol_count > 0,
            "symbol_count={}",
            status.symbol_count
        );
        assert!(
            session.index_path.is_file(),
            "expected real on-disk db at {}",
            session.index_path.display()
        );

        // Lexical/symbol search finds a known factory symbol.
        let resp = bundle
            .searcher
            .search("handle_request")
            .expect("search handle_request");
        assert!(
            resp.hits
                .iter()
                .any(|h| h.excerpt.contains("handle_request")
                    || h.symbol.as_deref() == Some("handle_request")),
            "expected hit for handle_request: {:?}",
            resp.hits
        );
    }

    #[test]
    fn credential_theme_indexes_and_finds_auth_refresh() {
        let session = isolated_index_session();
        let bundle = factory_ready_credential_theme(&session);

        assert!(bundle.stats.files_indexed >= CREDENTIAL_THEME_FILES.len());
        assert!(bundle.stats.symbols_extracted > 0);

        let status = bundle.indexer.store().status().expect("status");
        assert!(status.file_count > 0);
        assert!(status.symbol_count > 0);

        let resp = bundle
            .searcher
            .search("auth_refresh")
            .expect("search auth_refresh");
        assert!(
            !resp.hits.is_empty(),
            "credential theme should surface auth_refresh: {:?}",
            resp.hits
        );
    }

    #[test]
    fn factory_ready_basic_graph_is_searchable() {
        let session = isolated_index_session();
        let bundle = factory_ready_basic_graph(&session);
        assert!(bundle.stats.files_indexed > 0);
        assert!(bundle.stats.symbols_extracted > 0);
        let resp = bundle
            .searcher
            .search("charge_customer")
            .expect("search charge_customer");
        assert!(!resp.hits.is_empty(), "{:?}", resp.hits);
    }
}
