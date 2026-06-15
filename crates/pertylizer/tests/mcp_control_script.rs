//! End-to-end tests for the YAMS control-script MCP authoring tool
//! (`set_mod_matrix_script`, S2.3b) through the real bridge + server.

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
    let song = Arc::new(PlRwLock::new(Song::new("Scripts")));
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
async fn set_mod_matrix_script_compiles_clears_and_validates() {
    let server = build_server();

    // An instrument with a Mod Matrix to host the script.
    let created = call(
        &server,
        "create_instrument",
        serde_json::json!({ "names": ["Scripted"] }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&created).expect("create_instrument JSON");
    let inst = v["created"][0]["id"].as_u64().expect("instrument id");
    let add = call(
        &server,
        "add_module",
        serde_json::json!({ "instrument_id": inst, "module_types": ["mmx"] }),
    )
    .await;
    assert!(add.contains("mmx-1"), "add_module: {add}");

    // A valid script installs.
    let ok = call(
        &server,
        "set_mod_matrix_script",
        serde_json::json!({
            "instrument_id": inst, "module_id": "mmx-1", "slot": 1,
            "source": "out = velocity * 0.5"
        }),
    )
    .await;
    assert!(ok.starts_with("OK: installed"), "valid script: {ok}");

    // A syntactically invalid script returns a compile error, not a silent OK.
    let bad = call(
        &server,
        "set_mod_matrix_script",
        serde_json::json!({
            "instrument_id": inst, "module_id": "mmx-1", "slot": 1,
            "source": "out = velocity *"
        }),
    )
    .await;
    assert!(
        bad.contains("Error") && bad.contains("compile"),
        "invalid script: {bad}"
    );

    // An out-of-range slot is rejected up front.
    let oor = call(
        &server,
        "set_mod_matrix_script",
        serde_json::json!({
            "instrument_id": inst, "module_id": "mmx-1", "slot": 99, "source": "out = 1"
        }),
    )
    .await;
    assert!(
        oor.contains("Error") && oor.contains("out of range"),
        "out-of-range slot: {oor}"
    );

    // An empty source clears the slot (distinct from a compile error).
    let cleared = call(
        &server,
        "set_mod_matrix_script",
        serde_json::json!({
            "instrument_id": inst, "module_id": "mmx-1", "slot": 1, "source": ""
        }),
    )
    .await;
    assert!(cleared.starts_with("OK: cleared"), "clear: {cleared}");
}
