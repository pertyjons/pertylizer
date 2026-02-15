//! Kinetic Pluck — short, percussive pluck using Kinetic Modulator.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Kinetic Pluck — CubicOut easing drives filter and amp for a natural pluck.
pub fn patch_kinetic_pluck() -> Patch {
    let mut patch = Patch::new("Kinetic Pluck");
    patch.author = Some("Modular Synth".to_string());
    patch.description =
        Some("Short pluck using Kinetic Modulator's CubicOut curve for natural decay.".to_string());
    patch.tags = vec!["pluck".into(), "kinetic".into(), "percussive".into()];

    // OSC - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );

    // Filter (flt-1) - High cutoff swept down by kinetic mod
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.35)
            .build(),
    );

    // Kinetic Modulator (kin-1) - CubicOut, 150ms, OneShot
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::KineticModulator)
            .position(50.0, 300.0)
            .param_f("duration", 0.15)
            .param_choice("curve", "cubic_out")
            .param_f("overshoot", 0.5)
            .param_choice("loop_mode", "one_shot")
            .param_f("bipolar", 0.0)
            .param_f("retrigger", 1.0)
            .build(),
    );

    // Mod Matrix (mmx-1) - KineticPos -> FilterCutoff + AmpLevel
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::ModMatrix)
            .position(250.0, 300.0)
            .param_f("grid_size", 1.0) // 2x2
            // Slot 0: KineticPos -> Filter Cutoff
            .param_f("slot_1_source", 11.0) // KineticPos index
            .param_f("slot_1_dest", 3.0) // FilterCutoff(0) index
            .param_f("slot_1_amount", 0.8)
            .param_f("slot_1_enabled", 1.0)
            // Slot 1: KineticPos -> Amp Level
            .param_f("slot_2_source", 11.0) // KineticPos index
            .param_f("slot_2_dest", 5.0) // AmpLevel(0) index
            .param_f("slot_2_amount", 1.0)
            .param_f("slot_2_enabled", 1.0)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(650.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
