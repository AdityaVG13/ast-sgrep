//! FastMCP stdio adapter (fastmcp-rust 0.13, asupersync runtime).
//!
//! Tool dispatch stays synchronous in `McpServer`. Each tool is a
//! `CatalogTool` whose `definition()` is the byte-stable catalog entry from
//! `McpServer::tools_catalog()` — the same JSON source the golden pins, so
//! tool schemas and annotations survive the rmcp -> fastmcp swap unchanged.
//!
//! Cancellation: fastmcp's per-request cancellation domain
//! (`McpContext::request_cancellation`) is bridged onto the core's
//! `AtomicBool` by a short-lived watcher thread per call; the core polls the
//! flag between steps. The watcher exits when the call completes, and call
//! completion signals it through a condvar rather than waiting out its poll
//! interval: a plain `sleep` poll made every tool call pay the full interval
//! as a latency floor (measured 22-28ms for `index_status`, whose own work is
//! ~1ms).
//!
//! Concurrency: all tools share one FIFO `tool_lock`, preserving the previous
//! adapter's contract that a client pipelining `tools/call` sees serialized
//! session-map updates even though fastmcp dispatches modern requests in
//! independent request-owned children.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use asupersync::Cx;
use asupersync::runtime::{RuntimeBuilder, reactor::create_reactor};
use fastmcp_rust::prelude::*;
use fastmcp_rust::{McpRequestCancellation, ResultMeta, ToolErrorKind, ToolHandler};

use super::McpServer;

/// Watches the fastmcp cancellation domain and sets the core's cancel flag.
/// Exits as soon as the flag is set by call completion (flag set = done,
/// regardless of which side set it first).
///
/// `done` is signalled by the dispatching thread the moment the tool returns,
/// so the watcher never holds the call open for the remainder of its poll
/// interval. The interval only bounds how long a *cancellation* takes to
/// reach the core, which is the latency this bridge exists to provide.
fn spawn_cancel_watcher(
    cancellation: McpRequestCancellation,
    cancel: Arc<AtomicBool>,
    done: Arc<(Mutex<bool>, Condvar)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let (finished, wake) = &*done;
        let mut finished = finished.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if *finished || cancel.load(Ordering::Acquire) {
                return;
            }
            if cancellation.is_cancel_requested() {
                cancel.store(true, Ordering::Release);
                return;
            }
            let (guard, _) = wake
                .wait_timeout(finished, Duration::from_millis(20))
                .unwrap_or_else(|e| e.into_inner());
            finished = guard;
        }
    })
}

/// One catalog tool: fixed `definition()` (byte-stable JSON source) plus
/// synchronous dispatch into the shared `McpServer`.
pub(crate) struct CatalogTool {
    tool: Tool,
    server: Arc<McpServer>,
    tool_lock: Arc<Mutex<()>>,
}

impl CatalogTool {
    pub(crate) fn new(
        definition: serde_json::Value,
        server: Arc<McpServer>,
        tool_lock: Arc<Mutex<()>>,
    ) -> anyhow::Result<Self> {
        let tool: Tool = serde_json::from_value(definition)
            .map_err(|error| anyhow::anyhow!("catalog tool definition rejected: {error}"))?;
        Ok(Self {
            tool,
            server,
            tool_lock,
        })
    }

    /// Shared dispatch: serialize calls (FIFO contract), bridge cancellation,
    /// return the raw JSON text the core produced.
    fn dispatch(&self, ctx: &McpContext, arguments: &serde_json::Value) -> Result<String, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let watcher = spawn_cancel_watcher(
            ctx.request_cancellation(),
            cancel.clone(),
            Arc::clone(&done),
        );
        let _guard = self.tool_lock.lock().unwrap_or_else(|e| e.into_inner());
        let outcome = self
            .server
            .dispatch_tool(&self.tool.name, arguments, cancel.clone());
        cancel.store(true, Ordering::Release);
        {
            let (finished, wake) = &*done;
            *finished.lock().unwrap_or_else(|e| e.into_inner()) = true;
            wake.notify_all();
        }
        let _ = watcher.join();
        outcome.map_err(|error| format!("{error:#}"))
    }
}

