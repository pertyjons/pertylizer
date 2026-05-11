//! Determinism test for the offline arrangement renderer.
//!
//! `render_arrangement_to_buffer` must produce bit-exact identical
//! `samples: Vec<f32>` for the same engine state and tick range every call.
//! This is the load-bearing assumption behind `analyze_section` A/B use
//! cases (verse vs chorus, before vs after a knob tweak, master vs the
//! per-track sum).
//!
//! Surfaced by live testing on 2026-05-11: four consecutive
//! `analyze_section` calls on the same range produced ~0.44 dB LUFS-I
//! spread, ~10 % clipped-samples spread, ~0.3 dB RMS spread. See
//! `docs/mcp-music-tools-plan.md` §8.1.

mod common;

use synth_core::ModuleType;

use pertylizer::audio::arrangement_render::render_arrangement_to_buffer;
use pertylizer::audio::mix_analysis::analyze_mix_buffer;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};

use common::{build_arpeggio_song, first_divergence, setup_with_patch, sustain_patch};

#[test]
fn offline_render_is_bit_exact_across_calls() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let start_tick = 0u64;
    let end_tick = 3840u64;

    let first = render_arrangement_to_buffer(&rig.session, &shared, start_tick, end_tick)
        .expect("first render should succeed");
    let second = render_arrangement_to_buffer(&rig.session, &shared, start_tick, end_tick)
        .expect("second render should succeed");
    let third = render_arrangement_to_buffer(&rig.session, &shared, start_tick, end_tick)
        .expect("third render should succeed");

    if let Some(idx) = first_divergence(&first.samples, &second.samples) {
        let frame = idx / 2;
        let seconds = frame as f32 / first.sample_rate as f32;
        let a = first.samples[idx];
        let b = second.samples[idx];
        panic!(
            "render-1 and render-2 diverge at sample {idx} (frame {frame}, t={seconds:.4}s): \
             {a} vs {b} (bits: {:#x} vs {:#x})",
            a.to_bits(),
            b.to_bits()
        );
    }

    if let Some(idx) = first_divergence(&first.samples, &third.samples) {
        let frame = idx / 2;
        panic!("render-1 and render-3 diverge at sample {idx} (frame {frame})");
    }
}

/// A noise-driven patch hammers `fastrand::f32()` on every sample. If
/// the offline render didn't reseed deterministically the per-sample
/// values would diverge by O(1) immediately. This complements the
/// sawtooth test (which only diverges at note-on phase randomization).
fn noise_patch() -> Patch {
    let mut patch = Patch::new("DeterminismNoise");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .param_f("level", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.005)
            .param_f("decay", 0.0)
            .param_f("sustain", 1.0)
            .param_f("release", 0.05)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    patch.add_connection("noise-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

#[test]
fn offline_render_is_bit_exact_for_noise_patch() {
    let rig = setup_with_patch(&noise_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let first = render_arrangement_to_buffer(&rig.session, &shared, 0, 3840)
        .expect("first noise render should succeed");
    let second = render_arrangement_to_buffer(&rig.session, &shared, 0, 3840)
        .expect("second noise render should succeed");

    if let Some(idx) = first_divergence(&first.samples, &second.samples) {
        let frame = idx / 2;
        let a = first.samples[idx];
        let b = second.samples[idx];
        panic!("noise patch diverges at sample {idx} (frame {frame}): {a} vs {b}");
    }
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
        let r = render_arrangement_to_buffer(&rig.session, &shared, 0, 3840)
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
