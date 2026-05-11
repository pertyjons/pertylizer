//! Unit-level coverage for the `analysis::instrument_profile` axes and
//! decision tree. Builds `InstrumentSnapshot`/`ModuleStateSnapshot` values
//! directly — no audio engine required — so the heuristics can be exercised
//! across a wide range of patches cheaply.

use std::collections::HashMap;

use synth_core::params::EnvelopeParam;
use synth_core::types::{NormalizedValue, Seconds};
use synth_core::{
    BipolarValue, BypassState, Gain, MidiChannel, ModuleType, MuteState, Param, SoloState,
};
use synth_engine::instrument::InstrumentId;
use synth_engine::{
    InstrumentCategory, InstrumentSnapshot, ModuleConnectivityStatus, ModuleId, ModuleStateSnapshot,
};

use pertylizer::analysis::instrument_profile::{
    EnvelopeShape, GraphSignals, NoteRef, PatternStats, PitchRole, ProfileSignal, Register, Role,
    Texture, classify_role, envelope_shape, graph_signals, infer_instrument_profile, pattern_stats,
    pitch_role_from_stats, register_from_stats, role_from_name, texture_from_stats,
};

// ---------------------------------------------------------------------------
// Helpers — build module snapshots and instrument snapshots inline.
// ---------------------------------------------------------------------------

fn instrument_id(seq: u16) -> InstrumentId {
    InstrumentId::new(u64::from(seq))
}

fn module(seq: u16, module_type: ModuleType, instance: u16) -> ModuleStateSnapshot {
    ModuleStateSnapshot {
        id: ModuleId::new(module_type, instance),
        instrument_id: instrument_id(seq),
        module_type,
        name: format!("{module_type:?}-{instance}"),
        bypass_state: BypassState::Active,
        mute_state: MuteState::Unmuted,
        solo_state: SoloState::Normal,
        connectivity: ModuleConnectivityStatus::Connected,
        parameters: Vec::new(),
        input_connection_counts: HashMap::new(),
        output_connection_counts: HashMap::new(),
        cpu_usage: Default::default(),
        output_levels: HashMap::new(),
    }
}

fn envelope_module(seq: u16, instance: u16, a: f32, d: f32, s: f32, r: f32) -> ModuleStateSnapshot {
    let mut m = module(seq, ModuleType::Envelope, instance);
    m.parameters = vec![
        Param::Envelope(EnvelopeParam::Attack(Seconds::new(a))),
        Param::Envelope(EnvelopeParam::Decay(Seconds::new(d))),
        Param::Envelope(EnvelopeParam::Sustain(NormalizedValue::new(s))),
        Param::Envelope(EnvelopeParam::Release(Seconds::new(r))),
    ];
    m
}

fn instrument_snapshot(seq: u16, name: &str, category: InstrumentCategory) -> InstrumentSnapshot {
    InstrumentSnapshot {
        id: instrument_id(seq),
        seq_instrument_id: seq,
        name: name.to_string(),
        category,
        midi_channel: MidiChannel::CH1,
        volume: Gain::UNITY,
        pan: BipolarValue::CENTER,
        enabled: true,
        muted: false,
        solo: false,
        module_count: 0,
        effect_count: 0,
        effect_chain_order: Vec::new(),
    }
}

fn note(pitch: u8, start: u64, duration: Option<u64>) -> NoteRef {
    NoteRef {
        pitch,
        start_tick: start,
        duration_ticks: duration,
    }
}

fn signal(axis: &'static str, detail: &'static str) -> ProfileSignal {
    ProfileSignal { axis, detail }
}

// ---------------------------------------------------------------------------
// Patch fixtures (returned as (snapshot, modules)).
// ---------------------------------------------------------------------------

fn kick_patch() -> (InstrumentSnapshot, Vec<ModuleStateSnapshot>) {
    let snap = instrument_snapshot(0, "Kick", InstrumentCategory::Uncategorized);
    let modules = vec![
        module(0, ModuleType::Noise, 1),
        envelope_module(0, 1, 0.001, 0.05, 0.0, 0.05),
        module(0, ModuleType::Amplifier, 1),
    ];
    (snap, modules)
}

