//! Gap 2 regression: the spectral tools can select a single analysis window of
//! a sample, so a time-varying sound (a voiced frame vs an unvoiced frame) can
//! be compared frame-to-frame instead of averaging to "unvoiced".
//!
//! Builds a WAV that is a steady 1 kHz tone for the first half and silence for
//! the second half, then checks that `analyze_sample_spectrum_impl`:
//! - reports the tone window voiced and the silent window unvoiced,
//! - zero-pads a window that runs off the end (no panic),
//! - returns `WindowOutOfBounds` for a start past the end of the audio.

use std::sync::{Arc, RwLock};

use pertylizer::audio::preview::SharedSampleLibrary;
use pertylizer::mcp_bridge::analyze_sample_spectrum_impl;
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
