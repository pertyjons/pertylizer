//! Punchy Stab - Aggressive synth stab showcasing envelope curves.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Punchy Stab - Demonstrates the power of envelope curve parameters.
pub fn patch_punchy_stab() -> Patch {
    let mut patch = Patch::new("Punchy Stab");
    patch.author = Some("Modular Synth".to_string());
    patch.description =
        Some("Aggressive synth stab showcasing the new envelope curve parameters.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
This patch demonstrates the new envelope curve parameters for creating
punchy, aggressive synth stabs perfect for EDM and electronic music.

ENVELOPE CURVES:
The new curve parameters (-1 to +1) control the shape of each envelope stage:
- Negative values = logarithmic (fast initial change, then slower)
- Zero = linear (standard)
- Positive values = exponential (slow initial change, then faster)

ATTACK CURVE (-1.0):
Maximum punch - the attack reaches full level almost instantly,
creating that percussive "crack" at the start of each note.

DECAY CURVE (-0.8):
Fast initial drop creates the characteristic "stab" transient.
The level drops quickly after the attack, then settles.

OSCILLATORS:
Two detuned sawtooths create a thick, aggressive tone.
The detune adds fatness without being too dissonant.

FILTER:
A resonant lowpass with a fast, punchy envelope creates the
classic filter "zap" effect.

TRY: Play staccato chords for dance music stabs. The negative curves
give each note maximum impact and punch.
"#
        .to_string(),
    );
    patch.tags = vec![
        "stab".into(),
        "punchy".into(),
        "edm".into(),
        "aggressive".into(),
        "synth".into(),
    ];

    // OSC1 - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.6)
            .build(),
    );

    // OSC2 - Detuned Sawtooth (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("detune", 12.0) // +12 cents for thickness
            .param_f("level", 0.5)
            .build(),
    );

    // Filter - Resonant lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.5)
            .build(),
    );

    // Amp Envelope - Maximum punch (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.15)
            .param_f("sustain", 0.3)
            .param_f("release", 0.1)
            .param_f("atk_curve", -1.0) // Maximum instant punch
            .param_f("dec_curve", -0.8) // Fast initial drop
            .param_f("rel_curve", -0.6) // Tight cutoff
            .build(),
    );

    // Filter Envelope - Fast zap (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.1)
            .param_f("sustain", 0.1)
            .param_f("release", 0.08)
            .param_f("atk_curve", -1.0) // Instant filter open
            .param_f("dec_curve", -0.9) // Very fast filter close
            .param_f("rel_curve", -0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Distortion - Adds edge (dst-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Distortion)
            .position(1600.0, 350.0)
            .distortion_mode("tube")
            .param_f("drive", 0.4)
            .param_f("tone", 0.5)
            .param_f("mix", 0.4)
            .build(),
    );

    // Oscilloscope (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1600.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master level", 0.75)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("osc-2", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = 0;
    patch
}
