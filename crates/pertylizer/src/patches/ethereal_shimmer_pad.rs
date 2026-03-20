//! Ethereal Shimmer Pad — lush additive pad with Juno chorus, octave shimmer reverb, wide stereo.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Ethereal Shimmer Pad — AdditiveOsc through EnsembleChorus, ShimmerReverb, Mid/Side widening, and EQ shaping.
pub fn patch_ethereal_shimmer_pad() -> Patch {
    let mut patch = Patch::new("Ethereal Shimmer Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Lush additive harmonics through Juno-style ensemble chorus, octave shimmer reverb, \
         wide stereo field, and gentle EQ sculpting."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
An Additive Oscillator generates a rich harmonic spectrum with warm tilt
and moderate brightness. This feeds through a lowpass filter for gentle
shaping, then into an amplifier with a slow pad envelope.

EFFECTS CHAIN (auto-routed):
1. EQ — gentle low cut at 150 Hz, mid scoop at 800 Hz, high sparkle at 6 kHz
2. Ensemble Chorus — Juno-style 3-voice chorus for lush movement
3. Shimmer Reverb — large room with octave-up pitch shift for ethereal tails
4. Mid/Side — widens the stereo image for immersive spread

ADDITIVE OSCILLATOR:
- Tilt = 0.4 (warm rolloff favoring fundamentals)
- Odd/Even = 0.6 (slightly more even harmonics for warmth)
- Brightness = 0.6 (moderate high harmonics)
- Stretch = 0.05 (very subtle inharmonicity)

TRY: Increase Brightness to 0.9 for more crystalline quality.
Increase Shimmer to 0.6 for dramatic octave buildup.
Turn on LFO modulating filter cutoff for slow movement.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "shimmer".into(),
        "ethereal".into(),
        "ambient".into(),
        "additive".into(),
    ];

    // Additive Oscillator (add-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::AdditiveOsc)
            .position(50.0, 50.0)
            .param_f("tilt", 0.4)
            .param_f("odd/even", 0.6)
            .param_f("brightness", 0.6)
            .param_f("stretch", 0.05)
            .param_f("randomize", 0.15)
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - gentle lowpass shaping (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3500.0)
            .param_f("resonance", 0.15)
            .param_f("key_track", 0.5)
            .build(),
    );

    // LFO for subtle filter movement (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(450.0, 400.0)
            .waveform("sine")
            .param_f("rate", 0.15)
            .param_f("depth", 0.3)
            .build(),
    );

    // Pad envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 400.0)
            .param_f("attack", 1.5)
            .param_f("decay", 2.0)
            .param_f("sustain", 0.8)
            .param_f("release", 3.5)
            .param_f("atk_curve", 0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1250.0, 50.0)
            .param_f("master level", 0.75)
            .build(),
    );

    // === Effects (auto-routed) ===

    // EQ — sculpt the frequency spectrum (equ-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Eq)
            .position(1650.0, 50.0)
            .param_f("low freq", 150.0)
            .param_f("low gain", -3.0)
            .param_f("mid freq", 800.0)
            .param_f("mid gain", -2.0)
            .param_f("mid q", 1.5)
            .param_f("high freq", 6000.0)
            .param_f("high gain", 3.0)
            .build(),
    );

    // Ensemble Chorus — Juno-style warmth (enc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::EnsembleChorus)
            .position(1650.0, 400.0)
            .param_f("rate", 0.5)
            .param_f("depth", 1.5)
            .param_f("base delay", 12.0)
            .param_f("tone", 0.35)
            .param_f("noise", 0.08)
            .param_f("stereo width", 1.0)
            .param_f("voices", 3.0)
            .param_f("mix", 0.45)
            .build(),
    );

    // Shimmer Reverb — octave-up ethereal tails (shr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ShimmerReverb)
            .position(2050.0, 50.0)
            .param_f("room size", 0.8)
            .param_f("decay", 0.85)
            .param_f("damping", 0.3)
            .param_f("pre-delay", 0.04)
            .param_f("pitch", 12.0)
            .param_f("shimmer", 0.4)
            .param_f("mix", 0.35)
            .build(),
    );

    // Mid/Side — widen the stereo image (mds-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MidSide)
            .position(2050.0, 400.0)
            .param_f("width", 0.75)
            .param_f("mid gain", -1.0)
            .param_f("side gain", 3.0)
            .param_f("mix", 0.8)
            .build(),
    );

    // === Connections ===
    // AdditiveOsc → Filter → Amp → Output
    patch.add_connection("add-1", "out", "flt-1", "in");
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group(
        "Additive Voice",
        Some("#7B68EE"),
        &["add-1", "flt-1", "lfo-1"],
    );
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group(
        "Effects",
        Some("#D4A94A"),
        &["equ-1", "enc-1", "shr-1", "mds-1"],
    );

    patch
}
