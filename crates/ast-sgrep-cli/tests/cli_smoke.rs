use ast_sgrep_testkit::CliSession;
use std::path::PathBuf;
fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}
#[test]
fn cli_failure_oracle_preserves_diagnostics() {
    let session = CliSession::sample(asgrep_bin());
    assert!(!session
        .run_failure(&["--definitely-invalid-option"])
        .stderr
        .is_empty());
}
