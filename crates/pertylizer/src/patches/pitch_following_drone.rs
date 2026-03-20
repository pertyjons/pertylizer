//! Pitch Following Drone — self-modulating drone where pitch tracker creates evolving feedback.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Pitch Following Drone — PitchTracker, GranularOsc, ModalResonator, EnsembleChorus.
pub fn patch_pitch_following_drone() -> Patch {
    let mut patch = Patch::new("Pitch Following Drone");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Self-modulating drone combining standard and granular oscillators, \
         with pitch tracking creating evolving cross-modulation."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A standard sawtooth oscillator and a Granular Oscillator are mixed
together. The sawtooth provides a stable drone foundation while the
granular source adds textural complexity with sine grains.

A Pitch Tracker analyzes the mixed output and generates a pitch CV
that feeds back to modulate the granular oscillator's frequency.
This creates a self-referencing feedback loop where the detected
pitch influences the granular texture, causing gentle evolution.

EFFECTS CHAIN (auto-routed):
1. Modal Resonator — adds resonant body character to the drone,
   with 10 resonant modes tuned to create a rich harmonic spectrum
2. Ensemble Chorus — Juno-style 3-voice chorus for lush width

PITCH TRACKER:
- Sensitivity = 0.6 (moderate confidence threshold)
- Min Freq = 40 Hz (captures low drones)
- Max Freq = 3000 Hz
- Smoothing = 0.5 (smooth tracking for gentle modulation)

GRANULAR OSCILLATOR:
- Source = Sine (pure tones for clean granular texture)
- Grain Size = 100ms (longer grains for smoother texture)
- Density = 0.4 (moderate density, not overwhelming)
- Pan Spread = 0.4 (gentle stereo movement)

TRY: Increase PitchTracker smoothing for more stable modulation.
Change GranularOsc source to Noise for atmospheric texture.
Lower ModalResonator decay for percussive resonance pings.
"#
        .to_string(),
    );
    patch.tags = vec![
        "experimental".into(),
        "drone".into(),
        "pitch_tracker".into(),
        "granular".into(),
        "evolving".into(),
    ];

    // Oscillator - stable drone foundation (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.6)
            .build(),
    );

    // Granular Oscillator - textural layer (grn-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::GranularOsc)
            .position(50.0, 400.0)
            .param_f("grain size", 100.0)
            .param_f("density", 0.4)
            .param_f("position", 0.3)
            .param_f("pos spread", 0.2)
            .param_f("pitch spread", 0.1)
            .param_f("pan spread", 0.4)
            .param_b("freeze", false)
            .param_choice("window", "gaussian")
            .param_choice("source", "sine")
            .param_f("level", 0.5)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .build(),
    );

    // Pitch Tracker - analyzes mixed output (ptr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::PitchTracker)
            .position(450.0, 400.0)
            .param_f("sensitivity", 0.6)
            .param_f("min freq", 40.0)
            .param_f("max freq", 3000.0)
            .param_f("smoothing", 0.5)
            .build(),
    );

    // Filter - gentle warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2200.0)
            .param_f("resonance", 0.25)
            .param_f("key_track", 0.4)
            .build(),
    );

    // Slow drone envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 400.0)
            .param_f("attack", 2.0)
            .param_f("decay", 1.5)
            .param_f("sustain", 0.85)
            .param_f("release", 4.0)
            .param_f("atk_curve", 0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1650.0, 50.0)
            .param_f("master level", 0.7)
            .build(),
    );

    // === Effects (auto-routed) ===

    // Modal Resonator — resonant body character (mdr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModalResonator)
            .position(2050.0, 50.0)
            .param_f("base note", 48.0)
            .param_f("spread", 0.25)
            .param_f("modes", 10.0)
            .param_f("decay", 0.7)
            .param_f("brightness", 0.45)
            .param_f("mix", 0.5)
            .build(),
    );

    // Ensemble Chorus — Juno-style width (enc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::EnsembleChorus)
            .position(2050.0, 400.0)
            .param_f("rate", 0.4)
            .param_f("depth", 1.0)
            .param_f("base delay", 10.0)
            .param_f("tone", 0.5)
            .param_f("noise", 0.05)
            .param_f("stereo width", 0.9)
            .param_f("voices", 3.0)
            .param_f("mix", 0.4)
            .build(),
    );

    // === Connections ===
    // Both oscillators → Mixer
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("grn-1", "out", "mix-1", "in2");

    // Pitch Tracker analyzes the mixer output
    patch.add_connection("mix-1", "out", "ptr-1", "in");

    // Pitch Tracker pitch CV → Granular FM (self-modulation feedback)
    patch.add_connection("ptr-1", "pitch_cv", "grn-1", "freq_cv");

    // Mixer → Filter → Amp → Output
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group(
        "Drone Sources",
        Some("#6B8E8E"),
        &["osc-1", "grn-1", "mix-1"],
    );
    patch.add_group("Pitch Feedback", Some("#D96A4A"), &["ptr-1"]);
    patch.add_group(
        "Output",
        Some("#4A9D8F"),
        &["flt-1", "amp-1", "env-1", "out-1"],
    );
    patch.add_group("Effects", Some("#8B6BAE"), &["mdr-1", "enc-1"]);

    patch
}
