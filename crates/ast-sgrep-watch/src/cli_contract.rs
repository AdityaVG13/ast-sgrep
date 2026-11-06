//! Versioned pin of the volatile CLI surface (F30: N1/H4).
//!
//! `asgrep-watch` links `ast-sgrep-cli` for exactly one entry point —
//! [`run_watch_process`], which owns argv parsing, typo recovery, root
//! validation, the `IndexOptions` mapping, error envelopes, and exit
//! classes — so parsing and error contracts cannot drift from the `asgrep`
//! shim that re-execs this binary with verbatim argv.
//!
//! Contract v1: this module is the ONLY place in this crate that may name
//! `ast_sgrep_cli`. Any new CLI item used here must be re-exported through
//! this module, and the exact version pin
//! on `ast-sgrep-cli` in `Cargo.toml` (`=`, not caret) must be re-verified
//! at the same time so a CLI release can never silently widen this surface.
//!
//! [`run_watch_process`]: https://github.com/AdityaVG13/ast-sgrep/blob/main/crates/ast-sgrep-cli/src/lib.rs

pub use ast_sgrep_cli::run_watch_process;