fn unnamed_kick_patch() -> (InstrumentSnapshot, Vec<ModuleStateSnapshot>) {
    // Same modules as kick, but the name carries no hint — exercising the
    // graph + envelope path on its own.
    let snap = instrument_snapshot(0, "Track 5", InstrumentCategory::Uncategorized);
    let modules = vec![
        module(0, ModuleType::Noise, 1),
        envelope_module(0, 1, 0.001, 0.05, 0.0, 0.05),
        module(0, ModuleType::Amplifier, 1),
    ];
    (snap, modules)
}

fn bass_patch() -> (InstrumentSnapshot, Vec<ModuleStateSnapshot>) {
    let snap = instrument_snapshot(1, "Sub Bass", InstrumentCategory::Uncategorized);
    let modules = vec![
        module(1, ModuleType::Oscillator, 1),
        module(1, ModuleType::Oscillator, 2),
        module(1, ModuleType::Filter, 1),
        envelope_module(1, 1, 0.005, 0.1, 0.8, 0.1),
        module(1, ModuleType::Amplifier, 1),
    ];
    (snap, modules)
}

fn pad_patch() -> (InstrumentSnapshot, Vec<ModuleStateSnapshot>) {
    let snap = instrument_snapshot(2, "Strings", InstrumentCategory::Uncategorized);
    let modules = vec![
        module(2, ModuleType::Oscillator, 1),
        module(2, ModuleType::Oscillator, 2),
        module(2, ModuleType::Filter, 1),
        // Slow attack -> Evolving.
        envelope_module(2, 1, 1.5, 0.5, 0.8, 1.2),
        module(2, ModuleType::Amplifier, 1),
    ];
    (snap, modules)
}

fn pluck_patch() -> (InstrumentSnapshot, Vec<ModuleStateSnapshot>) {
    let snap = instrument_snapshot(3, "Pluck", InstrumentCategory::Uncategorized);
    let modules = vec![
        module(3, ModuleType::Oscillator, 1),
        // Plucked envelope: low sustain, short release.
        envelope_module(3, 1, 0.001, 0.2, 0.0, 0.3),
        module(3, ModuleType::Amplifier, 1),
    ];
    (snap, modules)
}

// ---------------------------------------------------------------------------
// Envelope-shape axis.
// ---------------------------------------------------------------------------

#[test]
fn envelope_shape_classifies_percussive() {
    let m = vec![envelope_module(0, 1, 0.001, 0.05, 0.0, 0.05)];
    assert_eq!(envelope_shape(&m), EnvelopeShape::Percussive);
}

#[test]
fn envelope_shape_classifies_plucked() {
    let m = vec![envelope_module(0, 1, 0.005, 0.2, 0.0, 0.3)];
    assert_eq!(envelope_shape(&m), EnvelopeShape::Plucked);
}

#[test]
fn envelope_shape_classifies_sustained() {
    let m = vec![envelope_module(0, 1, 0.005, 0.05, 0.8, 0.1)];
    assert_eq!(envelope_shape(&m), EnvelopeShape::Sustained);
}

#[test]
fn envelope_shape_classifies_evolving() {
    let m = vec![envelope_module(0, 1, 1.5, 0.5, 0.8, 1.0)];
    assert_eq!(envelope_shape(&m), EnvelopeShape::Evolving);
}

#[test]
fn envelope_shape_unknown_when_no_envelope() {
    let m = vec![module(0, ModuleType::Oscillator, 1)];
    assert_eq!(envelope_shape(&m), EnvelopeShape::Unknown);
}

#[test]
fn envelope_shape_picks_lowest_instance() {
    let mut m = vec![
        // Higher instance comes first in the input, but the lower instance
        // (env-1) must win regardless of insertion order.
        envelope_module(0, 7, 1.5, 0.5, 0.8, 1.0),
        envelope_module(0, 1, 0.001, 0.05, 0.0, 0.05),
    ];
    assert_eq!(envelope_shape(&m), EnvelopeShape::Percussive);
    m.reverse();
    assert_eq!(envelope_shape(&m), EnvelopeShape::Percussive);
}

// ---------------------------------------------------------------------------
// PitchRole / Register / Texture axes.
// ---------------------------------------------------------------------------

#[test]
fn pitch_role_atonal_for_single_pitch_class() {
    let notes = vec![note(42, 0, Some(120)), note(42, 240, Some(120))];
    let stats = pattern_stats(&notes);
    assert_eq!(pitch_role_from_stats(&stats), PitchRole::Atonal);
}

