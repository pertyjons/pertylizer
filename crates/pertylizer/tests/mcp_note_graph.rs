//! End-to-end regression for the Note Grid MCP surface: create a pooled graph,
//! add modules, wire them (stream + a Value modulation edge), bind it to a
//! pattern, read it back, and delete it. Exercises the real bridge and the
//! connect-time validation error path.

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
    let song = Arc::new(PlRwLock::new(Song::new("NoteGraph")));
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

/// Whether the given graph's module is a compiled `NoteScriptTransform`.
fn note_script_compiled(song: &Arc<PlRwLock<Song>>, graph_id: u32, module_id: u32) -> bool {
    let song = song.read();
    let graph = song
        .note_graph(synth_sequencer::NoteGraphId::new(graph_id))
        .expect("graph present");
    match graph
        .nodes
        .get(&synth_sequencer::NoteModuleId::new(module_id))
    {
        Some(synth_sequencer::NoteModuleConfig::NoteScriptTransform(t)) => t.is_compiled(),
        _ => false,
    }
}

#[tokio::test]
async fn note_graph_round_trip_through_mcp() {
    let (server, song) = build_server();

    // Create a graph.
    let resp = call(
        &server,
        "create_note_graph",
        serde_json::json!({ "name": "Trance Arp", "color": "#ff8800" }),
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(&resp).expect("create returns JSON");
    let graph_id = parsed["graph_id"].as_u64().expect("graph_id") as u32;

    // Add a Euclidean generator (module 0), a probability gate (module 1), and
    // an LFO (module 2).
    let resp = call(
        &server,
        "add_note_graph_module",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "description": "Generates the pulse stream", "module": { "Euclidean": {
                "steps": 16, "pulses": 8, "rotation": 0,
                "step_len": 240, "pitch": 60, "velocity": 0.7 } } },
            { "graph_id": graph_id, "module": { "ProbabilityGate": {
                "probability": 0.5, "seed": 1 } } },
            { "graph_id": graph_id, "module": { "NoteLfo": {
                "shape": "Sine", "period": 960, "phase": 0.0, "depth": 1.0 } } },
        ] }),
    )
    .await;
    assert!(resp.contains("module 0"), "resp: {resp}");
    assert!(resp.contains("module 1"), "resp: {resp}");
    assert!(resp.contains("module 2"), "resp: {resp}");

    // Wire the stream spine (0→1) and an LFO→gate-threshold Value edge (2→1@0).
    let resp = call(
        &server,
        "connect_note_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 0, "to": 1 },
            { "graph_id": graph_id, "from": 2, "to": 1, "port": "value", "to_input": 0 },
        ] }),
    )
    .await;
    assert!(
        resp.contains('2') && !resp.to_lowercase().contains("error"),
        "resp: {resp}"
    );

    // A stream edge into the LFO must be rejected (it has no stream port).
    let resp = call(
        &server,
        "connect_note_graph",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "from": 1, "to": 2 },
        ] }),
    )
    .await;
    assert!(
        resp.to_lowercase().contains("notestream")
            || resp.to_lowercase().contains("stream port")
            || resp.to_lowercase().contains("failed"),
        "expected a validation error, got: {resp}"
    );

    // Bind the graph to a pattern.
    let pid = {
        let mut s = song.write();
        s.create_pattern(synth_sequencer::Duration(3840)).0
    };
    let resp = call(
        &server,
        "set_pattern_note_graph",
        serde_json::json!({ "items": [ { "pattern_id": pid, "graph_id": graph_id } ] }),
    )
    .await;
    assert!(
        !resp.to_lowercase().contains("error"),
        "bind failed: {resp}"
    );
    assert_eq!(
        song.read()
            .pattern(synth_sequencer::PatternId::new(pid))
            .and_then(synth_sequencer::Pattern::note_graph)
            .map(|g| g.0),
        Some(graph_id)
    );

    // Read the graph back: 3 modules, 2 connections, used by 1 pattern.
    let resp = call(
        &server,
        "get_note_graph",
        serde_json::json!({ "graph_id": graph_id }),
    )
    .await;
    let detail: serde_json::Value = serde_json::from_str(&resp).expect("get returns JSON");
    assert_eq!(detail["info"]["used_by_patterns"], 1);
    assert_eq!(detail["modules"].as_array().unwrap().len(), 3);
    assert_eq!(
        detail["modules"][0]["description"],
        "Generates the pulse stream"
    );
    assert_eq!(detail["connections"].as_array().unwrap().len(), 2);
    assert_eq!(
        detail["info"]["color"]
            .as_str()
            .map(str::to_ascii_lowercase),
        Some("#ff8800".to_string())
    );

    let updated = call(
        &server,
        "set_note_graph_module",
        serde_json::json!({ "items": [{
            "graph_id": graph_id,
            "module_id": 1,
            "description": "Keeps roughly three quarters of events",
            "module": { "ProbabilityGate": { "probability": 0.75, "seed": 1 } }
        }] }),
    )
    .await;
    assert!(!updated.contains("failed"), "update: {updated}");

    let metadata = call(
        &server,
        "set_note_graph_metadata",
        serde_json::json!({ "items": [{
            "graph_id": graph_id,
            "name": "Updated Arp",
            "description": "Updated graph intent",
            "color": "#112233"
        }] }),
    )
    .await;
    assert!(!metadata.contains("failed"), "metadata: {metadata}");

    let bulk = call(
        &server,
        "get_note_graphs",
        serde_json::json!({ "graph_ids": [graph_id, 999] }),
    )
    .await;
    let bulk: serde_json::Value = serde_json::from_str(&bulk).expect("bulk returns JSON");
    assert_eq!(bulk[0]["graph_id"], graph_id);
    assert_eq!(bulk[0]["detail"]["info"]["name"], "Updated Arp");
    assert_eq!(bulk[1]["graph_id"], 999);
    assert!(bulk[1]["error"].is_string());

    // Duplicate it: a fresh id with the same content and layout.
    let resp = call(
        &server,
        "duplicate_note_graph",
        serde_json::json!({ "graph_id": graph_id }),
    )
    .await;
    let dup: serde_json::Value = serde_json::from_str(&resp).expect("duplicate returns JSON");
    let dup_id = dup["graph_id"].as_u64().expect("new graph id") as u32;
    assert_ne!(dup_id, graph_id);
    {
        let song_r = song.read();
        let src = song_r
            .note_graph(synth_sequencer::NoteGraphId::new(graph_id))
            .unwrap();
        let copy = song_r
            .note_graph(synth_sequencer::NoteGraphId::new(dup_id))
            .unwrap();
        assert_eq!(copy.nodes, src.nodes);
        assert_eq!(copy.connections, src.connections);
        assert_eq!(copy.name, format!("{} copy", src.name));
    }

    // Add a NoteScriptTransform (via the generic module tool), then set +
    // compile its source through the dedicated script tool and confirm the live
    // node ends up compiled (an MCP-set script must actually run, not stay
    // pass-through).
    let resp = call(
        &server,
        "add_note_graph_module",
        serde_json::json!({ "items": [
            { "graph_id": graph_id, "module": { "NoteScriptTransform": { "source": "" } } },
        ] }),
    )
    .await;
    // Module ids fill the smallest free slot: 0/1/2 are taken, so this is 3.
    assert!(resp.contains("module 3"), "resp: {resp}");

    let resp = call(
        &server,
        "set_note_graph_script",
        serde_json::json!({
            "graph_id": graph_id,
            "module_id": 3,
            "source": "out.pitch = note_pitch + 12",
        }),
    )
    .await;
    assert!(
        resp.to_lowercase().contains("compiled"),
        "expected a compiled status, got: {resp}"
    );
    assert!(
        note_script_compiled(&song, graph_id, 3),
        "the MCP-set script node must be compiled (not pass-through)"
    );

    // A syntactically invalid source is saved but left pass-through, with the
    // diagnostic surfaced in the status string (never a hard error).
    let resp = call(
        &server,
        "set_note_graph_script",
        serde_json::json!({
            "graph_id": graph_id,
            "module_id": 3,
            "source": "out.pitch = @@@",
        }),
    )
    .await;
    assert!(
        !resp.to_lowercase().starts_with("error"),
        "an invalid source must not be a hard error: {resp}"
    );
    assert!(
        !note_script_compiled(&song, graph_id, 3),
        "an invalid source must leave the node uncompiled (pass-through)"
    );

    // Delete it: the pattern is unbound (falls back to dry playback).
    let resp = call(
        &server,
        "delete_note_graph",
        serde_json::json!({ "graph_ids": [graph_id] }),
    )
    .await;
    assert!(resp.contains("unbound 1"), "resp: {resp}");
    assert_eq!(
        song.read()
            .pattern(synth_sequencer::PatternId::new(pid))
            .and_then(synth_sequencer::Pattern::note_graph),
        None
    );
}

