//! Acid Bass - Classic 303-style acid bass with Steiner-Parker filter.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Acid Bass - Squelchy acid bass with variable saturation and resonance sweep.
pub fn patch_acid_bass() -> Patch {
    let mut patch = Patch::new("Acid Bass");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Squelchy acid bass with Acid filter's resonance-driven saturation and envelope sweep."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Square oscillator -> Filter (Acid, LP mode) -> Amplifier -> Distortion -> Output

The Acid filter's variable saturation morphs from tanh to sine-fold as
resonance increases, creating the characteristic squelchy acid character.
A fast envelope sweep on the cutoff produces the classic 303 accent.

MODULATION:
- Env 2 -> Filter Cutoff (fast decay for squelchy sweep)

TRY: Higher resonance for more squelch and sine-fold saturation.
Shorter envelope decay for tighter, punchier accents.
Switch filter type to BP for a thinner, more nasal acid sound.
"#
        .to_string(),
    );
    patch.tags = vec!["bass".into(), "acid".into(), "303".into(), "squelch".into()];

    // Oscillator - Square wave (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("square")
            .param_f("level", 0.9)
            .build(),
    );

    // Filter - Acid model, LP mode (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(250.0, 50.0)
            .filter_model("acid")
            .filter_mode("lowpass")
            .param_f("cutoff", 300.0)
            .param_f("resonance", 0.65)
            .param_f("drive", 2.0)
            .param_f("key track", 0.5)
            .build(),
    );

    // Amp Envelope - Punchy (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.4)
            .param_f("release", 0.1)
            .build(),
    );

    // Filter Envelope - Fast squelchy sweep (env-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.12)
            .param_f("sustain", 0.0)
            .param_f("release", 0.08)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Mod Matrix - Env2 -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::ModMatrix)
            .position(450.0, 300.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "env2")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.9)
            .build(),
    );

    // Distortion - Light crunch (dst-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Distortion)
            .position(650.0, 50.0)
            .distortion_mode("soft_clip")
            .param_f("drive", 0.4)
            .param_f("mix", 0.5)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(850.0, 50.0)
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