impl ToolHandler for CatalogTool {
    fn definition(&self) -> Tool {
        self.tool.clone()
    }

    /// Framework-authored tool errors (input validation, handler failure)
    /// must carry structured content that validates against the declared
    /// outputSchema. The truthful value is the miss envelope: the required
    /// fields with a `why` classification and no fabricated results.
    fn final_tool_error_structured_content(
        &self,
        kind: ToolErrorKind,
    ) -> Option<serde_json::Value> {
        let why = match kind {
            ToolErrorKind::InputValidation => "input validation failed",
            ToolErrorKind::Handler => "handler error",
        };
        Some(serde_json::json!({"v": 1, "q": "", "why": why}))
    }

    /// Legacy (initialize-handshake) surface: plain text content; failures
    /// surface as tool errors, exactly as before.
    fn call(&self, ctx: &McpContext, arguments: serde_json::Value) -> McpResult<Vec<Content>> {
        match self.dispatch(ctx, &arguments) {
            Ok(text) => Ok(vec![Content::text(text)]),
            Err(message) => Err(McpError::tool_error(message)),
        }
    }

    /// Modern (2026-07-28) surface: the same text plus `structuredContent`
    /// parsed from it when it is a JSON object — mirroring the structured
    /// output contract the outputSchema declarations promise.
    fn call_final(
        &self,
        ctx: &McpContext,
        arguments: serde_json::Value,
    ) -> McpResult<CompleteResult<FinalCallToolResult>> {
        let outcome = self.dispatch(ctx, &arguments);
        let (text, is_error, structured) = match outcome {
            Ok(text) => {
                let structured = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .filter(|value| value.is_object());
                (text, false, structured)
            }
            // Error results must still validate against the declared
            // outputSchema: the truthful value is the miss envelope with the
            // error classified under `why` and the query echoed when known.
            Err(message) => {
                let q = arguments
                    .get("query")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let structured = if self.tool.output_schema.is_some() {
                    Some(serde_json::json!({"v": 1, "q": q, "why": message}))
                } else {
                    None
                };
                (message, true, structured)
            }
        };
        let content = vec![ContentBlock::Text {
            text,
            annotations: None,
            meta: None,
            additional: BTreeMap::new(),
        }];
        Ok(CompleteResult::new(
            FinalCallToolResult {
                content,
                is_error,
                structured_content: structured,
            },
            ResultMeta::empty(),
        ))
    }
}

/// Serves the stdio transport on the asupersync runtime. Never returns
/// (`run_stdio_with_cx` runs until process exit).
pub(crate) fn serve_stdio(server: McpServer) -> anyhow::Result<()> {
    let server = Arc::new(server);
    let tool_lock = Arc::new(Mutex::new(()));
    let definitions = server
        .tools_catalog()
        .get("tools")
        .and_then(|tools| tools.as_array())
        .ok_or_else(|| anyhow::anyhow!("tools catalog is malformed"))?
        .clone();

    let mut catalog_tools = Vec::with_capacity(definitions.len());
    for definition in definitions {
        catalog_tools.push(CatalogTool::new(
            definition,
            server.clone(),
            tool_lock.clone(),
        )?);
    }

    let runtime = RuntimeBuilder::current_thread()
        .with_reactor(create_reactor().context("create fastmcp I/O reactor")?)
        .blocking_threads(0, 16)
        .build()
        .context("create fastmcp runtime")?;

    runtime.block_on(async {
        let cx = Cx::current().expect("application context");
        let mut builder =
            fastmcp_rust::auto::ServerBuilder::new("ast-sgrep", env!("CARGO_PKG_VERSION"));
        for catalog_tool in catalog_tools {
            builder = builder.tool(catalog_tool);
        }
        builder.build().run_stdio_with_cx(&cx).await
    });
    Ok(())
}
