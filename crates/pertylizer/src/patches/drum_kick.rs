//! Kick Drum - Classic electronic kick with punch.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Kick Drum - Classic electronic kick with punch.
pub fn patch_drum_kick() -> Patch {
    let mut patch = Patch::new("Kick Drum");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some("Punchy electronic kick drum with pitch sweep.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
The kick uses a sine wave oscillator for its pure, subby fundamental.
Electronic kicks are characterized by a pitch sweep - the tone starts
higher and rapidly drops to the fundamental.

The oscillator frequency is modulated by a fast envelope that creates
this pitch sweep. The envelope attacks instantly and decays very quickly
(~50ms), sweeping the pitch from around 150Hz down to the base frequency.

AMPLITUDE:
A separate envelope controls volume with:
- Instant attack (1ms) for the initial transient "click"
- Short decay (~150ms) for the body of the kick
- No sustain - kicks are purely percussive
- Minimal release for tight sound

CHARACTERISTICS:
- Lower base frequencies = deeper, subby kicks
- Faster pitch decay = tighter, punchier kicks
- Slower pitch decay = more "boom", 808-style

TRY: Play single hits. Adjust the pitch envelope decay for different
kick characters. Works well in the lowest octave.
"#
        .to_string(),
    );
    patch.tags = vec![
        "drum".into(),
        "kick".into(),
        "percussion".into(),
        "808".into(),
    ];

    // OSC - Sine for pure sub (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sine")
            .param_f("level", 1.0)
            .build(),
    );

    // Pitch Envelope - Fast sweep with punchy curve (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 400.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.05)
            .param_f("sustain", 0.0)
            .param_f("release", 0.01)
            .param_f("atk_curve", -0.8) // Punchy, fast attack
            .param_f("dec_curve", -0.6) // Quick punch decay
            .param_f("rel_curve", -0.5) // Tight release
            .build(),
    );

    // Amp Envelope with punchy curves (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.15)
            .param_f("sustain", 0.0)
            .param_f("release", 0.05)
            .param_f("atk_curve", -1.0) // Instant punch
            .param_f("dec_curve", -0.7) // Fast initial drop for punch
            .param_f("rel_curve", -0.5) // Tight cutoff
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.9)
            .build(),
    );

    // Soft clip for warmth (dst-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Distortion)
            .position(1200.0, 350.0)
            .distortion_mode("soft_clip")
            .param_f("drive", 0.2)
            .param_f("mix", 0.3)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1200.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(800.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Connections (using string IDs: type-instance)
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "osc-1", "fm");
    patch.add_connection("env-2", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");
    patch.settings.octave_offset = -2;
    patch
}
