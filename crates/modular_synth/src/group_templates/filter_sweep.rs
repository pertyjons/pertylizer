//! Filter + Envelope for cutoff modulation (audio in/out).

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Filter Sweep");
    t.description =
        Some("Lowpass filter with envelope-controlled cutoff for sweeping effects".to_string());
    t.category = Some(GroupCategory::Utility);
    t.tags = vec!["utility".into(), "filter".into(), "sweep".into()];
    t.color = Some("#50C878".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(0.0, 0.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.5)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(0.0, 200.0)
            .param_f("attack", 0.1)
            .param_f("decay", 0.8)
            .param_f("sustain", 0.2)
            .param_f("release", 0.5)
            .build(),
    );

    t.add_connection("env-1", "out", "flt-1", "cutoff_cv");

    t.expose_input("Audio In", "flt-1", "in");
    t.expose_output("Audio Out", "flt-1", "out");

    t
}
