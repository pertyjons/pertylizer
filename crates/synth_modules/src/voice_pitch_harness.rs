//! Shared test harness for the continuous-voice-pitch rollout (see
//! `plans/voice-pitch-rollout-plan.md`). Each pitched source module gets a
//! regression test that `note_on`s at `F`, drives `set_voice_pitch(2F)` per
//! block, and asserts the output fundamental doubled — plus a static-note
//! no-regression check. This module is the shared estimator + render plumbing.
//!
//! Test-only.
#![cfg(test)]

use std::collections::HashMap;

use synth_core::VoicePitch;
use synth_core::{AudioBuffer, InputPorts, PolyModule, PortName, ProcessContext, SampleRate};

/// Estimate the fundamental (Hz) of `signal` via the Average Magnitude
/// Difference Function, searching a **bounded** lag window (±35%) around the
/// expected period. AMDF is robust where zero-crossing fails (harmonic-rich,
/// formant-heavy, noise-coupled outputs), and the bound both speeds the search
/// and prevents octave-jump errors. `AMDF(τ) = Σ|x(n) − x(n−τ)|`, normalized by
/// the overlap count; the period is the minimizing `τ`.
pub(crate) fn amdf_fundamental(signal: &[f32], sample_rate: f32, expected_hz: f32) -> f32 {
    let period = sample_rate / expected_hz;
    let min_lag = ((period * 0.65) as usize).max(2);
    let hi = ((period * 1.35) as usize).max(min_lag + 1);
    let max_lag = hi.min(signal.len().saturating_sub(1));
    let mut best_lag = min_lag;
    let mut best = f32::MAX;
    for lag in min_lag..=max_lag {
        let mut sum = 0.0f32;
        for i in lag..signal.len() {
            sum += (signal[i] - signal[i - lag]).abs();
        }
        let amdf = sum / (signal.len() - lag) as f32;
        if amdf < best {
            best = amdf;
            best_lag = lag;
        }
    }
    sample_rate / best_lag as f32
}

/// Render `blocks × block_len` samples from a source `module` at `sr`, calling
/// `before_block` immediately before each `process` (the seam where a test
/// pushes the per-block voice pitch). Returns one channel of output: the mono
/// `OUT` port when the module writes it, else the left channel (`OUT_L`) for
/// stereo sources. The default sample rate is [`SampleRate::DVD_QUALITY`].
pub(crate) fn render_mono(
    module: &mut dyn PolyModule,
    sr: SampleRate,
    blocks: usize,
    block_len: usize,
    mut before_block: impl FnMut(&mut dyn PolyModule),
) -> Vec<f32> {
    module.set_sample_rate(sr);
    let ctx = ProcessContext {
        samples: synth_core::SampleCount::new(block_len),
        sample_rate: sr,
        ..ProcessContext::default()
    };
    let mut out = Vec::with_capacity(blocks * block_len);
    for _ in 0..blocks {
        before_block(module);
        // Provide all three common source ports; pick whichever the module fills.
        let mut outs = HashMap::new();
        outs.insert(PortName::OUT, AudioBuffer::new(block_len));
        outs.insert(PortName::OUT_L, AudioBuffer::new(block_len));
        outs.insert(PortName::OUT_R, AudioBuffer::new(block_len));
        module.process(InputPorts::empty(), &mut outs, &ctx);
        let mono = &outs[&PortName::OUT];
        let has_mono = (0..mono.len()).any(|i| mono[i] != 0.0);
        let b = if has_mono {
            mono
        } else {
            &outs[&PortName::OUT_L]
        };
        for i in 0..b.len() {
            out.push(b[i]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The estimator recovers a pure sine's frequency within the bound.
    #[test]
    fn amdf_recovers_sine_fundamental() {
        let sr = 48_000.0;
        for hz in [110.0_f32, 220.0, 440.0, 880.0] {
            let n = 4096;
            let sig: Vec<f32> = (0..n)
                .map(|i| (i as f32 / sr * hz * std::f32::consts::TAU).sin())
                .collect();
            let est = amdf_fundamental(&sig, sr, hz);
            let cents = 1200.0 * (est / hz).log2();
            assert!(
                cents.abs() < 50.0,
                "hz={hz}: estimated {est} ({cents} cents off)"
            );
        }
    }

    /// End-to-end harness check against `Oscillator` (which already implements
    /// `set_voice_pitch`): driving 2× voice pitch doubles the rendered
    /// fundamental, and a static note stays put. Exercises `render_mono` + the
    /// estimator on a real module, the template every Phase 1+ test follows.
    #[test]
    fn harness_tracks_oscillator_voice_pitch() {
        use crate::oscillator::Oscillator;
        use synth_core::{Hertz, MidiNote, Velocity};

        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let f = MidiNote::A4.to_frequency().as_f32();

        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        // Static note: no per-block override → output stays at the note pitch.
        let mut osc = Oscillator::new();
        osc.note_on(MidiNote::A4, Velocity::MAX);
        let stat = render_mono(&mut osc, sr, 4, 1024, |_| {});
        let est_stat = amdf_fundamental(&stat[2048..], srf, f);
        assert!(cents(est_stat, f).abs() < 50.0, "static: {est_stat} vs {f}");

        // 2× voice pitch → fundamental doubles.
        let mut osc2 = Oscillator::new();
        osc2.note_on(MidiNote::A4, Velocity::MAX);
        let up = render_mono(&mut osc2, sr, 4, 1024, |m| {
            m.set_voice_pitch(VoicePitch::tracking(Hertz::new(f * 2.0)));
        });
        let est_up = amdf_fundamental(&up[2048..], srf, f * 2.0);
        assert!(
            cents(est_up, f * 2.0).abs() < 50.0,
            "2x: {est_up} vs {}",
            f * 2.0
        );
    }

    /// A harmonic-rich (saw-like) signal — where zero-crossing would fail —
    /// still resolves to its fundamental, not a harmonic.
    #[test]
    fn amdf_handles_harmonic_rich_signal() {
        let sr = 48_000.0;
        let hz = 220.0_f32;
        let n = 4096;
        let sig: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr;
                (1..=8)
                    .map(|h| (t * hz * h as f32 * std::f32::consts::TAU).sin() / h as f32)
                    .sum()
            })
            .collect();
        let est = amdf_fundamental(&sig, sr, hz);
        let cents = 1200.0 * (est / hz).log2();
        assert!(cents.abs() < 50.0, "estimated {est} ({cents} cents off)");
    }
}
