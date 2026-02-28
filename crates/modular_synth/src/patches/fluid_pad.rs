//! Fluid Pad - Evolving pad with morphing filter character.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Fluid Pad - Lush evolving pad using the Fluid filter's morph crossfade.
pub fn patch_fluid_pad() -> Patch {
    let mut patch = Patch::new("Fluid Pad");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Dreamy pad with Fluid filter morph swept by LFO for evolving timbral movement."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Triangle oscillator (5-voice unison) -> Filter (Fluid, Morph LFO) -> Amplifier -> Reverb -> Output

The Fluid filter's morph parameter smoothly crossfades LP→BP→HP→Notch.
An LFO slowly sweeps the morph position, creating a continuously evolving
timbral character. Drive adds warm Oberheim-style saturation.

MODULATION:
- LFO 1 -> Filter Morph (slow timbral sweep via mod matrix)
- Env 2 -> Filter Cutoff (brightness on attack)

TRY: Increase morph for brighter starting position.
Increase drive for warmer, more saturated character.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "fluid".into(),
        "morph".into(),
        "evolving".into(),
        "stereo".into(),
    ];

    // Oscillator - Triangle with 5-voice unison (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("triangle")
            .param_f("level", 0.85)
            .param_f("unison", 5.0)
            .param_f("uni detune", 10.0)
            .param_f("uni spread", 0.8)
            .param_f("uni phase", 1.0)
            .build(),
    );

    // Filter - Fluid model with morph (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_model("fluid")
            .param_f("morph", 0.15)
            .param_f("cutoff", 1200.0)
            .param_f("resonance", 0.35)
            .param_f("drive", 1.8)
            .build(),
    );

    // Amp Envelope - Slow pad (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.6)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.8)
            .param_f("release", 1.5)
            .build(),
    );

    // Filter Envelope - Brightness on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(2250.0, 50.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.8)
            .param_f("sustain", 0.2)
            .param_f("release", 0.5)
            .build(),
    );

    // LFO - Slow morph sweep (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(2250.0, 350.0)
            .param_choice("waveform", "triangle")
            .param_f("rate", 0.12)
            .param_f("depth", 0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // Mod Matrix - LFO->Morph, Env2->Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 350.0)
            .param_choice("grid size", "2x2")
            .param_choice("slot 1 source", "lfo1")
            .param_choice("slot 1 dest", "flt1_reso")
            .param_f("slot 1 amount", 0.3)
            .param_choice("slot 2 source", "env2")
            .param_choice("slot 2 dest", "flt1_cutoff")
            .param_f("slot 2 amount", 0.6)
            .build(),
    );

    // Reverb (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1600.0, 50.0)
            .param_f("room_size", 0.85)
            .param_f("damping", 0.4)
            .param_f("mix", 0.45)
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
    // Note: env-2 -> cutoff and lfo-1 -> resonance are routed via Mod Matrix, no cables needed.
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
