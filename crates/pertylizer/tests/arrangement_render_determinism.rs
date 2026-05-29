//! Determinism test for the offline arrangement renderer.
//!
//! `render_arrangement_to_buffer` must produce bit-exact identical
//! `samples: Vec<f32>` for the same engine state and tick range every call.
//! This is the load-bearing assumption behind `analyze_section` A/B use
//! cases (verse vs chorus, before vs after a knob tweak, master vs the
//! per-track sum).

mod common;

use synth_core::{BipolarValue, Gain, ModuleType};
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{AutomationTarget, GlobalParam, SeqInstrumentId, TrackParam};

use pertylizer::audio::arrangement_render::{OfflineEngineSession, render_arrangement_to_buffer};
use pertylizer::audio::mix_analysis::analyze_mix_buffer;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};

use common::{
    add_env_amp_out_tail, add_ramp_automation, assert_bit_exact, build_arpeggio_song,
    build_sustained_note_song, left_rms, process_block, right_rms, set_first_track_fader,
    setup_with_patch, sustain_patch,
};

#[test]
fn offline_render_is_bit_exact_across_calls() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let start_tick = 0u64;
    let end_tick = 3840u64;

    let first = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared,
        start_tick,
        end_tick,
    )
    .expect("first render should succeed");
    let second = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared,
        start_tick,
        end_tick,
    )
    .expect("second render should succeed");
    let third = render_arrangement_to_buffer(
        &rig.session,
        &rig.sample_library,
        &shared,
        start_tick,
        end_tick,
    )
    .expect("third render should succeed");

    assert_bit_exact(
        "render-1 vs render-2",
        &first.samples,
        &second.samples,
        first.sample_rate,
    );
    assert_bit_exact(
        "render-1 vs render-3",
        &first.samples,
        &third.samples,
        first.sample_rate,
    );
}

/// Noise hammers `fastrand::f32()` on every sample, so a non-reseeded
/// renderer diverges by O(1) immediately — complements the sawtooth test
/// which only diverges at note-on phase randomization.
fn noise_patch() -> Patch {
    let mut patch = Patch::new("DeterminismNoise");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .param_f("level", 0.5)
            .build(),
    );
    add_env_amp_out_tail(&mut patch);
    patch.add_connection("noise-1", "out", "amp-1", "in");
    patch
}

#[test]
fn offline_render_is_bit_exact_for_noise_patch() {
    let rig = setup_with_patch(&noise_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let first = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("first noise render should succeed");
    let second = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("second noise render should succeed");

    assert_bit_exact(
        "noise render-1 vs render-2",
        &first.samples,
        &second.samples,
        first.sample_rate,
    );
}

/// Two oscillators with phase randomization. Each `note_on` consumes
/// `fastrand` for unison phase, so whichever oscillator iterates first in
/// `ModuleGraph::note_on` ends up with a different phase. Catches any
/// regression in `nodes` iteration determinism.
fn dual_osc_patch() -> Patch {
    let mut patch = Patch::new("DeterminismDualOsc");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.4)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.4)
            .param_f("detune", 7.0)
            .build(),
    );
    add_env_amp_out_tail(&mut patch);
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("osc-2", "out", "amp-1", "in");
    patch
}

