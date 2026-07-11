use anyhow::Context;
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        match args[1].as_str() {
            "-h" | "--help" => {
                println!(
                    "asgrep-mcp {}\nMCP server for ast-sgrep hybrid code search (stdio).\n\nUSAGE:\n    asgrep-mcp\n\nFLAGS:\n    -h, --help       Print this help and exit\n    -V, --version    Print version and exit",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "-V" | "--version" => {
                println!("asgrep-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {}
        }
    }
    ast_sgrep_mcp::McpServer::from_env()
        .context("MCP server init failed")?
        .run_stdio()
}
