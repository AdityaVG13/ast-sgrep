fn main() {
    if matches!(std::env::args().nth(1).as_deref(), Some("-V" | "--version")) {
        println!("asgrep-mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if matches!(std::env::args().nth(1).as_deref(), Some("-h" | "--help")) {
        println!("asgrep-mcp {} — hollow stub", env!("CARGO_PKG_VERSION"));
        return;
    }
    let _ = ast_sgrep_mcp::McpServer::from_env().and_then(|s| s.run_stdio());
}
