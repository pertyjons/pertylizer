//! Glitch Pad - Evolving textured pad with wavefold waveshaping and LFO modulation.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Glitch Pad - Digital, textured pad with movement.
pub fn patch_glitch_pad() -> Patch {
    let mut patch = Patch::new("Glitch Pad");
    patch.author = Some("Pertylizer".to_string());
    patch.description =
        Some("Evolving pad with wavefold waveshaping and slow LFO filter modulation.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Triangle oscillator -> Waveshaper (Fold) -> Filter (Lowpass) -> Amplifier -> Output

The triangle wave provides a clean starting point for the wavefolder.
The fold curve adds rich west-coast harmonics, and a small DC bias
shifts the folding point for asymmetric character.

MODULATION:
- LFO -> Filter Cutoff (slow sweep for evolving texture)
- Mod Matrix routes LFO to filter for movement

PARAMETERS:
- Waveshaper drive controls fold intensity
- Waveshaper bias shifts the folding point
- LFO rate controls movement speed
- Filter cutoff sets the base brightness

TRY: Increase waveshaper drive for more aggressive folding.
Adjust bias for asymmetric harmonics. Slow LFO rate for ambient sweeps.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "waveshaper".into(),
        "fold".into(),
        "glitch".into(),
        "ambient".into(),
    ];

    // Oscillator - Triangle wave (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("triangle")
            .param_f("level", 0.8)
            .build(),
    );

    // Waveshaper - Fold (wsh-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Waveshaper)
            .position(1600.0, 50.0)
            .waveshaper_curve("fold")
            .param_f("drive", 0.4)
            .param_f("mix", 0.6)
            .param_f("bias", 0.15)
            .param_f("symmetry", 0.0)
            .build(),
    );

    // Filter - Lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 4000.0)
            .param_f("resonance", 0.2)
            .build(),
    );

    // Amp Envelope - Slow pad envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.4)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.8)
            .param_f("release", 1.2)
            .build(),
    );

    // LFO - Slow modulation (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(2250.0, 50.0)
            .param_choice("waveform", "sine")
            .param_f("rate", 0.3)
            .param_f("depth", 0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Mod Matrix - LFO -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 300.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "lfo1")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.4)
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
    // Note: lfo-1 -> filter cutoff is routed via Mod Matrix, no cable needed.
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
