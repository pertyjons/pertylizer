//! Integration tests for the offline render pipeline (`render_note_to_buffer`).
//!
//! These tests cover the regressions identified in the analyze_note code
//! review:
//! - finding #1: effect chain is mirrored into the offline render
//! - finding #3: NoteOff lands at the requested sample boundary
//! - finding #4: bypass state on voice modules is replicated in the offline
//!   render (covered by `voice_module_bypass_replicated_in_offline_render`)
//! - finding #10: warnings list is exposed on the result
//!
//! In addition the file pins the dynamic effect-chain regressions:
//! - effect bypass after patch load is reflected in subsequent renders
//! - effect parameter changes after patch load are reflected
//! - effect removal after patch load drops the effect from the render
//! - duplicate effects of the same type are addressable by `ModuleId`
//!
//! The tests construct a SynthEngine + SynthSession, populate it with a
//! known patch, drive a few process() ticks to drain the command queue
//! into shared state, and then call `render_note_to_buffer` (which spins
//! up its own offline engine). All assertions are on the returned PCM
//! buffer — no audio device, no GUI.

use std::sync::Arc;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{
    AudioCallbackContext, AudioProcessor, MidiNote, ModuleType, NormalizedValue, Param,
    ReverbParam, Velocity,
};
use synth_engine::SynthEngine;
use synth_engine::commands::ModuleId;
use synth_engine::instrument::InstrumentId;

use pertylizer::audio::preview::{RenderedNote, SharedSampleLibrary, render_note_to_buffer};
use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;

/// Build a minimal sustaining patch: Oscillator (sawtooth) → Amplifier → StereoOutput
/// with a sharp gate envelope (A=1ms, D=0, S=1.0, R=10ms). The fast attack
/// and short release give a deterministic boundary around note-off — RMS is
/// near full while held and drops to near-zero ~10 ms after NoteOff.
fn sustain_patch_no_envelope() -> Patch {
    use synth_core::ModuleType;
    let mut patch = Patch::new("Test Sustain");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.001)
            .param_f("decay", 0.0)
            .param_f("sustain", 1.0)
            .param_f("release", 0.01)
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

/// Build a similar patch but with a Reverb in the effect chain to exercise
/// finding #1 (effect chain present in offline render).
fn sustain_patch_with_reverb() -> Patch {
    use synth_core::ModuleType;
    let mut patch = sustain_patch_no_envelope();
    // Effects are routed through the effect_chain, not by add_connection.
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .param_f("mix", 1.0)
            .param_f("decay", 8.0)
            .build(),
    );
    patch
}

/// Spin up a SynthEngine, attach a SynthSession, install `patch` on a fresh
/// instrument, and drain the command queue so the shared graph snapshot
/// reflects the live patch. Returns the session along with the engine and
/// handle (kept alive for the duration of the test so commands can drain).
struct TestRig {
    engine: SynthEngine,
    handle: synth_engine::EngineHandle,
    session: SynthSession,
    sample_library: SharedSampleLibrary,
    instrument_id: InstrumentId,
}

const TEST_SR: u32 = 44_100;

impl TestRig {
    /// Drive the engine for a number of process() blocks so any pending
    /// commands land in shared state (and any audio is rendered/discarded).
    fn drain(&mut self, blocks: usize) {
        let mut block = vec![0.0f32; 256 * 2];
        let context = AudioCallbackContext {
            sample_rate: HwSampleRate(TEST_SR),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };
        for _ in 0..blocks {
            block.fill(0.0);
            self.engine.process(&mut block, &context);
        }
    }
}

fn setup_with_patch(patch: &Patch) -> TestRig {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    session
        .add_instrument_with_id(InstrumentId::FIRST, "Test")
        .expect("add instrument");

    // Drive on_stream_start + a couple of process() ticks so commands drain
    // and the shared_graph snapshot is populated with the instrument.
    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    // Apply the patch to the instrument.
    let _result = session.apply_patch(InstrumentId::FIRST, patch);

    // Drain a few more times so all add_module / add_effect / connect
    // commands land in the shared graph.
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    TestRig {
        engine,
        handle,
        session,
        sample_library: Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        )),
        instrument_id: InstrumentId::FIRST,
    }
}

