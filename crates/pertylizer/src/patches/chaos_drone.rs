//! Chaos Drone - Evolving chaotic textures using Lorenz attractor.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Chaos Drone - Evolving chaotic textures using Lorenz attractor.
pub fn patch_chaos_drone() -> Patch {
    let mut patch = Patch::new("Chaos Drone");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("Evolving chaotic textures using the Lorenz strange attractor.".to_string());
    patch.notes = Some(
        r"
SIGNAL FLOW:
The Math Oscillator uses the Lorenz chaos algorithm to create unpredictable,
evolving waveforms. The Lorenz attractor is a famous mathematical system
that exhibits chaotic behavior - small changes lead to vastly different outcomes.

PARAMETERS:
- Param A: Controls the chaos speed (higher = faster evolution)
- Param B: Controls the output scaling/amplitude
- Param C: Not used by Lorenz algorithm

The output is filtered through a lowpass filter to tame harsh frequencies,
then processed with reverb for an ethereal, spacious sound.

MODULATION:
An LFO slowly modulates Param A, causing the chaos to speed up and slow down
over time, creating organic variation in the texture.

TRY: Play sustained notes and let the chaos evolve. Each note will sound
different due to the chaotic nature of the algorithm.
"
        .to_string(),
    );
    patch.tags = vec![
        "math".into(),
        "chaos".into(),
        "drone".into(),
        "experimental".into(),
        "ambient".into(),
    ];

    // Math Oscillator - Lorenz chaos (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("lorenz")
            .param_f("param_a", 0.5)
            .param_f("param_b", 0.7)
            .param_f("param_c", 0.5)
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Smooth out harsh chaos (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2000.0)
            .param_f("resonance", 0.3)
            .build(),
    );

    // Amp Envelope - Long pad envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 2.0)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.7)
            .param_f("release", 4.0)
            .build(),
    );

    // LFO - Modulate chaos speed (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 300.0)
            .waveform("sine")
            .param_f("rate", 0.05)
            .param_f("depth", 0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Reverb - Spacious (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1600.0, 50.0)
            .param_f("room_size", 0.9)
            .param_f("damping", 0.3)
            .param_f("mix", 0.5)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1600.0, 350.0)
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
    patch.add_connection("lfo-1", "out", "mth-1", "param_a");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = -1;
    patch
}
