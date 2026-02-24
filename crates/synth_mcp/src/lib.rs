//! MCP (Model Context Protocol) server for modular-synth.
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

pub use bridge::SynthBridge;
pub use error::McpBridgeError;
pub use server::SynthMcpServer;
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
    let ct: RunningService<RoleServer, _> = server.serve(transport).await?;
    let _ = ct.waiting().await?;
    Ok(())
}

/// Serve MCP over a TCP listener on the given port.
///
/// Accepts connections concurrently. Returns when the listener is shut down.
pub async fn serve_tcp(
    bridge: Arc<dyn SynthBridge>,
    port: u16,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}")).await?;
    eprintln!("MCP server listening on 127.0.0.1:{port}");

    loop {
        let (stream, addr) = listener.accept().await?;
        eprintln!("MCP client connected from {addr}");

        let server = SynthMcpServer::new(Arc::clone(&bridge));
        let (reader, writer) = stream.into_split();

        match server.serve((reader, writer)).await {
            Ok(ct) => {
                tokio::spawn(async move {
                    if let Err(e) = ct.waiting().await {
                        eprintln!("MCP session error: {e}");
                    }
                    eprintln!("MCP client disconnected: {addr}");
                });
            }
            Err(e) => {
                eprintln!("MCP session setup error: {e}");
            }
        }
    }
}
