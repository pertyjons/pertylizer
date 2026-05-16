//! Determinism test for the offline arrangement renderer.
//!
//! `render_arrangement_to_buffer` must produce bit-exact identical
//! `samples: Vec<f32>` for the same engine state and tick range every call.
//! This is the load-bearing assumption behind `analyze_section` A/B use
//! cases (verse vs chorus, before vs after a knob tweak, master vs the
//! per-track sum).

mod common;

use synth_core::ModuleType;

use pertylizer::audio::arrangement_render::{OfflineEngineSession, render_arrangement_to_buffer};
use pertylizer::audio::mix_analysis::analyze_mix_buffer;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};

use common::{
    add_env_amp_out_tail, assert_bit_exact, build_arpeggio_song, setup_with_patch, sustain_patch,
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
