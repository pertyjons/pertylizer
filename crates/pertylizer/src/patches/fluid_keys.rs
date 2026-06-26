//! Fluid Keys - Warm electric piano with morphing filter overtones.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Fluid Keys - Warm keys with Fluid filter morph creating bell-like overtones.
pub fn patch_fluid_keys() -> Patch {
    let mut patch = Patch::new("Fluid Keys");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Warm electric piano using Fluid filter's morph to create evolving overtones on each note."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Saw oscillator -> Filter (Fluid, morph via envelope) -> Amplifier -> Chorus -> Output

The Fluid filter starts with morph at a mid position (BP-HP blend) and
sweeps down to LP via an envelope, creating bell-like attack overtones
that settle into warmth. Key tracking follows the keyboard for consistent
brightness across the range.

MODULATION:
- Env 2 -> Filter Cutoff (brightness decay)

TRY: Increase morph for more bell-like attack character.
Adjust drive for warmer Oberheim-style saturation.
Longer envelope decay for Rhodes-like tine sustain.
"#
        .to_string(),
    );
    patch.tags = vec![
        "keys".into(),
        "fluid".into(),
        "electric_piano".into(),
        "warm".into(),
    ];

    // Oscillator - Saw wave (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Fluid model with mid morph (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_model("fluid")
            .param_f("morph", 0.4)
            .param_f("cutoff", 2500.0)
            .param_f("resonance", 0.25)
            .param_f("drive", 1.4)
            .param_f("key track", 0.6)
            .build(),
    );

    // Amp Envelope - Piano-like (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.6)
            .param_f("sustain", 0.35)
            .param_f("release", 0.4)
            .build(),
    );

    // Filter Envelope - Brightness decay (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.35)
            .param_f("sustain", 0.1)
            .param_f("release", 0.2)
            .build(),
    );

    // Amplifier (amp-1).
    // Level 0.7 → 1.5 to clear the 0.05 `low_output` threshold (re-audit
    // 2026-05-10: the Fluid filter morph is intentionally dense but the
    // voice-graph peak had drifted to 0.039).
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 1.5)
            .build(),
    );

    // Mod Matrix - Env2 -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 300.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "env2")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.5)
            .build(),
    );

    // Chorus - Subtle width (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(1600.0, 50.0)
            .param_f("rate", 0.8)
            .param_f("depth", 0.2)
            .param_f("mix", 0.25)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master", 0.7)
            .build(),
    );

    // Connections
    // Note: env-2 -> filter cutoff is routed via Mod Matrix, no cable needed.
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");

    patch
}
