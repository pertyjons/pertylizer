//! Happy-path regression tests for the array-accepting MCP mutation tools.
//!
//! Most mutating tools take an array (one-or-many) and the unified `set_*_mixer`
//! tools fold volume/pan/mute/solo (+more) into a single call. These tests
//! exercise the array path end-to-end through the real bridge, asserting both
//! the success summary and the up-front range validation.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_mcp::SynthBridge;
use synth_mcp::SynthMcpServer;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

fn build_server() -> SynthMcpServer {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("ArrayTools")));
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
async fn pattern_placement_full_state_round_trips_and_updates() {
    let server = build_server();
    let created = call(
        &server,
        "create_pattern",
        serde_json::json!({
            "patterns": [{ "name": "Phrase", "length_beats": 4.0 }]
        }),
    )
    .await;
    let pattern_id = serde_json::from_str::<serde_json::Value>(&created).unwrap()["items"][0]["id"]
        .as_u64()
        .expect("pattern id");
    let _ = call(
        &server,
        "create_track",
        serde_json::json!({ "tracks": [{ "name": "A" }, { "name": "B" }] }),
    )
    .await;

    let placed = call(
        &server,
        "place_pattern",
        serde_json::json!({
            "placements": [{
                "pattern_id": pattern_id,
                "track_id": 0,
                "start_tick": 123,
                "transpose_semitones": 7.0,
                "gain": 0.5,
                "length_ticks": 777
            }]
        }),
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(&placed).expect("placement result");
    assert_eq!(result["succeeded"], 1, "{placed}");

    let arrangement = call(&server, "list_arrangement", serde_json::json!({})).await;
    let listed: serde_json::Value = serde_json::from_str(&arrangement).expect("arrangement");
    assert_eq!(listed[0]["start_tick"], 123);
    assert_eq!(listed[0]["transpose_semitones"], 7.0);
    assert_eq!(listed[0]["gain"], 0.5);
    assert_eq!(listed[0]["length_ticks"], 777);
    assert_eq!(listed[0]["effective_length_ticks"], 777);
    assert_eq!(listed[0]["end_tick"], 900);

    let updated = call(
        &server,
        "update_placement",
        serde_json::json!({
            "updates": [{
                "pattern_id": pattern_id,
                "track_id": 0,
                "start_beat": 0.128125,
                "start_tick": 123,
                "new_track_id": 1,
                "new_start_beat": 0.475,
                "new_start_tick": 456,
                "transpose_semitones": -5.0,
                "gain": 1.25,
                "clear_length_override": true
            }]
        }),
    )
    .await;
    let result: serde_json::Value = serde_json::from_str(&updated).expect("update result");
    assert_eq!(result["succeeded"], 1, "{updated}");

    let arrangement = call(&server, "list_arrangement", serde_json::json!({})).await;
    let listed: serde_json::Value = serde_json::from_str(&arrangement).expect("arrangement");
    assert_eq!(listed[0]["track_id"], 1);
    assert_eq!(listed[0]["start_tick"], 456);
    assert_eq!(listed[0]["transpose_semitones"], -5.0);
    assert_eq!(listed[0]["gain"], 1.25);
    assert!(listed[0].get("length_ticks").is_none());
    assert_eq!(listed[0]["effective_length_ticks"], 3840);
}

#[tokio::test]
async fn placement_mutations_report_duplicates_and_missing_items() {
    let server = build_server();
    let created = call(
        &server,
        "create_pattern",
        serde_json::json!({ "patterns": [{ "name": "P", "length_beats": 1.0 }] }),
    )
    .await;
    let pattern_id = serde_json::from_str::<serde_json::Value>(&created).unwrap()["items"][0]["id"]
        .as_u64()
        .expect("pattern id");
    let _ = call(
        &server,
        "create_track",
        serde_json::json!({ "tracks": [{ "name": "T" }] }),
    )
    .await;
    let placement = serde_json::json!({
        "pattern_id": pattern_id,
        "track_id": 0,
        "start_beat": 0.0
    });
    let first = call(
        &server,
        "place_pattern",
        serde_json::json!({ "placements": [placement.clone()] }),
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first).unwrap()["succeeded"],
        1
    );
    let duplicate = call(
        &server,
        "place_pattern",
        serde_json::json!({ "placements": [placement] }),
    )
    .await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&duplicate).unwrap()["failed"],
        1
    );

    let missing = call(
        &server,
        "remove_placement",
        serde_json::json!({
            "placements": [{ "pattern_id": pattern_id, "track_id": 0, "start_tick": 999 }]
        }),
    )
    .await;
    assert!(missing.contains("0 placements removed"), "{missing}");
    assert!(missing.contains("placement not found"), "{missing}");
}

#[tokio::test]
async fn set_song_accepts_complete_tick_addressed_placements() {
    let server = build_server();
    let response = call(
        &server,
        "set_song",
        serde_json::json!({
            "name": "Placed",
            "patterns": [{ "name": "P", "length_beats": 2.0, "notes": [] }],
            "tracks": [{ "name": "T" }],
            "placements": [{
                "pattern_index": 0,
                "track_index": 0,
                "start_tick": 37,
                "transpose_semitones": 12.0,
                "gain": 0.75,
                "length_beats": 0.5
            }]
        }),
    )
    .await;
    assert!(!response.starts_with("Error:"), "{response}");

    let arrangement = call(&server, "list_arrangement", serde_json::json!({})).await;
    let listed: serde_json::Value = serde_json::from_str(&arrangement).expect("arrangement");
    assert_eq!(listed[0]["start_tick"], 37);
    assert_eq!(listed[0]["transpose_semitones"], 12.0);
    assert_eq!(listed[0]["gain"], 0.75);
    assert_eq!(listed[0]["length_ticks"], 480);
}

#[tokio::test]
async fn placement_rejects_contradictory_position_units() {
    let server = build_server();
    let response = call(
        &server,
        "place_pattern",
        serde_json::json!({
            "placements": [{
                "pattern_id": 0,
                "track_id": 0,
                "start_beat": 1.0,
                "start_tick": 961
            }]
        }),
    )
    .await;
    assert!(response.starts_with("Error:"), "{response}");
    assert!(response.contains("different positions"), "{response}");
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

#[tokio::test]
async fn set_track_send_rejects_levels_above_unity() {
    let server = build_server();
    let _ = call(
        &server,
        "create_return_bus",
        serde_json::json!({ "names": ["Reverb"] }),
    )
    .await;
    let _ = call(
        &server,
        "create_track",
        serde_json::json!({ "tracks": [ { "name": "Lead" } ] }),
    )
    .await;

    // Above unity: the send level is capped at 1.0, so this must be rejected
    // with a clear range error — not silently clamped and reported OK.
    let over = call(
        &server,
        "set_track_send",
        serde_json::json!({ "sends": [ { "track_id": 0, "return_id": 0, "level": 1.2 } ] }),
    )
    .await;
    assert!(
        over.contains("level") && !over.starts_with("OK"),
        "over-unity send level must be rejected, got: {over}"
    );

    // Exactly unity is accepted.
    let unity = call(
        &server,
        "set_track_send",
        serde_json::json!({ "sends": [ { "track_id": 0, "return_id": 0, "level": 1.0 } ] }),
    )
    .await;
    assert!(
        unity.starts_with("OK"),
        "unity send level accepted: {unity}"
    );
}
