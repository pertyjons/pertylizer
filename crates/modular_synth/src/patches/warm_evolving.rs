//! Warm Evolving - Slowly morphing ambient texture with warm wavetable.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Warm Evolving - Gentle ambient texture with slow wavetable scanning.
pub fn patch_warm_evolving() -> Patch {
    let mut patch = Patch::new("Warm Evolving");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Ambient texture using the Warm wavetable with very slow position scanning and deep reverb."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (Warm bank) -> Filter (Fluid, low morph) -> Amplifier -> Reverb -> Output

The Warm wavetable contains soft analog-style variants with harmonic
saturation. A very slow LFO gently sweeps through the table, creating
subtle timbral shifts over time — perfect for ambient backgrounds.

The Fluid filter at low morph adds gentle Oberheim warmth without
coloring the sound too much. Deep reverb creates an immersive space.

MODULATION:
- LFO 1 -> Wavetable Position CV (very slow timbral drift)
- LFO 2 -> Filter Cutoff CV (subtle filter movement)
- Env 1 -> Amplifier (very slow fade in/out)

TRY: Hold long notes and listen to the slow evolution.
Layer multiple notes for rich ambient textures.
"#
        .to_string(),
    );
    patch.tags = vec![
        "ambient".into(),
        "wavetable".into(),
        "warm".into(),
        "evolving".into(),
        "texture".into(),
    ];

    // Wavetable Oscillator - Warm bank (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "warm")
            .param_f("position", 0.3)
            .param_f("level", 0.75)
            .build(),
    );

    // Filter - Fluid model for gentle warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(400.0, 50.0)
            .filter_model("fluid")
            .param_f("cutoff", 3000.0)
            .param_f("resonance", 0.2)
            .param_f("morph", 0.1)
            .param_f("drive", 1.3)
            .build(),
    );

    // Amp Envelope - Very slow ambient (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(800.0, 350.0)
            .param_f("attack", 2.0)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.85)
            .param_f("release", 4.0)
            .build(),
    );

    // LFO 1 - Very slow position sweep (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 250.0)
            .param_choice("waveform", "sine")
            .param_f("rate", 0.03)
            .param_f("depth", 0.5)
            .build(),
    );

    // LFO 2 - Subtle filter morph movement (lfo-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Lfo)
            .position(400.0, 350.0)
            .param_choice("waveform", "triangle")
            .param_f("rate", 0.07)
            .param_f("depth", 0.25)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(800.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Reverb - Deep immersive space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1550.0, 50.0)
            .param_f("room_size", 0.95)
            .param_f("damping", 0.25)
            .param_f("mix", 0.6)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1150.0, 50.0)
            .param_f("master level", 0.7)
            .build(),
    );

    // Connections
    patch.add_connection("wtb-1", "out", "flt-1", "in");
    patch.add_connection("lfo-1", "out", "wtb-1", "pos_cv");
    patch.add_connection("lfo-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
