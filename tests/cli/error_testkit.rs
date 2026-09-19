//! Shared CLI error-API kit for `error_api_pass{1,2,3,4}`.
//!
//! Thin facade: canonical beats live in `ast-sgrep-testkit` (re-exported
//! below); only the binary resolution — `env!("CARGO_BIN_EXE_asgrep")`
//! expands solely in the test target — and its two bin-binding adapters
//! stay target-local.
#![allow(dead_code)] // each pass file uses a subset; the 4 suites jointly use all

use std::path::{Path, PathBuf};
use std::process::Output;

#[allow(unused_imports)] // each pass file uses a subset; the 4 suites jointly use all
pub use ast_sgrep_testkit::{
    assert_failure_envelope, assert_fixture_hits, assert_human_error, assert_human_success,
    assert_no_success_shape, assert_success, corrupt_db_total as corrupt_index_db, dir_listing,
    envelope_shape, fixture_root, index_db_path, parse_stdout, searcher_at_root as lib_searcher,
};

// keep: `env!("CARGO_BIN_EXE_asgrep")` expands only in the test target — the
// testkit-crate fallback cannot see it, so the binary path resolves here.
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

// keep: bin-binding adapter over the canonical `testkit::run` (hermetic env,
// `ASGREP_*` scrub); `bin` is an explicit testkit parameter per the
// cli_recovery convention, resolved here.
pub fn run(args: &[&str]) -> Output {
    ast_sgrep_testkit::run(&bin(), args)
}

// keep: bin-binding adapter over the canonical `testkit::run_index_default`
// (default-state-path index beat); `bin` resolves here.
pub fn index_root(root: &Path) {
    ast_sgrep_testkit::run_index_default(&bin(), root)
}
