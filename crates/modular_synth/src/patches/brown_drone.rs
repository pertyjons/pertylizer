//! Brown Drone - Dark ambient drone using brown noise.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Brown Drone - Ambient drone showcasing the colored noise generator.
pub fn patch_brown_drone() -> Patch {
    let mut patch = Patch::new("Brown Drone");
    patch.author = Some("Modular Synth".to_string());
    patch.description =
        Some("Dark, rumbling ambient drone using brown noise filtering.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
This patch demonstrates the new NoiseGenerator module with brown noise.
Brown noise has a -6dB/octave slope, giving it a dark, rumbling character.

NOISE GENERATOR:
Brown noise is created by integrating white noise, resulting in a
"random walk" that emphasizes low frequencies. It's named after
Robert Brown (Brownian motion), not the color.

CHARACTER:
- Deep, rumbling low end
- Like distant thunder or ocean waves
- Warm and non-harsh

FILTER:
A resonant lowpass filter shapes the noise and adds harmonic focus.
The LFO slowly modulates the filter for evolving movement.

REVERB:
Heavy reverb creates an expansive, atmospheric space.

TRY: Hold notes and let the sound evolve. Great for ambient music,
sound design, and meditation soundscapes.
"#
        .to_string(),
    );
    patch.tags = vec![
        "ambient".into(),
        "drone".into(),
        "noise".into(),
        "experimental".into(),
        "dark".into(),
    ];

    // Brown Noise Generator (nse-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Noise)
            .position(50.0, 50.0)
            .param_choice("type", "brown") // Dark, rumbling noise
            .param_f("level", 0.9)
            .build(),
    );

    // Filter - Lowpass with resonance (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 400.0)
            .param_f("resonance", 0.5)
            .build(),
    );

    // Slow LFO for filter movement (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Lfo)
            .position(50.0, 300.0)
            .waveform("sine")
            .param_f("rate", 0.08)
            .param_f("depth", 0.4)
            .build(),
    );

    // Amp Envelope - Slow attack for drones (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 1.5)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.8)
            .param_f("release", 2.0)
            .param_f("atk_curve", 0.3) // Slow, gradual attack
            .param_f("dec_curve", 0.0) // Linear decay
            .param_f("rel_curve", 0.5) // Long, natural fade
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Reverb - Large space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Reverb)
            .position(650.0, 50.0)
            .param_f("room_size", 0.9)
            .param_f("damping", 0.3)
            .param_f("mix", 0.5)
            .build(),
    );

    // Oscilloscope (scp-1)
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
            .param_f("master level", 0.75)
            .build(),
    );

    // Connections
    patch.add_connection("nse-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