#[test]
fn pitch_role_tonal_for_diatonic_use() {
    let notes = vec![
        note(60, 0, Some(120)),
        note(62, 120, Some(120)),
        note(64, 240, Some(120)),
        note(65, 360, Some(120)),
        note(67, 480, Some(120)),
    ];
    let stats = pattern_stats(&notes);
    assert_eq!(pitch_role_from_stats(&stats), PitchRole::Tonal);
}

#[test]
fn pitch_role_mixed_for_three_classes() {
    let notes = vec![
        note(60, 0, Some(120)),
        note(64, 120, Some(120)),
        note(67, 240, Some(120)),
    ];
    let stats = pattern_stats(&notes);
    assert_eq!(pitch_role_from_stats(&stats), PitchRole::Mixed);
}

#[test]
fn pitch_role_unused_for_empty() {
    let stats = pattern_stats(&[]);
    assert_eq!(pitch_role_from_stats(&stats), PitchRole::Unused);
}

#[test]
fn register_sub_for_low_median() {
    let notes = vec![note(28, 0, Some(120)), note(30, 120, Some(120))];
    let stats = pattern_stats(&notes);
    assert_eq!(register_from_stats(&stats), Register::Sub);
}

#[test]
fn register_full_range_when_spread_large() {
    let notes = vec![note(24, 0, Some(120)), note(96, 120, Some(120))];
    let stats = pattern_stats(&notes);
    assert_eq!(register_from_stats(&stats), Register::FullRange);
}

#[test]
fn texture_monophonic_when_serial() {
    let notes = vec![
        note(60, 0, Some(100)),
        note(62, 100, Some(100)),
        note(64, 200, Some(100)),
    ];
    let stats = pattern_stats(&notes);
    assert_eq!(stats.max_simultaneous, 1);
    assert_eq!(texture_from_stats(&stats), Texture::Monophonic);
}

#[test]
fn texture_chordal_when_5plus() {
    let notes = vec![
        note(60, 0, Some(400)),
        note(64, 0, Some(400)),
        note(67, 0, Some(400)),
        note(71, 0, Some(400)),
        note(74, 0, Some(400)),
    ];
    let stats = pattern_stats(&notes);
    assert_eq!(stats.max_simultaneous, 5);
    assert_eq!(texture_from_stats(&stats), Texture::Chordal);
}

#[test]
fn texture_handles_shared_start_tick() {
    // Three notes starting at the exact same tick should report
    // max_simultaneous = 3, not 1 (sweep-line edge case).
    let notes = vec![
        note(60, 100, Some(50)),
        note(64, 100, Some(50)),
        note(67, 100, Some(50)),
    ];
    let stats = pattern_stats(&notes);
    assert_eq!(stats.max_simultaneous, 3);
}

#[test]
fn texture_treats_back_to_back_as_monophonic() {
    // Two notes where the second starts exactly when the first ends. The
    // sweep-line processes end events before start events at the same tick,
    // so these don't count as overlap.
    let notes = vec![note(60, 0, Some(100)), note(62, 100, Some(100))];
    let stats = pattern_stats(&notes);
    assert_eq!(stats.max_simultaneous, 1);
}

// ---------------------------------------------------------------------------
// GraphSignals.
// ---------------------------------------------------------------------------

#[test]
fn graph_signals_detects_noise_only() {
    let m = vec![module(0, ModuleType::Noise, 1)];
    let sig = graph_signals(&m);
    assert!(sig.has_noise_source);
    assert!(!sig.has_oscillator);
    assert!(!sig.has_sampler);
    assert_eq!(sig.osc_count, 0);
}

#[test]
fn graph_signals_detects_dual_oscillator() {
    let m = vec![
        module(0, ModuleType::Oscillator, 1),
        module(0, ModuleType::WavetableOsc, 1),
    ];
    let sig = graph_signals(&m);
    assert!(sig.has_oscillator);
    assert_eq!(sig.osc_count, 2);
}

#[test]
fn graph_signals_detects_physical_only() {
    let m = vec![module(0, ModuleType::ModalResonator, 1)];
    let sig = graph_signals(&m);
    assert!(sig.has_physical);
    assert!(!sig.has_oscillator);
}

