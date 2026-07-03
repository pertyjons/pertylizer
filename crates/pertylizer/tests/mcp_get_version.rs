//! End-to-end test for the `get_version` MCP tool: dispatch → bridge → JSON.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use parking_lot::RwLock as PlRwLock;
use synth_engine::SynthEngine;
use synth_mcp::SynthBridge;
use synth_mcp::SynthMcpServer;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

fn build_bridge() -> AppSynthBridge {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("Headless")));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });
    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&song)));
    AppSynthBridge::new(session, shared, sample_library)
}

#[tokio::test]
async fn get_version_returns_build_info_json() {
    let server = SynthMcpServer::new(Arc::new(build_bridge()) as Arc<dyn SynthBridge>);

    let resp = server
        .dispatch_tool_for_test("get_version", serde_json::json!({}))
        .await
        .expect("get_version must dispatch");
    let info: serde_json::Value = serde_json::from_str(&resp).expect("must return JSON");

    assert_eq!(
        info.get("version").and_then(|v| v.as_str()),
        Some(env!("CARGO_PKG_VERSION"))
    );
    let ts = info
        .get("build_timestamp")
        .and_then(|v| v.as_str())
        .expect("build_timestamp present");
    assert!(ts.ends_with(" UTC"), "timestamp: {ts}");

    // Dev/CI test runs happen inside the git checkout, so the git fields
    // must be populated and consistent.
    let hash = info
        .get("commit_hash")
        .and_then(|v| v.as_str())
        .expect("commit_hash present in a git checkout");
    assert_eq!(hash.len(), 40, "commit hash: {hash}");
    assert!(info.get("branch").is_some_and(|v| v.is_string()));
    assert!(info.get("dirty").is_some_and(serde_json::Value::is_boolean));
}
