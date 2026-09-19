//! Shared binary-path helper for the consolidated CLI oracle suites.
//!
//! The harness itself lives in testkit (`OracleCorpus`, `oracle_run_*`,
//! seed builders); only the binary resolution stays target-local:
//! `env!("CARGO_BIN_EXE_asgrep")` expands only in the test target, never
//! inside the testkit dependency.

use std::path::PathBuf;

// Target-local by construction (Cargo env scoping): keep this 1-liner here.
pub fn asgrep() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}
