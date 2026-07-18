//! Integration tests for the spectrum MCP tools (`analyze_spectrum_impl`,
//! `analyze_sample_spectrum_impl`, `compare_spectra_impl`,
//! `analyze_spectrogram_impl`).
//!
//! Builds a three-instrument project — a sawtooth (harmonic, pitched), a noise
//! source (broadband, unpitched), and a sine (fundamental only) — all sounding
//! the same note, and checks that:
//! - soloing one instrument isolates its spectrum from the full mix,
//! - the descriptors (voiced verdict, flatness, partials) separate the timbres
//!   the 4-band `analyze_mix_bus` energy metric cannot, and
//! - `compare_spectra` reports the distance plus the missing/extra partials that
//!   drive a timbre-matching loop.

mod common;

use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::AudioProcessor;
use synth_core::audio::SampleRate as HwSampleRate;
use synth_engine::SynthEngine;
use synth_sequencer::{
    Duration as SeqDuration, InstrumentId, PatternTick, Pitch, Song, Tick, Velocity,
};

use pertylizer::audio::preview::SharedSampleLibrary;
use pertylizer::mcp_bridge::{
    analyze_sample_spectrum_impl, analyze_spectrogram_impl, analyze_spectrum_impl,
    compare_envelopes_impl, compare_spectra_impl, render_to_wav_impl,
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

fn sine_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sine")
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

/// Three instruments — 0: sawtooth, 1: noise, 2: sine — each on its own track,
/// all sounding a sustained A3 (220 Hz) across the whole pattern.
fn setup() -> (Rig, Arc<RwLock<Song>>) {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    session
        .add_instrument_with_id(InstrumentId::new(0), "Saw")
        .expect("add saw");
    session
        .add_instrument_with_id(InstrumentId::new(1), "Noise")
        .expect("add noise");
    session
        .add_instrument_with_id(InstrumentId::new(2), "Sine")
        .expect("add sine");

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
    let _ = session.apply_patch(InstrumentId::new(2), &sine_patch("Sine"));
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
    for (idx, label) in [(0u16, "Saw"), (1u16, "Noise"), (2u16, "Sine")] {
        let tid = song.create_track(label);
        if let Some(t) = song.track_mut(tid) {
            t.instrument = InstrumentId(u64::from(idx));
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
    instrument_id: Option<InstrumentId>,
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

    let saw = analyze(&rig, &shared, Some(InstrumentId::new(0)));
    let noise = analyze(&rig, &shared, Some(InstrumentId::new(1)));
    let full = analyze(&rig, &shared, None);

    assert_eq!(saw.soloed_instrument_id, Some(InstrumentId::new(0)));
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

    let saw = analyze(&rig, &shared, Some(InstrumentId::new(0)));
    let noise = analyze(&rig, &shared, Some(InstrumentId::new(1)));

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
        Some(InstrumentId::new(0)), // solo the sawtooth
        AnalysisScope::default(),
    )
    .expect("render_to_wav should succeed");

    let from_file = analyze_sample_spectrum_impl(
        &rig.sample_library,
        path.to_string_lossy().into_owned(),
        None,
        None,
        Some(64),
        None,
        None,
    )
    .expect("analyze_sample_spectrum should succeed");
    let from_render = analyze(&rig, &shared, Some(InstrumentId::new(0)));

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

/// Build a render `SpectrumSource` for the given soloed instrument.
fn render_source(instrument_id: InstrumentId) -> synth_mcp::SpectrumSource {
    synth_mcp::SpectrumSource {
        sample_id_or_path: None,
        instrument_id: Some(instrument_id),
        start_tick: Some(0),
        duration_seconds: Some(2.0),
        start_ms: None,
        window_len_ms: None,
    }
}

fn compare(
    rig: &Rig,
    shared: &McpSharedState,
    target: synth_mcp::SpectrumSource,
    candidate: synth_mcp::SpectrumSource,
) -> synth_mcp::types::CompareSpectraResult {
    compare_spectra_impl(
        &rig.session,
        &rig.sample_library,
        shared,
        target,
        candidate,
        None,
        None,
        None,
        None,
        AnalysisScope::default(),
        synth_mcp::TimeResolvedOptions::default(),
    )
    .expect("compare_spectra should succeed")
}

#[test]
fn compare_spectra_identical_render_is_near_zero() {
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);
    let d = compare(
        &rig,
        &shared,
        render_source(InstrumentId::new(0)),
        render_source(InstrumentId::new(0)),
    );
    assert!(!d.voicing_mismatch, "same source is not a voicing mismatch");
    assert!(
        d.log_spectral_distance < 1.0,
        "identical sources should be ~0 apart, got {}",
        d.log_spectral_distance
    );
    assert!(
        d.mel_l2_distance < 1.0,
        "identical sources should have ~0 mel-L2, got {}",
        d.mel_l2_distance
    );
    assert!(
        d.missing_partials.is_empty() && d.extra_partials.is_empty(),
        "identical sources have no missing/extra partials"
    );
    // Aggregate mode leaves the time-resolved fields unset.
    assert!(d.time_resolved_lsd.is_none());
    assert!(d.worst_frames.is_none());
}

#[test]
fn compare_spectra_time_resolved_identical_is_near_zero_and_populated() {
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);
    let d = compare_spectra_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        render_source(InstrumentId::new(0)),
        render_source(InstrumentId::new(0)),
        None,
        None,
        None,
        None,
        AnalysisScope::default(),
        synth_mcp::TimeResolvedOptions {
            enabled: true,
            mask_target_energy: true,
            align_envelope: true,
            ..Default::default()
        },
    )
    .expect("time-resolved compare should succeed");

    // The framed fields are populated…
    let tr_lsd = d
        .time_resolved_lsd
        .expect("time_resolved_lsd set when enabled");
    let frames = d.frames_compared.expect("frames_compared set when enabled");
    assert!(d.alignment_offset_ms.is_some(), "alignment offset reported");
    assert!(d.worst_frames.is_some(), "worst_frames reported");
    assert!(frames > 0, "some frames should be compared, got {frames}");
    // …and an identical source frames to ~0 per-frame distance.
    assert!(
        tr_lsd < 1.0,
        "identical sources should be ~0 apart per frame, got {tr_lsd}"
    );
    // A steady 2 s render is not time-sparse, so no honesty warning is added.
    assert!(
        d.warnings.iter().all(|w| !w.contains("time-sparse")),
        "steady render should not trigger the time-sparse warning: {:?}",
        d.warnings
    );
}

