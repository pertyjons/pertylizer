//! Gap 2 + Gap 3 regression for the spectral sample tools.
//!
//! Gap 2 — `analyze_sample_spectrum_impl` can select a single analysis window of
//! a sample, so a time-varying sound (a voiced frame vs an unvoiced frame) is
//! compared frame-to-frame instead of averaging to "unvoiced": the tone window
//! reads voiced and the silent window unvoiced, a window past the end zero-pads
//! (no panic), an absurd length is bounded, and a start past the end errors.
//!
//! Gap 3 — `analyze_sample_spectrogram_impl` slides an FFT over a WAV at its
//! NATIVE rate, so a sound alternating tone/silence shows frames flipping
//! voiced↔unvoiced and reports the file's real sample rate.

use std::sync::{Arc, RwLock};

use pertylizer::audio::preview::SharedSampleLibrary;
use pertylizer::mcp_bridge::{analyze_sample_spectrogram_impl, analyze_sample_spectrum_impl};
use synth_mcp::McpBridgeError;

const SR: u32 = 44_100;
const HALF_MS: f32 = 250.0;

/// Write a WAV: `tone_ms` of a 1 kHz sine, then `silence_ms` of silence.
fn write_tone_then_silence(path: &std::path::Path, tone_ms: f32, silence_ms: f32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    let tone_samples = (tone_ms / 1000.0 * SR as f32) as usize;
    let silence_samples = (silence_ms / 1000.0 * SR as f32) as usize;
    for i in 0..tone_samples {
        let s = 0.8 * (std::f32::consts::TAU * 1_000.0 * i as f32 / SR as f32).sin();
        w.write_sample(s).expect("write tone");
    }
    for _ in 0..silence_samples {
        w.write_sample(0.0f32).expect("write silence");
    }
    w.finalize().expect("finalize wav");
}

fn empty_library() -> SharedSampleLibrary {
    Arc::new(RwLock::new(synth_sampler::SampleLibrary::default()))
}

/// Write a WAV at `rate` Hz that alternates `seg_ms` of a 1 kHz tone with
/// `seg_ms` of silence for `segments` segments total (tone first).
fn write_alternating(path: &std::path::Path, rate: u32, seg_ms: f32, segments: usize) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("create wav");
    let seg_samples = (seg_ms / 1000.0 * rate as f32) as usize;
    for seg in 0..segments {
        let tone = seg % 2 == 0;
        for i in 0..seg_samples {
            let n = seg * seg_samples + i;
            let s = if tone {
                0.8 * (std::f32::consts::TAU * 1_000.0 * n as f32 / rate as f32).sin()
            } else {
                0.0
            };
            w.write_sample(s).expect("write sample");
        }
    }
    w.finalize().expect("finalize wav");
}

