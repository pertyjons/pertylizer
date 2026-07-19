//! Integration tests for the offline analysis metrics in
//! [`pertylizer::mcp_bridge::analyze_rendered_buffer`].
//!
//! These tests cover:
//! - **Bug 3** (`stereo_width`): a continuous 0..1 measure derived from
//!   `side / (mid + side)`; anti-phase signal must report wide, mono must
//!   report 0.
//! - **Bug 4** (release window placement): the release-spectrum window must
//!   never slip backward past `note_off + 25 ms`, even when the post-note-off
//!   tail is too short for a full nominal window.
//! - **Bug 5** (anti-phase pitch detection): pitch / spectrum / energy
//!   analysis must operate on a phase-robust signal, not the L+R mono mix
//!   that cancels anti-phase tones.
//!
//! All tests synthesize a [`RenderedNote`] directly so we can exercise the
//! analysis path without spinning up the full audio engine.

use std::f32::consts::TAU;

use synth_core::MidiNote;

use pertylizer::audio::preview::RenderedNote;
use pertylizer::mcp_bridge::{analyze_rendered_buffer, analyze_rendered_buffer_with_window};
use synth_mcp::types::AnalysisSignalMode;

const SAMPLE_RATE: u32 = 44_100;

/// Build a stereo-interleaved buffer where each frame is `(l, r)` from the
/// supplied closure. `total_frames` is the desired output length in frames.
fn synth_stereo<F: FnMut(usize) -> (f32, f32)>(total_frames: usize, mut f: F) -> Vec<f32> {
    let mut out = Vec::with_capacity(total_frames * 2);
    for i in 0..total_frames {
        let (l, r) = f(i);
        out.push(l);
        out.push(r);
    }
    out
}

fn synth_mono<F: FnMut(usize) -> f32>(total_frames: usize, mut f: F) -> Vec<f32> {
    (0..total_frames).map(&mut f).collect()
}

/// Wrap a stereo-interleaved buffer in a `RenderedNote` ready for analysis.
fn make_stereo_rendered(
    samples: Vec<f32>,
    note_frames: u64,
    total_frames: usize,
    sample_rate: u32,
) -> RenderedNote {
    let duration_seconds = total_frames as f32 / sample_rate as f32;
    RenderedNote {
        samples,
        sample_rate,
        duration_seconds,
        channels: 2,
        effective_note: MidiNote::new(57), // A3 (220 Hz)
        note_off_frame: note_frames,
        warnings: Vec::new(),
    }
}

fn make_mono_rendered(
    samples: Vec<f32>,
    note_frames: u64,
    total_frames: usize,
    sample_rate: u32,
) -> RenderedNote {
    let duration_seconds = total_frames as f32 / sample_rate as f32;
    RenderedNote {
        samples,
        sample_rate,
        duration_seconds,
        channels: 1,
        effective_note: MidiNote::new(57),
        note_off_frame: note_frames,
        warnings: Vec::new(),
    }
}

// -------------------------------------------------------------------------
// Configurable envelope window: a fine window resolves a fast attack that the
// default 50 ms window collapses into its first frame (attack_ms = 0).
// -------------------------------------------------------------------------

