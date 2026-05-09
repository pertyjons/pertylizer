//! Spectral Freeze Pad — frozen spectral textures from additive harmonics, blurred and shimmering.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Spectral Freeze Pad — AdditiveOsc through PhaseVocoder freeze, SpectralBlur, ShimmerReverb, and Limiter.
pub fn patch_spectral_freeze_pad() -> Patch {
    let mut patch = Patch::new("Spectral Freeze Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Rich additive harmonics frozen in spectral time by the Phase Vocoder, \
         blurred across frequencies, and shimmered into infinite sustain."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
An Additive Oscillator generates a bright harmonic spectrum with high
brightness and some inharmonic stretch. A gentle lowpass filter shapes
the initial tone, then the amplifier applies a slow pad envelope.

EFFECTS CHAIN (auto-routed):
1. Phase Vocoder — freezes the spectral content, suspending harmonics
   in time. Slight pitch shift (+7 semitones) adds a fifth above.
2. Spectral Blur — smears the frozen spectrum across both time and
   frequency dimensions for an impossibly smooth texture
3. Shimmer Reverb — feeds the blurred output into reverb with octave-up
   pitch shifting, creating infinite ascending harmonic tails
4. Limiter — controls the accumulated energy from the dense processing

ADDITIVE OSCILLATOR:
- Brightness = 0.8 (rich upper harmonics for interesting freeze content)
- Stretch = 0.1 (subtle inharmonicity for bell-like quality)
- Odd/Even = 0.45 (slightly odd-favored for warmth)

PHASE VOCODER:
- Pitch Shift = +7 semitones (perfect fifth above for harmonic richness)
- FFT Size = 4096 (high resolution for smooth freeze)
- Freeze = off (toggle on during performance for frozen textures)

TRY: Toggle Phase Vocoder Freeze on while holding a chord.
Change pitch shift to 0 for pure freeze without transposition.
Increase Spectral Blur time to max for extreme smearing.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "spectral".into(),
        "freeze".into(),
        "ambient".into(),
        "additive".into(),
    ];

    // Additive Oscillator (add-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::AdditiveOsc)
            .position(50.0, 50.0)
            .param_f("tilt", 0.45)
            .param_f("odd/even", 0.45)
            .param_f("brightness", 0.8)
            .param_f("stretch", 0.1)
            .param_f("randomize", 0.2)
            .param_f("level", 0.75)
            .build(),
    );

    // Filter - gentle shaping (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 4000.0)
            .param_f("resonance", 0.1)
            .param_f("key_track", 0.4)
            .build(),
    );

    // Very slow pad envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 400.0)
            .param_f("attack", 2.5)
            .param_f("decay", 1.5)
            .param_f("sustain", 0.85)
            .param_f("release", 5.0)
            .param_f("atk_curve", 0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1250.0, 50.0)
            .param_f("master", 0.7)
            .build(),
    );

    // === Effects (auto-routed) ===

    // Phase Vocoder — spectral freeze with fifth shift (pvc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::PhaseVocoder)
            .position(1650.0, 50.0)
            .param_f("pitch shift", 7.0)
            .param_b("freeze", false)
            .param_choice("fft size", "4096")
            .param_f("mix", 0.6)
            .build(),
    );

    // Spectral Blur — frequency smearing (sbl-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SpectralBlur)
            .position(1650.0, 400.0)
            .param_f("fft size", 2.0)
            .param_f("blur time", 0.85)
            .param_f("blur freq", 0.5)
            .param_b("freeze", false)
            .param_f("mix", 0.65)
            .build(),
    );

    // Shimmer Reverb — octave-up infinite tails (shr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ShimmerReverb)
            .position(2050.0, 50.0)
            .param_f("room size", 0.9)
            .param_f("decay", 0.9)
            .param_f("damping", 0.25)
            .param_f("pre-delay", 0.06)
            .param_f("pitch", 12.0)
            .param_f("shimmer", 0.5)
            .param_f("mix", 0.4)
            .build(),
    );

    // Crossover Splitter — separate low/high frequency processing (cxo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::CrossoverSplitter)
            .position(2050.0, 400.0)
            .param_f("frequency", 800.0)
            .param_f("low gain", 0.8)
            .param_f("high gain", 1.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // Limiter — control accumulated energy (lmt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Limiter)
            .position(2450.0, 400.0)
            .param_f("ceiling", -0.5)
            .param_f("look-ahead", 3.0)
            .param_f("release", 120.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // === Connections ===
    // AdditiveOsc → Filter → Amp → Output
    patch.add_connection("add-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group("Additive Voice", Some("#7B68EE"), &["add-1", "flt-1"]);
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group(
        "Spectral FX",
        Some("#D4A94A"),
        &["pvc-1", "sbl-1", "shr-1", "cxo-1", "lmt-1"],
    );

    patch
}
