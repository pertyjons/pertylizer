//! Distortion -> EQ effect chain.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Distortion + EQ");
    t.description = Some("Soft-clip distortion followed by EQ for tone shaping".to_string());
    t.category = Some(GroupCategory::Effect);
    t.tags = vec!["effect".into(), "distortion".into(), "eq".into()];
    t.color = Some("#E06060".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Distortion)
            .position(0.0, 0.0)
            .distortion_mode("soft_clip")
            .param_f("drive", 0.5)
            .param_f("mix", 0.7)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Eq)
            .position(200.0, 0.0)
            .param_f("low_gain", 0.0)
            .param_f("mid_gain", 2.0)
            .param_f("high_gain", -1.0)
            .build(),
    );

    t.add_connection("dst-1", "out", "equ-1", "in");

    t.expose_input("Audio In", "dst-1", "in");
    t.expose_output("Audio Out", "equ-1", "out");

    t
}