#[test]
fn fine_envelope_window_resolves_a_fast_attack() {
    // 220 Hz tone with a 10 ms linear attack, then held. At the default 50 ms
    // window the whole attack lives inside frame 0; at 2 ms it spans ~5 frames.
    let total_frames = SAMPLE_RATE as usize; // 1 s
    let note_frames = (SAMPLE_RATE as f32 * 0.7) as u64;
    let attack_frames = SAMPLE_RATE as f32 * 0.010; // 10 ms
    let samples = synth_mono(total_frames, |i| {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (i as f32 / attack_frames).min(1.0); // 0→1 over 10 ms, then 1
        (TAU * 220.0 * t).sin() * 0.5 * env
    });
    let rendered = make_mono_rendered(samples, note_frames, total_frames, SAMPLE_RATE);

    let coarse = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, 700, None); // default 50 ms
    let fine =
        analyze_rendered_buffer_with_window(&rendered, MidiNote::new(57), 100, 700, None, 2.0);

    // The chosen resolution is echoed back and drives the envelope length.
    assert_eq!(coarse.envelope_window_ms, 50.0);
    assert_eq!(fine.envelope_window_ms, 2.0);
    assert!(
        fine.rms_envelope.len() > coarse.rms_envelope.len(),
        "finer window must yield more frames: fine={} coarse={}",
        fine.rms_envelope.len(),
        coarse.rms_envelope.len()
    );

    // The 50 ms window can't see a 10 ms attack (collapses to 0); the 2 ms one
    // resolves it to roughly its true length.
    assert_eq!(
        coarse.envelope_estimate.attack_ms, 0.0,
        "default window should collapse the fast attack"
    );
    let fine_attack = fine.envelope_estimate.attack_ms;
    assert!(
        (2.0..=30.0).contains(&fine_attack),
        "fine window should resolve the ~10 ms attack, got {fine_attack} ms"
    );
}

// -------------------------------------------------------------------------
// Bug 3 — stereo_width is continuous: 0 for mono, ~1 for anti-phase.
// -------------------------------------------------------------------------

#[test]
fn stereo_width_high_for_antiphase_signal() {
    // 220 Hz tone, L = +sin, R = -sin → mid≈0, side≈signal → width ≈ 1.
    let total_frames = SAMPLE_RATE as usize; // 1 s
    let note_frames = (SAMPLE_RATE as f32 * 0.7) as u64;
    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (TAU * 220.0 * t).sin() * 0.5;
        (s, -s)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, SAMPLE_RATE);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, 700, None);

    let width = result.stereo_width.expect("stereo_width should be set");
    assert!(
        width > 0.7,
        "anti-phase signal should report wide stereo, got {width}"
    );
    assert!(
        width <= 1.0 + 1e-3,
        "stereo_width should stay in 0..1, got {width}"
    );
}

#[test]
fn stereo_width_zero_for_mono() {
    // Mono input → stereo_width is None (no second channel to compare).
    let total_frames = SAMPLE_RATE as usize;
    let note_frames = (SAMPLE_RATE as f32 * 0.7) as u64;
    let samples = synth_mono(total_frames, |i| {
        let t = i as f32 / SAMPLE_RATE as f32;
        (TAU * 220.0 * t).sin() * 0.5
    });
    let rendered = make_mono_rendered(samples, note_frames, total_frames, SAMPLE_RATE);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, 700, None);

    assert!(
        result.stereo_width.is_none(),
        "mono input should not produce a stereo_width value, got {:?}",
        result.stereo_width
    );
}

#[test]
fn stereo_width_zero_for_identical_lr() {
    // Stereo buffer where L == R is "mono routed to both outs": all energy
    // in the mid channel, side is empty → width should be ~0.
    let total_frames = SAMPLE_RATE as usize;
    let note_frames = (SAMPLE_RATE as f32 * 0.7) as u64;
    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (TAU * 220.0 * t).sin() * 0.5;
        (s, s)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, SAMPLE_RATE);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, 700, None);

    let width = result.stereo_width.expect("stereo_width should be set");
    assert!(
        width.abs() < 0.05,
        "L == R stereo should report ~0 width, got {width}"
    );
}

// -------------------------------------------------------------------------
// Bug 4 — release window must not slip backward past note_off + offset.
// -------------------------------------------------------------------------