#[test]
fn offline_render_is_bit_exact_for_dual_oscillator_patch() {
    let rig = setup_with_patch(&dual_osc_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let first = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("first dual-osc render should succeed");
    let second = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("second dual-osc render should succeed");
    let third = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("third dual-osc render should succeed");

    assert_bit_exact(
        "dual-osc render-1 vs render-2",
        &first.samples,
        &second.samples,
        first.sample_rate,
    );
    assert_bit_exact(
        "dual-osc render-1 vs render-3",
        &first.samples,
        &third.samples,
        first.sample_rate,
    );
}

// §7.1: `OfflineEngineSession` amortizes engine + instrument-load setup
// across N renders. The contract is that a session reused across calls
// produces bit-exact identical buffers to a freshly-built engine. The
// dual-oscillator patch is the canonical regression target — it's the
// smallest patch that exposes `note_on`'s `fastrand`-consumption order
// dependency (caught §8.1 Round 2 of the determinism work).

#[test]
fn session_reuse_matches_fresh_engine_dual_osc() {
    let rig = setup_with_patch(&dual_osc_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let fresh = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("fresh-engine render should succeed");

    let (mut sess, _setup_warnings) = OfflineEngineSession::new(&rig.session, &rig.sample_library)
        .expect("session::new should succeed");
    let session_first = sess
        .render_range(&shared.song, 0, 3840)
        .expect("session render-1 should succeed");

    assert_bit_exact(
        "fresh-engine vs session render-1",
        &fresh.samples,
        &session_first.samples,
        fresh.sample_rate,
    );
}

#[test]
fn session_render_range_is_bit_exact_across_three_calls() {
    let rig = setup_with_patch(&dual_osc_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let (mut sess, _setup_warnings) = OfflineEngineSession::new(&rig.session, &rig.sample_library)
        .expect("session::new should succeed");
    let first = sess
        .render_range(&shared.song, 0, 3840)
        .expect("render-1 should succeed");
    let second = sess
        .render_range(&shared.song, 0, 3840)
        .expect("render-2 should succeed");
    let third = sess
        .render_range(&shared.song, 0, 3840)
        .expect("render-3 should succeed");

    assert_bit_exact(
        "session render-1 vs render-2",
        &first.samples,
        &second.samples,
        first.sample_rate,
    );
    assert_bit_exact(
        "session render-1 vs render-3",
        &first.samples,
        &third.samples,
        first.sample_rate,
    );
}

#[test]
fn session_render_range_is_bit_exact_for_noise_patch() {
    // Noise hammers `fastrand::f32()` every sample, so an unreseeded
    // session would diverge immediately on render-2.
    let rig = setup_with_patch(&noise_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let (mut sess, _setup_warnings) = OfflineEngineSession::new(&rig.session, &rig.sample_library)
        .expect("session::new should succeed");
    let first = sess
        .render_range(&shared.song, 0, 3840)
        .expect("noise render-1 should succeed");
    let second = sess
        .render_range(&shared.song, 0, 3840)
        .expect("noise render-2 should succeed");

    assert_bit_exact(
        "session noise render-1 vs render-2",
        &first.samples,
        &second.samples,
        first.sample_rate,
    );
}

#[test]
fn analyze_mix_metrics_are_stable_across_calls() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let mut lufs: Vec<f32> = Vec::with_capacity(4);
    let mut rms: Vec<f32> = Vec::with_capacity(4);
    let mut clipped: Vec<u32> = Vec::with_capacity(4);
    for _ in 0..4 {
        let r = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
            .expect("render should succeed");
        let a = analyze_mix_buffer(&r.samples, r.sample_rate);
        lufs.push(a.lufs_integrated);
        rms.push(a.rms);
        clipped.push(a.clipped_samples);
    }

    assert!(
        lufs.windows(2).all(|w| w[0].to_bits() == w[1].to_bits()),
        "LUFS-I must be bit-exact across calls: {lufs:?}"
    );
    assert!(
        rms.windows(2).all(|w| w[0].to_bits() == w[1].to_bits()),
        "RMS must be bit-exact across calls: {rms:?}"
    );
    assert!(
        clipped.windows(2).all(|w| w[0] == w[1]),
        "clipped-sample count must be exact across calls: {clipped:?}"
    );
}

// --- Channel-bus stage (Phase 1) regression guards -------------------------
//
// The per-instrument fader/pan (`Instrument::stereo_gain`) moved out of
// `Instrument::process` into the engine's channel-bus stage
// (`mix_channel_busses`). These tests pin that the fader/pan are still applied
// — to the correct channel — and that the move did not introduce any
// non-determinism.

#[test]
fn channel_bus_pan_biases_left_channel() {
    let mut rig = setup_with_patch(&sustain_patch());

    // Hard-ish pan left at unity volume. With the source panned left, the bus
    // stage's stereo_gain must bias energy to L; if the fader/pan were dropped
    // when moved to the bus stage, a centered source would give L == R.
    rig.session
        .set_instrument_volume(InstrumentId::FIRST, Gain::new(1.0))
        .expect("set volume");
    rig.session
        .set_instrument_pan(InstrumentId::FIRST, BipolarValue::new(-0.7))
        .expect("set pan");
    process_block(&mut rig._engine, 8); // flush commands -> refresh snapshot

    let shared = McpSharedState::with_song(build_arpeggio_song());

    let a = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render a");
    let b = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render b");

    // The move must not have introduced non-determinism.
    assert_bit_exact(
        "pan/vol bus render-1 vs render-2",
        &a.samples,
        &b.samples,
        a.sample_rate,
    );

    let l = left_rms(&a.samples);
    let r = right_rms(&a.samples);
    assert!(l > 0.0, "left channel should be non-silent");
    assert!(
        r > 0.0,
        "right channel should still be non-silent for pan -0.7"
    );
    assert!(
        l > r * 1.3,
        "pan-left should bias energy to the left (L={l}, R={r})"
    );
}

#[test]
fn channel_bus_volume_scales_level() {
    let mut rig = setup_with_patch(&sustain_patch());
    let shared = McpSharedState::with_song(build_arpeggio_song());

    // Center pan throughout so only the volume fader differs between renders.
    rig.session
        .set_instrument_pan(InstrumentId::FIRST, BipolarValue::CENTER)
        .expect("set pan");

    rig.session
        .set_instrument_volume(InstrumentId::FIRST, Gain::new(1.0))
        .expect("set unity volume");
    process_block(&mut rig._engine, 8);
    let unity = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render unity");

    rig.session
        .set_instrument_volume(InstrumentId::FIRST, Gain::new(0.5))
        .expect("set half volume");
    process_block(&mut rig._engine, 8);
    let half = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render half");

    // The test signal sits well below the soft-clip threshold, so halving the
    // channel fader halves the level (roughly) rather than being reshaped.
    let unity_rms = left_rms(&unity.samples);
    let half_rms = left_rms(&half.samples);
    assert!(unity_rms > 0.0, "unity render should be audible");
    let ratio = half_rms / unity_rms;
    assert!(
        (0.4..=0.6).contains(&ratio),
        "half volume should roughly halve the level (ratio={ratio}, unity={unity_rms}, half={half_rms})"
    );
}

// --- Track fader (Phase 2) regression guards -------------------------------
//
// docs/TODO.md §0.1: SequencerTrack volume/pan were stored but never reached
// audio. These pin that the OWNING TRACK's fader now composes into the channel
// bus. The instrument's own pan/volume stay neutral, so any effect here comes
// purely from the track fader (read live from the Song during render).

#[test]
fn track_pan_biases_output() {
    let rig = setup_with_patch(&sustain_patch());
    let shared = McpSharedState::with_song(build_arpeggio_song());

    // Instrument pan/volume left at default (centre/unity); pan the TRACK left.
    set_first_track_fader(&shared.song, 1.0, -0.7);

    let out = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render");

    let l = left_rms(&out.samples);
    let r = right_rms(&out.samples);
    assert!(
        l > 0.0 && r > 0.0,
        "both channels non-silent (L={l}, R={r})"
    );
    assert!(
        l > r * 1.3,
        "track pan-left should bias energy to the left (L={l}, R={r})"
    );
}

#[test]
fn track_volume_scales_output() {
    let rig = setup_with_patch(&sustain_patch());
    let shared = McpSharedState::with_song(build_arpeggio_song());

    // Centre pan throughout; only the TRACK volume fader differs.
    set_first_track_fader(&shared.song, 1.0, 0.0);
    let unity = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render unity");

    set_first_track_fader(&shared.song, 0.5, 0.0);
    let half = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render half");

    let unity_rms = left_rms(&unity.samples);
    let half_rms = left_rms(&half.samples);
    assert!(unity_rms > 0.0, "unity-track render should be audible");
    let ratio = half_rms / unity_rms;
    assert!(
        (0.4..=0.6).contains(&ratio),
        "half track volume should roughly halve the level (ratio={ratio})"
    );
}

// --- Automation arms (Phase 2) regression guards ---------------------------
//
// Track Volume/Pan/Mute automation now drives the channel-bus fader (via the
// sequencer's track_auto override), and Global MasterVolume drives the engine
// master gain. These render a steady sustained note with a fader ramping
// 1.0 -> 0.0 across the pattern and assert the first half is louder than the
// second — proof the automation reaches audio.

#[test]
fn track_volume_automation_ramps_down() {
    let rig = setup_with_patch(&sustain_patch());
    let (song, pattern_id, track_id) = build_sustained_note_song("TrackVolRamp");
    add_ramp_automation(
        &song,
        pattern_id,
        AutomationTarget::Track {
            track: track_id,
            param: TrackParam::Volume,
        },
        1.0,
        0.0,
    );
    let shared = McpSharedState::with_song(song);

    let out = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render");

    let mid = (out.samples.len() / 2) & !1; // keep stereo-frame aligned
    let first = left_rms(&out.samples[..mid]);
    let second = left_rms(&out.samples[mid..]);
    assert!(first > 0.0, "first half should be audible");
    assert!(
        first > second * 1.5,
        "track-volume ramp-down: first half louder (first={first}, second={second})"
    );
}

#[test]
fn global_master_volume_automation_ramps_down() {
    let rig = setup_with_patch(&sustain_patch());
    let (song, pattern_id, _track_id) = build_sustained_note_song("MasterVolRamp");
    add_ramp_automation(
        &song,
        pattern_id,
        AutomationTarget::Global(GlobalParam::MasterVolume),
        1.0,
        0.0,
    );
    let shared = McpSharedState::with_song(song);

    let out = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render");

    let mid = (out.samples.len() / 2) & !1;
    let first = left_rms(&out.samples[..mid]);
    let second = left_rms(&out.samples[mid..]);
    assert!(first > 0.0, "first half should be audible");
    assert!(
        first > second * 1.5,
        "master-volume ramp-down: first half louder (first={first}, second={second})"
    );
}

// F4: per-module automation (`AutomationTarget::Module`) must reach the offline
// render too — the `analyze_*` path runs the same engine `process()` (Play→Seek→
// process), which routes sequencer Parameter events through the override layer.
// This is the case the roadmap pitfall worried analysis "reads base values" for;
// it does not. Ramps the amplifier's `level` param 1.0 → 0.0 via a Module lane
// and asserts the rendered first half is clearly louder than the second.
#[test]
fn module_param_automation_ramps_down() {
    let rig = setup_with_patch(&sustain_patch());
    let (song, pattern_id, _track_id) = build_sustained_note_song("ModuleLevelRamp");
    add_ramp_automation(
        &song,
        pattern_id,
        AutomationTarget::Module {
            instrument: SeqInstrumentId(0),
            module_type: ModuleType::Amplifier,
            instance: 1,
            param_id: "level".into(),
        },
        1.0,
        0.0,
    );
    let shared = McpSharedState::with_song(song);

    let out = render_arrangement_to_buffer(&rig.session, &rig.sample_library, &shared, 0, 3840)
        .expect("render");

    let mid = (out.samples.len() / 2) & !1;
    let first = left_rms(&out.samples[..mid]);
    let second = left_rms(&out.samples[mid..]);
    assert!(first > 0.0, "first half should be audible");
    assert!(
        first > second * 1.5,
        "module-param (amp level) ramp-down must reach the offline render: \
         first={first}, second={second}"
    );
}
