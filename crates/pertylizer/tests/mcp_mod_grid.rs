//! End-to-end regression for the Mod Grid MCP surface: create a track-scope
//! graph, assign it to a track, add an LFO source and a Target routing sink,
//! wire them, read the graph back, enumerate provenance, and delete it.
//! Exercises the real bridge and connect-time validation.

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

fn build_server() -> (SynthMcpServer, Arc<PlRwLock<Song>>) {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("ModGrid")));
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
    (
        SynthMcpServer::new(Arc::new(bridge) as Arc<dyn SynthBridge>),
        song,
    )
}

async fn call(server: &SynthMcpServer, tool: &str, params: serde_json::Value) -> String {
    match server.dispatch_tool_for_test(tool, params).await {
        Ok(s) => s,
        Err(e) => e,
    }
}

#[tokio::test]
async fn mod_grid_create_route_enumerate_delete() {
    let (server, song) = build_server();

    // A track to assign the graph to.
    let track_id = u32::from(song.write().create_track("lead").0);

    // 1. Create a track-scope graph.
    let created = call(
        &server,
        "create_mod_graph",
        serde_json::json!({ "name": "wobble", "scope": "track" }),
    )
    .await;
    let graph_id = serde_json::from_str::<serde_json::Value>(&created)
        .unwrap()
        .get("graph_id")
        .and_then(|v| v.as_u64())
        .expect("graph_id in response") as u32;

    let unknown_assignment = call(
        &server,
        "assign_mod_graph",
        serde_json::json!({ "graph_id": graph_id, "tracks": [999] }),
    )
    .await;
    assert!(
        unknown_assignment.contains("track not found: 999"),
        "unknown assignment: {unknown_assignment}"
    );

    // 2. Assign it to the track.
    let assigned = call(
        &server,
        "assign_mod_graph",
        serde_json::json!({ "graph_id": graph_id, "tracks": [track_id] }),
    )
    .await;
    assert!(!assigned.starts_with("Error"), "assign failed: {assigned}");

    let invalid_cc = call(
        &server,
        "add_mod_graph_node",
        serde_json::json!({ "items": [{
            "graph_id": graph_id,
            "node": { "MidiCc": { "cc": 128 } }
        }] }),
    )
    .await;
    assert!(
        invalid_cc.contains("MIDI CC must be in 0..=127"),
        "invalid CC: {invalid_cc}"
    );

    // 3. Add an LFO source (node 0) and a Target sink (node 1).
    let add = call(
        &server,
        "add_mod_graph_node",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "description": "Slow movement source", "node": { "Module": { "module_type": "lfo", "params": {} } } },
            { "graph_id": graph_id, "node": { "Target": { "target": { "Track": { "param": "Volume" } }, "amount": 0.25 } } },
        ] }),
    )
    .await;
    assert!(
        add.contains("node 0") && add.contains("node 1"),
        "add: {add}"
    );

    // 4. Wire the LFO out → Target in.
    let wired = call(
        &server,
        "connect_mod_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 0, "from_port": "out", "to": 1, "to_port": "in" },
        ] }),
    )
    .await;
    assert!(
        !wired.contains("Error") && wired.contains("cables added"),
        "wire: {wired}"
    );

    // 5. Read the graph back: 2 nodes, 1 cable, track scope + assignment.
    let detail = call(
        &server,
        "get_mod_graph",
        serde_json::json!({ "graph_id": graph_id }),
    )
    .await;
    let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(detail["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(detail["nodes"][0]["description"], "Slow movement source");
    assert_eq!(detail["connections"].as_array().unwrap().len(), 1);
    assert_eq!(detail["info"]["scope"], "track");
    assert_eq!(detail["info"]["assigned_tracks"][0], track_id);

    let metadata = call(
        &server,
        "set_mod_graph_metadata",
        serde_json::json!({ "items": [{
            "graph_id": graph_id,
            "name": "updated wobble",
            "description": "Updated graph intent",
            "color": "#334455"
        }] }),
    )
    .await;
    assert!(!metadata.contains("failed"), "metadata: {metadata}");

    let bulk = call(
        &server,
        "get_mod_graph",
        serde_json::json!({ "graph_ids": [graph_id, 999] }),
    )
    .await;
    let bulk: serde_json::Value = serde_json::from_str(&bulk).expect("bulk returns JSON");
    assert_eq!(bulk[0]["detail"]["info"]["name"], "updated wobble");
    assert_eq!(bulk[1]["graph_id"], 999);
    assert!(bulk[1]["error"].is_string());

    // 6. Provenance: exactly one routing sink, on this graph.
    let targets = call(
        &server,
        "list_mod_targets",
        serde_json::json!({ "graph_id": null }),
    )
    .await;
    let targets: serde_json::Value = serde_json::from_str(&targets).unwrap();
    let arr = targets.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["graph_id"], graph_id);
    assert_eq!(arr[0]["amount"], 0.25);

    // 7. A cable from the Target sink (node 1) as a source is rejected.
    let bad = call(
        &server,
        "connect_mod_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 1, "from_port": "out", "to": 0, "to_port": "rate_cv" },
        ] }),
    )
    .await;
    assert!(
        bad.contains("Error") || bad.contains("failed"),
        "expected rejection: {bad}"
    );

    // 8. Delete the graph.
    let deleted = call(
        &server,
        "delete_mod_graph",
        serde_json::json!({ "graph_ids": [graph_id] }),
    )
    .await;
    assert!(deleted.contains("deleted"), "delete: {deleted}");
    let list = call(&server, "list_mod_graphs", serde_json::json!({})).await;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&list)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn mod_grid_set_node_and_disconnect() {
    let (server, _song) = build_server();

    // A global graph: LFO(0) → Target(1) at amount 0.25, wired.
    let created = call(
        &server,
        "create_mod_graph",
        serde_json::json!({ "name": "g" }),
    )
    .await;
    let graph_id = serde_json::from_str::<serde_json::Value>(&created).unwrap()["graph_id"]
        .as_u64()
        .expect("graph_id") as u32;
    call(
        &server,
        "add_mod_graph_node",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "node": { "Module": { "module_type": "lfo", "params": {} } } },
            { "graph_id": graph_id, "node": { "Target": { "target": { "Global": "MasterVolume" }, "amount": 0.25 } } },
        ] }),
    )
    .await;
    call(
        &server,
        "connect_mod_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 0, "from_port": "out", "to": 1, "to_port": "in" },
        ] }),
    )
    .await;

    // set_mod_graph_node: edit the Target's amount in place (0.25 → 0.9).
    let edited = call(
        &server,
        "set_mod_graph_node",
        serde_json::json!({ "items": [{
            "graph_id": graph_id, "node_id": 1, "description": "Master movement sink",
            "node": { "Target": { "target": { "Global": "MasterVolume" }, "amount": 0.9 } }
        }] }),
    )
    .await;
    assert!(!edited.starts_with("Error"), "set failed: {edited}");

    // The amount changed, and the incoming cable was kept.
    let targets = call(
        &server,
        "list_mod_targets",
        serde_json::json!({ "graph_id": null }),
    )
    .await;
    let targets: serde_json::Value = serde_json::from_str(&targets).unwrap();
    assert_eq!(targets[0]["amount"], 0.9, "amount not updated in place");
    let detail = call(
        &server,
        "get_mod_graph",
        serde_json::json!({ "graph_id": graph_id }),
    )
    .await;
    let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(
        detail["connections"].as_array().unwrap().len(),
        1,
        "set_mod_graph_node must keep the node's cables"
    );

    // Editing a missing node fails.
    let missing = call(
        &server,
        "set_mod_graph_node",
        serde_json::json!({ "items": [{
            "graph_id": graph_id, "node_id": 99,
            "node": { "Macro": { "name": "x", "value": 0.0 } }
        }] }),
    )
    .await;
    assert!(
        missing.starts_with("Error"),
        "editing a missing node should fail: {missing}"
    );

    // disconnect_mod_graph: remove the cable, leaving both nodes.
    let cut = call(
        &server,
        "disconnect_mod_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 0, "from_port": "out", "to": 1, "to_port": "in" },
        ] }),
    )
    .await;
    assert!(
        cut.contains("cables removed") && !cut.contains("Error"),
        "disconnect: {cut}"
    );
    let detail = call(
        &server,
        "get_mod_graph",
        serde_json::json!({ "graph_id": graph_id }),
    )
    .await;
    let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(
        detail["connections"].as_array().unwrap().len(),
        0,
        "cable not removed"
    );
    assert_eq!(
        detail["nodes"].as_array().unwrap().len(),
        2,
        "nodes must survive disconnect"
    );

    // Removing a cable that no longer exists reports a per-item error.
    let again = call(
        &server,
        "disconnect_mod_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 0, "from_port": "out", "to": 1, "to_port": "in" },
        ] }),
    )
    .await;
    assert!(
        again.contains("no cable") || again.contains("Error") || again.contains("failed"),
        "expected a missing-cable error: {again}"
    );
}
