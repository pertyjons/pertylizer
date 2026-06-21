//! Integration tests for the `analyze_spectrum` MCP tool
//! (`pertylizer::mcp_bridge::analyze_spectrum_impl`).
//!
//! Builds a two-instrument project — a sawtooth oscillator (harmonic, pitched)
//! and a noise source (broadband, unpitched) both sounding the same note — and
//! checks that:
//! - soloing one instrument isolates its spectrum from the full mix, and
//! - the detailed descriptors (voiced verdict, flatness, partials) clearly
//!   separate the two timbres — the regression that motivates the whole tool,
//!   since the 4-band `analyze_mix_bus` energy metric does not reliably tell a
//!   pitched source from a noisy one.

mod common;

use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::AudioProcessor;
use synth_core::audio::SampleRate as HwSampleRate;
use synth_engine::SynthEngine;
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SeqInstrumentId, Song, Tick, Velocity,
};

use pertylizer::audio::preview::SharedSampleLibrary;
use pertylizer::mcp_bridge::{
    analyze_sample_spectrum_impl, analyze_spectrum_impl, render_to_wav_impl,
};
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;
use synth_core::ModuleType;
use synth_mcp::AnalysisScope;

use common::TEST_SR;

fn saw_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
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
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

fn noise_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
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
    patch.add_connection("nse-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

struct Rig {
    _engine: SynthEngine,
    _handle: synth_engine::EngineHandle,
    session: SynthSession,
    sample_library: SharedSampleLibrary,
}

/// Two instruments — 0: sawtooth, 1: noise — each on its own track, both
/// sounding a sustained A3 (220 Hz) across the whole pattern.
fn setup() -> (Rig, Arc<RwLock<Song>>) {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    session
        .add_instrument_with_id(InstrumentId::new(0), "Saw")
        .expect("add saw");
    session
        .add_instrument_with_id(InstrumentId::new(1), "Noise")
        .expect("add noise");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = synth_core::AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(InstrumentId::new(0), &saw_patch("Saw"));
    let _ = session.apply_patch(InstrumentId::new(1), &noise_patch("Noise"));
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    // One sustained A3 (MIDI 57 = 220 Hz) shared by both tracks.
    let mut song = Song::new("Two");
    let pat = song.create_pattern(SeqDuration::WHOLE);
    {
        let p = song.pattern_mut(pat).expect("pattern exists");
        let nid = p.add_note(PatternTick(0), Pitch::new(57).expect("A3"), Velocity::MF);
        if let Some(n) = p.note_mut(nid) {
            n.duration = Some(SeqDuration::WHOLE);
        }
    }
    for (idx, label) in [(0u16, "Saw"), (1u16, "Noise")] {
        let tid = song.create_track(label);
        if let Some(t) = song.track_mut(tid) {
            t.instrument = SeqInstrumentId(idx);
        }
        assert!(song.place_pattern(pat, tid, Tick(0)), "place {label}");
    }

    let rig = Rig {
        _engine: engine,
        _handle: handle,
        session,
        sample_library: Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        )),
    };
    (rig, Arc::new(RwLock::new(song)))
}

fn analyze(
    rig: &Rig,
    shared: &McpSharedState,
    instrument_id: Option<u16>,
) -> synth_mcp::types::AnalyzeSpectrumResult {
    analyze_spectrum_impl(
        &rig.session,
        &rig.sample_library,
        shared,
        2.0,
        Some(0),
        instrument_id,
        None,
        None,
        Some(64),
        AnalysisScope::default(),
    )
    .expect("analyze_spectrum should succeed")
}

#[test]
fn analyze_spectrum_solo_isolates_instrument() {
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);

    let saw = analyze(&rig, &shared, Some(0));
    let noise = analyze(&rig, &shared, Some(1));
    let full = analyze(&rig, &shared, None);

    assert_eq!(saw.soloed_instrument_id, Some(0));
    // The soloed sawtooth is a clean pitched tone; the soloed noise is not. The
    // full mix contains both, so its flatness sits above the pure sawtooth's.
    assert!(saw.spectrum.voiced, "soloed sawtooth should be voiced");
    assert!(!noise.spectrum.voiced, "soloed noise should be unvoiced");
    assert!(
        full.spectrum.flatness > saw.spectrum.flatness,
        "full mix (saw+noise) should be less tonal than the soloed saw: full {} vs saw {}",
        full.spectrum.flatness,
        saw.spectrum.flatness
    );
}

