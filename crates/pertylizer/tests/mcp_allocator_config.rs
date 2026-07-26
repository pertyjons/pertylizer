//! Round-trip tests for the `set_allocator_config` MCP tool and the allocator
//! fields it surfaces on `get_instrument_info`.
//!
//! The allocator config (allocation mode, stealing strategy, unison detune /
//! spread, max voices) had no MCP surface at all before this — `get_instrument_info`
//! omitted it and there was no setter. These tests exercise the array path
//! end-to-end through the real bridge *and the real engine*: a mutation is sent
//! as an `EngineCommand`, the engine is driven so it drains the queue and writes
//! fresh instrument snapshots, and the value is then read back via
//! `get_instrument_info`. That verifies the command actually reached the engine,
//! not just the session's optimistic snapshot patch.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor};
use synth_engine::SynthEngine;
use synth_mcp::SynthMcpServer;
use synth_mcp::bridge::*;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

struct Rig {
    engine: SynthEngine,
    server: SynthMcpServer,
}

fn build_rig() -> Rig {
    let (mut engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new(
        "AllocatorConfig",
    )));
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
    let server = SynthMcpServer::new(Arc::new(bridge) as Arc<dyn SynthBridge>);

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate::new(TEST_SR),
        buffer_size: synth_core::BufferSize::new(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    Rig { engine, server }
}

impl Rig {
    /// Drive the engine for a few blocks so it drains queued commands and
    /// refreshes the shared instrument snapshots that `get_instrument_info` reads.
    fn pump(&mut self) {
        let context = AudioCallbackContext {
            sample_rate: HwSampleRate::new(TEST_SR),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        let mut block = vec![0.0f32; 256 * 2];
        for _ in 0..4 {
            block.fill(0.0);
            self.engine.process(&mut block, &context);
        }
    }

    async fn call(&self, tool: &str, params: serde_json::Value) -> String {
        match self.server.dispatch_tool_for_test(tool, params).await {
            Ok(s) => s,
            Err(e) => e,
        }
    }

    /// Create one instrument, drive the engine so it lands in the snapshot, and
    /// return its id.
    async fn make_instrument(&mut self, name: &str) -> u64 {
        let created = self
            .call("create_instrument", serde_json::json!({ "names": [name] }))
            .await;
        let infos: serde_json::Value =
            serde_json::from_str(&created).expect("create_instrument returns JSON");
        let id = infos["created"][0]["id"]
            .as_u64()
            .expect("created instrument id");
        self.pump();
        id
    }

    async fn info(&self, id: u64) -> serde_json::Value {
        let resp = self
            .call(
                "get_instrument_info",
                serde_json::json!({ "instrument_id": id }),
            )
            .await;
        serde_json::from_str(&resp).expect("get_instrument_info returns JSON")
    }
}

#[tokio::test]
async fn get_instrument_info_exposes_allocator_defaults() {
    let mut rig = build_rig();
    let id = rig.make_instrument("Poly").await;
    let inst = rig.info(id).await;

    // Defaults mirror `AllocatorConfig::default()`.
    assert_eq!(inst["allocation_mode"], "Polyphonic", "info: {inst}");
    assert_eq!(inst["stealing_strategy"], "Oldest", "info: {inst}");
    assert_eq!(inst["unison_detune"], 10.0, "info: {inst}");
    assert_eq!(inst["unison_spread"], 0.0, "info: {inst}");
    assert_eq!(inst["max_voices"], 8, "info: {inst}");
}

#[tokio::test]
async fn set_allocator_config_round_trips_every_field() {
    let mut rig = build_rig();
    let id = rig.make_instrument("Unison").await;

    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({
                "items": [{
                    "instrument_id": id,
                    "allocation_mode": "Unison",
                    "stealing_strategy": "Quietest",
                    "unison_detune": 25.0,
                    "unison_spread": 0.5,
                    "max_voices": 16
                }]
            }),
        )
        .await;
    assert!(
        resp.contains("1 allocator configs updated"),
        "expected success summary, got: {resp}"
    );
    rig.pump();

    let inst = rig.info(id).await;
    assert_eq!(inst["allocation_mode"], "Unison", "info: {inst}");
    assert_eq!(inst["stealing_strategy"], "Quietest", "info: {inst}");
    assert_eq!(inst["unison_detune"], 25.0, "info: {inst}");
    assert_eq!(inst["unison_spread"], 0.5, "info: {inst}");
    assert_eq!(inst["max_voices"], 16, "info: {inst}");
}

