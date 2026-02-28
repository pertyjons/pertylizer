//! Dual envelope: fast attack + slow pad envelopes.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Dual Envelope");
    t.description = Some(
        "Two envelopes: a fast percussive one and a slow pad envelope for layered modulation"
            .to_string(),
    );
    t.category = Some(GroupCategory::Utility);
    t.tags = vec!["utility".into(), "envelope".into(), "modulation".into()];
    t.color = Some("#50C878".to_string());

    // Fast envelope
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(0.0, 0.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.0)
            .param_f("release", 0.1)
            .build(),
    );
    // Slow envelope
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(0.0, 200.0)
            .param_f("attack", 0.5)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.6)
            .param_f("release", 1.5)
            .build(),
    );

    t.expose_output("Fast Env", "env-1", "out");
    t.expose_output("Slow Env", "env-2", "out");

    t
}
