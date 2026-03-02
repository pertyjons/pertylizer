//! Formant Voice - Vocal-like synthesis.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Formant Voice - Vocal-like synthesis.
pub fn patch_formant_voice() -> Patch {
    let mut patch = Patch::new("Formant Voice");
    patch.author = Some("Pertylizer".to_string());
    patch.description = Some("Vocal-like sounds using formant synthesis.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Formant synthesis creates vocal-like sounds by simulating the resonances
of the human vocal tract. The formant algorithm generates a carrier
wave with decaying resonant peaks at specific frequencies.

PARAMETERS:
- Param A: Formant frequency ratio (vowel character)
- Param B: Decay rate (how quickly each cycle fades)
- Param C: Not actively used

Different Param A values create different "vowels":
- Low values (~0.2-0.3): "oo" sounds
- Mid values (~0.5): "ah" sounds
- High values (~0.8-0.9): "ee" sounds

TRY: Play melody lines and slowly sweep Param A to create "talking"
or "singing" effects. LFO modulation adds natural movement.
"#
        .to_string(),
    );
    patch.tags = vec![
        "math".into(),
        "formant".into(),
        "voice".into(),
        "vocal".into(),
    ];

    // Math Oscillator - Formant (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("formant")
            .param_f("param_a", 0.5) // Formant frequency
            .param_f("param_b", 0.4) // Decay
            .param_f("param_c", 0.5)
            .param_f("level", 0.8)
            .build(),
    );

    // Filter (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3500.0)
            .param_f("resonance", 0.2)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.02)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.7)
            .param_f("release", 0.4)
            .build(),
    );

    // LFO - Modulate formant (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 300.0)
            .waveform("sine")
            .param_f("rate", 0.3)
            .param_f("depth", 0.15)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Chorus - Add width (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(1600.0, 50.0)
            .param_f("rate", 0.6)
            .param_f("depth", 0.3)
            .param_f("mix", 0.3)
            .build(),
    );

    // Reverb (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1600.0, 300.0)
            .param_f("room_size", 0.6)
            .param_f("damping", 0.4)
            .param_f("mix", 0.3)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1600.0, 600.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("mth-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("lfo-1", "out", "mth-1", "param_a");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
