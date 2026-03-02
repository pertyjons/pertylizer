//! Screamer Lead - Aggressive MS-20 style lead with diode distortion.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Screamer Lead - Raw, aggressive lead with MS-20 inspired diode clipping.
pub fn patch_screamer_lead() -> Patch {
    let mut patch = Patch::new("Screamer Lead");
    patch.author = Some("Pertylizer".to_string());
    patch.description = Some(
        "Aggressive lead with Screamer filter's asymmetric diode clipping and high resonance."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Saw oscillator -> Filter (Screamer, high resonance) -> Amplifier -> Output

The Screamer filter's diode-clipped feedback creates the characteristic
MS-20 scream at high resonance. Drive pushes the clipping harder for
increasingly nasty, harmonically rich distortion.

MODULATION:
- Env 2 -> Filter Cutoff (aggressive sweep on attack)

TRY: Push resonance above 0.7 for the classic MS-20 scream.
Increase drive for more diode-clipping aggression.
Lower cutoff for darker, growling character.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "screamer".into(),
        "aggressive".into(),
        "ms20".into(),
        "distorted".into(),
    ];

    // Oscillator - Saw wave (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.9)
            .build(),
    );

    // Filter - Screamer model with high resonance (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_model("screamer")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.72)
            .param_f("drive", 2.5)
            .build(),
    );

    // Amp Envelope - Snappy (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.003)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.7)
            .param_f("release", 0.15)
            .build(),
    );

    // Filter Envelope - Aggressive sweep (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(2250.0, 50.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.18)
            .param_f("sustain", 0.1)
            .param_f("release", 0.1)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Mod Matrix - Env2 -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 50.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "env2")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.8)
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
