//! Bridge integration tests for Group D: `analyze_tension_curve` and
//! `suggest_music_fixes`. Both tools take a `SynthSession` because they
//! may render audio (tension curve when `include_audio=true`,
//! suggest_music_fixes when the mix-bus / masking rules are enabled).
//! Tests stay in pattern scope with `include_audio=false` so the engine
//! never has to actually render — the analyzers go through the pure-
//! symbolic path.

use std::sync::Arc;

use parking_lot::RwLock;

use synth_engine::SynthEngine;
use synth_sequencer::{
    Duration as SeqDuration, NoteId, PatternId, PatternTick, Pitch, Song, Velocity,
};

use pertylizer::mcp_bridge::{analyze_tension_curve_impl, suggest_music_fixes_impl};
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

struct Rig {
    _engine: SynthEngine,
    _handle: synth_engine::EngineHandle,
    session: SynthSession,
    shared: Arc<McpSharedState>,
    sample_library: pertylizer::audio::preview::SharedSampleLibrary,
    pattern_id: PatternId,
}

fn rig_with_pattern(notes: &[(u8, u32, u32, u8)]) -> Rig {
    let (engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    // Four bars of 4/4 at 960 PPQN.
    let mut song = Song::new("Group D Test");
    let pid = song.create_pattern(SeqDuration(15_360));
    if let Some(pattern) = song.pattern_mut(pid) {
        for (idx, (pitch, start, dur, vel)) in notes.iter().enumerate() {
            let velocity = Velocity::from_midi(*vel);
            let mut n = synth_sequencer::Note::new(
                NoteId(idx as u64),
                PatternTick(*start),
                Pitch::new(*pitch).expect("valid pitch"),
                velocity,
            );
            n = n.with_duration(SeqDuration(*dur));
            let _ = pattern.insert_note(n);
        }
    }
    let shared = Arc::new(McpSharedState::with_song(Arc::new(RwLock::new(song))));
    let sample_library = pertylizer::audio::preview::SharedSampleLibrary::default();
    Rig {
        _engine: engine,
        _handle: handle,
        session,
        shared,
        sample_library,
        pattern_id: pid,
    }
}

#[test]
fn analyze_tension_curve_pattern_scope_returns_per_bar_rows() {
    // 4 bars: bar 1 sparse, bar 2 medium, bar 3 dense, bar 4 medium again.
    let bar = 3840u32;
    let mut notes: Vec<(u8, u32, u32, u8)> = Vec::new();
    // Bar 1: 2 notes
    notes.push((60, 0, bar / 2, 64));
    notes.push((64, bar / 2, bar / 2, 64));
    // Bar 2: 4 notes
    for i in 0..4 {
        notes.push((60 + i as u8, bar + i * (bar / 4), bar / 4, 80));
    }
    // Bar 3: 16 notes — saturated density
    for i in 0..16 {
        notes.push((60 + (i % 6) as u8, 2 * bar + i * (bar / 16), bar / 16, 96));
    }
    // Bar 4: 4 notes
    for i in 0..4 {
        notes.push((60 + i as u8, 3 * bar + i * (bar / 4), bar / 4, 80));
    }
    let rig = rig_with_pattern(&notes);
    let out = analyze_tension_curve_impl(
        &rig.session,
        &rig.sample_library,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(false), // include_audio off — pure symbolic
        None,
        None,
        Some(false), // exclude_drums irrelevant (pattern scope)
        None,
    )
    .expect("tension curve runs");
    assert_eq!(out.length_bars, 4);
    assert_eq!(out.bars.len(), 4);
    assert!(!out.has_audio);
    // Dense bar (bar 3) must beat sparse bar (bar 1) on composite tension.
    assert!(
        out.bars[2].composite_tension > out.bars[0].composite_tension,
        "dense bar should outscore sparse bar: {:?}",
        out.bars
    );
    assert!(out.summary.peak_bar.is_some());
}

#[test]
fn analyze_tension_curve_audio_axes_absent_in_symbolic_mode() {
    let bar = 3840u32;
    let notes: Vec<(u8, u32, u32, u8)> = (0..8)
        .map(|i| (60u8 + i as u8, i * (bar / 4), bar / 4, 80u8))
        .collect();
    let rig = rig_with_pattern(&notes);
    let out = analyze_tension_curve_impl(
        &rig.session,
        &rig.sample_library,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(false),
        None,
        None,
        Some(false),
        None,
    )
    .expect("tension curve runs");
    for b in &out.bars {
        assert!(b.loudness_score.is_none(), "expected no loudness axis");
        assert!(b.brightness.is_none());
        assert!(b.band_entropy.is_none());
        assert!(b.stereo_width_score.is_none());
    }
}

#[test]
fn analyze_tension_curve_handles_empty_pattern() {
    let rig = rig_with_pattern(&[]);
    let out = analyze_tension_curve_impl(
        &rig.session,
        &rig.sample_library,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(false),
        None,
        None,
        Some(false),
        None,
    )
    .expect("tension curve runs on empty pattern");
    // Empty pattern has total_bars > 0 (length is 4 bars) but every bar is
    // empty. Composite scores all zero; the summary is well-defined.
    assert_eq!(out.length_bars, 4);
    assert!(out.bars.iter().all(|b| b.composite_tension <= 0.01));
}

#[test]
fn suggest_music_fixes_pattern_scope_runs_symbolic_rules() {
    // 4 bars with a single sustained note → harmony will likely report
    // weak chord identification, hook will be zero.
    let bar = 3840u32;
    let notes: Vec<(u8, u32, u32, u8)> = (0..4).map(|b| (60u8, b * bar, bar, 80u8)).collect();
    let rig = rig_with_pattern(&notes);
    let out = suggest_music_fixes_impl(
        &rig.session,
        &rig.sample_library,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        None,        // categories — all
        Some(false), // include_audio off
        Some(15),
        Some(false),
        None,
    )
    .expect("suggest runs on minimal song");
    assert!(!out.include_audio);
    // Every suggestion's category must be one of the canonical strings.
    for s in &out.suggestions {
        assert!(
            [
                "harmony",
                "mix",
                "groove",
                "arrangement",
                "composition",
                "patch"
            ]
            .contains(&s.category.as_str()),
            "unknown category in suggestion: {s:?}"
        );
        assert!(s.severity >= 0.0 && s.severity <= 1.0);
        assert!(!s.id.is_empty());
        assert!(!s.title.is_empty());
    }
    // Severity sort descending invariant.
    for w in out.suggestions.windows(2) {
        assert!(w[0].severity >= w[1].severity);
    }
}

#[test]
fn suggest_music_fixes_respects_category_filter() {
    let bar = 3840u32;
    let notes: Vec<(u8, u32, u32, u8)> = (0..8)
        .map(|i| (60u8 + i as u8, i * (bar / 4), bar / 4, 80u8))
        .collect();
    let rig = rig_with_pattern(&notes);
    let out = suggest_music_fixes_impl(
        &rig.session,
        &rig.sample_library,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(vec!["harmony".to_string()]),
        Some(false),
        Some(15),
        Some(false),
        None,
    )
    .expect("suggest runs with harmony-only filter");
    assert_eq!(out.categories, vec!["harmony".to_string()]);
    for s in &out.suggestions {
        assert_eq!(s.category, "harmony");
    }
}

#[test]
fn suggest_music_fixes_ignores_unknown_categories() {
    let rig = rig_with_pattern(&[(60u8, 0u32, 960u32, 64u8)]);
    let out = suggest_music_fixes_impl(
        &rig.session,
        &rig.sample_library,
        &rig.shared,
        Some(rig.pattern_id),
        None,
        None,
        Some(vec!["bogus_category".to_string()]),
        Some(false),
        Some(15),
        Some(false),
        None,
    )
    .expect("suggest accepts but filters unknown categories");
    // Filter list filtered down to empty → behaves like "all" because
    // suggest() treats empty as no-filter. We only assert that the bridge
    // didn't reject the call.
    assert!(out.categories.is_empty());
}
