//! FM pair: Modulator oscillator -> Amplifier -> Carrier oscillator FM input + envelope.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("FM Pair");
    t.description = Some(
        "Two-operator FM: modulator through amp into carrier FM input with modulation envelope"
            .to_string(),
    );
    t.category = Some(GroupCategory::Voice);
    t.tags = vec!["fm".into(), "voice".into(), "bell".into()];
    t.color = Some("#D4A94A".to_string());

    // Modulator oscillator
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(0.0, 0.0)
            .waveform("sine")
            .param_f("level", 1.0)
            .build(),
    );
    // Modulator amp (controls FM depth)
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(200.0, 0.0)
            .param_f("gain", 0.5)
            .build(),
    );
    // Carrier oscillator
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(400.0, 0.0)
            .waveform("sine")
            .param_f("level", 1.0)
            .build(),
    );
    // Modulation envelope
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(0.0, 200.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.8)
            .param_f("sustain", 0.0)
            .param_f("release", 0.5)
            .build(),
    );
    // Carrier envelope
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(400.0, 200.0)
            .param_f("attack", 0.001)
            .param_f("decay", 1.5)
            .param_f("sustain", 0.0)
            .param_f("release", 1.0)
            .build(),
    );

    t.add_connection("osc-1", "out", "amp-1", "in");
    t.add_connection("amp-1", "out", "osc-2", "fm");
    t.add_connection("env-1", "out", "amp-1", "cv");

    t.expose_output("Audio Out", "osc-2", "out");
    t.expose_output("Carrier Env", "env-2", "out");

    t
}
