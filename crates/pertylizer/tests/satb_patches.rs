//! Integration check for the Vocal & Choir built-in patches, including the
//! four SATB section voices (VocalTract + the Length control).
//!
//! There is no global "every built-in patch builds" harness, so this pins the
//! voice presets specifically: each one is installed on a real
//! SynthEngine + SynthSession and a note is rendered offline through the same
//! `render_note_to_buffer` path the MCP `preview_note` tool uses. A patch with
//! a broken connection, a bad port/param name, or a silent graph would render
//! near-zero — so a non-trivial RMS proves the graph wires up and the
//! VocalTract (with its `length` param) actually sounds.

use std::sync::Arc;

use synth_core::audio::DeviceSampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, MidiNote, Velocity};
use synth_engine::SynthEngine;
use synth_engine::instrument::InstrumentId;

use pertylizer::audio::preview::{SharedSampleLibrary, render_note_to_buffer};
use pertylizer::patch::{ParamValue, Patch};
use pertylizer::patches::categorized_patches;
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

fn render_rms(patch: &Patch) -> f32 {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    session
        .add_instrument_with_id(InstrumentId::FIRST, "Test")
        .expect("add instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate::new(TEST_SR),
        buffer_size: synth_core::BufferSize::new(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate::new(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    session.apply_patch(InstrumentId::FIRST, patch);

    // Drain so every add_module / add_effect / connect command lands.
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let rendered = render_note_to_buffer(
        &session,
        &sample_library,
        InstrumentId::FIRST,
        MidiNote::new(60),
        Velocity::from_midi(100),
        500,
        500,
    )
    .expect("render note");

    let lefts: Vec<f32> = rendered
        .samples
        .as_chunks::<2>()
        .0
        .iter()
        .map(|f| f[0])
        .collect();
    if lefts.is_empty() {
        return 0.0;
    }
    let sq: f32 = lefts.iter().map(|s| s * s).sum();
    (sq / lefts.len() as f32).sqrt()
}

/// Every patch in the Vocal & Choir category renders audible sound — proves the
/// graph (voice module → amp → output, plus the effect chain) wires up.
#[test]
fn vocal_and_choir_patches_render_sound() {
    let voice_patches: Vec<Patch> = categorized_patches()
        .into_iter()
        .find(|(name, _)| name.contains("Vocal & Choir"))
        .map(|(_, patches)| patches)
        .expect("Vocal & Choir category exists");

    assert!(
        voice_patches.len() >= 7,
        "expected solo/choir/vocal-tract + 4 SATB voices, got {}",
        voice_patches.len()
    );

    for patch in &voice_patches {
        let name = patch.name.clone();
        let rms = render_rms(patch);
        assert!(
            rms.is_finite() && rms > 1e-4,
            "voice patch '{name}' rendered silent/invalid (rms={rms})"
        );
    }
}

/// The four SATB presets are registered and each carries a distinct VocalTract
/// `length` (the whole point — different tract lengths give the section voices).
#[test]
fn satb_presets_present_with_distinct_lengths() {
    let voice_patches: Vec<Patch> = categorized_patches()
        .into_iter()
        .find(|(name, _)| name.contains("Vocal & Choir"))
        .map(|(_, patches)| patches)
        .expect("Vocal & Choir category exists");

    let mut lengths = Vec::new();
    for section in ["SATB Soprano", "SATB Alto", "SATB Tenor", "SATB Bass"] {
        let patch = voice_patches
            .iter()
            .find(|p| p.name == section)
            .unwrap_or_else(|| panic!("{section} preset is registered"));
        let vtr = patch
            .modules
            .iter()
            .find(|m| m.id == "vtr-1")
            .unwrap_or_else(|| panic!("{section} has a VocalTract module"));
        let length = match vtr.parameters.get("length") {
            Some(ParamValue::Float(f)) => *f,
            _ => panic!("{section} VocalTract sets a float length param"),
        };
        lengths.push(length);
    }

    // Soprano (short) < Alto < Tenor < Bass (long).
    assert!(
        lengths.windows(2).all(|w| w[0] < w[1]),
        "SATB lengths should increase soprano→bass, got {lengths:?}"
    );
}