#[test]
fn compare_spectra_reports_missing_partial() {
    // Target = sawtooth (full harmonic series); candidate = sine (fundamental
    // only). The saw's upper harmonics must show up as missing in the candidate.
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);
    let d = compare(
        &rig,
        &shared,
        render_source(InstrumentId::new(0)),
        render_source(InstrumentId::new(2)),
    );
    assert!(!d.voicing_mismatch, "both saw and sine are voiced");
    assert!(
        !d.missing_partials.is_empty(),
        "the sawtooth's upper harmonics should be missing from the sine candidate"
    );
    // The missing partials should sit above the fundamental (~220 Hz).
    assert!(
        d.missing_partials.iter().any(|p| p.frequency_hz > 300.0),
        "a missing partial should be an upper harmonic (> 300 Hz)"
    );
    assert!(d.log_spectral_distance > 0.0);
    assert!(
        d.mel_l2_distance > 0.0,
        "differing timbres should carry a non-zero mel-L2, got {}",
        d.mel_l2_distance
    );
}

#[test]
fn compare_spectra_voicing_mismatch_is_penalised() {
    // Sawtooth (voiced) vs noise (unvoiced) — a gross timbral mismatch.
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);
    let d = compare(
        &rig,
        &shared,
        render_source(InstrumentId::new(0)),
        render_source(InstrumentId::new(1)),
    );
    assert!(d.voicing_mismatch, "voiced vs noise is a voicing mismatch");
    // The penalty is reported on its own field, not folded into the spectral
    // scalar. A folded value would be raw_lsd + 60 ≥ 60; observing < 60 proves
    // the +60 dB penalty is NOT in log_spectral_distance.
    assert_eq!(
        d.voicing_penalty_db, 60.0,
        "a voicing mismatch charges the penalty on voicing_penalty_db"
    );
    assert!(
        d.log_spectral_distance < 60.0,
        "the pure spectral scalar must not carry the +60 dB penalty, got {}",
        d.log_spectral_distance
    );
    assert!(
        d.missing_partials.is_empty() && d.extra_partials.is_empty(),
        "no partial matching across a voicing mismatch"
    );
}

