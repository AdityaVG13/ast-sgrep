//! Hollow MCP stub — package kept in workspace for compile/feature gates.

pub struct McpServer;

impl McpServer {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn run_stdio(&self) -> anyhow::Result<()> {
        eprintln!("asgrep-mcp: protocol hollowed; no tools available");
        Ok(())
    }
}
