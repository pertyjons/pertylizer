//! Expressive Lead - Performance lead using Mod Matrix for MIDI expression.

use crate::patch::{ModuleBuilder, Patch, PatchAuthor};
use synth_core::ModuleType;

/// Expressive Lead - Mono lead with mod wheel, aftertouch, and velocity expression.
pub fn patch_expressive_lead() -> Patch {
    let mut patch = Patch::new("Expressive Lead");
    patch.author = Some(PatchAuthor::from("Pertylizer"));
    patch.description = Some(
        "Performance lead with mod wheel vibrato, aftertouch filter, and velocity dynamics.".into(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A bright sawtooth oscillator paired with a slightly detuned square wave creates
a strong, characterful lead tone. A resonant lowpass filter tames the highs,
and the amplifier provides envelope-shaped dynamics.

MOD MATRIX ROUTING (6 active slots):
- Slot 1: Mod Wheel → LFO 1 Depth (amount +0.8)
    Mod wheel controls vibrato intensity — no vibrato at rest, full vibrato
    when the wheel is pushed up. Classic performance technique.
- Slot 2: LFO 1 → Osc 1 Pitch (amount +0.15)
    LFO provides the actual pitch vibrato, but its depth is controlled by
    the mod wheel via slot 1.
- Slot 3: Aftertouch → Filter 1 Cutoff (amount +0.7)
    Press harder on keys to open the filter for brighter, more intense tone.
    Gives "cry" expression to sustained notes.
- Slot 4: Velocity → Filter 1 Cutoff (amount +0.5)
    Initial strike brightness responds to playing dynamics.
- Slot 5: Velocity → Amp Level (amount +0.3)
    Volume responds to velocity for natural dynamics.
- Slot 6: Envelope 2 → Filter 1 Cutoff (amount +0.6)
    Filter envelope provides the attack "bite" for each note.

This patch demonstrates the Mod Matrix's power: 6 simultaneous modulation
routings that would require 6 cables and potentially extra mixer modules
in a traditional modular setup.

TRY: Use mod wheel for vibrato on sustained notes. Vary velocity for
dynamics. Use aftertouch (if your controller supports it) for filter sweeps
during held notes.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "expressive".into(),
        "mod_matrix".into(),
        "performance".into(),
    ];

    // OSC1 - Sawtooth lead (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.65)
            .build(),
    );

    // OSC2 - Square, slightly detuned for width (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("square")
            .param_f("level", 0.35)
            .param_f("detune", -7.0) // -7 cents for width
            .build(),
    );

    // Filter - Resonant lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.45)
            .build(),
    );

    // Amp Envelope - Snappy lead (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 550.0)
            .param_f("attack", 0.003)
            .param_f("decay", 0.15)
            .param_f("sustain", 0.75)
            .param_f("release", 0.25)
            .build(),
    );

    // Filter Envelope - Attack bite (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.25)
            .param_f("sustain", 0.2)
            .param_f("release", 0.2)
            .build(),
    );

    // Vibrato LFO (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(450.0, 350.0)
            .waveform("sine")
            .param_f("rate", 5.5)
            .param_f("depth", 0.0) // Depth starts at 0, mod wheel controls it
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.75)
            .build(),
    );

    // Mod Matrix - 6 active performance routings (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 350.0)
            .param_choice("grid size", "3x3")
            // Slot 1: Mod Wheel → LFO 1 Depth (vibrato amount)
            .param_choice("slot 1 source", "mod_wheel")
            .param_choice("slot 1 dest", "lfo1_depth")
            .param_f("slot 1 amount", 0.8)
            // Slot 2: LFO 1 → Osc 1 Pitch (vibrato)
            .param_choice("slot 2 source", "lfo1")
            .param_choice("slot 2 dest", "osc1_pitch")
            .param_f("slot 2 amount", 0.15)
            // Slot 3: Aftertouch → Filter Cutoff (expressive filter)
            .param_choice("slot 3 source", "aftertouch")
            .param_choice("slot 3 dest", "flt1_cutoff")
            .param_f("slot 3 amount", 0.7)
            // Slot 4: Velocity → Filter Cutoff (dynamic brightness)
            .param_choice("slot 4 source", "velocity")
            .param_choice("slot 4 dest", "flt1_cutoff")
            .param_f("slot 4 amount", 0.5)
            // Slot 5: Velocity → Amp Level (dynamic volume)
            .param_choice("slot 5 source", "velocity")
            .param_choice("slot 5 dest", "amp_level")
            .param_f("slot 5 amount", 0.3)
            // Slot 6: Env 2 → Filter Cutoff (attack bite)
            .param_choice("slot 6 source", "env2")
            .param_choice("slot 6 dest", "flt1_cutoff")
            .param_f("slot 6 amount", 0.6)
            .build(),
    );

    // Delay effect (dly-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Delay)
            .position(1600.0, 50.0)
            .delay_mode("ping_pong")
            .param_f("time", 0.3)
            .param_f("feedback", 0.35)
            .param_f("mix", 0.25)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections - minimal cabling thanks to Mod Matrix
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("osc-2", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    // No filter CV cables needed - Mod Matrix handles all modulation!

    patch.settings.octave_offset = 1;
    patch
}
