//! Bridge integration tests for the Group-B symbolic composition helpers.
//!
//! These don't touch a `SynthEngine` — the four tools are purely symbolic
//! operations over song data. We construct an `McpSharedState`, populate one
//! pattern with raw `Note`s, call the bridge impls, and assert the pattern
//! state and aggregate result.

use std::sync::Arc;

use pertylizer::mcp_bridge::{
    generate_chord_impl, quantize_notes_to_grid_impl, quantize_notes_to_scale_impl,
    transpose_notes_impl,
};
use pertylizer::mcp_shared::McpSharedState;
use synth_core::Semitones;
use synth_sequencer::{
    Duration as SeqDuration, NoteId, PatternId, PatternTick, Pitch, Song, Velocity,
};

fn shared_with_pattern(notes: &[(u8, u32)]) -> (Arc<McpSharedState>, PatternId) {
    let mut song = Song::new("Test");
    let pid = song.create_pattern(SeqDuration(3840));
    if let Some(pattern) = song.pattern_mut(pid) {
        for (idx, (pitch, start)) in notes.iter().enumerate() {
            // `insert_note` reassigns the ID; pass NoteId(0) as a placeholder.
            let mut n = synth_sequencer::Note::new(
                NoteId(idx as u64),
                PatternTick(*start),
                Pitch::new(*pitch).expect("valid pitch"),
                Velocity::MF,
            );
            n = n.with_duration(SeqDuration(240));
            let _ = pattern.insert_note(n);
        }
    }
    let shared = Arc::new(McpSharedState::with_song(Arc::new(
        synth_sequencer::SharedSong::new(song),
    )));
    (shared, pid)
}

fn pattern_pitches(shared: &McpSharedState, pid: PatternId) -> Vec<u8> {
    let song = shared.song.read();
    let pattern = song.pattern(pid).expect("pattern exists");
    pattern.notes().iter().map(|n| n.pitch.as_midi()).collect()
}

fn pattern_starts(shared: &McpSharedState, pid: PatternId) -> Vec<u32> {
    let song = shared.song.read();
    let pattern = song.pattern(pid).expect("pattern exists");
    pattern.notes().iter().map(|n| n.start.0).collect()
}

#[test]
fn generate_chord_returns_close_voicing_for_cmaj7() {
    let result = generate_chord_impl("Cmaj7", 4, Some("close")).expect("Cmaj7 is valid");
    assert_eq!(result.notes, vec![60, 64, 67, 71]);
    assert_eq!(result.quality, "major7");
    assert_eq!(result.voicing, "close");
}

#[test]
fn generate_chord_rejects_unknown_voicing() {
    let err =
        generate_chord_impl("C", 4, Some("rotated")).expect_err("unknown voicing should error");
    let msg = err.to_string();
    assert!(msg.contains("voicing"), "{msg}");
}

#[test]
fn transpose_notes_shifts_each_pitch() {
    let (shared, pid) = shared_with_pattern(&[(60, 0), (62, 240), (64, 480)]);
    let result = transpose_notes_impl(&shared, pid, Semitones::new(5.0), None, None, None)
        .expect("transpose works");
    assert_eq!(result.notes_transposed, 3);
    assert_eq!(result.notes_out_of_range, 0);
    assert_eq!(pattern_pitches(&shared, pid), vec![65, 67, 69]);
}

#[test]
fn transpose_notes_with_scale_snaps_off_key_results() {
    // Bb (58) and E (64), shift +2 → C (in scale) and F# (out). Snap to G
    // (NearestUp).
    let (shared, pid) = shared_with_pattern(&[(58, 0), (64, 240)]);
    let result = transpose_notes_impl(
        &shared,
        pid,
        Semitones::new(2.0),
        Some(0),
        Some("major"),
        Some("up"),
    )
    .expect("transpose works");
    assert_eq!(result.notes_snapped_to_scale, 1);
    assert_eq!(pattern_pitches(&shared, pid), vec![60, 67]);
    assert_eq!(result.scale_name.as_deref(), Some("major"));
}

#[test]
fn transpose_partial_scale_constraint_emits_warning() {
    let (shared, pid) = shared_with_pattern(&[(60, 0)]);
    let result = transpose_notes_impl(&shared, pid, Semitones::new(1.0), Some(0), None, None)
        .expect("partial scale is non-fatal");
    assert!(result.warnings.iter().any(|w| w.contains("scale")));
    assert_eq!(pattern_pitches(&shared, pid), vec![61]);
}

#[test]
fn quantize_to_scale_snaps_chromatic_passing_tones() {
    // C major scale. Notes: C, C#, F#, G — only C# and F# should move.
    let (shared, pid) = shared_with_pattern(&[(60, 0), (61, 240), (66, 480), (67, 720)]);
    let result = quantize_notes_to_scale_impl(&shared, pid, 0, "major", Some("up"))
        .expect("scale snap works");
    assert_eq!(result.notes_moved, 2);
    assert_eq!(result.notes_already_in_scale, 2);
    assert_eq!(result.max_correction_semitones, 1);
    assert_eq!(pattern_pitches(&shared, pid), vec![60, 62, 67, 67]);
}

#[test]
fn quantize_to_grid_snaps_off_grid_notes() {
    // Grid 240. Notes off the grid: 100, 250, 470, 1000.
    let (shared, pid) = shared_with_pattern(&[(60, 100), (61, 250), (62, 470), (63, 1000)]);
    let result =
        quantize_notes_to_grid_impl(&shared, pid, 240, Some(1.0), Some(0.0), Some(0), Some(0))
            .expect("grid snap works");
    let starts = pattern_starts(&shared, pid);
    assert_eq!(starts, vec![0, 240, 480, 960]);
    assert_eq!(result.notes_moved, 4);
    assert!(result.mean_delta_ticks > 0.0);
}

#[test]
fn quantize_to_grid_with_humanize_is_deterministic_for_same_seed() {
    let (a, pid_a) = shared_with_pattern(&[(60, 100), (61, 300)]);
    let (b, pid_b) = shared_with_pattern(&[(60, 100), (61, 300)]);
    let _ = quantize_notes_to_grid_impl(&a, pid_a, 240, Some(1.0), Some(0.0), Some(40), Some(99))
        .expect("first pass");
    let _ = quantize_notes_to_grid_impl(&b, pid_b, 240, Some(1.0), Some(0.0), Some(40), Some(99))
        .expect("second pass");
    assert_eq!(pattern_starts(&a, pid_a), pattern_starts(&b, pid_b));
}

#[test]
fn quantize_to_grid_disabled_grid_returns_warning() {
    let (shared, pid) = shared_with_pattern(&[(60, 100)]);
    let result = quantize_notes_to_grid_impl(&shared, pid, 0, None, None, None, None)
        .expect("zero grid is non-fatal");
    assert_eq!(result.notes_moved, 0);
    assert!(result.warnings.iter().any(|w| w.contains("grid_ticks")));
    assert_eq!(pattern_starts(&shared, pid), vec![100]);
}

#[test]
fn missing_pattern_returns_error() {
    let shared = Arc::new(McpSharedState::new());
    let err = transpose_notes_impl(
        &shared,
        PatternId(999),
        Semitones::new(5.0),
        None,
        None,
        None,
    )
    .expect_err("nonexistent pattern should error");
    let msg = err.to_string();
    assert!(msg.contains("999") || msg.contains("Pattern"), "{msg}");
}
