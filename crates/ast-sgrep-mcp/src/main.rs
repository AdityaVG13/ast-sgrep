#![forbid(unsafe_code)]

use anyhow::Context;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 2 {
        match args[1].as_str() {
            "-h" | "--help" => {
                println!("asgrep-mcp {}\nMCP server (stdio) exposing ast-sgrep search, read, and index tools.", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => anyhow::bail!("unexpected argument {:?} (only -h/--help is recognized); the server speaks MCP on stdin/stdout", args[1]),
        }
    }

    let server = ast_sgrep_mcp::McpServer::from_env().context("initialize MCP server")?;
    ast_sgrep_mcp::serve_stdio_blocking(server)
}
