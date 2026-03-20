//! Wave Folder Bass - West coast synthesis bass.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Wave Folder Bass - West coast synthesis bass.
pub fn patch_wave_folder_bass() -> Patch {
    let mut patch = Patch::new("Wave Folder Bass");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some("Rich, harmonically complex bass using wave folding.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wave folding is a classic West Coast synthesis technique. When a waveform
exceeds a threshold, it "folds" back, creating rich harmonics. More folding
= more harmonics = brighter, more complex sound.

PARAMETERS:
- Param A: Fold amount (more = richer harmonics)
- Param B: DC offset (shifts the folding point)
- Param C: Not actively used

Unlike subtractive synthesis (filtering harmonics out), wave folding
ADDS harmonics, creating sounds impossible with traditional oscillators.

ENVELOPE MODULATION:
The filter envelope modulates the fold amount, creating dynamic harmonic
content - bright attack that settles into a warm sustain.

TRY: Play bass lines and adjust Param A for different harmonic density.
Low values = warm, high values = aggressive and metallic.
"#
        .to_string(),
    );
    patch.tags = vec![
        "math".into(),
        "wave_folder".into(),
        "bass".into(),
        "west_coast".into(),
    ];

    // Math Oscillator - Wave folder (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("wave_folder")
            .param_f("param_a", 0.4) // Fold amount
            .param_f("param_b", 0.3) // Offset
            .param_f("param_c", 0.5)
            .param_f("level", 0.9)
            .build(),
    );

    // Filter - Shape the bass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 600.0)
            .param_f("resonance", 0.4)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.6)
            .param_f("release", 0.15)
            .build(),
    );

    // Filter Envelope (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.15)
            .param_f("sustain", 0.2)
            .param_f("release", 0.1)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Distortion - Add grit (dst-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Distortion)
            .position(1600.0, 350.0)
            .distortion_mode("tube")
            .param_f("drive", 0.3)
            .param_f("mix", 0.4)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
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
            .param_f("master level", 0.7)
            .build(),
    );

    // Connections
    patch.add_connection("mth-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("env-2", "out", "mth-1", "param_a");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = -2;
    patch
}
