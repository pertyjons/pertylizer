//! Karplus Guitar - Physical modeling plucked string sound.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Karplus Guitar - Physical modeling plucked string sound.
pub fn patch_karplus_guitar() -> Patch {
    let mut patch = Patch::new("Karplus Guitar");
    patch.author = Some("Modular Synth".to_string());
    patch.description =
        Some("Physical modeling plucked string using Karplus-Strong synthesis.".to_string());
    patch.notes = Some(
        r"
SIGNAL FLOW:
The Karplus-Strong algorithm simulates a vibrating string. When a note is
triggered, a burst of noise fills a delay line. The output is fed back through
a low-pass filter, creating decaying harmonics like a real plucked string.

PARAMETERS:
- Param A: String damping (0.9-0.99) - higher = longer sustain
- Param B: Pluck strength - intensity of the initial burst
- Param C: Not actively used

This creates remarkably realistic guitar and string-like tones using pure
mathematics - no samples required!

EFFECTS:
A subtle chorus adds width, and reverb places the guitar in an acoustic space.

TRY: Play melodic lines. Try different octaves - lower notes sound like bass
guitar, higher notes like acoustic guitar or harp.
"
        .to_string(),
    );
    patch.tags = vec![
        "math".into(),
        "physical_modeling".into(),
        "guitar".into(),
        "pluck".into(),
        "string".into(),
    ];

    // Math Oscillator - Karplus-Strong (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("karplus_strong")
            .param_f("param_a", 0.7) // Damping
            .param_f("param_b", 0.8) // Pluck strength
            .param_f("param_c", 0.5)
            .param_f("level", 0.9)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.1)
            .param_f("sustain", 0.8)
            .param_f("release", 0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(250.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Chorus - Add width (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Chorus)
            .position(450.0, 50.0)
            .param_f("rate", 0.8)
            .param_f("depth", 0.2)
            .param_f("mix", 0.25)
            .build(),
    );

    // Reverb - Room sound (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Reverb)
            .position(650.0, 50.0)
            .param_f("room_size", 0.5)
            .param_f("damping", 0.4)
            .param_f("mix", 0.3)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscilloscope)
            .position(850.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(1050.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("mth-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
