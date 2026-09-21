#![forbid(unsafe_code)]

//! `asgrep-watch`: the `watch` implementation behind the shim in
//! `ast-sgrep-cli` (`asgrep watch` re-execs this binary). This crate owns
//! the `notify` dependency — and with it the CoreFoundation/CoreServices
//! framework link on macOS — so one-shot search never pays that dyld tax.
//! Argv arrives verbatim from the shim and is re-parsed with the identical
//! parser (`ast_sgrep_cli::parse_watch_launch`).

mod watch;

fn main() -> anyhow::Result<()> {
    let launch = ast_sgrep_cli::parse_watch_launch()?;
    watch::run_watch(launch.opts, launch.debounce_ms)
}