// ---------------------------------------------------------------------------
// Name vocabulary.
// ---------------------------------------------------------------------------

#[test]
fn role_from_name_matches_drums() {
    assert_eq!(role_from_name("Kick 808", None), Some(Role::Drums));
    assert_eq!(role_from_name("Hat", None), Some(Role::Drums));
    assert_eq!(role_from_name("Tom Tom", None), Some(Role::Drums));
}

#[test]
fn role_from_name_does_not_substring_match() {
    // "bassoon" must not trigger "bass".
    assert_eq!(role_from_name("Bassoon", None), None);
}

#[test]
fn role_from_name_falls_back_to_track_name() {
    assert_eq!(role_from_name("Patch 12", Some("Pad")), Some(Role::Pad));
}

#[test]
fn role_from_name_recognizes_lead_and_pluck() {
    assert_eq!(role_from_name("Lead Solo", None), Some(Role::Lead));
    assert_eq!(role_from_name("Harp Pluck", None), Some(Role::Pluck));
}

// ---------------------------------------------------------------------------
// Decision tree (classify_role).
// ---------------------------------------------------------------------------

fn empty_graph() -> GraphSignals {
    GraphSignals {
        has_oscillator: false,
        has_noise_source: false,
        has_sampler: false,
        has_physical: false,
        osc_count: 0,
    }
}

#[test]
fn classify_drums_via_envelope_only() {
    let g = empty_graph();
    let r = classify_role(
        None,
        &g,
        EnvelopeShape::Percussive,
        PitchRole::Atonal,
        Register::Bass,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Drums);
    assert!(r.confidence >= 0.6);
}

#[test]
fn classify_drums_via_noise_only() {
    let mut g = empty_graph();
    g.has_noise_source = true;
    let r = classify_role(
        None,
        &g,
        EnvelopeShape::Sustained,
        PitchRole::Atonal,
        Register::Mid,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Drums);
}

#[test]
fn classify_bass_when_low_register_and_tonal() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    g.osc_count = 2;
    let r = classify_role(
        Some(Role::Bass),
        &g,
        EnvelopeShape::Sustained,
        PitchRole::Tonal,
        Register::Bass,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Bass);
}

#[test]
fn classify_pad_when_evolving_and_chordal() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    g.osc_count = 2;
    let r = classify_role(
        Some(Role::Pad),
        &g,
        EnvelopeShape::Evolving,
        PitchRole::Tonal,
        Register::Mid,
        Texture::Chordal,
    );
    assert_eq!(r.role, Role::Pad);
}

#[test]
fn classify_pluck_when_plucked_monophonic() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    let r = classify_role(
        Some(Role::Pluck),
        &g,
        EnvelopeShape::Plucked,
        PitchRole::Tonal,
        Register::Mid,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Pluck);
}

#[test]
fn classify_keys_when_plucked_chordal_tonal() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    let r = classify_role(
        Some(Role::Keys),
        &g,
        EnvelopeShape::Plucked,
        PitchRole::Tonal,
        Register::Mid,
        Texture::Polyphonic,
    );
    assert_eq!(r.role, Role::Keys);
}

#[test]
fn classify_lead_when_sustained_monophonic_high_register() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    let r = classify_role(
        Some(Role::Lead),
        &g,
        EnvelopeShape::Sustained,
        PitchRole::Tonal,
        Register::High,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Lead);
}

#[test]
fn name_match_caps_confidence_to_0_85_minimum() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    let r = classify_role(
        Some(Role::Pluck),
        &g,
        EnvelopeShape::Plucked,
        PitchRole::Tonal,
        Register::Mid,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Pluck);
    assert!(r.confidence >= 0.85);
}

#[test]
fn name_conflict_lowers_confidence_and_records_signal() {
    let mut g = empty_graph();
    g.has_oscillator = true;
    let r = classify_role(
        // The decision tree says Bass (register Bass + monophonic tonal)
        // but the name shouts Pad. We keep Bass but penalize and tag.
        Some(Role::Pad),
        &g,
        EnvelopeShape::Sustained,
        PitchRole::Tonal,
        Register::Bass,
        Texture::Monophonic,
    );
    assert_eq!(r.role, Role::Bass);
    assert!(r.signals.contains(&signal("name", "name-conflict")));
}

