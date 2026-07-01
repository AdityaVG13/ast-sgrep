use anyhow::Context;

fn main() -> anyhow::Result<()> {
    let server = ast_sgrep_mcp::McpServer::from_env().context("MCP server init failed")?;
    server.run_stdio()
}
