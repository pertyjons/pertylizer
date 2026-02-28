//! Dual oscillator voice: 2 Oscillators -> Mixer -> Filter -> Amplifier + 2 envelopes.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Dual Oscillator Voice");
    t.description = Some(
        "Two detuned oscillators mixed into a filter and amplifier with envelopes".to_string(),
    );
    t.category = Some(GroupCategory::Voice);
    t.tags = vec!["voice".into(), "detuned".into(), "thick".into()];
    t.color = Some("#4A90D9".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(0.0, 0.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(0.0, 200.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .param_f("detune", 7.0)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(200.0, 100.0)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(400.0, 100.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3000.0)
            .param_f("resonance", 0.2)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(600.0, 100.0)
            .param_f("gain", 0.8)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(400.0, 300.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.7)
            .param_f("release", 0.3)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(200.0, 300.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.6)
            .param_f("sustain", 0.3)
            .param_f("release", 0.5)
            .build(),
    );

    t.add_connection("osc-1", "out", "mix-1", "in1");
    t.add_connection("osc-2", "out", "mix-1", "in2");
    t.add_connection("mix-1", "out", "flt-1", "in");
    t.add_connection("flt-1", "out", "amp-1", "in");
    t.add_connection("env-1", "out", "amp-1", "cv");
    t.add_connection("env-2", "out", "flt-1", "cutoff_cv");

    t.expose_output("Audio Out", "amp-1", "out");

    t
}