fn extract_left(samples: &[f32]) -> Vec<f32> {
    samples.chunks_exact(2).map(|f| f[0]).collect()
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sq: f32 = samples.iter().map(|s| s * s).sum();
    (sq / samples.len() as f32).sqrt()
}

// -------------------------------------------------------------------------
// finding #3: NoteOff is sent at the exact sample boundary, not "next block".
// -------------------------------------------------------------------------
#[test]
fn note_off_frame_matches_requested_duration() {
    let rig = setup_with_patch(&sustain_patch_no_envelope());
    let rendered = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        500,
        500,
    )
    .expect("render");

    let expected = (0.5 * 44_100.0) as u64;
    assert_eq!(
        rendered.note_off_frame, expected,
        "note_off_frame should be exactly note_frames; got {} expected {}",
        rendered.note_off_frame, expected
    );
}

#[test]
fn note_off_lands_inside_a_short_note_without_overflow() {
    // 17 ms note: doesn't divide evenly into 256-frame blocks (256/44100 ≈
    // 5.8 ms/block), so the note-off boundary lands mid-block. The fix
    // splits the block at the boundary so frames_written never overshoots
    // note_frames before NoteOff is queued.
    let rig = setup_with_patch(&sustain_patch_no_envelope());
    let rendered = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        17,
        500,
    )
    .expect("render");
    let expected = (0.017 * 44_100.0) as u64;
    assert_eq!(rendered.note_off_frame, expected);
}

