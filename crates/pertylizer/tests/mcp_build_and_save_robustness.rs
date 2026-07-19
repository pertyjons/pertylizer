//! Integration tests for two MCP robustness behaviors:
//!
//! 1. `build_instrument` rolls back a freshly-created instrument when every
//!    requested connection fails (so a fully-failed build leaves no orphan),
//!    but leaves an existing caller-owned instrument (targeted by
//!    `instrument_id`) alone.
//! 2. `save_patch` reports a genuinely unknown instrument clearly. (Its
//!    happy-path save + the snapshot-sync wait are covered by the driven
//!    `save_patch_writes_a_loadable_single_instrument_patch` test in
//!    `project_apply`; the engine here is never driven, so a save of an existing
//!    instrument would only hit the bounded-wait timeout.)

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_engine::{InstrumentId, SynthEngine};
use synth_mcp::SynthBridge;
use synth_mcp::bridge::{BridgeConnectionDef, BridgeInstrumentDef, BridgeModuleDef};
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

fn build_bridge() -> (AppSynthBridge, Arc<SynthSession>) {
    let (_engine, handle) = SynthEngine::new();
    let song = Arc::new(synth_sequencer::SharedSong::new(Song::new("Robustness")));
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
    let bridge = AppSynthBridge::new(Arc::clone(&session), shared, sample_library);
    (bridge, session)
}

/// osc → out, with the single connection using `from_port` as the source port
/// name. Pass a valid `"out"` for a working build, or a bogus name to make the
/// one connection (and thus the whole build's wiring) fail.
fn osc_out(instrument_id: Option<InstrumentId>, from_port: &str) -> BridgeInstrumentDef {
    BridgeInstrumentDef {
        instrument_id,
        name: "Lead".to_string(),
        midi_channel: None,
        volume: None,
        pan: None,
        modules: vec![
            BridgeModuleDef {
                module_type: "osc".to_string(),
                params: vec![],
            },
            BridgeModuleDef {
                module_type: "out".to_string(),
                params: vec![],
            },
        ],
        connections: vec![BridgeConnectionDef {
            from_index: 0,
            from_port: from_port.to_string(),
            to_index: 1,
            to_port: "in_l".to_string(),
        }],
    }
}

#[test]
fn build_instrument_rolls_back_a_new_instrument_when_all_connections_fail() {
    let (bridge, session) = build_bridge();
    // The only connection names a nonexistent source port, so all wiring fails.
    let err = bridge
        .build_instrument(&osc_out(None, "bogus"))
        .expect_err("build should fail when no connection lands");
    assert!(err.to_string().contains("rolled back"), "got: {err}");
    // A fresh session's first created instrument is id 1 (id 0 is the reserved
    // default), so a surviving orphan would be alive at id 1. It must not be.
    assert!(
        !session.instrument_exists(InstrumentId::new(1)),
        "an orphan instrument was left behind"
    );
    // The session is still usable after the rollback: a subsequent valid build
    // produces a live instrument.
    let ok = bridge
        .build_instrument(&osc_out(None, "out"))
        .expect("valid build after a rolled-back one");
    assert!(session.instrument_exists(ok.instrument_id));
}

#[test]
fn build_instrument_keeps_an_existing_instrument_on_all_connections_failed() {
    let (bridge, session) = build_bridge();
    // Create a valid instrument first.
    let ok = bridge
        .build_instrument(&osc_out(None, "out"))
        .expect("valid build");
    let id = ok.instrument_id;
    assert!(session.instrument_exists(id));

    // Rebuild into the SAME id with all-bad connections. The hard-error +
    // rollback only applies to freshly-created instruments; an existing
    // caller-owned instrument returns a normal Ok result (with the failures in
    // `errors`) so callers like rebuild_instrument_preserve_automation still get
    // their report, and the instrument survives.
    let result = bridge
        .build_instrument(&osc_out(Some(id), "bogus"))
        .expect("rebuild of an existing instrument returns Ok even if wiring fails");
    assert_eq!(result.connection_count, 0);
    assert!(!result.errors.is_empty(), "the failed wiring is reported");
    assert!(
        session.instrument_exists(id),
        "an existing caller-owned instrument must survive a failed rebuild"
    );
}

#[test]
fn build_instruments_batch_rolls_back_all_new_instruments_when_one_fails() {
    let (bridge, session) = build_bridge();
    // A two-instrument batch where the first builds fine but the second's only
    // connection can't land. The whole call must fail AND leave no instrument
    // behind — otherwise the first instrument's id is lost and a retry would
    // duplicate it.
    let specs = vec![osc_out(None, "out"), osc_out(None, "bogus")];
    let err = bridge
        .build_instruments(&specs)
        .expect_err("batch must fail when one instrument's wiring all fails");
    assert!(err.to_string().contains("rolled back"), "got: {err}");
    // Ids 1 and 2 would be the two created instruments — both must be gone.
    assert!(
        !session.instrument_exists(InstrumentId::new(1)),
        "first orphaned"
    );
    assert!(
        !session.instrument_exists(InstrumentId::new(2)),
        "second orphaned"
    );
}

#[test]
fn save_patch_reports_an_unknown_instrument() {
    let (bridge, _session) = build_bridge();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("p.json");
    let err = bridge
        .save_patch(InstrumentId::new(999), path.to_str().expect("utf-8 path"))
        .expect_err("save should fail for a nonexistent id");
    assert!(err.to_string().contains("not found"), "got: {err}");
}
