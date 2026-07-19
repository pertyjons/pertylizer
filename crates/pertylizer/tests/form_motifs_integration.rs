//! Bridge integration tests for Group C: `analyze_arrangement`,
//! `analyze_form_map`, `find_motifs`, `analyze_hook_strength`.
//!
//! All four tools are pure-symbolic — no audio render — but the impls take
//! a `SynthSession` because arrangement scope uses `infer_all_profiles` for
//! drum-track filtering. These tests stay in pattern scope where the
//! session is unused, so we wire up a minimal empty engine + session and
//! exercise the bridge round-trip on hand-crafted patterns.

use std::sync::Arc;

use synth_engine::SynthEngine;
use synth_sequencer::{
    Duration as SeqDuration, NoteId, PatternId, PatternTick, Pitch, Song, Velocity,
};

use pertylizer::mcp_bridge::{
    analyze_arrangement_impl, analyze_form_map_impl, analyze_hook_strength_impl, find_motifs_impl,
};
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

struct Rig {
    _engine: SynthEngine,
    _handle: synth_engine::EngineHandle,
    session: SynthSession,
    shared: Arc<McpSharedState>,
    pattern_id: PatternId,
}

/// Build a 4-bar pattern with the supplied note list (pitch, tick,
/// duration) and wrap it in an `Rig` that owns the engine + session + song.
fn rig_with_pattern(notes: &[(u8, u32, u32)]) -> Rig {
    let (engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    // Four bars of 4/4 at 960 PPQN = 4 * 4 * 960 = 15360 ticks.
    let mut song = Song::new("Form & Motifs Test");
    let pid = song.create_pattern(SeqDuration(15_360));
    if let Some(pattern) = song.pattern_mut(pid) {
        for (idx, (pitch, start, dur)) in notes.iter().enumerate() {
            let mut n = synth_sequencer::Note::new(
                NoteId(idx as u64),
                PatternTick(*start),
                Pitch::new(*pitch).expect("valid pitch"),
                Velocity::MF,
            );
            n = n.with_duration(SeqDuration(*dur));
            let _ = pattern.insert_note(n);
        }
    }
    let shared = Arc::new(McpSharedState::with_song(Arc::new(
        synth_sequencer::SharedSong::new(song),
    )));
    Rig {
        _engine: engine,
        _handle: handle,
        session,
        shared,
        pattern_id: pid,
    }
}

/// Build a single bar of [pitch1, pitch2] played as two half-notes.
fn half_note_bar(pitch1: u8, pitch2: u8, bar_index: u32) -> Vec<(u8, u32, u32)> {
    let bar_ticks = 3840u32; // 4/4 at 960 PPQN
    let start = bar_index * bar_ticks;
    vec![
        (pitch1, start, bar_ticks / 2),
        (pitch2, start + bar_ticks / 2, bar_ticks / 2),
    ]
}

#[test]
fn analyze_arrangement_pattern_scope_groups_uniform_bars_into_one_section() {
    let mut notes = Vec::new();
    for b in 0..4u32 {
        notes.extend(half_note_bar(60, 64, b));
    }
    let rig = rig_with_pattern(&notes);
    let result = analyze_arrangement_impl(
        &rig.session,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        None,
        Some(1),
        Some(false),
        None,
    )
    .expect("analyze_arrangement runs");
    assert_eq!(result.length_bars, 4);
    assert_eq!(result.bars.len(), 4);
    assert_eq!(result.sections.len(), 1, "{result:#?}");
    assert_eq!(result.sections[0].label, "A");
    assert_eq!(result.sections[0].start_bar, 1);
    assert_eq!(result.sections[0].end_bar, 4);
    assert_eq!(result.distinct_section_count, 1);
}

#[test]
fn analyze_form_map_returns_letter_per_bar_and_form_string() {
    // 4 bars: A A B A. Bar 3 uses a contrasting Bb/F pair.
    let mut notes = Vec::new();
    notes.extend(half_note_bar(60, 64, 0));
    notes.extend(half_note_bar(60, 64, 1));
    notes.extend(half_note_bar(70, 73, 2));
    notes.extend(half_note_bar(60, 64, 3));
    let rig = rig_with_pattern(&notes);
    let result = analyze_form_map_impl(
        &rig.session,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        None,
        Some(1),
        Some(false),
        None,
    )
    .expect("analyze_form_map runs");
    assert_eq!(result.length_bars, 4);
    assert_eq!(result.bar_labels.len(), 4);
    // Form string is one character per *distinct adjacent* label. At least
    // two distinct labels should appear (A and B).
    assert!(
        result
            .form_string
            .chars()
            .collect::<std::collections::HashSet<_>>()
            .len()
            >= 2,
        "expected ≥2 distinct labels, got form_string={:?} sections={:?}",
        result.form_string,
        result.sections
    );
    assert!(result.distinct_section_count >= 2);
}

#[test]
fn find_motifs_returns_repeating_phrase() {
    // Three reps of C-D-E-F (intervals +2 +2 +1), one note per beat, plus a
    // few transposed reps so transposition invariance gets exercised.
    let mut notes = Vec::new();
    let beat = 240u32;
    for rep in 0..3u32 {
        let base = 60 + rep * 5;
        let t0 = rep * 16 * beat;
        for (i, delta) in [0u8, 2, 4, 5].into_iter().enumerate() {
            notes.push((
                (base + u32::from(delta)) as u8,
                t0 + (i as u32) * beat,
                beat,
            ));
        }
    }
    let rig = rig_with_pattern(&notes);
    let result = find_motifs_impl(
        &rig.session,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(3),
        Some(3),
        Some(3),
        Some(10),
        None,
        Some(false),
        None,
    )
    .expect("find_motifs runs");
    assert!(!result.motifs.is_empty(), "expected at least one motif");
    let target = vec![2i16, 2, 1];
    let hit = result.motifs.iter().find(|m| m.intervals == target);
    let hit = hit.expect("expected to find [+2, +2, +1] motif");
    assert_eq!(hit.count, 3);
    assert_eq!(hit.occurrences.len(), 3);
}

#[test]
fn analyze_hook_strength_zero_for_unique_melody() {
    // Random ascending sequence — no n-gram repeats 3+ times.
    let beat = 240u32;
    let pitches = [60u8, 62, 64, 67, 71, 74, 77, 79, 81];
    let notes: Vec<(u8, u32, u32)> = pitches
        .iter()
        .enumerate()
        .map(|(i, p)| (*p, i as u32 * beat, beat))
        .collect();
    let rig = rig_with_pattern(&notes);
    let result = analyze_hook_strength_impl(
        &rig.session,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(3),
        Some(3),
        None,
        Some(false),
        None,
    )
    .expect("analyze_hook_strength runs");
    assert_eq!(result.hook_score, 0.0);
    assert!(result.strongest_motif.is_none());
}

#[test]
fn analyze_hook_strength_positive_for_strong_motif() {
    // 4 reps of a 5-note motif — should produce a positive hook score and a
    // strongest_motif of length 4.
    let mut notes = Vec::new();
    let beat = 240u32;
    for rep in 0..4u32 {
        let t0 = rep * 16 * beat;
        for (i, p) in [60u8, 62, 64, 65, 67].into_iter().enumerate() {
            notes.push((p, t0 + (i as u32) * beat, beat));
        }
    }
    let rig = rig_with_pattern(&notes);
    let result = analyze_hook_strength_impl(
        &rig.session,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(3),
        Some(3),
        None,
        Some(false),
        None,
    )
    .expect("analyze_hook_strength runs");
    assert!(
        result.hook_score > 0.0,
        "expected positive hook score, got {result:#?}"
    );
    let best = result
        .strongest_motif
        .as_ref()
        .expect("expected a strongest motif");
    assert!(best.length >= 3);
    assert!(best.count >= 3);
    assert!(result.coverage_ratio > 0.0);
}
