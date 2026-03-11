//! Unison PWM Strings - Lush string ensemble using pulse-width modulation and unison.

use crate::patch::{ModuleBuilder, Patch, PatchAuthor};
use synth_core::ModuleType;

/// Unison PWM Strings - Rich, animated string section with PWM and stereo unison.
pub fn patch_unison_pwm_strings() -> Patch {
    let mut patch = Patch::new("Unison PWM Strings");
    patch.author = Some(PatchAuthor::from("Pertylizer"));
    patch.description = Some(
        "Lush string ensemble combining pulse-width modulation with 5-voice stereo unison."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Pulse oscillator (5-voice unison) -> Filter (Lowpass) -> Amplifier -> Chorus -> Reverb -> Output

TECHNIQUE:
Classic analog string machines achieved their lush sound by combining
pulse-width modulation with multiple detuned oscillators. This patch
recreates that approach:

1. Pulse wave with LFO-driven PWM creates the animated, hollow character
2. 5-voice unison with moderate detune (15 cents) adds ensemble thickness
3. Wide stereo spread (0.7) gives spatial dimension
4. Gentle lowpass filter removes harsh upper harmonics
5. Chorus + reverb complete the string machine sound

The combination of PWM (timbral movement) and unison detune (pitch
chorusing) creates a doubly-rich animation that neither technique
achieves alone.

MODULATION:
- LFO 1 -> PWM: Slow pulse width sweep for timbral animation
- LFO 2 -> Osc FM: Subtle vibrato for expressiveness
- Mod Matrix: Velocity -> Amp Level for dynamic playing

TRY: Play slow chords. Adjust PWM LFO rate for faster or slower animation.
Reduce unison to 3 for a thinner, more intimate chamber string sound.
"#
        .to_string(),
    );
    patch.tags = vec![
        "strings".into(),
        "pad".into(),
        "unison".into(),
        "pwm".into(),
        "ensemble".into(),
        "stereo".into(),
    ];

    // Oscillator - Pulse wave with 5-voice unison (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("pulse")
            .param_f("pulse width", 0.5)
            .param_f("level", 0.75)
            .param_f("unison", 5.0)
            .param_f("uni detune", 15.0)
            .param_f("uni spread", 0.7)
            .param_f("uni phase", 1.0)
            .build(),
    );

    // Filter - Gentle lowpass for warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3500.0)
            .param_f("resonance", 0.1)
            .build(),
    );

    // Amp Envelope - Slow string attack (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.5)
            .param_f("decay", 0.4)
            .param_f("sustain", 0.8)
            .param_f("release", 1.5)
            .build(),
    );

    // PWM LFO - Slow pulse width sweep (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 600.0)
            .param_choice("waveform", "triangle")
            .param_f("rate", 0.4)
            .param_f("depth", 0.6)
            .build(),
    );

    // Vibrato LFO - Subtle pitch movement (lfo-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Lfo)
            .position(50.0, 400.0)
            .param_choice("waveform", "sine")
            .param_f("rate", 5.0)
            .param_f("depth", 0.015)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // Mod Matrix - Velocity -> Amp Level (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 600.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "velocity")
            .param_choice("slot 1 dest", "amp_level")
            .param_f("slot 1 amount", 0.3)
            .build(),
    );

    // Chorus - Extra ensemble width (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(1600.0, 50.0)
            .param_f("rate", 0.3)
            .param_f("depth", 0.4)
            .param_f("mix", 0.4)
            .build(),
    );

    // Reverb - Concert hall (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1600.0, 300.0)
            .param_f("room_size", 0.8)
            .param_f("damping", 0.35)
            .param_f("mix", 0.35)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master level", 0.7)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("lfo-1", "out", "osc-1", "pwm"); // PWM modulation
    patch.add_connection("lfo-2", "out", "osc-1", "fm"); // Vibrato
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