#[test]
fn release_window_does_not_cross_note_off_on_short_tail() {
    // 50 ms note + 10 ms tail at 44.1 kHz = 60 ms total.
    // Pre-fix: the release window slid backward to fit a full 100 ms slice,
    // pulling sustain audio into the "release" slice.
    let sr = SAMPLE_RATE;
    let total_ms = 60u32;
    let note_ms = 50u32;
    let total_frames = (f64::from(total_ms) / 1000.0 * f64::from(sr)) as usize;
    let note_frames = (f64::from(note_ms) / 1000.0 * f64::from(sr)) as u64;
    // A simple held tone with a hard cutoff at note-off keeps RMS predictable
    // but the precise content does not matter — we are only inspecting the
    // window placement.
    let samples = synth_stereo(total_frames, |i| {
        let after_off = (i as u64) >= note_frames;
        let t = i as f32 / sr as f32;
        let s = if after_off {
            0.0
        } else {
            (TAU * 220.0 * t).sin() * 0.4
        };
        (s, s)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, sr);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, note_ms, None);

    let release_start_ms = result
        .release_window_start_ms
        .expect("release_window_start_ms");
    let note_off_ms = note_frames as f32 * 1000.0 / sr as f32;
    assert!(
        release_start_ms >= note_off_ms,
        "release window must not slip before note_off: \
         got release_start={release_start_ms} ms, note_off={note_off_ms} ms"
    );
    // The 25 ms post-note-off offset should be applied when the tail allows
    // it. Here total = 60 ms, note_off = 50 ms, offset = 25 ms → the offset
    // pushes the start past total, so release_start should clamp to total.
    let total_ms_f = total_frames as f32 * 1000.0 / sr as f32;
    assert!(
        release_start_ms >= note_off_ms + 0.0 && release_start_ms <= total_ms_f + 0.5,
        "release window should sit between note_off and total render length, \
         got {release_start_ms} ms (note_off={note_off_ms}, total={total_ms_f})"
    );
}

#[test]
fn release_window_offset_applied_when_tail_allows() {
    // Long tail (500 ms after note-off): the release window should land
    // exactly at note_off + 25 ms.
    let sr = SAMPLE_RATE;
    let note_ms = 200u32;
    let tail_ms = 500u32;
    let total_ms = note_ms + tail_ms;
    let total_frames = (f64::from(total_ms) / 1000.0 * f64::from(sr)) as usize;
    let note_frames = (f64::from(note_ms) / 1000.0 * f64::from(sr)) as u64;
    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / sr as f32;
        let after_off = (i as u64) >= note_frames;
        let env = if after_off {
            0.5 * (-(t - note_ms as f32 * 1e-3) * 4.0).exp()
        } else {
            0.5
        };
        let s = (TAU * 220.0 * t).sin() * env;
        (s, s)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, sr);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, note_ms, None);

    let release_start_ms = result
        .release_window_start_ms
        .expect("release_window_start_ms");
    let note_off_ms = note_frames as f32 * 1000.0 / sr as f32;
    let expected = note_off_ms + 25.0;
    let delta = (release_start_ms - expected).abs();
    assert!(
        delta < 1.0,
        "release window should start at note_off + 25 ms when tail allows: \
         got {release_start_ms} ms, expected ~{expected} ms"
    );
}

// -------------------------------------------------------------------------
// Bug 5 — anti-phase tonal content must still produce a fundamental_hz.
// -------------------------------------------------------------------------

#[test]
fn fundamental_detected_for_antiphase_tonal_signal() {
    // Pure 220 Hz sine, anti-phase between L and R. The mono mix is ≈ 0
    // and the old code would report fundamental_hz = 0. The fix runs pitch
    // detection on a phase-robust per-sample max(|L|,|R|) signal, so the
    // 220 Hz peak survives.
    let sr = SAMPLE_RATE;
    let note_ms = 800u32;
    let total_ms = 1000u32;
    let total_frames = (f64::from(total_ms) / 1000.0 * f64::from(sr)) as usize;
    let note_frames = (f64::from(note_ms) / 1000.0 * f64::from(sr)) as u64;
    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / sr as f32;
        let s = (TAU * 220.0 * t).sin() * 0.5;
        (s, -s)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, sr);
    // expected_note=A3 (57) so the fundamental search is anchored at 220 Hz.
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, note_ms, Some(57));

    let f0 = result.fundamental_hz;
    assert!(
        f0 > 0.0,
        "anti-phase tonal signal should still yield a fundamental, got {f0}"
    );
    let err_ratio = (f0 - 220.0).abs() / 220.0;
    assert!(
        err_ratio < 0.05,
        "fundamental should be near 220 Hz (±5 %), got {f0} ({:.1} %)",
        err_ratio * 100.0
    );
}

