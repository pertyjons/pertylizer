//! Ring Mod Drone - Evolving metallic drone with LFO-modulated ring mod.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Ring Mod Drone - Deep evolving drone with slowly shifting ring modulation.
pub fn patch_ring_mod_drone() -> Patch {
    let mut patch = Patch::new("Ring Mod Drone");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Deep evolving drone where an LFO modulates the ring modulator carrier frequency."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Sawtooth Osc -> Ring Mod (triangle carrier, LFO on freq CV) -> Filter -> Amp -> Reverb -> Output

A rich sawtooth feeds the Ring Modulator whose carrier frequency is
slowly swept by an LFO. This creates continuously shifting sidebands —
metallic textures that evolve unpredictably over time.

The triangle carrier waveform produces a softer ring modulation than
a sine, with additional harmonic content. Low keyboard tracking means
the effect varies across the keyboard.

A lowpass filter tames the harshest frequencies and deep reverb
creates an immersive dronescape.

MODULATION:
- LFO 1 -> Ring Mod Freq CV (shifting sidebands)
- Env 1 -> Amplifier (slow drone envelope)

TRY: Hold a single low note and listen to the evolving textures.
Play clusters of notes for dense, shimmering drones.
"#
        .to_string(),
    );
    patch.tags = vec![
        "drone".into(),
        "ring_mod".into(),
        "ambient".into(),
        "evolving".into(),
        "metallic".into(),
    ];

    // Oscillator - Rich sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );

    // Ring Mod - Triangle carrier, no keyboard tracking (rng-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::RingMod)
            .position(450.0, 50.0)
            .param_choice("carrier_wave", "triangle")
            .param_f("carrier_freq", 180.0)
            .param_f("freq_ratio", 0.4)
            .param_f("key_track", 0.2)
            .param_f("mix", 0.55)
            .build(),
    );

    // Filter - Lowpass to tame highs (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(800.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1800.0)
            .param_f("resonance", 0.25)
            .build(),
    );

    // Amp Envelope - Very slow drone (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1200.0, 350.0)
            .param_f("attack", 3.0)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.8)
            .param_f("release", 5.0)
            .build(),
    );

    // LFO - Slow carrier frequency modulation (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(450.0, 250.0)
            .param_choice("waveform", "sine")
            .param_f("rate", 0.06)
            .param_f("depth", 0.4)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1200.0, 50.0)
            .param_f("level", 1.8)
            .build(),
    );

    // Reverb - Deep space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1950.0, 50.0)
            .param_f("room_size", 0.92)
            .param_f("damping", 0.3)
            .param_f("mix", 0.55)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1550.0, 50.0)
            .param_f("master", 0.9)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "rng-1", "in");
    patch.add_connection("lfo-1", "out", "rng-1", "freq_cv");
    patch.add_connection("rng-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch.settings.octave_offset = -1;
    patch
}
