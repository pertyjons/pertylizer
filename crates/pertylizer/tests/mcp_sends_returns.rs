//! MCP-level round-trip for sends / return busses (Phase 7a).
//!
//! Exercises the thin `SynthBridge` pass-through: create/list/configure return
//! busses, upsert/remove track sends, and confirm deleting a return bus strips
//! the sends that targeted it. Lower-level DSP and serde round-trips are
//! covered in the engine and sequencer crates.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use parking_lot::RwLock as PlRwLock;
use synth_engine::SynthEngine;
use synth_mcp::SynthBridge;
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

#[test]
fn return_bus_and_send_round_trip_via_bridge() {
    let bridge = build_bridge();

    let rid = bridge
        .create_return_bus("Reverb")
        .expect("create return bus");
    let busses = bridge.list_return_busses().expect("list");
    assert_eq!(busses.len(), 1);
    assert_eq!(busses[0].id, rid);
    assert_eq!(busses[0].name, "Reverb");

    bridge.set_return_bus_volume(rid, 0.6).unwrap();
    bridge.set_return_bus_pan(rid, -0.4).unwrap();
    bridge.set_return_bus_mute(rid, true).unwrap();
    bridge.rename_return_bus(rid, "Hall").unwrap();
    let b = bridge.list_return_busses().unwrap().remove(0);
    assert_eq!(b.name, "Hall");
    assert!((b.volume - 0.6).abs() < 1e-6);
    assert!((b.pan - (-0.4)).abs() < 1e-6);
    assert!(b.mute);

    // Send: upsert by target.
    let tid = bridge.create_track("lead", None).expect("create track");
    bridge.set_track_send(tid, rid, 0.5, true).unwrap();
    let t = bridge
        .list_tracks()
        .unwrap()
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    assert_eq!(t.sends.len(), 1);
    assert_eq!(t.sends[0].target, rid);
    assert!((t.sends[0].level - 0.5).abs() < 1e-6);
    assert!(t.sends[0].pre_fader);

    bridge.set_track_send(tid, rid, 0.2, false).unwrap();
    let t = bridge
        .list_tracks()
        .unwrap()
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    assert_eq!(
        t.sends.len(),
        1,
        "second send to same target must update, not duplicate"
    );
    assert!((t.sends[0].level - 0.2).abs() < 1e-6);
    assert!(!t.sends[0].pre_fader);

    // A send to a non-existent return bus is rejected.
    assert!(bridge.set_track_send(tid, 999, 0.5, false).is_err());

    // Deleting the return bus strips the targeting send.
    bridge.delete_return_bus(rid).unwrap();
    assert!(bridge.list_return_busses().unwrap().is_empty());
    let t = bridge
        .list_tracks()
        .unwrap()
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    assert!(
        t.sends.is_empty(),
        "sends to a deleted return bus must be removed"
    );
    assert!(bridge.set_return_bus_volume(rid, 0.5).is_err());
}
