//! Happy-path regression tests for the array-accepting MCP mutation tools.
//!
//! Most mutating tools take an array (one-or-many) and the unified `set_*_mixer`
//! tools fold volume/pan/mute/solo (+more) into a single call. These tests
//! exercise the array path end-to-end through the real bridge, asserting both
//! the success summary and the up-front range validation.

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

fn build_server() -> SynthMcpServer {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("ArrayTools")));
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
    let bridge = AppSynthBridge::new(session, shared, sample_library);
    SynthMcpServer::new(Arc::new(bridge) as Arc<dyn SynthBridge>)
}

async fn call(server: &SynthMcpServer, tool: &str, params: serde_json::Value) -> String {
    match server.dispatch_tool_for_test(tool, params).await {
        Ok(s) => s,
        Err(e) => e,
    }
}

#[tokio::test]
async fn create_instrument_accepts_an_array_of_names() {
    let server = build_server();
    let resp = call(
        &server,
        "create_instrument",
        serde_json::json!({ "names": ["Pad", "Bass"] }),
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(&resp)
        .expect("create_instrument should return a JSON object (valid on full + partial success)");
    let created = parsed["created"].as_array().expect("created array");
    assert_eq!(created.len(), 2, "both instruments created: {resp}");
    assert!(
        parsed["errors"].as_array().is_some_and(|a| a.is_empty()),
        "no errors expected: {resp}"
    );
}

#[tokio::test]
async fn set_instrument_mixer_applies_an_array_of_partial_updates() {
    let server = build_server();
    let created = call(
        &server,
        "create_instrument",
        serde_json::json!({ "names": ["Lead"] }),
    )
    .await;
    let infos: serde_json::Value = serde_json::from_str(&created).unwrap();
    let id = infos["created"][0]["id"].as_u64().expect("instrument id");

    // Only the fields present are changed; pan/enabled omitted on the second item.
    let resp = call(
        &server,
        "set_instrument_mixer",
        serde_json::json!({
            "items": [
                { "instrument_id": id, "volume": 0.5, "pan": -0.25, "muted": true },
                { "instrument_id": id, "solo": true }
            ]
        }),
    )
    .await;
    assert!(
        resp.starts_with("OK: 2 instrument mixer updates applied"),
        "expected both updates to apply: {resp}"
    );
}

#[tokio::test]
async fn set_instrument_mixer_rejects_out_of_range_volume_before_applying() {
    let server = build_server();
    let created = call(
        &server,
        "create_instrument",
        serde_json::json!({ "names": ["Lead"] }),
    )
    .await;
    let infos: serde_json::Value = serde_json::from_str(&created).unwrap();
    let id = infos["created"][0]["id"].as_u64().unwrap();

    let resp = call(
        &server,
        "set_instrument_mixer",
        serde_json::json!({ "items": [ { "instrument_id": id, "volume": 9.0 } ] }),
    )
    .await;
    assert!(
        resp.starts_with("Error:"),
        "out-of-range volume rejected: {resp}"
    );
}

#[tokio::test]
async fn note_on_strikes_a_whole_chord_in_one_call() {
    let server = build_server();
    let resp = call(
        &server,
        "note_on",
        serde_json::json!({
            "notes": [
                { "note": 60, "velocity": 100 },
                { "note": 64, "velocity": 100 },
                { "note": 67, "velocity": 100 }
            ]
        }),
    )
    .await;
    assert!(
        resp.starts_with("OK: 3 notes on"),
        "chord triggered: {resp}"
    );
}

#[tokio::test]
async fn set_track_instrument_assigns_and_unassigns_via_null() {
    let server = build_server();
    let created = call(
        &server,
        "create_instrument",
        serde_json::json!({ "names": ["Inst"] }),
    )
    .await;
    let id = serde_json::from_str::<serde_json::Value>(&created).unwrap()["created"][0]["id"]
        .as_u64()
        .unwrap();
    let _ = call(
        &server,
        "create_track",
        serde_json::json!({ "tracks": [ { "name": "T" } ] }),
    )
    .await;

    // Assign, then unassign with an explicit null (the capability the mixer fold dropped).
    let assigned = call(
        &server,
        "set_track_instrument",
        serde_json::json!({ "items": [ { "track_id": 0, "instrument_id": id } ] }),
    )
    .await;
    assert!(assigned.starts_with("OK: 1"), "assign: {assigned}");

    let unassigned = call(
        &server,
        "set_track_instrument",
        serde_json::json!({ "items": [ { "track_id": 0, "instrument_id": null } ] }),
    )
    .await;
    assert!(
        unassigned.starts_with("OK: 1"),
        "unassign via null: {unassigned}"
    );
}

#[tokio::test]
async fn set_track_mixer_applies_across_a_track() {
    let server = build_server();
    // First track in a fresh song is TrackId(0).
    let _ = call(
        &server,
        "create_track",
        serde_json::json!({ "tracks": [ { "name": "Drums" } ] }),
    )
    .await;
    let resp = call(
        &server,
        "set_track_mixer",
        serde_json::json!({
            "items": [ { "track_id": 0, "volume": 0.8, "pan": 0.1, "muted": false } ]
        }),
    )
    .await;
    assert!(
        resp.starts_with("OK: 1 track mixer updates applied"),
        "track mixer applied: {resp}"
    );
}
