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
use pertylizer::mcp_bridge::analyze_rendered_buffer;

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
    let result = analyze_rendered_buffer(&rendered, 57, 100, 700, None);

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
    let result = analyze_rendered_buffer(&rendered, 57, 100, 700, None);

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
    let result = analyze_rendered_buffer(&rendered, 57, 100, 700, None);

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
    let result = analyze_rendered_buffer(&rendered, 57, 100, note_ms, None);

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
    let result = analyze_rendered_buffer(&rendered, 57, 100, note_ms, None);

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
    let result = analyze_rendered_buffer(&rendered, 57, 100, note_ms, Some(57));

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
