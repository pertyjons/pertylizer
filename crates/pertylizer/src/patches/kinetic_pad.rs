//! Kinetic Pad — evolving pad using ElasticOut easing in loop mode.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Kinetic Pad — ElasticOut curve creates organic movement in filter and pitch.
pub fn patch_kinetic_pad() -> Patch {
    let mut patch = Patch::new("Kinetic Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Evolving pad using Kinetic Modulator's ElasticOut curve for organic movement.".to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "kinetic".into(),
        "evolving".into(),
        "ambient".into(),
    ];

    // OSC 1 - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );

    // OSC 2 - Square (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("square")
            .param_f("level", 0.4)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .build(),
    );

    // Filter (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1200.0)
            .param_f("resonance", 0.3)
            .build(),
    );

    // Envelope for amp (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 350.0)
            .param_f("attack", 0.3)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.7)
            .param_f("release", 1.0)
            .build(),
    );

    // Kinetic Modulator (kin-1) - ElasticOut, 2s, Loop, Bipolar
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::KineticModulator)
            .position(250.0, 400.0)
            .param_f("duration", 2.0)
            .param_choice("curve", "elastic_out")
            .param_f("overshoot", 0.6)
            .param_choice("loop_mode", "loop")
            .param_f("bipolar", 1.0)
            .param_f("retrigger", 1.0)
            .build(),
    );

    // Mod Matrix (mmx-1) - KineticPos -> FilterCutoff, KineticVel -> Osc2 Pitch
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(2000.0, 50.0)
            .param_choice("grid size", "2x2")
            // Slot 1: KineticPos -> Filter Cutoff
            .param_choice("slot 1 source", "kinetic_pos")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.5)
            // Slot 2: KineticVel -> Osc 2 Pitch (subtle vibrato)
            .param_choice("slot 2 source", "kinetic_vel")
            .param_choice("slot 2 dest", "osc2_pitch")
            .param_f("slot 2 amount", 0.15)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1600.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