/// The note-scope graph bound to note `nid` in pattern `pid`, as a raw id.
fn note_binding(song: &Arc<PlRwLock<Song>>, pid: u32, nid: u64) -> Option<u32> {
    let s = song.read();
    s.pattern(synth_sequencer::PatternId::new(pid))?
        .notes()
        .iter()
        .find(|n| n.id.0 == nid)?
        .note_graph
        .map(|g| g.0)
}

#[tokio::test]
async fn set_note_note_graph_binds_clears_and_validates() {
    let (server, song) = build_server();
    // A pooled graph + a pattern with one note.
    let (gid, pid, nid) = {
        let mut s = song.write();
        let gid = s.create_note_graph("flam").0;
        let pid = s.create_pattern(synth_sequencer::Duration(3840)).0;
        let nid = s
            .pattern_mut(synth_sequencer::PatternId::new(pid))
            .unwrap()
            .add_note(
                synth_sequencer::PatternTick(0),
                synth_sequencer::Pitch::new(60).unwrap(),
                synth_sequencer::Velocity::MF,
            )
            .0;
        (gid, pid, nid)
    };

    // Bind the note to the graph (per-note articulation).
    let resp = call(
        &server,
        "set_note_note_graph",
        serde_json::json!({ "items": [ { "pattern_id": pid, "note_id": nid, "graph_id": gid } ] }),
    )
    .await;
    assert!(
        !resp.to_lowercase().contains("error"),
        "bind failed: {resp}"
    );
    assert_eq!(note_binding(&song, pid, nid), Some(gid));

    // The binding round-trips through list_notes (read-back over MCP).
    let resp = call(
        &server,
        "list_notes",
        serde_json::json!({ "pattern_id": pid }),
    )
    .await;
    let notes: serde_json::Value = serde_json::from_str(&resp).expect("list_notes returns JSON");
    assert_eq!(
        notes[0]["note_graph"].as_u64(),
        Some(u64::from(gid)),
        "note_graph must read back via list_notes: {resp}"
    );

    // Clear it with null.
    let resp = call(
        &server,
        "set_note_note_graph",
        serde_json::json!({ "items": [ { "pattern_id": pid, "note_id": nid, "graph_id": null } ] }),
    )
    .await;
    assert!(
        !resp.to_lowercase().contains("error"),
        "clear failed: {resp}"
    );
    assert_eq!(note_binding(&song, pid, nid), None);

    // A dangling graph id is rejected, and the binding stays cleared.
    let resp = call(
        &server,
        "set_note_note_graph",
        serde_json::json!({ "items": [ { "pattern_id": pid, "note_id": nid, "graph_id": 9999 } ] }),
    )
    .await;
    assert!(
        resp.to_lowercase().contains("not found") || resp.to_lowercase().contains("error"),
        "dangling graph id should error: {resp}"
    );
    assert_eq!(note_binding(&song, pid, nid), None);

    // An unknown note id is rejected too.
    let resp = call(
        &server,
        "set_note_note_graph",
        serde_json::json!({ "items": [ { "pattern_id": pid, "note_id": 4242, "graph_id": gid } ] }),
    )
    .await;
    assert!(
        resp.to_lowercase().contains("not found") || resp.to_lowercase().contains("error"),
        "unknown note id should error: {resp}"
    );
}

