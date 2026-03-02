//! Delay -> Reverb effect chain (stereo in/out).

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Delay + Reverb");
    t.description = Some("Ping-pong delay into reverb for spacious echo effects".to_string());
    t.category = Some(GroupCategory::Effect);
    t.tags = vec![
        "effect".into(),
        "delay".into(),
        "reverb".into(),
        "echo".into(),
    ];
    t.color = Some("#7B68EE".to_string());

    t.add_module(
        ModuleBuilder::new(1, ModuleType::Delay)
            .position(0.0, 0.0)
            .delay_mode("ping_pong")
            .param_f("time", 0.375)
            .param_f("feedback", 0.4)
            .param_f("mix", 0.35)
            .build(),
    );
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(200.0, 0.0)
            .param_f("room_size", 0.6)
            .param_f("damping", 0.4)
            .param_f("mix", 0.3)
            .build(),
    );

    t.add_connection("dly-1", "left", "rev-1", "in_l");
    t.add_connection("dly-1", "right", "rev-1", "in_r");

    t.expose_input("Audio In L", "dly-1", "in_l");
    t.expose_input("Audio In R", "dly-1", "in_r");
    t.expose_output("Audio Out L", "rev-1", "left");
    t.expose_output("Audio Out R", "rev-1", "right");

    t
}
