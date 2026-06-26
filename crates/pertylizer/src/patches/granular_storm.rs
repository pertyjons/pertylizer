//! Granular Storm — dense aggressive granular clouds with phaser sweep, wide stereo, compressed.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Granular Storm — GranularOsc with GranularFx, Phaser, Mid/Side, and Compressor.
pub fn patch_granular_storm() -> Patch {
    let mut patch = Patch::new("Granular Storm");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Dense, aggressive granular clouds with rapid grain scatter, \
         phaser sweeps, wide stereo spread, and heavy compression."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A Granular Oscillator generates chaotic, dense grain clouds using a
square wave source for harsh digital texture. High density and wide
pitch/pan spread create a wall of granular noise. A sawtooth LFO
sweeps the bandpass filter cutoff for constant movement.

EFFECTS CHAIN (auto-routed):
1. Granular FX — re-granulates the output for extreme density
2. Phaser — sweeping phase cancellation adds motion and depth
3. Mid/Side — wide stereo field with boosted sides
4. Compressor — heavy compression glues the chaotic grains together

GRANULAR OSCILLATOR:
- Source = Square (harsh digital character)
- Grain Size = 25ms (short, choppy grains)
- Density = 0.9 (very dense cloud)
- Pitch Spread = 0.4 (wide pitch variation for dissonance)
- Pan Spread = 0.8 (extreme stereo scatter)
- Position Spread = 0.6 (varied positions)

COMPRESSOR:
- Threshold = -24 dB, Ratio = 8:1 (heavy limiting compression)
- Fast attack/medium release for aggressive control
- 6 dB makeup gain to restore volume

TRY: Reduce density for sparser, more rhythmic texture.
Enable Freeze on GranularOsc for static frozen clouds.
Increase Phaser feedback to 0.9 for extreme sweeps.
"#
        .to_string(),
    );
    patch.tags = vec![
        "ambient".into(),
        "granular".into(),
        "aggressive".into(),
        "texture".into(),
        "experimental".into(),
    ];

    // Granular Oscillator - dense chaotic clouds (grn-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::GranularOsc)
            .position(50.0, 50.0)
            .param_f("grain size", 25.0)
            .param_f("density", 0.9)
            .param_f("position", 0.3)
            .param_f("pos spread", 0.6)
            .param_f("pitch spread", 0.4)
            .param_f("pan spread", 0.8)
            .param_b("freeze", false)
            .param_choice("window", "trapezoid")
            .param_choice("source", "square")
            .param_f("level", 0.75)
            .build(),
    );

    // LFO - position modulation (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("rate", 0.25)
            .param_f("depth", 0.6)
            .build(),
    );

    // Filter - slight shaping (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("bandpass")
            .param_f("cutoff", 2000.0)
            .param_f("resonance", 0.35)
            .build(),
    );

    // Aggressive envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 400.0)
            .param_f("attack", 0.5)
            .param_f("decay", 0.8)
            .param_f("sustain", 0.7)
            .param_f("release", 1.5)
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
            .param_f("master", 0.7)
            .build(),
    );

    // === Effects (auto-routed) ===

    // Granular FX — re-granulation for extreme density (gfx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::GranularFx)
            .position(1650.0, 50.0)
            .param_f("buffer", 1.5)
            .param_f("grain size", 30.0)
            .param_f("density", 0.8)
            .param_f("position", 0.5)
            .param_f("pos spread", 0.5)
            .param_f("pitch spread", 0.3)
            .param_f("pan spread", 0.7)
            .param_b("freeze", false)
            .param_f("mix", 0.6)
            .build(),
    );

    // Phaser — sweeping phase cancellation (phs-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Phaser)
            .position(1650.0, 400.0)
            .param_f("rate", 0.4)
            .param_f("depth", 0.8)
            .param_f("feedback", 0.75)
            .param_f("center freq", 1500.0)
            .param_f("mix", 0.45)
            .build(),
    );

    // Mid/Side — wide stereo without phase inversion (mds-1).
    // mid gain -2 dB + side gain +4 dB combined with the granular pan
    // spread was producing `stereo_correlation` ≈ -0.48 (more energy in
    // side than mid → mono summing partially cancels). Brought levels back
    // toward unity so the patch is wide AND mono-compatible
    // (re-audit 2026-05-10).
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MidSide)
            .position(2050.0, 50.0)
            .param_f("width", 0.7)
            .param_f("mid gain", 0.0)
            .param_f("side gain", 1.5)
            .param_f("mix", 0.8)
            .build(),
    );

    // Compressor — heavy glue compression (cmp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Compressor)
            .position(2050.0, 400.0)
            .param_f("threshold", -24.0)
            .param_f("ratio", 8.0)
            .param_f("attack", 5.0)
            .param_f("release", 80.0)
            .param_f("makeup", 6.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // === Connections ===
    // GranularOsc → Filter → Amp → Output
    patch.add_connection("grn-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");

    // LFO → Filter cutoff — sweeps the bandpass for constant motion
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");

    // Groups
    patch.add_group(
        "Granular Voice",
        Some("#D96A4A"),
        &["grn-1", "flt-1", "lfo-1"],
    );
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group(
        "Storm FX",
        Some("#8B6BAE"),
        &["gfx-1", "phs-1", "mds-1", "cmp-1"],
    );

    patch
}
