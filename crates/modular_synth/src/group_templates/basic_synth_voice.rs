//! Basic synth voice: Oscillator -> Filter -> Amplifier + 2 envelopes.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Basic Synth Voice");
    t.description = Some(
        "Classic subtractive voice: Osc -> Filter -> Amp with amp and filter envelopes".to_string(),
    );
    t.category = Some(GroupCategory::Voice);
    t.tags = vec!["voice".into(), "subtractive".into(), "basic".into()];
    t.color = Some("#4A90D9".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(0.0, 0.0)
            .waveform("sawtooth")
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(200.0, 0.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2000.0)
            .param_f("resonance", 0.3)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(400.0, 0.0)
            .param_f("gain", 0.8)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(200.0, 200.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.7)
            .param_f("release", 0.3)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(0.0, 200.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.4)
            .param_f("release", 0.4)
            .build(),
    );

    t.add_connection("osc-1", "out", "flt-1", "in");
    t.add_connection("flt-1", "out", "amp-1", "in");
    t.add_connection("env-1", "out", "amp-1", "cv");
    t.add_connection("env-2", "out", "flt-1", "cutoff_cv");

    t.expose_output("Audio Out", "amp-1", "out");
    t.expose_input("FM", "osc-1", "fm");

    t
}
