//! MCP-level round-trip for sends / return busses (Phase 7a).
//!
//! Exercises the thin `SynthBridge` pass-through: create/list/configure return
//! busses, upsert/remove track sends, and confirm deleting a return bus strips
//! the sends that targeted it. Lower-level DSP and serde round-trips are
//! covered in the engine and sequencer crates.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_mcp::bridge::*;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

fn build_bridge() -> AppSynthBridge {
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

    bridge.set_return_bus_volume(rid, 0.6.into()).unwrap();
    bridge.set_return_bus_pan(rid, (-0.4).into()).unwrap();
    bridge.set_return_bus_mute(rid, true).unwrap();
    bridge.rename_return_bus(rid, "Hall").unwrap();
    let b = bridge.list_return_busses().unwrap().remove(0);
    assert_eq!(b.name, "Hall");
    assert!((b.volume.as_f32() - 0.6).abs() < 1e-6);
    assert!((b.pan.as_f32() - (-0.4)).abs() < 1e-6);
    assert!(b.mute);

    // Send: upsert by target.
    let tid = bridge.create_track("lead", None).expect("create track");
    bridge
        .set_track_send(tid, rid, 0.5.into(), true, true)
        .unwrap();
    let t = bridge
        .list_tracks()
        .unwrap()
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    assert_eq!(t.sends.len(), 1);
    assert_eq!(t.sends[0].target, rid);
    assert!((t.sends[0].level.as_f32() - 0.5).abs() < 1e-6);
    assert!(t.sends[0].pre_fader);
    assert!(t.sends[0].enabled, "a fresh send is enabled");

    bridge
        .set_track_send(tid, rid, 0.2.into(), false, true)
        .unwrap();
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
    assert!((t.sends[0].level.as_f32() - 0.2).abs() < 1e-6);
    assert!(!t.sends[0].pre_fader);

    // Bypassing keeps the send (and its level/tap) but flips `enabled`.
    bridge
        .set_track_send(tid, rid, 0.2.into(), false, false)
        .unwrap();
    let t = bridge
        .list_tracks()
        .unwrap()
        .into_iter()
        .find(|t| t.id == tid)
        .unwrap();
    assert_eq!(t.sends.len(), 1, "bypass must not remove the send");
    assert!((t.sends[0].level.as_f32() - 0.2).abs() < 1e-6);
    assert!(!t.sends[0].enabled, "send is now bypassed");

    // A send to a non-existent return bus is rejected.
    assert!(
        bridge
            .set_track_send(
                tid,
                synth_sequencer::ReturnBusId(999),
                0.5.into(),
                false,
                true
            )
            .is_err()
    );

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
    assert!(bridge.set_return_bus_volume(rid, 0.5.into()).is_err());
}

#[test]
fn return_bus_to_bus_sends_and_metadata_via_bridge() {
    let bridge = build_bridge();
    let delay = bridge.create_return_bus("Delay").unwrap();
    let reverb = bridge.create_return_bus("Reverb").unwrap();

    // Bus-to-bus: delay feeds reverb.
    bridge
        .set_return_send(delay, reverb, 0.5.into(), true)
        .unwrap();
    let busses = bridge.list_return_busses().unwrap();
    let d = busses.iter().find(|b| b.id == delay).unwrap();
    assert_eq!(d.sends.len(), 1);
    assert_eq!(d.sends[0].target, reverb);
    assert!((d.sends[0].level.as_f32() - 0.5).abs() < 1e-6);
    assert!(d.sends[0].enabled);

    // A self-send and a cycle (reverb -> delay closes delay -> reverb) are rejected.
    assert!(
        bridge
            .set_return_send(delay, delay, 0.5.into(), true)
            .is_err()
    );
    assert!(
        bridge
            .set_return_send(reverb, delay, 0.5.into(), true)
            .is_err()
    );

    // Bypass keeps the send; removal drops it.
    bridge
        .set_return_send(delay, reverb, 0.5.into(), false)
        .unwrap();
    let busses = bridge.list_return_busses().unwrap();
    assert!(!busses.iter().find(|b| b.id == delay).unwrap().sends[0].enabled);
    bridge.remove_return_send(delay, reverb).unwrap();
    let busses = bridge.list_return_busses().unwrap();
    assert!(
        busses
            .iter()
            .find(|b| b.id == delay)
            .unwrap()
            .sends
            .is_empty()
    );

    // Solo / color / description.
    bridge.set_return_bus_solo(reverb, true).unwrap();
    bridge.set_return_bus_color(reverb, "#123456").unwrap();
    bridge
        .set_return_bus_description(reverb, "shared plate")
        .unwrap();
    assert!(bridge.set_return_bus_color(reverb, "nothex").is_err());
    let busses = bridge.list_return_busses().unwrap();
    let r = busses.iter().find(|b| b.id == reverb).unwrap();
    assert!(r.solo);
    assert_eq!(r.color, "#123456");
    assert_eq!(r.description, "shared plate");

    // Master volume: set sends a command; get reads the engine atomic.
    bridge.set_master_volume(0.5.into()).unwrap();
    assert!(bridge.get_master_volume().is_ok());
}
