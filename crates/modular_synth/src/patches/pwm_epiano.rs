//! PWM E-Piano - Electric piano using PWM wavetable with velocity response.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// PWM E-Piano - Warm electric piano using PWM wavetable morphing.
pub fn patch_pwm_epiano() -> Patch {
    let mut patch = Patch::new("PWM E-Piano");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Warm electric piano using the PWM wavetable with envelope-driven pulse width sweep."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (PWM bank) -> Filter (Fluid) -> Amplifier -> Chorus -> Reverb -> Output

The PWM wavetable morphs from a 50% square wave to a very narrow 5% pulse.
An envelope sweeps the position on each note attack, creating the classic
hollow-to-thin timbral shift characteristic of electric pianos.

The Fluid filter adds warm analog character, and chorus provides the
classic Rhodes-like stereo movement.

MODULATION:
- Env 2 -> Wavetable Position CV (pulse width sweep)
- Env 1 -> Amplifier (piano-like envelope)
- Velocity -> Amp Level (via mod matrix)

TRY: Play chord voicings for classic EP sounds.
Velocity controls brightness — play soft for mellow, hard for punchy.
"#
        .to_string(),
    );
    patch.tags = vec![
        "keys".into(),
        "wavetable".into(),
        "pwm".into(),
        "electric_piano".into(),
        "warm".into(),
    ];

    // Wavetable Oscillator - PWM bank (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "pwm")
            .param_f("position", 0.15)
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Fluid model for warm EP character (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(250.0, 50.0)
            .filter_model("fluid")
            .param_f("cutoff", 3500.0)
            .param_f("resonance", 0.15)
            .param_f("morph", 0.05)
            .param_f("drive", 1.2)
            .build(),
    );

    // Amp Envelope - Piano-like (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.005)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.4)
            .param_f("release", 0.5)
            .build(),
    );

    // Position Envelope - PWM sweep on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.6)
            .param_f("sustain", 0.1)
            .param_f("release", 0.3)
            .build(),
    );

    // Mod Matrix - Velocity -> Amp level (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::ModMatrix)
            .position(450.0, 300.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "velocity")
            .param_choice("slot 1 dest", "amp_level")
            .param_f("slot 1 amount", 0.4)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Chorus - Classic EP chorus (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Chorus)
            .position(650.0, 50.0)
            .param_f("rate", 0.8)
            .param_f("depth", 0.35)
            .param_f("mix", 0.3)
            .build(),
    );

    // Reverb - Small room (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Reverb)
            .position(650.0, 200.0)
            .param_f("room_size", 0.45)
            .param_f("damping", 0.5)
            .param_f("mix", 0.2)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(850.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("wtb-1", "out", "flt-1", "in");
    patch.add_connection("env-2", "out", "wtb-1", "pos_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
