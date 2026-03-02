//! Sub Bass - Deep, weighty bass using the sub-oscillator.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Sub Bass - Deep bass patch showcasing the sub-oscillator module.
pub fn patch_sub_bass() -> Patch {
    let mut patch = Patch::new("Sub Bass");
    patch.author = Some("Pertylizer".to_string());
    patch.description =
        Some("Deep, weighty bass using the new sub-oscillator for massive low end.".to_string());
    patch.notes = Some(
        r"
SIGNAL FLOW:
This patch demonstrates the new SubOscillator module for adding weight to bass sounds.
The main oscillator provides harmonic content while the sub-oscillator adds pure fundamental.

MAIN OSCILLATOR (Sawtooth):
A sawtooth wave provides rich harmonics that define the character of the bass.
Level is reduced to leave headroom for the sub.

SUB-OSCILLATOR:
A pure sine wave one octave below the main oscillator adds the deep sub bass.
The sine wave is clean and doesn't add mud - just pure low-end weight.

FILTER:
A lowpass filter shapes the tone and adds some resonance for presence.
The filter envelope provides movement on each note.

ENVELOPE CURVES:
Negative curves give the bass a punchy, immediate character:
- Fast attack for instant response
- Quick initial decay for punch
- Controlled release for tight sound

TRY: Play in the lowest octave for massive sub bass. Works great for
dubstep, EDM, or any music needing powerful low end.
"
        .to_string(),
    );
    patch.tags = vec![
        "bass".into(),
        "sub".into(),
        "deep".into(),
        "edm".into(),
        "dubstep".into(),
    ];

    // Main OSC - Sawtooth for harmonics (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );

    // Sub-Oscillator - Pure sine one octave down (sub-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SubOscillator)
            .position(50.0, 400.0)
            .param_choice("waveform", "sine") // Pure fundamental
            .param_choice("octave", "minus1") // One octave down
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 300.0)
            .param_f("resonance", 0.4)
            .build(),
    );

    // Amp Envelope with punchy curves (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.003)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.7)
            .param_f("release", 0.15)
            .param_f("atk_curve", -0.7) // Fast punch
            .param_f("dec_curve", -0.3) // Quick settle
            .param_f("rel_curve", -0.4) // Controlled release
            .build(),
    );

    // Filter Envelope (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.2)
            .param_f("release", 0.2)
            .param_f("atk_curve", -0.8)
            .param_f("dec_curve", -0.4)
            .param_f("rel_curve", -0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.85)
            .build(),
    );

    // Soft clip for warmth (dst-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Distortion)
            .position(1600.0, 350.0)
            .distortion_mode("soft_clip")
            .param_f("drive", 0.15)
            .param_f("mix", 0.3)
            .build(),
    );

    // Oscilloscope (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1600.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("sub-1", "out", "flt-1", "in"); // Sub adds weight
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = -2;
    patch
}