// -------------------------------------------------------------------------
// finding #1: the effect chain is mirrored into the offline render. A patch
// with reverb produces a non-trivial tail after note-off; the same patch
// without reverb has a sharply quieter tail.
// -------------------------------------------------------------------------
#[test]
fn effect_chain_replicated_in_offline_render() {
    // Render same osc/amp/envelope twice, once with and once without a
    // reverb in the effect chain. Compare the tail RMS 200 ms after
    // note-off: with the reverb mirrored into the offline render, the
    // tail decays slowly; without it the amp envelope's 10 ms release
    // is essentially complete by 200 ms in.
    let rig_dry = setup_with_patch(&sustain_patch_no_envelope());
    let rig_wet = setup_with_patch(&sustain_patch_with_reverb());

    let dry = render_note_to_buffer(
        &rig_dry.session,
        &rig_dry.sample_library,
        rig_dry.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("dry render");
    let wet = render_note_to_buffer(
        &rig_wet.session,
        &rig_wet.sample_library,
        rig_wet.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("wet render");

    let dry_left = extract_left(&dry.samples);
    let wet_left = extract_left(&wet.samples);

    // 200..400 ms after NoteOff. The amp envelope's 10 ms release is long
    // gone by 200 ms, so the only thing that can keep the wet patch loud
    // there is the reverb. If finding #1 weren't fixed, both renders
    // would be silent in this window.
    let sr = dry.sample_rate as usize;
    let dry_off = dry.note_off_frame as usize;
    let wet_off = wet.note_off_frame as usize;
    let dry_tail = rms(&dry_left[dry_off + sr / 5..(dry_off + 2 * sr / 5).min(dry_left.len())]);
    let wet_tail = rms(&wet_left[wet_off + sr / 5..(wet_off + 2 * sr / 5).min(wet_left.len())]);

    assert!(
        wet_tail > 0.001,
        "reverb-tail patch should produce audible tail 200 ms after note-off: \
         wet_tail = {wet_tail} (effect chain not in offline render?)"
    );
    assert!(
        wet_tail > 5.0 * dry_tail,
        "reverb tail should be much louder than the dry tail: \
         wet_tail = {wet_tail}, dry_tail = {dry_tail}"
    );
}

// -------------------------------------------------------------------------
// finding #10: warnings field is reachable and empty on a clean render.
// -------------------------------------------------------------------------
#[test]
fn warnings_empty_on_clean_render() {
    let rig = setup_with_patch(&sustain_patch_no_envelope());
    let rendered = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        100,
    )
    .expect("render");
    assert!(
        rendered.warnings.is_empty(),
        "clean render should produce no warnings, got: {:?}",
        rendered.warnings
    );
}

#[test]
fn missing_instrument_returns_error() {
    // Empty session: instrument doesn't exist.
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(44_100),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let result = render_note_to_buffer(
        &session,
        &sample_library,
        InstrumentId::new(99),
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        100,
    );
    assert!(
        result.is_err(),
        "render should fail when instrument missing"
    );
}

// -------------------------------------------------------------------------
// finding #3 (continued): produced buffer has exactly the expected sample
// count regardless of the requested durations.
// -------------------------------------------------------------------------
#[test]
fn buffer_length_matches_requested_duration() {
    let rig = setup_with_patch(&sustain_patch_no_envelope());
    for &(dur, tail) in &[(100u32, 100u32), (250, 250), (1, 1000), (1500, 0)] {
        let rendered: RenderedNote = render_note_to_buffer(
            &rig.session,
            &rig.sample_library,
            rig.instrument_id,
            MidiNote::new(60),
            Velocity::from_midi(100),
            dur,
            tail,
        )
        .expect("render");
        let expected_frames = ((dur as u64 + tail as u64) * 44_100) / 1000;
        let actual_frames = (rendered.samples.len() / usize::from(rendered.channels)) as u64;
        assert_eq!(
            actual_frames, expected_frames,
            "duration {dur}+{tail}: got {actual_frames} frames, expected {expected_frames}"
        );
    }
}

// -------------------------------------------------------------------------
// AnalyzeNoteResult schema sanity: new optional fields round-trip through
// serde without losing values, and absent fields don't appear in the JSON.
// -------------------------------------------------------------------------
#[test]
fn analyze_note_result_schema_is_extensible() {
    use synth_mcp::types::{
        AnalysisSignalMode, AnalyzeEnergyBands, AnalyzeEnvelopeEstimate, AnalyzeFlags,
        AnalyzeHarmonicContent, AnalyzeNoteResult,
    };
    let r = AnalyzeNoteResult {
        note_requested: 60,
        note_played: 60,
        velocity: 100,
        sample_rate: 44_100,
        duration_seconds: 1.0,
        fundamental_hz: 440.0,
        analysis_signal_mode: AnalysisSignalMode::MaxAbsStereo,
        fundamental_left: Some(440.0),
        fundamental_right: Some(441.0),
        fundamental_left_confidence: Some(0.95),
        fundamental_right_confidence: Some(0.95),
        expected_fundamental_hz: 440.0,
        pitch_error_cents: 0.0,
        peak_amplitude: 0.5,
        rms_overall: 0.3,
        dc_offset: 0.0,
        clipped_samples: 0,
        envelope_window_ms: 50.0,
        rms_envelope: vec![0.3; 10],
        centroid_envelope: vec![1000.0; 10],
        spectrum_attack: vec![],
        spectrum_sustain: vec![],
        spectrum_release: vec![],
        pitch_envelope: vec![],
        pitch_envelope_window_ms: 200.0,
        stereo_correlation: 0.5,
        energy_bands: AnalyzeEnergyBands {
            sub: 0.0,
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        },
        harmonic_content: AnalyzeHarmonicContent {
            thd_db: -60.0,
            odd_even_ratio_db: 0.0,
            n_harmonics: 1,
        },
        envelope_estimate: AnalyzeEnvelopeEstimate {
            attack_ms: 10.0,
            decay_ms: 0.0,
            sustain_level: 0.5,
            release_ms: 100.0,
        },
        centroid_trend_hz_per_sec: 0.0,
        flags: AnalyzeFlags {
            silent: false,
            clipping: false,
            has_dc_offset: false,
            low_output: false,
            off_pitch: false,
        },
        peak_left: Some(0.4),
        peak_right: Some(0.6),
        rms_left: Some(0.25),
        rms_right: Some(0.35),
        dc_left: Some(0.0),
        dc_right: Some(0.0),
        clipped_left: Some(0),
        clipped_right: Some(0),
        mid_rms: Some(0.3),
        side_rms: Some(0.05),
        stereo_width: Some(0.17),
        pitch_confidence: Some(0.95),
        trimmed_tail_windows: None,
        attack_window_start_ms: Some(50.0),
        sustain_window_start_ms: Some(500.0),
        release_window_start_ms: Some(1025.0),
        warnings: vec!["test warning".to_string()],
    };

    let json = serde_json::to_string(&r).expect("serialize");
    // Set fields show up.
    assert!(
        json.contains("\"peak_left\":0.4"),
        "peak_left missing: {json}"
    );
    assert!(json.contains("\"pitch_confidence\":0.95"));
    assert!(json.contains("\"warnings\":[\"test warning\"]"));
    // None fields are omitted.
    assert!(
        !json.contains("\"trimmed_tail_windows\""),
        "None fields should not serialize"
    );
}

// =========================================================================
// Dynamic effect-chain regressions: after a patch is loaded, mutating the
// effect chain (bypass, param, remove) and bypassing voice-graph modules
// must show up in the next offline render. The shared_graph snapshot is the
// only thing the offline render path reads, so these tests ultimately pin
// that the engine refreshes shared_graph after every effect-chain command.
// =========================================================================

/// Render a 100ms note + 1000ms tail and return the left-channel samples.
fn render_left_for(rig: &TestRig) -> Vec<f32> {
    let rendered = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("render");
    extract_left(&rendered.samples)
}

/// RMS of the "tail" window: 200..400 ms after note-off.
fn tail_rms(samples: &[f32], note_off_frame: usize, sr: usize) -> f32 {
    let lo = note_off_frame + sr / 5;
    let hi = (note_off_frame + 2 * sr / 5).min(samples.len());
    if lo >= hi {
        return 0.0;
    }
    rms(&samples[lo..hi])
}

#[test]
fn effect_bypass_after_patch_load_removes_tail() {
    // Baseline: render the reverb patch, observe a wet tail.
    let mut rig = setup_with_patch(&sustain_patch_with_reverb());
    let baseline = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("baseline render");
    let baseline_left = extract_left(&baseline.samples);
    let baseline_tail = tail_rms(
        &baseline_left,
        baseline.note_off_frame as usize,
        baseline.sample_rate as usize,
    );
    assert!(
        baseline_tail > 0.001,
        "baseline reverb tail should be audible: {baseline_tail}"
    );

    // Disable the reverb effect on the live instrument.
    let reverb_id = ModuleId::new(ModuleType::Reverb, 1);
    let sent = rig
        .handle
        .send_blocking(synth_engine::EngineCommand::SetEffectEnabled {
            instrument_id: Some(rig.instrument_id),
            module_id: reverb_id,
            enabled: false,
        });
    assert!(sent, "SetEffectEnabled command did not enqueue");
    rig.drain(8);

    // The next offline render should reflect the bypassed reverb: the tail
    // window should drop to roughly the dry-patch level.
    let after = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("post-bypass render");
    let after_left = extract_left(&after.samples);
    let after_tail = tail_rms(
        &after_left,
        after.note_off_frame as usize,
        after.sample_rate as usize,
    );
    assert!(
        after_tail * 5.0 < baseline_tail,
        "bypassed reverb should drop tail RMS substantially: \
         baseline = {baseline_tail}, after = {after_tail}"
    );
}

#[test]
fn effect_param_change_after_patch_load_reflected_in_render() {
    // Baseline render with the default reverb mix=1.0 / decay=8.0.
    let mut rig = setup_with_patch(&sustain_patch_with_reverb());
    let baseline = render_left_for(&rig);

    // Change the reverb mix to 0 (effectively dry).
    let reverb_id = ModuleId::new(ModuleType::Reverb, 1);
    let sent = rig
        .handle
        .send_blocking(synth_engine::EngineCommand::SetEffectParameter {
            instrument_id: Some(rig.instrument_id),
            module_id: reverb_id,
            param: Param::Reverb(ReverbParam::Mix(NormalizedValue::new(0.0))),
        });
    assert!(sent, "SetEffectParameter command did not enqueue");
    rig.drain(8);

    let after = render_left_for(&rig);

    // The two renders should not be byte-equal — at least one sample must
    // differ by more than a tiny epsilon. Comparing tail windows specifically
    // is the strongest signal because the tail is dominated by the reverb.
    let n = baseline.len().min(after.len());
    let mut max_diff = 0.0_f32;
    for i in 0..n {
        let d = (baseline[i] - after[i]).abs();
        if d > max_diff {
            max_diff = d;
        }
    }
    assert!(
        max_diff > 1e-4,
        "param change should produce an audibly different render \
         (max sample diff = {max_diff})"
    );
}

#[test]
fn effect_remove_after_patch_load_drops_effect() {
    // Baseline: reverb tail audible.
    let mut rig = setup_with_patch(&sustain_patch_with_reverb());
    let baseline = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("baseline");
    let baseline_left = extract_left(&baseline.samples);
    let baseline_tail = tail_rms(
        &baseline_left,
        baseline.note_off_frame as usize,
        baseline.sample_rate as usize,
    );
    assert!(
        baseline_tail > 0.001,
        "baseline reverb tail should be audible: {baseline_tail}"
    );

    // Remove the reverb instance from the chain.
    let reverb_id = ModuleId::new(ModuleType::Reverb, 1);
    let sent = rig
        .handle
        .send_blocking(synth_engine::EngineCommand::RemoveEffect {
            instrument_id: Some(rig.instrument_id),
            id: reverb_id,
        });
    assert!(sent, "RemoveEffect command did not enqueue");
    rig.drain(8);

    // Post-remove render must not include the reverb in shared_graph (so the
    // offline render won't replicate it). The tail should drop substantially.
    let after = render_note_to_buffer(
        &rig.session,
        &rig.sample_library,
        rig.instrument_id,
        MidiNote::new(60),
        Velocity::from_midi(100),
        100,
        1000,
    )
    .expect("post-remove render");
    let after_left = extract_left(&after.samples);
    let after_tail = tail_rms(
        &after_left,
        after.note_off_frame as usize,
        after.sample_rate as usize,
    );
    assert!(
        after_tail * 5.0 < baseline_tail,
        "removed reverb should drop tail RMS substantially: \
         baseline = {baseline_tail}, after = {after_tail}"
    );

    // shared_graph should also no longer contain a snapshot for the reverb.
    let modules = rig
        .session
        .state()
        .shared_graph
        .get_modules_for_instrument(rig.instrument_id);
    assert!(
        !modules.iter().any(|m| m.id == reverb_id),
        "shared_graph still has snapshot for removed reverb: {:?}",
        modules.iter().map(|m| m.id).collect::<Vec<_>>()
    );
}

#[test]
fn duplicate_effect_type_targets_correct_instance_by_id() {
    // Build a patch with TWO reverbs of the same type but distinct IDs.
    // Both start with the same default params so we can use a parameter
    // change as a probe: changing rev-2 must NOT alter rev-1.
    use synth_core::ModuleType;
    let mut patch = sustain_patch_no_envelope();
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .param_f("mix", 0.5)
            .param_f("decay", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Reverb)
            .param_f("mix", 0.5)
            .param_f("decay", 0.5)
            .build(),
    );

    let mut rig = setup_with_patch(&patch);
    let rev1 = ModuleId::new(ModuleType::Reverb, 1);
    let rev2 = ModuleId::new(ModuleType::Reverb, 2);

    // Sanity: both reverbs are present in shared_graph.
    let snap_before = rig
        .session
        .state()
        .shared_graph
        .get_modules_for_instrument(rig.instrument_id);
    assert!(
        snap_before.iter().any(|m| m.id == rev1),
        "rev-1 missing from shared_graph"
    );
    assert!(
        snap_before.iter().any(|m| m.id == rev2),
        "rev-2 missing from shared_graph"
    );

    // Change rev-2's Mix to a value distinct from rev-1.
    let new_mix = 0.123_f32;
    let sent = rig
        .handle
        .send_blocking(synth_engine::EngineCommand::SetEffectParameter {
            instrument_id: Some(rig.instrument_id),
            module_id: rev2,
            param: Param::Reverb(ReverbParam::Mix(NormalizedValue::new(new_mix))),
        });
    assert!(sent, "SetEffectParameter on rev-2 did not enqueue");
    rig.drain(8);

    // Inspect shared_graph: rev-2's Mix should equal new_mix; rev-1's must
    // still be the original 0.5 — confirming the engine routed the param
    // change by ModuleId, not by ModuleType (which would have hit rev-1 first).
    let mix_for = |id: ModuleId| -> f32 {
        let snap = rig
            .session
            .state()
            .shared_graph
            .get_module(rig.instrument_id, id)
            .unwrap_or_else(|| panic!("module {id} missing from shared_graph"));
        snap.parameters
            .iter()
            .find_map(|p| match p {
                Param::Reverb(ReverbParam::Mix(v)) => Some(v.as_f32()),
                _ => None,
            })
            .expect("Mix param missing")
    };
    let mix1 = mix_for(rev1);
    let mix2 = mix_for(rev2);
    assert!(
        (mix2 - new_mix).abs() < 1e-3,
        "rev-2 Mix should be {new_mix}, got {mix2}"
    );
    assert!(
        (mix1 - 0.5).abs() < 1e-3,
        "rev-1 Mix should be unchanged at 0.5, got {mix1}"
    );
}