// -------------------------------------------------------------------------
// Diagnostic: per-channel fundamentals + analysis_signal_mode tag. Lets the
// caller distinguish the wide-stereo case (L and R have different
// fundamentals) from the typical L=R / anti-phase case where the pooled
// `fundamental_hz` is meaningful.
// -------------------------------------------------------------------------

#[test]
fn fundamental_per_channel_distinct_tones() {
    // Stereo signal with 220 Hz on L and 330 Hz on R. The synthetic
    // analysis_signal (max(|L|,|R|)) blends both, so `fundamental_hz`
    // reports a single value that picks whichever peak ends up loudest in
    // the combined spectrum (here both channels have equal amplitude, so
    // we don't pin a specific winner — see the assertion below). The
    // per-channel detectors run on each channel in isolation and must
    // recover 220 Hz and 330 Hz respectively.
    let sr = SAMPLE_RATE;
    let note_ms = 800u32;
    let total_ms = 1000u32;
    let total_frames = (f64::from(total_ms) / 1000.0 * f64::from(sr)) as usize;
    let note_frames = (f64::from(note_ms) / 1000.0 * f64::from(sr)) as u64;
    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / sr as f32;
        let l = (TAU * 220.0 * t).sin() * 0.5;
        let r = (TAU * 330.0 * t).sin() * 0.5;
        (l, r)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, sr);
    // No `expected_note` — the wide search range catches both tones.
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, note_ms, None);

    assert_eq!(
        result.analysis_signal_mode,
        AnalysisSignalMode::MaxAbsStereo,
        "stereo input should report MaxAbsStereo"
    );

    let f_l = result.fundamental_left.expect("fundamental_left set");
    let f_r = result.fundamental_right.expect("fundamental_right set");
    let err_l = (f_l - 220.0).abs() / 220.0;
    let err_r = (f_r - 330.0).abs() / 330.0;
    assert!(
        err_l < 0.05,
        "fundamental_left should be ~220 Hz (±5 %), got {f_l} ({:.1} %)",
        err_l * 100.0
    );
    assert!(
        err_r < 0.05,
        "fundamental_right should be ~330 Hz (±5 %), got {f_r} ({:.1} %)",
        err_r * 100.0
    );

    // Both channels carry a single clean sine, so confidence should be high.
    let c_l = result
        .fundamental_left_confidence
        .expect("fundamental_left_confidence set");
    let c_r = result
        .fundamental_right_confidence
        .expect("fundamental_right_confidence set");
    assert!(
        c_l > 0.7,
        "fundamental_left_confidence should be high for a clean sine, got {c_l}"
    );
    assert!(
        c_r > 0.7,
        "fundamental_right_confidence should be high for a clean sine, got {c_r}"
    );

    // Documented behavior of the pooled `fundamental_hz` on this input:
    // the per-sample max(|L|,|R|) signal flips between L and R sample-by-
    // sample, picking whichever channel has the larger instantaneous
    // magnitude. With equal amplitudes that biases toward the higher
    // frequency (its peaks come more often), so the resulting spectrum
    // is dominated by the 330 Hz peak — the observed value is ~330 Hz.
    // This assertion nails current behavior down so regressions are
    // visible; the per-channel asserts above are the real contract for
    // the new fields.
    let f0 = result.fundamental_hz;
    let err_pooled = (f0 - 330.0).abs() / 330.0;
    assert!(
        err_pooled < 0.05,
        "pooled fundamental_hz on equal-amplitude L=220/R=330 stereo \
         currently lands ~330 Hz (R wins on max-abs); got {f0} ({:.1} %)",
        err_pooled * 100.0
    );
}

