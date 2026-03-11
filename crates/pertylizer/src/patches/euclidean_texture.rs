//! Euclidean Texture — generative rhythmic texture with euclidean patterns and turing machine.

use crate::patch::{ModuleBuilder, Patch, PatchAuthor};
use synth_core::ModuleType;

/// Euclidean Texture — Euclidean, TuringMachine, and RandomGates create generative modulation.
pub fn patch_euclidean_texture() -> Patch {
    let mut patch = Patch::new("Euclidean Texture");
    patch.author = Some(PatchAuthor::from("Pertylizer"));
    patch.description = Some(
        "Generative rhythmic texture using Euclidean sequencer patterns, \
         Turing Machine melodic evolution, and Random Gates for stochastic modulation."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A sawtooth oscillator provides the sound source, filtered through a
resonant lowpass. An LFO at ~6 Hz serves as the master clock, driving
three generative modules simultaneously:

GENERATIVE MODULES:
1. Euclidean Sequencer — generates rhythmic gate patterns using the
   Euclidean algorithm (5 pulses in 16 steps with rotation). The
   accent output modulates filter resonance for rhythmic emphasis.

2. Turing Machine — shift-register-based melody generator. At 50%
   mutation, patterns slowly evolve over time. Pitch output modulates
   the oscillator via FM for melodic movement.

3. Random Gates — stochastic gate generator with 40% density.
   The random CV output modulates filter cutoff for unpredictable
   filter sweeps.

EFFECTS CHAIN (auto-routed):
1. Phase Vocoder — subtle pitch shifting (+5 semitones) adds harmonic
   interest to the evolving texture
2. Reverse/Gate Reverb — reverse mode creates backward swells that
   complement the rhythmic patterns

ENVELOPE:
- Slow pad-style to sustain the drone texture while generative
  modules create movement on top

TRY: Change Euclidean Steps/Pulses for different rhythmic feels.
Adjust TuringMachine Mutation: 0=locked loop, 1=fully random.
Increase LFO rate for faster generative patterns.
"#
        .to_string(),
    );
    patch.tags = vec![
        "experimental".into(),
        "generative".into(),
        "euclidean".into(),
        "texture".into(),
        "ambient".into(),
    ];

    // Oscillator - sound source (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(450.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.7)
            .build(),
    );

    // LFO - master clock for generative modules (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 50.0)
            .waveform("square")
            .param_f("rate", 6.0)
            .param_f("depth", 1.0)
            .build(),
    );

    // Euclidean Sequencer (euc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Euclidean)
            .position(50.0, 400.0)
            .param_f("steps", 16.0)
            .param_f("pulses", 5.0)
            .param_f("rotation", 2.0)
            .param_f("swing", 0.1)
            .build(),
    );

    // Turing Machine (tur-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::TuringMachine)
            .position(250.0, 400.0)
            .param_f("mutation", 0.5)
            .param_f("range", 0.4)
            .param_choice("scale", "pentatonic")
            .param_f("length", 16.0)
            .build(),
    );

    // Random Gates (rgt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::RandomGates)
            .position(450.0, 400.0)
            .param_f("density", 0.4)
            .param_f("seed", 137.0)
            .param_f("burst", 0.15)
            .param_f("gate len", 0.5)
            .build(),
    );

    // Filter - modulated by generative sources (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1500.0)
            .param_f("resonance", 0.4)
            .param_f("key_track", 0.5)
            .param_f("cv_amt", 3000.0)
            .build(),
    );

    // Pad envelope - sustaining drone (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 400.0)
            .param_f("attack", 1.0)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.8)
            .param_f("release", 2.0)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1650.0, 50.0)
            .param_f("master level", 0.75)
            .build(),
    );

    // === Effects (auto-routed) ===

    // Phase Vocoder — harmonic interest (pvc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::PhaseVocoder)
            .position(2050.0, 50.0)
            .param_f("pitch shift", 5.0)
            .param_b("freeze", false)
            .param_choice("fft size", "2048")
            .param_f("mix", 0.35)
            .build(),
    );

    // Reverse/Gate Reverb — backward swells (rgr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ReverseGateReverb)
            .position(2050.0, 400.0)
            .param_f("window", 600.0)
            .param_choice("mode", "reverse")
            .param_choice("trigger", "periodic")
            .param_f("threshold", -24.0)
            .param_f("gate time", 150.0)
            .param_f("mix", 0.4)
            .build(),
    );

    // === Connections ===
    // LFO clock → all generative modules
    patch.add_connection("lfo-1", "out", "euc-1", "clock");
    patch.add_connection("lfo-1", "out", "tur-1", "clock");
    patch.add_connection("lfo-1", "out", "rgt-1", "clock");

    // Turing Machine pitch → Osc FM (melodic variation)
    patch.add_connection("tur-1", "pitch", "osc-1", "fm");

    // Random Gates CV → Filter cutoff (random filter sweeps)
    patch.add_connection("rgt-1", "cv", "flt-1", "cutoff_cv");

    // Main signal path: Osc → Filter → Amp → Output
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group(
        "Generative",
        Some("#D96A4A"),
        &["lfo-1", "euc-1", "tur-1", "rgt-1"],
    );
    patch.add_group("Voice", Some("#7B68EE"), &["osc-1", "flt-1"]);
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group("Effects", Some("#D4A94A"), &["pvc-1", "rgr-1"]);

    patch
}
