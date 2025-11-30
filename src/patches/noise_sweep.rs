//! Noise Sweep - Filter sweep effect with modulated noise.

use crate::patch::{Patch, ModuleBuilder, ModuleType};

/// Noise Sweep - Filter sweep effect with modulated noise.
pub fn patch_noise_sweep() -> Patch {
    let mut patch = Patch::new("Noise Sweep");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Dramatic filter sweep effect with noise and resonance.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
This patch creates dramatic sweep/riser effects using noise through
a resonant filter, perfect for transitions and buildups.

THE TECHNIQUE:
White noise contains all frequencies equally. When passed through a
resonant bandpass filter, only a narrow band of frequencies passes
through, with the resonance adding a pitched "whistle" at the cutoff.

By modulating the filter cutoff with an LFO, we create sweeping
motion - the effect sounds like it's rising or falling in pitch.

RESONANCE:
High resonance (0.7) causes the filter to "ring" at its cutoff
frequency, creating a clear pitched element within the noise.
This is key to the dramatic sweep effect.

MODULATION:
The LFO slowly sweeps the filter cutoff up and down. At very slow
rates (0.15 Hz), this creates long, dramatic sweeps. Faster rates
create more rhythmic, pulsing effects.

DISTORTION:
Adds edge and presence, helping the sweep cut through a mix.

TRY: Hold a note and listen to the sweep. Adjust LFO rate for
different effect speeds. Great for transitions, buildups, and risers.
"#.to_string());
    patch.tags = vec!["effect".into(), "sweep".into(), "noise".into(), "riser".into(), "experimental".into()];

    // Noise Generator - Pink for natural sweep (nse-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Noise)
        .position(50.0, 50.0)
        .param_choice("type", "pink")  // Pink noise for more natural sweep
        .param_f("level", 0.9)
        .build());

    // Filter - Bandpass with high resonance (flt-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("bandpass")
        .param_f("cutoff", 1500.0)
        .param_f("resonance", 0.7)
        .build());

    // Sweep LFO (lfo-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Lfo)
        .position(50.0, 300.0)
        .waveform("triangle")
        .param_f("rate", 0.15)
        .param_f("depth", 0.8)
        .build());

    // Amp Envelope (env-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.5)
        .param_f("decay", 0.2)
        .param_f("sustain", 0.8)
        .param_f("release", 1.0)
        .build());

    // Amplifier (amp-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.6)
        .build());

    // Distortion (dst-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Distortion)
        .position(650.0, 50.0)
        .distortion_mode("soft_clip")
        .param_f("drive", 0.4)
        .param_f("mix", 0.4)
        .build());

    // Reverb (rev-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Reverb)
        .position(850.0, 50.0)
        .param_f("room_size", 0.7)
        .param_f("damping", 0.4)
        .param_f("mix", 0.35)
        .build());

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscilloscope)
        .position(1050.0, 50.0)
        .param_f("time", 1.0)
        .param_f("gain", 1.0)
        .build());

    // Stereo Output - Final destination (out-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::StereoOutput)
        .position(1250.0, 50.0)
        .param_f("master", 0.8)
        .build());

    // Connections (using string IDs: type-instance)
    patch.add_connection("nse-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out", "dst-1", "in");
    patch.add_connection("dst-1", "out", "rev-1", "in_l");
    // Route to oscilloscope and output
    patch.add_connection("rev-1", "out_l", "scp-1", "in_l");
    patch.add_connection("scp-1", "out_l", "out-1", "in_l");

    patch
}
