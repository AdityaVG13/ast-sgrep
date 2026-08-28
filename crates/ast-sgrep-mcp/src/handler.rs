//! rmcp stdio adapter. Tool dispatch stays synchronous in `McpServer`.
//!
//! Search/index run on `spawn_blocking` so the stdio reader can still accept
//! `ping` and `notifications/cancelled` while a tool holds SQLite.

use super::McpServer;
use anyhow::Context;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, ProtocolVersion, ServerCapabilities, ServerInfo,
};
use rmcp::service::{RequestContext, RoleServer, ServerInitializeError};
use rmcp::{ErrorData as McpError, ServiceExt};
use serde_json::Value;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const SUPPORTED_PROTOCOL_VERSIONS: [ProtocolVersion; 2] =
    [ProtocolVersion::V_2024_11_05, ProtocolVersion::V_2025_11_25];

#[derive(Clone)]
pub(crate) struct McpService {
    inner: Arc<McpServer>,
    /// FIFO-ish tool serialization so session maps stay consistent if a client
    /// pipelines `tools/call`. Ping still runs on the tokio reader.
    tool_lock: Arc<tokio::sync::Mutex<()>>,
}

impl McpService {
    pub(crate) fn new(server: McpServer) -> Self {
        Self {
            inner: Arc::new(server),
            tool_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

pub(crate) async fn serve_stdio(server: McpServer) -> anyhow::Result<()> {
    match McpService::new(server)
        .serve(rmcp::transport::stdio())
        .await
    {
        Ok(running) => {
            running
                .waiting()
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            Ok(())
        }
        // EOF before initialize: the client closed stdio; not a server failure.
        Err(ServerInitializeError::ConnectionClosed(_)) => Ok(()),
        Err(error) => Err(anyhow::anyhow!("{error}")).context("serve MCP stdio"),
    }
}

impl ServerHandler for McpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ast-sgrep", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(&SUPPORTED_PROTOCOL_VERSIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        serde_json::from_value(self.inner.tools_catalog())
            .map_err(|error| McpError::internal_error(error.to_string(), None))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let cancel = Arc::new(AtomicBool::new(context.ct.is_cancelled()));
        if !cancel.load(Ordering::Acquire) {
            let flag = cancel.clone();
            let token = context.ct.clone();
            tokio::spawn(async move {
                token.cancelled().await;
                flag.store(true, Ordering::Release);
            });
        }
        let _tool = self.tool_lock.lock().await;
        let inner = self.inner.clone();
        let name = request.name;
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let outcome =
            tokio::task::spawn_blocking(move || inner.dispatch_tool(&name, &arguments, cancel))
                .await
                .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        Ok(match outcome {
            Ok(text) => tool_success(text).into(),
            Err(error) => tool_error(error.to_string()).into(),
        })
    }
}

fn tool_success(text: String) -> CallToolResult {
    let structured = serde_json::from_str(&text).ok();
    let mut result = CallToolResult::success(vec![content_text(&text)]);
    result.structured_content = structured;
    result
}

fn tool_error(message: String) -> CallToolResult {
    CallToolResult::error(vec![content_text(&message)])
}

fn content_text(text: &str) -> ContentBlock {
    ContentBlock::text(text.to_owned())
}