#[test]
fn descriptors_separate_harmonic_from_noise() {
    // The motivating regression: a pitched source and a noisy source can have
    // similar coarse 4-band energy, yet the detailed descriptors must clearly
    // tell them apart. flatness + the voiced verdict + the partial list are the
    // distinguishing information the 4-band metric discards.
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);

    let saw = analyze(&rig, &shared, Some(0));
    let noise = analyze(&rig, &shared, Some(1));

    // Sawtooth: pitched at ~220 Hz, low flatness, a real harmonic series.
    assert!(
        saw.spectrum.voiced && saw.spectrum.f0_hz.is_some(),
        "saw voiced with an f0"
    );
    let f0 = saw.spectrum.f0_hz.unwrap();
    assert!((f0 - 220.0).abs() < 6.0, "saw f0 ≈ 220 Hz, got {f0}");
    assert!(
        saw.spectrum.flatness < 0.2,
        "saw is tonal, flatness {}",
        saw.spectrum.flatness
    );
    let tagged = saw
        .spectrum
        .partials
        .iter()
        .filter(|p| p.harmonic_number.is_some())
        .count();
    assert!(
        tagged >= 4,
        "saw should expose a harmonic series, tagged {tagged}"
    );

    // Noise: unvoiced, much higher flatness, no fundamental.
    assert!(
        !noise.spectrum.voiced && noise.spectrum.f0_hz.is_none(),
        "noise unvoiced, no f0"
    );
    assert!(
        noise.spectrum.flatness > 0.3,
        "noise is broadband, flatness {}",
        noise.spectrum.flatness
    );

    // The decisive separation the 4-band energy metric cannot make.
    assert!(
        noise.spectrum.flatness > saw.spectrum.flatness + 0.2,
        "flatness must clearly separate the two: noise {} vs saw {}",
        noise.spectrum.flatness,
        saw.spectrum.flatness
    );
}

#[test]
fn analyze_sample_spectrum_roundtrips() {
    // render_to_wav (soloed saw) → analyze that WAV → it must match
    // analyze_spectrum of the very same render, since the file is a lossless
    // 32-bit float copy of the rendered buffer.
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("saw.wav");
    render_to_wav_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        path.to_string_lossy().into_owned(),
        2.0,
        Some(0),
        Some(0), // solo the sawtooth
        AnalysisScope::default(),
    )
    .expect("render_to_wav should succeed");

    let from_file = analyze_sample_spectrum_impl(
        &rig.sample_library,
        path.to_string_lossy().into_owned(),
        None,
        None,
        Some(64),
    )
    .expect("analyze_sample_spectrum should succeed");
    let from_render = analyze(&rig, &shared, Some(0));

    assert_eq!(from_file.sample_rate, TEST_SR);
    assert_eq!(
        from_file.spectrum.voiced, from_render.spectrum.voiced,
        "voiced verdict must agree"
    );
    let f_file = from_file.spectrum.f0_hz.expect("file f0");
    let f_render = from_render.spectrum.f0_hz.expect("render f0");
    assert!(
        (f_file - f_render).abs() < 1.0,
        "f0 from file ({f_file}) and render ({f_render}) must match"
    );
    assert!(
        (from_file.spectrum.flatness - from_render.spectrum.flatness).abs() < 0.02,
        "flatness must match: file {} vs render {}",
        from_file.spectrum.flatness,
        from_render.spectrum.flatness
    );
    assert!(
        (from_file.spectrum.centroid_hz - from_render.spectrum.centroid_hz).abs() < 10.0,
        "centroid must match: file {} vs render {}",
        from_file.spectrum.centroid_hz,
        from_render.spectrum.centroid_hz
    );
}