#[test]
fn analysis_signal_mode_mono_for_mono_input() {
    let total_frames = SAMPLE_RATE as usize;
    let note_frames = (SAMPLE_RATE as f32 * 0.7) as u64;
    let samples = synth_mono(total_frames, |i| {
        let t = i as f32 / SAMPLE_RATE as f32;
        (TAU * 220.0 * t).sin() * 0.5
    });
    let rendered = make_mono_rendered(samples, note_frames, total_frames, SAMPLE_RATE);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, 700, None);

    assert_eq!(
        result.analysis_signal_mode,
        AnalysisSignalMode::Mono,
        "mono input should tag analysis_signal_mode = Mono"
    );
    assert!(
        result.fundamental_left.is_none(),
        "mono input must not populate fundamental_left, got {:?}",
        result.fundamental_left
    );
    assert!(
        result.fundamental_right.is_none(),
        "mono input must not populate fundamental_right, got {:?}",
        result.fundamental_right
    );
    assert!(
        result.fundamental_left_confidence.is_none(),
        "mono input must not populate fundamental_left_confidence"
    );
    assert!(
        result.fundamental_right_confidence.is_none(),
        "mono input must not populate fundamental_right_confidence"
    );
}

#[test]
fn analysis_signal_mode_stereo_for_stereo_input() {
    // Reuse the anti-phase 220 Hz signal from the Bug 5 test — any stereo
    // input is enough to verify the tag is set and per-channel fields are
    // populated.
    let sr = SAMPLE_RATE;
    let note_ms = 800u32;
    let total_ms = 1000u32;
    let total_frames = (f64::from(total_ms) / 1000.0 * f64::from(sr)) as usize;
    let note_frames = (f64::from(note_ms) / 1000.0 * f64::from(sr)) as u64;
    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / sr as f32;
        let s = (TAU * 220.0 * t).sin() * 0.5;
        (s, -s)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, sr);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, note_ms, Some(57));

    assert_eq!(
        result.analysis_signal_mode,
        AnalysisSignalMode::MaxAbsStereo,
        "stereo input should tag analysis_signal_mode = MaxAbsStereo"
    );
    assert!(
        result.fundamental_left.is_some(),
        "stereo input must populate fundamental_left"
    );
    assert!(
        result.fundamental_right.is_some(),
        "stereo input must populate fundamental_right"
    );
    assert!(
        result.fundamental_left_confidence.is_some(),
        "stereo input must populate fundamental_left_confidence"
    );
    assert!(
        result.fundamental_right_confidence.is_some(),
        "stereo input must populate fundamental_right_confidence"
    );
}

#[test]
fn fundamental_per_channel_confidence_drops_for_noisy_channel() {
    // Left: clean 220 Hz sine — high confidence expected.
    // Right: deterministic broadband noise — low confidence expected.
    // The new per-channel confidence fields let the caller see that the
    // right-channel fundamental (whatever number it picks) is unreliable
    // even when fundamental_right itself is non-zero.
    let sr = SAMPLE_RATE;
    let note_ms = 800u32;
    let total_ms = 1000u32;
    let total_frames = (f64::from(total_ms) / 1000.0 * f64::from(sr)) as usize;
    let note_frames = (f64::from(note_ms) / 1000.0 * f64::from(sr)) as u64;

    let samples = synth_stereo(total_frames, |i| {
        let t = i as f32 / sr as f32;
        let left = (TAU * 220.0 * t).sin() * 0.5;
        // Deterministic LCG-style pseudo-noise in [-0.5, 0.5].
        let bits = (i as u32)
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let right = bits as f32 / u32::MAX as f32 - 0.5;
        (left, right)
    });
    let rendered = make_stereo_rendered(samples, note_frames, total_frames, sr);
    let result = analyze_rendered_buffer(&rendered, MidiNote::new(57), 100, note_ms, Some(57));

    let c_l = result
        .fundamental_left_confidence
        .expect("fundamental_left_confidence set");
    let c_r = result
        .fundamental_right_confidence
        .expect("fundamental_right_confidence set");

    assert!(
        c_l > 0.7,
        "clean-tone left channel should have high confidence, got {c_l}"
    );
    assert!(
        c_r < c_l,
        "noisy right channel must have lower confidence than clean left \
         channel; got c_l={c_l}, c_r={c_r}"
    );
    assert!(
        c_r < 0.5,
        "noisy right channel should have low confidence (<0.5), got {c_r}"
    );
}
