//! Shepard Riser - Infinite rising tone effect.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Shepard Riser - Infinite rising tone effect.
pub fn patch_shepard_riser() -> Patch {
    let mut patch = Patch::new("Shepard Riser");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("The Shepard tone - an auditory illusion of endlessly rising pitch.".to_string());
    patch.notes = Some(
        r"
SIGNAL FLOW:
The Shepard tone is a famous auditory illusion where multiple octaves of
a tone rise in pitch, with the amplitudes carefully balanced so that as
high frequencies fade out, low frequencies fade in. The result sounds
like it's perpetually rising without ever getting higher.

PARAMETERS:
- Param A: Frequency center point (where the loudest octave is)
- Param B: Rise speed (negative = falling, positive = rising)
- Param C: Not actively used

PERFECT FOR:
- Transitions and buildups in music
- Sound design for film/games
- Psychoacoustic experiments

TRY: Hold a note and listen - it seems to rise forever! Adjust Param B
to control speed, or set it negative for a falling effect.
"
        .to_string(),
    );
    patch.tags = vec![
        "math".into(),
        "shepard".into(),
        "illusion".into(),
        "riser".into(),
        "experimental".into(),
    ];

    // Math Oscillator - Shepard tone (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("shepard")
            .param_f("param_a", 0.5) // Center frequency
            .param_f("param_b", 0.7) // Rising speed
            .param_f("param_c", 0.5)
            .param_f("level", 0.7)
            .build(),
    );

    // Filter - Smooth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 4000.0)
            .param_f("resonance", 0.1)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 1.0)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.8)
            .param_f("release", 2.0)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.6)
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
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("mth-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