/// The offline note analyzers reflect a pattern's bound note graph (plan §7): a
/// single C4 through a default (major-triad) Chord node reads as three notes,
/// not one — proving both `analyze_pattern` AND `analyze_harmony` expand via
/// `Song::expanded_pattern_notes` rather than the raw authored source.
#[tokio::test]
async fn analyze_reflects_bound_note_graph() {
    let (server, song) = build_server();

    // A pattern with a single C4.
    let pid = {
        let mut s = song.write();
        let pid = s.create_pattern(synth_sequencer::Duration(960)).0;
        let _ = s
            .pattern_mut(synth_sequencer::PatternId::new(pid))
            .expect("pattern")
            .add_note(
                synth_sequencer::PatternTick(0),
                synth_sequencer::Pitch::new(60).expect("pitch"),
                synth_sequencer::Velocity::MF,
            );
        pid
    };

    // Pull a numeric field out of an analyzer's pattern-scope JSON response.
    async fn metric(server: &SynthMcpServer, tool: &str, pid: u32, path: &[&str]) -> i64 {
        let resp = call(server, tool, serde_json::json!({ "pattern_id": pid })).await;
        let mut v: serde_json::Value =
            serde_json::from_str(&resp).unwrap_or_else(|_| panic!("{tool} not JSON: {resp}"));
        for key in path {
            v = v[key].clone();
        }
        v.as_i64()
            .unwrap_or_else(|| panic!("{tool} {path:?} not a number: {resp}"))
    }

    // Unbound: the authored source has one note.
    assert_eq!(
        metric(&server, "analyze_pattern", pid, &["note_count"]).await,
        1
    );
    assert_eq!(
        metric(&server, "analyze_harmony", pid, &["stats", "total_notes"]).await,
        1
    );

    // Bind a default Chord (major-triad) graph.
    let resp = call(
        &server,
        "create_note_graph",
        serde_json::json!({ "name": "triad" }),
    )
    .await;
    let gid = serde_json::from_str::<serde_json::Value>(&resp).unwrap()["graph_id"]
        .as_u64()
        .unwrap() as u32;
    call(
        &server,
        "add_note_graph_module",
        serde_json::json!({ "items": [
            { "graph_id": gid, "module": { "Processor": { "Chord": {} } } },
        ] }),
    )
    .await;
    call(
        &server,
        "set_pattern_note_graph",
        serde_json::json!({ "items": [ { "pattern_id": pid, "graph_id": gid } ] }),
    )
    .await;

    // Bound: both analyzers expand through the graph and see the played triad.
    assert_eq!(
        metric(&server, "analyze_pattern", pid, &["note_count"]).await,
        3,
        "analyze_pattern must expand through the bound graph"
    );
    assert_eq!(
        metric(&server, "analyze_harmony", pid, &["stats", "total_notes"]).await,
        3,
        "analyze_harmony must expand through the bound graph"
    );
}
