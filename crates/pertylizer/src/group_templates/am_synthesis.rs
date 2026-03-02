//! Tutorial: Ring modulator AM synthesis demo.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("AM Synthesis");
    t.description = Some(
        "Amplitude modulation demo using a ring modulator. \
         One oscillator (carrier) has its amplitude modulated by another (modulator), \
         creating metallic, bell-like sidebands. Try different modulator frequencies \
         for different timbres."
            .to_string(),
    );
    t.category = Some(GroupCategory::Tutorial);
    t.tags = vec![
        "tutorial".into(),
        "am".into(),
        "ring mod".into(),
        "learning".into(),
    ];
    t.color = Some("#FFD700".to_string());

    // Carrier oscillator
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(0.0, 0.0)
            .waveform("sine")
            .param_f("level", 1.0)
            .build(),
    );
    // Modulator oscillator
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(0.0, 200.0)
            .waveform("sine")
            .param_f("level", 1.0)
            .param_f("detune", 200.0)
            .build(),
    );
    // Ring modulator
    t.add_module(
        ModuleBuilder::new(1, ModuleType::RingMod)
            .position(200.0, 100.0)
            .build(),
    );
    // Amp envelope for note shaping
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(400.0, 200.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.5)
            .param_f("release", 0.5)
            .build(),
    );
    // Output amplifier
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(400.0, 0.0)
            .param_f("gain", 0.7)
            .build(),
    );

    t.add_connection("osc-1", "out", "rng-1", "in1");
    t.add_connection("osc-2", "out", "rng-1", "in2");
    t.add_connection("rng-1", "out", "amp-1", "in");
    t.add_connection("env-1", "out", "amp-1", "cv");

    t.expose_output("Audio Out", "amp-1", "out");

    t
}
