//! Deep Space Pad - Evolving atmospheric pad with multiple modulation sources.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Deep Space Pad - Evolving atmospheric pad with multiple modulation sources.
pub fn patch_deep_space_pad() -> Patch {
    let mut patch = Patch::new("Deep Space Pad");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Evolving atmospheric pad with rich modulation.".to_string());
    patch.notes = Some(
        r"
SIGNAL FLOW:
Two detuned sawtooth oscillators create a rich, full sound. OSC1 is the main
oscillator while OSC2 is slightly detuned (+7 cents) for natural chorus effect.

Both oscillators feed into a lowpass filter with moderate resonance, creating
warmth. The filter cutoff is modulated by both an envelope (for attack brightness)
and a slow LFO (for movement).

MODULATION:
- LFO1 (0.1 Hz): Slowly modulates filter cutoff for evolving texture
- LFO2 (0.08 Hz): Modulates OSC2 pitch for subtle detuning variation
- Filter Envelope: Opens filter on attack, then settles

The chorus effect adds width and dimension, while the reverb places the sound
in a large space. Long attack/release times create the pad-like quality.

TRY: Play sustained chords in the low-to-mid range. Layer with arpeggios.
"
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "ambient".into(),
        "atmospheric".into(),
        "evolving".into(),
    ];

    // OSC1 - Main sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .param_f("detune", 0.0)
            .build(),
    );

    // OSC2 - Detuned sawtooth (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 200.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .param_f("detune", 7.0)
            .build(),
    );

    // Mixer for oscillators (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(250.0, 100.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Filter - Lowpass with resonance (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 100.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.35)
            .build(),
    );

    // Amp Envelope - Slow pad envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 400.0)
            .param_f("attack", 1.5)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.7)
            .param_f("release", 3.0)
            .build(),
    );

    // Filter Envelope - Brighter on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(250.0, 400.0)
            .param_f("attack", 0.8)
            .param_f("decay", 1.5)
            .param_f("sustain", 0.3)
            .param_f("release", 2.0)
            .build(),
    );

    // LFO1 - Filter modulation (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(450.0, 400.0)
            .waveform("sine")
            .param_f("rate", 0.1)
            .param_f("depth", 0.4)
            .build(),
    );

    // LFO2 - Pitch modulation for OSC2 (lfo-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Lfo)
            .position(650.0, 400.0)
            .waveform("triangle")
            .param_f("rate", 0.08)
            .param_f("depth", 0.1)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(650.0, 100.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Chorus - For width (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(850.0, 100.0)
            .param_f("rate", 0.5)
            .param_f("depth", 0.4)
            .param_f("mix", 0.35)
            .build(),
    );

    // Reverb - Large space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1050.0, 100.0)
            .param_f("room_size", 0.85)
            .param_f("damping", 0.3)
            .param_f("mix", 0.4)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1250.0, 100.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1450.0, 100.0)
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
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("lfo-2", "out", "osc-2", "fm");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = -1;
    patch
}
