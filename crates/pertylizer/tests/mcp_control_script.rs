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

#[test]
fn script_module_readback_via_get_module_info() {
    // §3.2: a Script (`scr`) module must expose its declared output ports
    // (`out1`..`out8`) even when unconnected, plus the installed scripts per
    // slot — symmetric with set_mod_matrix_script so a client can inspect/diff.
    // Needs a live (pumped) engine so the module reaches the shared graph that
    // get_module_info reads; the server-dispatch rig drops the engine.
    use pertylizer::mcp_bridge::AppSynthBridge;
    use pertylizer::patch::{ModuleBuilder, Patch};
    use synth_core::audio::SampleRate as HwSampleRate;
    use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
    use synth_engine::instrument::InstrumentId;
    use synth_mcp::SynthBridge;

    let (mut engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("ScrReadback")));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });
    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    session
        .add_instrument_with_id(InstrumentId::FIRST, "Test")
        .expect("add instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(44_100),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);
    let ctx = AudioCallbackContext {
        sample_rate: HwSampleRate(44_100),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    let mut block = vec![0.0f32; 256 * 2];
    engine.process(&mut block, &ctx);

    // A Script module with one slot occupied — apply_patch installs the script.
    let mut patch = Patch::new("ScrReadback");
    let mut scr = ModuleBuilder::new(1, ModuleType::Script).build();
    scr.scripts
        .insert("1".to_string(), "out = velocity * 0.5".to_string());
    patch.add_module(scr);
    let _ = session.apply_patch(InstrumentId::FIRST, &patch);
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &ctx);
    }

    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&song)));
    let bridge = AppSynthBridge::new(Arc::clone(&session), shared, sample_library);

    let info = bridge
        .get_module_info(InstrumentId::FIRST.as_u64(), "scr-1")
        .expect("get_module_info scr-1");

    assert!(
        info.output_ports.iter().any(|p| p == "out1")
            && info.output_ports.iter().any(|p| p == "out8"),
        "scr output ports out1..out8 must be listed even unconnected: {:?}",
        info.output_ports
    );
    let scripts = info
        .scripts
        .expect("scripts array present for a Script module");
    assert_eq!(scripts.len(), 1, "one slot installed");
    assert_eq!(scripts[0].slot, 1);
    assert_eq!(scripts[0].output_port, "out1");
    assert_eq!(
        scripts[0].source, "out = velocity * 0.5",
        "installed source must round-trip"
    );

    // §3.1: a Script module has only 8 slots, so a slot past 8 is rejected up
    // front (not silently dropped at the engine like the old 1..=16 check did).
    let err = bridge.set_mod_matrix_script(InstrumentId::FIRST.as_u64(), "scr-1", 9, "out = 1");
    assert!(
        err.is_err(),
        "scr slot 9 must be rejected — only out1..out8 exist"
    );
}

#[test]
fn set_parameters_sets_address_based_mod_matrix_destination() {
    // §3: the batch set_parameter tool used to carry a plain `f32` value, so a Mod
    // Matrix slot destination could only be a legacy `ModDestination` enum index
    // (which has no `spp`). With a string value it now routes through the
    // address-aware single-parameter path, so the routing reads back as the full
    // address `spp-1.x` instead of collapsing to a legacy slot.
    use pertylizer::mcp_bridge::AppSynthBridge;
    use pertylizer::patch::{ModuleBuilder, Patch};
    use synth_core::audio::SampleRate as HwSampleRate;
    use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
    use synth_engine::instrument::InstrumentId;
    use synth_mcp::SynthBridge;
    use synth_mcp::bridge::{BridgeParamSet, BridgeParamValue};

    let (mut engine, handle) = SynthEngine::new();
    let song = Arc::new(PlRwLock::new(Song::new("AddrRoute")));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });
    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    session
        .add_instrument_with_id(InstrumentId::FIRST, "Test")
        .expect("add instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(44_100),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);
    let ctx = AudioCallbackContext {
        sample_rate: HwSampleRate(44_100),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    let mut block = vec![0.0f32; 256 * 2];
    let pump = |engine: &mut SynthEngine, block: &mut Vec<f32>| {
        for _ in 0..16 {
            block.fill(0.0);
            engine.process(block, &ctx);
        }
    };
    pump(&mut engine, &mut block);

    // Instrument with a Mod Matrix + a Spatial Panner (the address target).
    let mut patch = Patch::new("AddrRoute");
    patch.add_module(ModuleBuilder::new(1, ModuleType::ModMatrix).build());
    patch.add_module(ModuleBuilder::new(1, ModuleType::SpatialPanner).build());
    let _ = session.apply_patch(InstrumentId::FIRST, &patch);
    pump(&mut engine, &mut block);

    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&song)));
    let bridge = AppSynthBridge::new(Arc::clone(&session), shared, sample_library);

    // Batch set: an address destination (string) + enable (bool) in one call.
    let result = bridge
        .set_parameters(
            InstrumentId::FIRST.as_u64(),
            &[
                BridgeParamSet {
                    module_id: "mmx-1".to_string(),
                    param_name: "slot_1_dest".to_string(),
                    value: BridgeParamValue::Choice("spp-1.x".to_string()),
                },
                BridgeParamSet {
                    module_id: "mmx-1".to_string(),
                    param_name: "slot_1_enabled".to_string(),
                    value: BridgeParamValue::Bool(true),
                },
            ],
        )
        .expect("set_parameters");
    assert_eq!(
        result.succeeded, 2,
        "both param sets should succeed: {result:?}"
    );
    pump(&mut engine, &mut block);

    let routings = bridge
        .get_mod_matrix_routings(InstrumentId::FIRST.as_u64())
        .expect("get_mod_matrix_routings");
    let slot1 = routings
        .iter()
        .find(|r| r.slot == 1)
        .expect("slot 1 should be present after routing it");
    assert_eq!(
        slot1.destination, "spp-1.x",
        "the address destination must round-trip, not collapse to a legacy slot"
    );
    assert!(slot1.enabled, "slot 1 should be enabled");
}

#[tokio::test]
async fn get_yams_reference_returns_language_docs() {
    // §3.3: the YAMS language reference must be reachable over MCP, not just on
    // disk, so an agent can author scripts without reverse-engineering grammar.
    let server = build_server();
    let doc = call(&server, "get_yams_reference", serde_json::json!({})).await;
    assert!(
        doc.len() > 1000,
        "reference should be the full doc, got {} bytes",
        doc.len()
    );
    for needle in ["YAMS", "out", "src", "arr"] {
        assert!(doc.contains(needle), "reference missing '{needle}'");
    }
}
