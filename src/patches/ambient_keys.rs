//! Ambient Keys - Soft, dreamy electric piano with shimmer.

use crate::patch::{Patch, ModuleBuilder, ModuleType};

/// Ambient Keys - Soft, dreamy electric piano with shimmer.
pub fn patch_ambient_keys() -> Patch {
    let mut patch = Patch::new("Ambient Keys");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Soft, dreamy electric piano with shimmer and reverb.".to_string());
    patch.notes = Some(r#"
SIGNAL FLOW:
A triangle wave provides a soft, pure fundamental - similar to a tine-based
electric piano like a Rhodes. The triangle has fewer harmonics than a saw
or square, giving a gentler character.

The filter is set relatively open with low resonance, just gently rolling
off the highest frequencies for warmth without dulling the sound.

DYNAMICS:
The amp envelope mimics a piano: instant attack, moderate decay to a
lower sustain level (simulating the tine's natural decay), and medium
release for notes that fade naturally.

The filter envelope adds subtle brightness on attack - that characteristic
"bell" of an electric piano - then settles to a warmer sustained tone.

EFFECTS CHAIN:
1. Chorus adds subtle width and movement
2. Heavy reverb creates the ambient, dreamy quality
3. Together they create a lush, spacious sound

TRY: Play soft chords, let them ring and overlap. Works beautifully for
ambient music, ballads, or as a bed under other instruments.
"#.to_string());
    patch.tags = vec!["keys".into(), "ambient".into(), "electric_piano".into(), "dreamy".into()];

    // OSC - Triangle for soft tone (osc-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Oscillator)
        .position(50.0, 50.0)
        .waveform("triangle")
        .param_f("level", 0.8)
        .build());

    // Filter (flt-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Filter)
        .position(250.0, 50.0)
        .filter_mode("lowpass")
        .param_f("cutoff", 3000.0)
        .param_f("resonance", 0.15)
        .build());

    // Amp Envelope (env-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Envelope)
        .position(50.0, 300.0)
        .param_f("attack", 0.003)
        .param_f("decay", 0.8)
        .param_f("sustain", 0.4)
        .param_f("release", 0.6)
        .build());

    // Filter Envelope (env-2)
    patch.add_module(ModuleBuilder::new(2, ModuleType::Envelope)
        .position(250.0, 300.0)
        .param_f("attack", 0.001)
        .param_f("decay", 0.4)
        .param_f("sustain", 0.2)
        .param_f("release", 0.3)
        .build());

    // Subtle tremolo LFO (lfo-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Lfo)
        .position(450.0, 300.0)
        .waveform("sine")
        .param_f("rate", 4.0)
        .param_f("depth", 0.06)
        .build());

    // Amplifier (amp-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Amplifier)
        .position(450.0, 50.0)
        .param_f("level", 0.65)
        .build());

    // Chorus (chr-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Chorus)
        .position(650.0, 50.0)
        .param_f("rate", 0.7)
        .param_f("depth", 0.3)
        .param_f("mix", 0.3)
        .build());

    // Reverb - Large and lush (rev-1)
    patch.add_module(ModuleBuilder::new(1, ModuleType::Reverb)
        .position(850.0, 50.0)
        .param_f("room_size", 0.9)
        .param_f("damping", 0.25)
        .param_f("mix", 0.5)
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
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("lfo-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out", "chr-1", "in_l");
    patch.add_connection("chr-1", "out_l", "rev-1", "in_l");
    patch.add_connection("chr-1", "out_r", "rev-1", "in_r");
    // Route to oscilloscope and output
    patch.add_connection("rev-1", "out_l", "scp-1", "in_l");
    patch.add_connection("rev-1", "out_r", "scp-1", "in_r");
    patch.add_connection("scp-1", "out_l", "out-1", "in_l");
    patch.add_connection("scp-1", "out_r", "out-1", "in_r");

    patch
}