#[test]
fn sample_spectrogram_flips_voiced_at_native_rate() {
    // A 32 kHz WAV alternating tone/silence every 50 ms. Analysed with a 50 ms
    // hop, consecutive frames should flip voiced↔unvoiced — the per-frame
    // evolution a single analyze_sample_spectrum aggregate would hide. The
    // native 32 kHz rate must be reported (not the engine's 44.1 kHz).
    let dir = std::env::temp_dir();
    let path = dir.join("pertylizer_sample_spectrogram_test.wav");
    write_alternating(&path, 32_000, 50.0, 8);
    let lib = empty_library();
    let p = path.to_string_lossy().into_owned();

    let r = analyze_sample_spectrogram_impl(
        &lib,
        p,
        Some(1_000.0),
        None,
        Some(0),
        Some(50.0),
        Some(40.0),
    )
    .expect("sample spectrogram analysis");

    assert_eq!(r.sample_rate, 32_000, "native sample rate must be reported");
    assert!(
        r.frames.len() >= 6,
        "expected several frames, got {}",
        r.frames.len()
    );
    let voiced = r.frames.iter().filter(|f| f.spectrum.voiced).count();
    let unvoiced = r.frames.len() - voiced;
    assert!(
        voiced > 0 && unvoiced > 0,
        "frames must flip voiced↔unvoiced: {voiced} voiced, {unvoiced} unvoiced"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn window_selects_voiced_vs_unvoiced_frame() {
    let dir = std::env::temp_dir();
    let path = dir.join("pertylizer_spectrum_window_test.wav");
    write_tone_then_silence(&path, HALF_MS, HALF_MS);
    let lib = empty_library();
    let p = path.to_string_lossy().into_owned();

    // First-half window: the 1 kHz tone → voiced, f0 near 1 kHz.
    let voiced = analyze_sample_spectrum_impl(
        &lib,
        p.clone(),
        Some(1_000.0),
        None,
        Some(64),
        Some(0.0),
        Some(100.0),
    )
    .expect("voiced window analysis");
    assert!(voiced.spectrum.voiced, "the tone window should be voiced");
    let f0 = voiced.spectrum.f0_hz.expect("voiced window has f0");
    assert!((f0 - 1_000.0).abs() < 20.0, "f0 should be ~1 kHz, got {f0}");

    // Second-half window: silence → unvoiced.
    let silent = analyze_sample_spectrum_impl(
        &lib,
        p.clone(),
        None,
        None,
        Some(64),
        Some(300.0),
        Some(100.0),
    )
    .expect("silent window analysis");
    assert!(
        !silent.spectrum.voiced,
        "the silent window must be unvoiced"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn window_past_end_is_zero_padded() {
    let dir = std::env::temp_dir();
    let path = dir.join("pertylizer_spectrum_window_pad_test.wav");
    write_tone_then_silence(&path, 60.0, 0.0); // 60 ms total
    let lib = empty_library();
    let p = path.to_string_lossy().into_owned();

    // Request a 200 ms window from 20 ms — runs ~160 ms past the 60 ms buffer.
    // It must zero-pad to a full 200 ms frame (no panic) and stay voiced.
    let r = analyze_sample_spectrum_impl(
        &lib,
        p,
        Some(1_000.0),
        None,
        Some(64),
        Some(20.0),
        Some(200.0),
    )
    .expect("padded window analysis");
    let expected = (0.200 * SR as f32).round() as u64;
    assert_eq!(r.frame_count, expected, "frame should be padded to 200 ms");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn absurd_window_len_is_bounded() {
    // A pathological window_len_ms must not attempt a multi-gigabyte allocation;
    // the frame is capped (here to 60 s) rather than honouring 1e9 ms.
    let dir = std::env::temp_dir();
    let path = dir.join("pertylizer_spectrum_window_huge_test.wav");
    write_tone_then_silence(&path, 50.0, 0.0);
    let lib = empty_library();
    let p = path.to_string_lossy().into_owned();

    let r = analyze_sample_spectrum_impl(
        &lib,
        p,
        Some(1_000.0),
        None,
        Some(64),
        Some(0.0),
        Some(1.0e9), // 1e9 ms ≈ 11.5 days — must be clamped
    )
    .expect("absurd window must still succeed, just bounded");
    let cap = (60.0 * SR as f32) as u64;
    assert_eq!(
        r.frame_count, cap,
        "frame should be clamped to the 60 s cap"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn window_start_past_end_errors() {
    let dir = std::env::temp_dir();
    let path = dir.join("pertylizer_spectrum_window_oob_test.wav");
    write_tone_then_silence(&path, 100.0, 0.0); // 100 ms total
    let lib = empty_library();
    let p = path.to_string_lossy().into_owned();

    let err = analyze_sample_spectrum_impl(
        &lib,
        p,
        None,
        None,
        Some(64),
        Some(500.0), // starts well past the 100 ms of audio
        Some(50.0),
    )
    .expect_err("start past end must be an error");
    assert!(
        matches!(err, McpBridgeError::WindowOutOfBounds { .. }),
        "expected WindowOutOfBounds, got {err:?}"
    );

    let _ = std::fs::remove_file(&path);
}
