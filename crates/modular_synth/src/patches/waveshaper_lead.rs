//! Waveshaper Lead - Sharp, harmonically rich lead with sine fold waveshaping.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Waveshaper Lead - Metallic, aggressive lead with harmonic movement.
pub fn patch_waveshaper_lead() -> Patch {
    let mut patch = Patch::new("Waveshaper Lead");
    patch.author = Some("Modular Synth".to_string());
    patch.description =
        Some("Sharp lead with sine fold waveshaping and filter envelope modulation.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Saw oscillator -> Waveshaper (Sine Fold) -> Filter (Lowpass) -> Amplifier -> Output

The sine fold curve creates FM-like metallic harmonics from the saw wave.
Filter envelope opens the cutoff on attack for timbral movement.

MODULATION:
- Env 2 -> Filter Cutoff (opens filter on attack)
- Mod Matrix routes envelope to filter for dynamic brightness

PARAMETERS:
- Waveshaper drive controls harmonic intensity
- Filter cutoff sets the base brightness
- Envelope 2 decay controls how quickly brightness fades

TRY: Increase waveshaper drive for more metallic character.
Lower the filter cutoff for a darker, more muted tone.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "waveshaper".into(),
        "sine_fold".into(),
        "metallic".into(),
    ];

    // Oscillator - Saw wave (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );

    // Waveshaper - Sine Fold (wsh-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Waveshaper)
            .position(1600.0, 50.0)
            .waveshaper_curve("sine_fold")
            .param_f("drive", 0.6)
            .param_f("mix", 0.8)
            .param_f("bias", 0.0)
            .param_f("symmetry", 0.0)
            .build(),
    );

    // Filter - Lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2000.0)
            .param_f("resonance", 0.3)
            .build(),
    );

    // Amp Envelope - Snappy attack (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.6)
            .param_f("release", 0.2)
            .build(),
    );

    // Filter Envelope - Opens cutoff on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(2250.0, 50.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.25)
            .param_f("sustain", 0.15)
            .param_f("release", 0.15)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Mod Matrix - Env2 -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 300.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "env2")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.6)
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
    // Note: env-2 -> filter cutoff is routed via Mod Matrix, no cable needed.
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
