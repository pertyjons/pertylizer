//! Integration test for the `simplify_automation` MCP tool: dry-run previews
//! without mutating, apply rewrites the lane, and a re-run finds nothing left.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_core::{InstrumentId, NormalizedValue};
use synth_engine::SynthEngine;
use synth_mcp::SynthBridge;
use synth_sequencer::{
    AutomationPoint, AutomationTarget, CurveType, Duration as SeqDuration, GlobalParam, PatternId,
    PatternTick, SharedSong, Song,
};

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

/// A pattern with one global-volume lane holding a perfectly straight 9-point
/// ramp — 7 interior points are redundant under any sane tolerance.
fn setup() -> (AppSynthBridge, Arc<SharedSong>, PatternId) {
    let (_engine, handle) = SynthEngine::new();
    let mut song = Song::new("Simp");
    let pid = song.create_pattern(SeqDuration(3840));
    {
        let pat = song.pattern_mut(pid).expect("pattern");
        let lane =
            pat.get_or_create_automation(AutomationTarget::Global(GlobalParam::MasterVolume));
        for i in 0..=8u32 {
            lane.add_point(AutomationPoint {
                tick: PatternTick(i * 100),
                value: NormalizedValue::new(i as f32 / 8.0),
                curve: CurveType::Linear,
            });
        }
    }
    let shared_song = Arc::new(SharedSong::new(song));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&shared_song),
        });
    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&shared_song)));
    let bridge = AppSynthBridge::new(session, shared, sample_library);
    (bridge, shared_song, pid)
}

fn lane_len(song: &SharedSong, pid: PatternId) -> usize {
    song.read().pattern(pid).unwrap().automation[0]
        .points()
        .len()
}

#[test]
fn dry_run_previews_without_mutating_then_apply_rewrites() {
    let (bridge, song, pid) = setup();
    assert_eq!(lane_len(&song, pid), 9);

    // Dry-run: reports the removal but leaves the lane untouched.
    let preview = bridge
        .simplify_automation(Some(pid), None, InstrumentId::default(), 0.01, false)
        .expect("dry-run");
    assert!(!preview.applied, "dry-run must not report applied");
    assert_eq!(
        preview.total_removed, 7,
        "7 interior points are redundant: {preview:?}"
    );
    assert!(preview.lanes[0].max_error <= 0.01);
    assert_eq!(lane_len(&song, pid), 9, "dry-run must not mutate the lane");

    // Apply: rewrites the lane down to its endpoints.
    let applied = bridge
        .simplify_automation(Some(pid), None, InstrumentId::default(), 0.01, true)
        .expect("apply");
    assert!(applied.applied);
    assert_eq!(applied.total_removed, 7);
    assert_eq!(lane_len(&song, pid), 2, "apply keeps only the endpoints");

    // Re-running finds nothing left to remove.
    let again = bridge
        .simplify_automation(Some(pid), None, InstrumentId::default(), 0.01, false)
        .expect("re-run");
    assert!(
        again.lanes.is_empty(),
        "already-minimal lane is not reported: {again:?}"
    );
    assert_eq!(again.total_removed, 0);
}
