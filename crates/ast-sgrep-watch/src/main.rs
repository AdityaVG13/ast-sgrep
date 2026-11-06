#![forbid(unsafe_code)]

//! `asgrep-watch`: the `watch` implementation behind the shim in
//! `ast-sgrep-cli` (`asgrep watch` re-execs this binary). This crate owns
//! the `notify` dependency — and with it the CoreFoundation/CoreServices
//! framework link on macOS — so one-shot search never pays that dyld tax.
//! Argv arrives verbatim from the shim and is re-parsed with the identical
//! parser and error boundary (`cli_contract::run_watch_process`, pinned to
//! CLI contract v1 — see `cli_contract`).

mod cli_contract;
mod watch;

fn main() {
    cli_contract::run_watch_process(watch::run_watch)
}
