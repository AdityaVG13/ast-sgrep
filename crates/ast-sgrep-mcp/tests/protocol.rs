//! MCP protocol hollowed — keep crate tests compiling.

#[test]
fn mcp_server_from_env_constructs() {
    let server = ast_sgrep_mcp::McpServer::from_env().expect("from_env");
    let _ = server;
}
