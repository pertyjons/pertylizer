//! MCP (Model Context Protocol) server for Pertylizer.
//!
//! Provides remote inspection and control of the running synthesizer
//! via the MCP protocol. AI agents can read module graphs, inspect
//! parameters, play notes, and change settings.
//!
//! # Architecture
//!
//! The [`SynthBridge`] trait abstracts over the synth engine, allowing
//! the MCP server to remain independent of engine internals. The
//! [`SynthMcpServer`] implements rmcp's `ServerHandler` and delegates
//! all operations to the bridge.

#![allow(clippy::must_use_candidate)]

pub mod bridge;
pub mod error;
pub mod server;
pub mod types;

pub use bridge::{AnalysisScope, RenderQuality, SpectrumSource, SynthBridge};
pub use error::McpBridgeError;
pub use server::{McpSessionInfo, McpSessionRegistry, SynthMcpServer};
pub use types::*;

use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::service::{RoleServer, RunningService};

/// Serve MCP over stdio (for headless / `--mcp` mode).
///
/// Blocks until the client disconnects.
pub async fn serve_stdio(bridge: Arc<dyn SynthBridge>) -> Result<(), Box<dyn std::error::Error>> {
    let server = SynthMcpServer::new(bridge);
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    tracing::info!(transport = "stdio", "MCP server starting");
    let ct: RunningService<RoleServer, _> = match server.serve(transport).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(transport = "stdio", error = %e, "MCP server failed to start");
            return Err(e.into());
        }
    };
    let result = ct.waiting().await;
    match &result {
        Ok(_) => tracing::info!(transport = "stdio", "MCP stdio client disconnected"),
        Err(e) => {
            tracing::error!(transport = "stdio", error = %e, "MCP stdio session ended with error")
        }
    }
    let _ = result?;
    Ok(())
}

/// Serve MCP over Streamable HTTP on the given port.
///
/// Claude Code connects directly to `http://127.0.0.1:{port}/mcp`.
/// No bridge process needed.
///
/// Uses a [`McpSessionRegistry`] to track connected clients with their identity.
pub async fn serve_http(
    bridge: Arc<dyn SynthBridge>,
    port: u16,
    registry: Option<McpSessionRegistry>,
) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    let ct = tokio_util::sync::CancellationToken::new();

    let service = StreamableHttpService::new(
        move || {
            if let Some(ref registry) = registry {
                Ok(SynthMcpServer::with_registry(
                    Arc::clone(&bridge),
                    registry.clone(),
                ))
            } else {
                Ok(SynthMcpServer::new(Arc::clone(&bridge)))
            }
        },
        Arc::new(LocalSessionManager::default()),
        {
            let mut config = StreamableHttpServerConfig::default();
            config.stateful_mode = true;
            config.cancellation_token = ct.child_token();
            config
        },
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = match tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(port, error = %e, "MCP HTTP server: failed to bind TCP listener");
            return Err(e.into());
        }
    };
    tracing::info!(
        url = %format!("http://127.0.0.1:{port}/mcp"),
        port,
        stateful_mode = true,
        "MCP HTTP server listening"
    );

    let serve_result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            ct.cancelled().await;
        })
        .await;

    match &serve_result {
        Ok(()) => tracing::info!(port, "MCP HTTP server shut down gracefully"),
        Err(e) => tracing::error!(port, error = %e, "MCP HTTP server exited with error"),
    }
    serve_result?;

    Ok(())
}
