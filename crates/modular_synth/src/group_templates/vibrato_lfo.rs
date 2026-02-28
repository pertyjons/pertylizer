//! Vibrato LFO: sine wave at 5.5 Hz for pitch modulation.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Vibrato LFO");
    t.description = Some("Sine LFO at 5.5 Hz for classic vibrato modulation".to_string());
    t.category = Some(GroupCategory::Utility);
    t.tags = vec![
        "utility".into(),
        "lfo".into(),
        "vibrato".into(),
        "modulation".into(),
    ];
    t.color = Some("#50C878".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(0.0, 0.0)
            .waveform("sine")
            .param_f("rate", 5.5)
            .param_f("depth", 0.3)
            .build(),
    );

    t.expose_output("Mod Out", "lfo-1", "out");

    t
}
