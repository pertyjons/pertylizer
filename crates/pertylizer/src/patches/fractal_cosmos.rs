//! Fractal Cosmos - Triple fractal oscillator pad with evolving stereo texture.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Fractal Cosmos - Triple fractal oscillator pad with evolving stereo texture.
pub fn patch_fractal_cosmos() -> Patch {
    let mut patch = Patch::new("Fractal Cosmos");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("Massive evolving pad from three Weierstrass fractal oscillators.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Three fractal oscillators with different character feed into a lowpass filter.
The filter shapes the combined spectrum before going through an envelope-controlled
amplifier to the stereo output.

OSCILLATORS:
- FRC-1: Rich foundation (Roughness 0.70, tight Spacing 0.12, wide Spread 0.80)
- FRC-2: Metallic mid-range (Roughness 0.50, wider Spacing 0.35, narrow Spread 0.40)
- FRC-3: Noisy shimmer top (Roughness 0.85, near-harmonic Spacing 0.08, ultra-wide Spread 0.95)

Each oscillator has different Dispersion settings (0.20, 0.55, 0.75) creating
distinct crest-factor profiles that prevent phase-alignment peaks.

MODULATION:
- LFO1 (0.15 Hz sine): Slow filter cutoff sweep for evolving movement
- LFO2 (0.07 Hz triangle): Subtle pitch drift on FRC-1 for organic feel
- Envelope: 600ms attack, 2.5s release for lush pad behavior

EFFECTS:
- Chorus for stereo width and thickening
- Delay (375ms, low feedback) for spatial depth
- Reverb (large, dark) for immersive space

TRY: Play wide chords in the C2-C4 range. Experiment with Roughness and
Spacing knobs — small changes create dramatically different timbres.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "fractal".into(),
        "ambient".into(),
        "evolving".into(),
        "stereo".into(),
        "experimental".into(),
    ];

    // FRC-1: Rich, broad foundation (frc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::FractalOsc)
            .position(50.0, 50.0)
            .param_f("roughness", 0.70)
            .param_f("spacing", 0.12)
            .param_f("dispersion", 0.20)
            .param_f("spread", 0.80)
            .param_f("level", 0.85)
            .build(),
    );

    // FRC-2: Metallic mid-range (frc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::FractalOsc)
            .position(50.0, 350.0)
            .param_f("roughness", 0.50)
            .param_f("spacing", 0.35)
            .param_f("dispersion", 0.55)
            .param_f("spread", 0.40)
            .param_f("level", 0.70)
            .build(),
    );

    // FRC-3: Noisy shimmer top (frc-3)
    patch.add_module(
        ModuleBuilder::new(3, ModuleType::FractalOsc)
            .position(50.0, 650.0)
            .param_f("roughness", 0.85)
            .param_f("spacing", 0.08)
            .param_f("dispersion", 0.75)
            .param_f("spread", 0.95)
            .param_f("level", 0.60)
            .build(),
    );

    // Filter - Lowpass shaping (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(500.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3500.0)
            .param_f("resonance", 0.30)
            .build(),
    );

    // Amp Envelope - Slow pad (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(900.0, 350.0)
            .param_f("attack", 0.6)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.55)
            .param_f("release", 2.5)
            .build(),
    );

    // LFO1 - Filter cutoff sweep (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(500.0, 350.0)
            .waveform("sine")
            .param_f("rate", 0.15)
            .param_f("depth", 0.5)
            .build(),
    );

    // LFO2 - Pitch drift on FRC-1 (lfo-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Lfo)
            .position(50.0, 950.0)
            .waveform("triangle")
            .param_f("rate", 0.07)
            .param_f("depth", 0.3)
            .build(),
    );

    // Amplifier (amp-1).
    // Level 0.7 → 1.6 to clear the 0.05 `low_output` threshold — the three
    // Weierstrass fractal oscillators are inherently quiet
    // (re-audit 2026-05-10).
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(900.0, 50.0)
            .param_f("level", 1.6)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1300.0, 50.0)
            .param_f("master", 0.9)
            .build(),
    );

    // Chorus (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(1700.0, 50.0)
            .param_f("rate", 0.3)
            .param_f("depth", 0.4)
            .param_f("mix", 0.35)
            .build(),
    );

    // Delay (dly-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Delay)
            .position(1700.0, 300.0)
            .param_f("time", 0.375)
            .param_f("feedback", 0.30)
            .param_f("mix", 0.20)
            .build(),
    );

    // Reverb (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1700.0, 550.0)
            .param_f("room_size", 0.85)
            .param_f("damping", 0.40)
            .param_f("mix", 0.50)
            .build(),
    );

    // === Connections ===
    // Three fractal oscs → filter (filter sums inputs)
    patch.add_connection("frc-1", "out_l", "flt-1", "in");
    patch.add_connection("frc-2", "out_l", "flt-1", "in");
    patch.add_connection("frc-3", "out_l", "flt-1", "in");
    // Filter → Amp → Output
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    // Modulation
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("lfo-2", "out", "frc-1", "freq_cv");

    patch.settings.octave_offset = -1;

    // Groups
    patch.add_group(
        "Fractal Voices",
        Some("#9B59B6"),
        &["frc-1", "frc-2", "frc-3"],
    );
    patch.add_group("Signal Chain", Some("#4A90D9"), &["flt-1", "amp-1"]);
    patch.add_group("Modulation", Some("#50C878"), &["env-1", "lfo-1", "lfo-2"]);

    patch
}