#[tokio::test]
async fn set_allocator_config_leaves_omitted_fields_unchanged() {
    let mut rig = build_rig();
    let id = rig.make_instrument("Partial").await;

    // Only change the mode; everything else must keep its default.
    let _ = rig
        .call(
            "set_allocator_config",
            serde_json::json!({ "items": [{ "instrument_id": id, "allocation_mode": "Mono" }] }),
        )
        .await;
    rig.pump();

    let inst = rig.info(id).await;
    assert_eq!(inst["allocation_mode"], "Mono", "info: {inst}");
    assert_eq!(inst["stealing_strategy"], "Oldest", "info: {inst}");
    assert_eq!(inst["unison_detune"], 10.0, "info: {inst}");
    assert_eq!(inst["unison_spread"], 0.0, "info: {inst}");
    assert_eq!(inst["max_voices"], 8, "info: {inst}");
}

#[tokio::test]
async fn set_allocator_config_applies_to_multiple_instruments() {
    let mut rig = build_rig();
    let a = rig.make_instrument("A").await;
    let b = rig.make_instrument("B").await;

    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({
                "items": [
                    { "instrument_id": a, "allocation_mode": "Legato" },
                    { "instrument_id": b, "max_voices": 4 }
                ]
            }),
        )
        .await;
    assert!(
        resp.contains("2 allocator configs updated"),
        "expected both applied, got: {resp}"
    );
    rig.pump();

    assert_eq!(rig.info(a).await["allocation_mode"], "Legato");
    assert_eq!(rig.info(b).await["max_voices"], 4);
}

#[tokio::test]
async fn set_allocator_config_reports_per_item_success_and_failure() {
    let mut rig = build_rig();
    let good = rig.make_instrument("Good").await;

    // One valid item, one with a bad enum: the good item applies, the bad one
    // is surfaced as a per-item error (partial success, like set_instrument_mixer).
    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({
                "items": [
                    { "instrument_id": good, "allocation_mode": "Unison" },
                    { "instrument_id": good, "stealing_strategy": "Nope" }
                ]
            }),
        )
        .await;
    assert!(
        resp.contains("1 allocator configs updated"),
        "expected one success, got: {resp}"
    );
    assert!(
        resp.contains("invalid stealing_strategy"),
        "expected the bad item's error surfaced, got: {resp}"
    );
    rig.pump();
    assert_eq!(rig.info(good).await["allocation_mode"], "Unison");
}

#[tokio::test]
async fn set_allocator_config_field_less_item_on_missing_instrument_errors() {
    let rig = build_rig();
    // An item that sets no fields on a nonexistent instrument must NOT report a
    // phantom success — it has to validate the instrument exists.
    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({ "items": [{ "instrument_id": 9999 }] }),
        )
        .await;
    assert!(
        !resp.contains("1 allocator configs updated"),
        "a no-op on a missing instrument must not report success, got: {resp}"
    );
    assert!(
        resp.contains("9999"),
        "expected the missing-instrument error surfaced, got: {resp}"
    );
}

#[tokio::test]
async fn set_allocator_config_rejects_bad_enum_string() {
    let mut rig = build_rig();
    let id = rig.make_instrument("Bad").await;

    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({ "items": [{ "instrument_id": id, "allocation_mode": "Duophonic" }] }),
        )
        .await;
    assert!(
        resp.contains("invalid allocation_mode"),
        "expected enum-validation error, got: {resp}"
    );
    rig.pump();
    // The instrument keeps its default mode.
    assert_eq!(rig.info(id).await["allocation_mode"], "Polyphonic");
}

#[tokio::test]
async fn set_allocator_config_rejects_out_of_range_values() {
    let mut rig = build_rig();
    let id = rig.make_instrument("Range").await;

    // Detune above the 0..100 cent range is rejected before any field is applied.
    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({ "items": [{ "instrument_id": id, "unison_detune": 250.0 }] }),
        )
        .await;
    assert!(
        resp.starts_with("Error:"),
        "expected range error, got: {resp}"
    );

    // max_voices outside 1..128 is likewise rejected.
    let resp = rig
        .call(
            "set_allocator_config",
            serde_json::json!({ "items": [{ "instrument_id": id, "max_voices": 999 }] }),
        )
        .await;
    assert!(
        resp.contains("max_voices must be in 1..=128"),
        "expected max_voices error, got: {resp}"
    );
}