#[test]
fn classify_unknown_when_nothing_matches() {
    let g = empty_graph();
    let r = classify_role(
        None,
        &g,
        EnvelopeShape::Unknown,
        PitchRole::Unused,
        Register::Unused,
        Texture::Unused,
    );
    assert_eq!(r.role, Role::Unknown);
    assert!(r.confidence < f32::EPSILON);
}

// ---------------------------------------------------------------------------
// Top-level infer_instrument_profile.
// ---------------------------------------------------------------------------

#[test]
fn kick_patch_classified_as_drums_via_name() {
    let (snap, modules) = kick_patch();
    let notes = vec![note(42, 0, Some(120)), note(42, 240, Some(120))];
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::Drums);
    assert!(profile.role.confidence >= 0.85);
    assert_eq!(profile.envelope_shape, EnvelopeShape::Percussive);
}

#[test]
fn unnamed_kick_classified_as_drums_via_graph_and_envelope() {
    // §8.2 regression target — the name is uninformative but the noise
    // source + percussive envelope + single-pitch-class notes must still
    // resolve to Drums.
    let (snap, modules) = unnamed_kick_patch();
    let notes = vec![note(42, 0, Some(120)), note(42, 240, Some(120))];
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::Drums);
    assert!(profile.role.confidence >= 0.6);
}

#[test]
fn bass_patch_classified_as_bass() {
    let (snap, modules) = bass_patch();
    // Walking line over multiple pitch classes — keeps pitch_role at Tonal
    // so the Bass gate can fire.
    let notes = vec![
        note(36, 0, Some(240)),
        note(38, 480, Some(240)),
        note(40, 960, Some(240)),
        note(43, 1440, Some(240)),
        note(45, 1920, Some(240)),
    ];
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::Bass);
    assert_eq!(profile.register, Register::Bass);
}

#[test]
fn pad_patch_classified_as_pad() {
    let (snap, modules) = pad_patch();
    // Chordal use of the pad: 5 simultaneous notes.
    let notes = vec![
        note(60, 0, Some(960)),
        note(64, 0, Some(960)),
        note(67, 0, Some(960)),
        note(71, 0, Some(960)),
        note(74, 0, Some(960)),
    ];
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::Pad);
    assert_eq!(profile.envelope_shape, EnvelopeShape::Evolving);
    assert_eq!(profile.texture, Texture::Chordal);
}

#[test]
fn pluck_patch_classified_as_pluck() {
    let (snap, modules) = pluck_patch();
    let notes = vec![
        note(60, 0, Some(120)),
        note(64, 120, Some(120)),
        note(67, 240, Some(120)),
        note(72, 360, Some(120)),
    ];
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::Pluck);
}

#[test]
fn manual_category_overrides_inference() {
    // Even with a noisy graph + percussive envelope + atonal notes, a
    // user-tagged Pad must come through as Pad with full confidence.
    let mut snap = instrument_snapshot(0, "Track 5", InstrumentCategory::Pad);
    let modules = vec![
        module(0, ModuleType::Noise, 1),
        envelope_module(0, 1, 0.001, 0.05, 0.0, 0.05),
    ];
    let notes = vec![note(42, 0, Some(120))];
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::Pad);
    assert!((profile.role.confidence - 1.0).abs() < f32::EPSILON);

    // And every non-Uncategorized variant should map to a role.
    snap.category = InstrumentCategory::FX;
    let profile = infer_instrument_profile(&snap, &modules, &[], &notes);
    assert_eq!(profile.role.role, Role::FX);
}

#[test]
fn pattern_stats_match_expected_summary() {
    // Sanity check the stats struct itself for a known input.
    let notes = vec![
        note(60, 0, Some(100)),
        note(64, 0, Some(100)),
        note(67, 0, Some(100)),
        note(72, 200, Some(100)),
    ];
    let stats = pattern_stats(&notes);
    // MIDI 60, 64, 67, 72 → pitch classes {0, 4, 7, 0} = 3 distinct.
    let expected = PatternStats {
        distinct_pitch_classes: 3,
        median_pitch: 67,
        pitch_spread: 12,
        max_simultaneous: 3,
        note_count: 4,
    };
    assert_eq!(stats, expected);
}
