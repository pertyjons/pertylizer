//! Bridge-level round-trip test for the tempo-map MCP surface
//! (`set_tempo_at` / `get_tempo_map` / `remove_tempo_at`, TODO §1.1 Phase 1).

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_core::Bpm;
use synth_engine::SynthEngine;
use synth_mcp::bridge::*;
use synth_sequencer::{Song, Tick};

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
fn tempo_map_set_get_remove_round_trips() {
    let bridge = build_bridge();

    // Empty to start.
    assert!(bridge.get_tempo_map().unwrap().is_empty());

    // Set three points out of order — the map must come back sorted by tick.
    // The middle one is a ramp; the ramp flag must round-trip.
    bridge
        .set_tempo_at(&[
            (Tick(3840), 90.0, false),
            (Tick(960), 140.0, false),
            (Tick(1920), 160.0, true),
        ])
        .unwrap();
    let map = bridge.get_tempo_map().unwrap();
    assert_eq!(map.len(), 3);
    assert_eq!(
        (map[0].tick, map[0].bpm, map[0].ramp),
        (Tick(960), Bpm::new(140.0), false)
    );
    assert_eq!(
        (map[1].tick, map[1].bpm, map[1].ramp),
        (Tick(1920), Bpm::new(160.0), true)
    );
    assert_eq!(
        (map[2].tick, map[2].bpm, map[2].ramp),
        (Tick(3840), Bpm::new(90.0), false)
    );

    // Replacing a point at the same tick overwrites rather than duplicates,
    // and updates its ramp flag.
    bridge.set_tempo_at(&[(Tick(960), 128.0, true)]).unwrap();
    let map = bridge.get_tempo_map().unwrap();
    assert_eq!(map.len(), 3);
    assert_eq!(
        (map[0].tick, map[0].bpm, map[0].ramp),
        (Tick(960), Bpm::new(128.0), true)
    );

    // Remove two (one present, one absent) — only the present one counts.
    let removed = bridge.remove_tempo_at(&[Tick(1920), Tick(5000)]).unwrap();
    assert_eq!(removed, 1);
    let map = bridge.get_tempo_map().unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(map[0].tick, Tick(960));
    assert_eq!(map[1].tick, Tick(3840));
}
