//! Chorus -> Reverb effect chain (stereo in/out).

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Chorus + Reverb");
    t.description = Some("Stereo chorus into reverb for lush spatial effects".to_string());
    t.category = Some(GroupCategory::Effect);
    t.tags = vec![
        "effect".into(),
        "chorus".into(),
        "reverb".into(),
        "spatial".into(),
    ];
    t.color = Some("#7B68EE".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(0.0, 0.0)
            .param_f("rate", 1.2)
            .param_f("depth", 0.4)
            .param_f("mix", 0.5)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(200.0, 0.0)
            .param_f("room_size", 0.7)
            .param_f("damping", 0.5)
            .param_f("mix", 0.35)
            .build(),
    );

    t.add_connection("chr-1", "left", "rev-1", "in_l");
    t.add_connection("chr-1", "right", "rev-1", "in_r");

    t.expose_input("Audio In L", "chr-1", "in_l");
    t.expose_input("Audio In R", "chr-1", "in_r");
    t.expose_output("Audio Out L", "rev-1", "left");
    t.expose_output("Audio Out R", "rev-1", "right");

    t
}
