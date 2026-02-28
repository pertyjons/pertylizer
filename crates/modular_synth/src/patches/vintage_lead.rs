//! Vintage Lead - Classic analog-style mono lead with vibrato.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Vintage Lead - Classic analog-style mono lead with vibrato.
pub fn patch_vintage_lead() -> Patch {
    let mut patch = Patch::new("Vintage Lead");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Classic analog-style mono lead with vibrato and delay.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Two oscillators create the core sound: a sawtooth for brightness and a
pulse wave (with PWM) for movement and thickness. The pulse width is
modulated by a slow LFO, creating the classic "breathing" analog sound.

The mixed signal passes through a resonant lowpass filter. The filter
envelope provides the attack "bite" that helps notes stand out.

VIBRATO:
A dedicated vibrato LFO modulates OSC1's pitch at about 5.5 Hz. The depth
is subtle (1.5%) to add expression without being overbearing. Classic
lead synths use this technique for a more human, expressive quality.

EFFECTS:
The delay adds rhythmic interest and fills space between notes. The
ping-pong mode creates stereo width. Moderate feedback creates trails
without overwhelming the dry signal.

TRY: Play melodic lines in the upper register. The vibrato adds expression
to held notes. Use pitch bends for extra expressiveness.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "vintage".into(),
        "analog".into(),
        "mono".into(),
    ];

    // OSC1 - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("level", 0.6)
            .build(),
    );

    // OSC2 - Pulse with PWM (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("pulse")
            .param_f("level", 0.4)
            .param_f("pulse_width", 0.3)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .param_f("master", 0.85)
            .build(),
    );

    // Filter (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2000.0)
            .param_f("resonance", 0.4)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 350.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.1)
            .param_f("sustain", 0.8)
            .param_f("release", 0.2)
            .build(),
    );

    // Filter Envelope (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.4)
            .param_f("release", 0.15)
            .build(),
    );

    // Vibrato LFO (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 950.0)
            .waveform("sine")
            .param_f("rate", 5.5)
            .param_f("depth", 0.015)
            .build(),
    );

    // PWM LFO (lfo-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Lfo)
            .position(50.0, 750.0)
            .waveform("triangle")
            .param_f("rate", 0.4)
            .param_f("depth", 0.35)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.75)
            .build(),
    );

    // Delay (dly-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Delay)
            .position(2000.0, 350.0)
            .delay_mode("ping_pong")
            .param_f("time", 0.35)
            .param_f("feedback", 0.4)
            .param_f("mix", 0.3)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(2000.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1600.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections (using string IDs: type-instance)
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("lfo-1", "out", "osc-1", "fm");
    patch.add_connection("lfo-2", "out", "osc-2", "pwm");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = 1;

    // Groups
    patch.add_group(
        "Voice",
        Some("#4A90D9"),
        &["osc-1", "osc-2", "mix-1", "flt-1", "amp-1"],
    );
    patch.add_group(
        "Modulation",
        Some("#50C878"),
        &["env-1", "env-2", "lfo-1", "lfo-2"],
    );

    patch
}