#[test]
fn voice_module_bypass_replicated_in_offline_render() {
    // Build a patch with a heavily resonant low-pass filter between osc and
    // amp. With the filter active, only low frequencies pass; bypassing it
    // lets the full sawtooth through, which has very different spectral
    // content (and very different RMS overall).
    use synth_core::ModuleType;
    let mut patch = Patch::new("Filter Bypass Test");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .filter_mode("lowpass")
            .param_f("cutoff", 200.0)
            .param_f("resonance", 0.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.001)
            .param_f("decay", 0.0)
            .param_f("sustain", 1.0)
            .param_f("release", 0.01)
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
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    let mut rig = setup_with_patch(&patch);

    // Baseline render — filter active, only low harmonics survive.
    let baseline = render_left_for(&rig);
    let sustain_window = |samples: &[f32]| -> f32 {
        // Use the held portion of the note (10..90 ms) so we measure the
        // steady-state rather than the release tail.
        let sr = 44_100;
        let lo = sr / 100; // 10 ms
        let hi = (9 * sr / 100).min(samples.len()); // 90 ms
        if lo >= hi {
            return 0.0;
        }
        rms(&samples[lo..hi])
    };
    let baseline_rms = sustain_window(&baseline);

    // Bypass the filter on the live instrument.
    let filter_id = ModuleId::new(ModuleType::Filter, 1);
    let sent = rig
        .handle
        .send_blocking(synth_engine::EngineCommand::SetBypass {
            instrument_id: Some(rig.instrument_id),
            module: filter_id,
            bypass: true,
        });
    assert!(sent, "SetBypass command did not enqueue");
    rig.drain(8);

    // shared_graph should reflect the bypass for the filter module.
    let snap = rig
        .session
        .state()
        .shared_graph
        .get_module(rig.instrument_id, filter_id)
        .expect("filter snapshot missing");
    assert!(
        matches!(snap.bypass_state, synth_core::BypassState::Bypassed),
        "filter bypass not mirrored to shared_graph: {:?}",
        snap.bypass_state
    );

    let after = render_left_for(&rig);
    let after_rms = sustain_window(&after);

    // Bypassing the lowpass filter must change the rendered audio — typically
    // the bypassed signal is louder because nothing is attenuating the highs.
    let n = baseline.len().min(after.len());
    let mut max_diff = 0.0_f32;
    for i in 0..n {
        let d = (baseline[i] - after[i]).abs();
        if d > max_diff {
            max_diff = d;
        }
    }
    assert!(
        max_diff > 1e-3,
        "bypassing filter should change the offline render (max diff = {max_diff}, \
         baseline_rms = {baseline_rms}, after_rms = {after_rms})"
    );
}