fn compare_env(
    rig: &Rig,
    shared: &McpSharedState,
    target: synth_mcp::SpectrumSource,
    candidate: synth_mcp::SpectrumSource,
) -> synth_mcp::types::CompareEnvelopesResult {
    compare_envelopes_impl(
        &rig.session,
        &rig.sample_library,
        shared,
        target,
        candidate,
        None,
        Some(1500),
        None,
        AnalysisScope::default(),
    )
    .expect("compare_envelopes should succeed")
}

#[test]
fn compare_envelopes_identical_render_is_near_zero() {
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);
    let d = compare_env(
        &rig,
        &shared,
        render_source(InstrumentId::new(0)),
        render_source(InstrumentId::new(0)),
    );
    assert!(
        d.dtw_distance < 1.0e-3,
        "identical contours should warp to ~0, got {}",
        d.dtw_distance
    );
    assert_eq!(d.attack_delta_ms, 0.0);
    assert_eq!(d.crest_factor_delta_db, 0.0);
    assert!(
        d.target.num_windows > 0,
        "a 2 s render should yield envelope windows"
    );
}

#[test]
fn compare_envelopes_differing_shapes_have_positive_distance() {
    // Saw (instrument 0) vs noise (instrument 1) — different amplitude contours
    // over the note, so the warp distance must be clearly non-zero.
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);
    let d = compare_env(
        &rig,
        &shared,
        render_source(InstrumentId::new(0)),
        render_source(InstrumentId::new(1)),
    );
    assert!(
        d.dtw_distance > 0.0,
        "different contours should carry a non-zero DTW distance, got {}",
        d.dtw_distance
    );
    assert!(
        d.candidate.num_windows > 0 && d.target.num_windows > 0,
        "both sides should have envelope windows"
    );
}

#[test]
fn compare_spectra_render_vs_sample_matches() {
    // Render the saw to a WAV, then compare that sample against the same render.
    // Identical audio → near-zero distance through both source paths.
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
        Some(InstrumentId::new(0)),
        AnalysisScope::default(),
    )
    .expect("render_to_wav");

    let sample_source = synth_mcp::SpectrumSource {
        sample_id_or_path: Some(path.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let d = compare(
        &rig,
        &shared,
        sample_source,
        render_source(InstrumentId::new(0)),
    );
    assert!(!d.voicing_mismatch);
    assert!(
        d.log_spectral_distance < 1.0,
        "the WAV and its source render should match, got {}",
        d.log_spectral_distance
    );
}

#[test]
fn analyze_spectrogram_frames_track_the_soloed_saw() {
    // One render of the soloed sawtooth, sliced into ~20 ms frames. Every frame
    // is the steady saw, so all should be voiced at ~220 Hz with ascending time.
    let (rig, song) = setup();
    let shared = McpSharedState::with_song(song);

    let result = analyze_spectrogram_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        2.0,
        Some(0),
        Some(InstrumentId::new(0)), // solo the sawtooth
        None,
        None,
        Some(0),
        Some(20.0), // 20 ms hop
        Some(40.0), // 40 ms window
        AnalysisScope::default(),
    )
    .expect("analyze_spectrogram should succeed");

    assert_eq!(result.soloed_instrument_id, Some(InstrumentId::new(0)));
    assert_eq!(result.sample_rate, TEST_SR);
    // ~2 s at a 20 ms hop ≈ 100 frames.
    assert!(
        result.frames.len() > 50,
        "expected many frames over a 2 s render, got {}",
        result.frames.len()
    );

    // Timestamps strictly ascend and stay within the rendered window.
    for pair in result.frames.windows(2) {
        assert!(
            pair[1].time_seconds > pair[0].time_seconds,
            "frame timestamps must ascend"
        );
    }
    assert!(result.frames.last().unwrap().time_seconds < 2.1);

    // The steady sawtooth: a frame in the middle is voiced near 220 Hz.
    let mid = &result.frames[result.frames.len() / 2];
    assert!(mid.spectrum.voiced, "a mid sawtooth frame should be voiced");
    let f0 = mid.spectrum.f0_hz.expect("voiced frame has f0");
    assert!((f0 - 220.0).abs() < 6.0, "mid frame f0 ≈ 220 Hz, got {f0}");
    // Almost every frame of a continuous tone is voiced.
    let voiced = result.frames.iter().filter(|f| f.spectrum.voiced).count();
    assert!(
        voiced * 10 >= result.frames.len() * 8,
        "most frames of a steady tone should be voiced ({voiced}/{})",
        result.frames.len()
    );
}
