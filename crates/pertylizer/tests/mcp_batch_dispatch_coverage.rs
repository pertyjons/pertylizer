//! Guard test for `batch_execute` dispatch coverage (TODO §6.2).
//!
//! Every `#[tool]` is registered with rmcp's tool-router for standalone calls,
//! but `batch_execute` routes through a *separate* hand-maintained dispatch
//! table (`dispatch_tool_inner`). A tool added to the router without a matching
//! dispatch arm silently fails with `unknown tool` only via batch — exactly the
//! bug fixed in 00845fb. This test enumerates the router's registered tools and
//! asserts each is reachable through the dispatch table, so the two cannot drift.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_mcp::SynthMcpServer;
use synth_mcp::bridge::*;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

/// Tools intentionally absent from the batch dispatch table:
/// - `batch_execute` must not nest inside itself.
/// - `preview_note` answers a base64 `audio/wav` content block, which the batch
///   path cannot carry — it renders each op to one result value.
///
/// `analyze_note` used to be exempt for the same stated reason, but it never
/// returned audio: it hand-serialized a plain `AnalyzeNoteResult` into a text
/// block. Typed, it dispatches like any other reader.
const BATCH_EXEMPT: &[&str] = &["batch_execute", "preview_note"];

fn build_bridge() -> AppSynthBridge {
    build_bridge_with_shared().0
}

fn build_bridge_with_shared() -> (AppSynthBridge, Arc<McpSharedState>) {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("Headless")));
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
    // Keep the engine handle alive for the duration of the bridge via the
    // session (which holds the command sender + shared state Arc).
    (
        AppSynthBridge::new(session, Arc::clone(&shared), sample_library),
        shared,
    )
}

#[tokio::test]
async fn every_router_tool_is_reachable_via_batch_dispatch() {
    let server = SynthMcpServer::new(Arc::new(build_bridge()) as Arc<dyn SynthBridge>);

    let names = server.router_tool_names();
    assert!(
        !names.is_empty(),
        "tool-router reported no registered tools"
    );

    let mut missing = Vec::new();
    for name in &names {
        if BATCH_EXEMPT.contains(&name.as_str()) {
            continue;
        }
        // A scalar params value fails deserialization for every tool's
        // parameter struct, so no tool body runs (no side effects, no blocking
        // project-IO). The only observable signal is `unknown tool` when the
        // dispatch table has no arm for this name.
        let resp = server
            .dispatch_tool_for_test(name, serde_json::json!(0))
            .await;
        let text = match resp {
            Ok(s) => s,
            Err(e) => e,
        };
        if text.contains("unknown tool") {
            missing.push(name.clone());
        }
    }

    assert!(
        missing.is_empty(),
        "tools registered with the rmcp router but missing from the \
         batch_execute dispatch table: {missing:?}"
    );

    // The exemptions must remain real router tools; otherwise the list is stale.
    for ex in BATCH_EXEMPT {
        assert!(
            names.iter().any(|n| n == ex),
            "exempt tool '{ex}' is no longer registered with the router — \
             update BATCH_EXEMPT"
        );
    }
}

#[tokio::test]
async fn v2_status_is_reachable_and_batch_polling_does_not_count_mutations() {
    let (bridge, shared) = build_bridge_with_shared();
    let server = SynthMcpServer::new(Arc::new(bridge) as Arc<dyn SynthBridge>);
    let before = shared
        .mutation_seq
        .load(std::sync::atomic::Ordering::Acquire);
    for _ in 0..3 {
        let response = server
            .dispatch_tool_for_test("get_v2_playback_status", serde_json::json!({}))
            .await
            .unwrap();
        let status: synth_mcp::v2_playback::V2PlaybackStatus =
            serde_json::from_str(&response).unwrap();
        assert!(matches!(
            status.observation,
            synth_mcp::v2_playback::PlaybackObservation::Unavailable { .. }
        ));
    }
    let response = server
        .batch_execute_for_test(serde_json::json!({"operations":[
            {"tool":"get_v2_playback_status","params":{}},
            {"tool":"get_v2_playback_status","params":{}}
        ]}))
        .await;
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(response["succeeded"], 2);
    assert_eq!(
        shared
            .mutation_seq
            .load(std::sync::atomic::Ordering::Acquire),
        before
    );
}
