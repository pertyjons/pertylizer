//! Stereo Unison Pad - Wide, evolving pad using stereo unison outputs directly.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Stereo Unison Pad - Immersive, wide ambient pad with direct stereo routing.
pub fn patch_stereo_unison_pad() -> Patch {
    let mut patch = Patch::new("Stereo Unison Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Lush ambient pad using triangle unison with stereo outputs, MidSide processing for enhanced width."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Triangle oscillator (5-voice unison) --out_l/out_r--> Amplifier (in_l/in_r) -> Reverb -> Output

This patch demonstrates the stereo unison outputs (out_l/out_r) routed
directly to the amplifier's stereo inputs, bypassing the filter entirely.
This preserves the full stereo spread from the unison panning.

The triangle wave provides a soft, pure starting point. With 5 voices and
moderate detune, it creates a shimmering, choir-like texture. The slow LFO
modulates oscillator pitch for gentle vibrato/movement.

KEY DESIGN:
- Uses out_l/out_r for true stereo unison (not mono sum)
- No filter in path = full frequency content, airy character
- Reverb adds space and depth
- Low detune (8 cents) keeps it musical, not dissonant

TRY: Increase uni detune for a more dissonant, experimental texture.
Set uni spread to 0 for mono collapse, then sweep back to 1.0.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "ambient".into(),
        "unison".into(),
        "stereo".into(),
        "wide".into(),
    ];

    // Oscillator - 5-voice triangle unison (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("triangle")
            .param_f("level", 0.9)
            .param_f("unison", 5.0)
            .param_f("uni detune", 8.0)
            .param_f("uni spread", 0.85)
            .param_f("uni phase", 1.0)
            .build(),
    );

    // Amp Envelope - Slow, dreamy pad (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.8)
            .param_f("decay", 0.6)
            .param_f("sustain", 0.85)
            .param_f("release", 2.0)
            .build(),
    );

    // LFO - Slow pitch drift (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 400.0)
            .param_choice("waveform", "sine")
            .param_f("rate", 0.15)
            .param_f("depth", 0.02)
            .build(),
    );

    // Amplifier - Stereo input (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // MidSide - Stereo width and rotation (mds-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MidSide)
            .position(1200.0, 300.0)
            .param_f("width", 0.65)
            .param_f("mid_gain", 0.0)
            .param_f("side_gain", 2.0)
            .param_f("rotation", 0.15)
            .param_f("mix", 0.8)
            .build(),
    );

    // Reverb - Large space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 50.0)
            .param_f("room_size", 0.9)
            .param_f("damping", 0.3)
            .param_f("mix", 0.5)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(800.0, 50.0)
            .param_f("master level", 0.7)
            .build(),
    );

    // Connections - stereo path, no filter
    patch.add_connection("osc-1", "out_l", "amp-1", "in_l");
    patch.add_connection("osc-1", "out_r", "amp-1", "in_r");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("lfo-1", "out", "osc-1", "fm");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
