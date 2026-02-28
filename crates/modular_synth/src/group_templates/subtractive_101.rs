//! Tutorial: Full subtractive synth voice with detailed description.

use synth_core::ModuleType;

use crate::patch::{GroupCategory, GroupTemplate, ModuleBuilder};

pub fn template() -> GroupTemplate {
    let mut t = GroupTemplate::new("Subtractive 101");
    t.description = Some(
        "Complete subtractive synth voice for learning. \
         Signal flow: Oscillator generates a harmonically rich sawtooth wave, \
         the filter removes upper harmonics (controlled by an envelope), \
         and the amplifier shapes the volume (controlled by another envelope)."
            .to_string(),
    );
    t.category = Some(GroupCategory::Tutorial);
    t.tags = vec![
        "tutorial".into(),
        "subtractive".into(),
        "learning".into(),
        "voice".into(),
    ];
    t.color = Some("#FFD700".to_string());

    // Sound source: sawtooth oscillator (rich in harmonics)
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(0.0, 0.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );
    // Tone control: lowpass filter sculpts the timbre
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(200.0, 0.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1500.0)
            .param_f("resonance", 0.3)
            .build(),
    );
    // Volume control: amplifier shapes the loudness
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(400.0, 0.0)
            .param_f("gain", 0.8)
            .build(),
    );
    // Amp envelope: shapes how the volume changes over time
    t.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(400.0, 200.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.4)
            .param_f("sustain", 0.6)
            .param_f("release", 0.4)
            .build(),
    );
    // Filter envelope: makes the filter open and close on each note
    t.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(200.0, 200.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.3)
            .param_f("release", 0.5)
            .build(),
    );
    // Output module
    t.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(600.0, 0.0)
            .build(),
    );

    // Signal chain
    t.add_connection("osc-1", "out", "flt-1", "in");
    t.add_connection("flt-1", "out", "amp-1", "in");
    t.add_connection("amp-1", "out", "out-1", "in_l");
    t.add_connection("amp-1", "out", "out-1", "in_r");
    // Modulation
    t.add_connection("env-1", "out", "amp-1", "cv");
    t.add_connection("env-2", "out", "flt-1", "cutoff_cv");

    t
}
