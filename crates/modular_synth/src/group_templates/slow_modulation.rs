//! Slow modulation: sine LFO at 0.1 Hz for evolving textures.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Slow Modulation");
    t.description =
        Some("Very slow sine LFO at 0.1 Hz for gradual, evolving parameter changes".to_string());
    t.category = Some(GroupCategory::Utility);
    t.tags = vec![
        "utility".into(),
        "lfo".into(),
        "slow".into(),
        "evolving".into(),
    ];
    t.color = Some("#50C878".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(0.0, 0.0)
            .waveform("sine")
            .param_f("rate", 0.1)
            .param_f("depth", 0.5)
            .build(),
    );

    t.expose_output("Mod Out", "lfo-1", "out");

    t
}
