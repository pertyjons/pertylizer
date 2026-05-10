//! Granular Cathedral — enormous frozen grain textures through cathedral convolution with spectral smearing.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Granular Cathedral — GranularOsc through Convolver, SpectralBlur, GranularFx, and Limiter.
pub fn patch_granular_cathedral() -> Patch {
    let mut patch = Patch::new("Granular Cathedral");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Enormous frozen grain textures through cathedral convolution reverb \
         with spectral smearing and granular re-processing."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A Granular Oscillator generates dense clouds of grains from a sawtooth
source waveform. An LFO slowly sweeps the filter cutoff, creating
evolving timbral movement. The grains pass through a lowpass filter
for warmth, then through the amp with a very slow pad envelope.

EFFECTS CHAIN (auto-routed):
1. Spectral Blur — smears the frequency spectrum for dreamlike quality
2. Granular FX — re-granulates the audio for further texture
3. Convolver (Hall) — massive cathedral impulse response with dynamic convolution
4. Limiter — tames the dense layered output

GRANULAR OSCILLATOR:
- Source = Saw (rich harmonics for interesting grains)
- Grain Size = 80ms (medium grains for smooth texture)
- Density = 0.7 (dense cloud)
- Position Spread = 0.3 (varied grain positions)
- Pitch Spread = 0.15 (subtle pitch variation)
- Pan Spread = 0.5 (wide stereo scatter)
- Window = Hann (smooth grain envelope)

TRY: Enable Freeze on GranularOsc for frozen textures.
Increase Spectral Blur time for even more smeared sound.
Change Convolver IR type to Spring for metallic quality.
"#
        .to_string(),
    );
    patch.tags = vec![
        "ambient".into(),
        "granular".into(),
        "cathedral".into(),
        "texture".into(),
        "drone".into(),
    ];

    // Granular Oscillator (grn-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::GranularOsc)
            .position(50.0, 50.0)
            .param_f("grain size", 80.0)
            .param_f("density", 0.7)
            .param_f("position", 0.2)
            .param_f("pos spread", 0.3)
            .param_f("pitch spread", 0.15)
            .param_f("pan spread", 0.5)
            .param_b("freeze", false)
            .param_choice("window", "hann")
            .param_choice("source", "saw")
            .param_f("level", 0.8)
            .build(),
    );

    // LFO - slow position sweep (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 400.0)
            .waveform("triangle")
            .param_f("rate", 0.08)
            .param_f("depth", 0.5)
            .build(),
    );

    // Filter - warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2500.0)
            .param_f("resonance", 0.2)
            .build(),
    );

    // Pad envelope - very slow (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 400.0)
            .param_f("attack", 3.0)
            .param_f("decay", 2.0)
            .param_f("sustain", 0.9)
            .param_f("release", 5.0)
            .param_f("atk_curve", 0.4)
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

    // Spectral Blur — dreamlike smearing (sbl-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SpectralBlur)
            .position(1650.0, 50.0)
            .param_f("fft size", 2.0)
            .param_f("blur time", 0.8)
            .param_f("blur freq", 0.4)
            .param_b("freeze", false)
            .param_f("mix", 0.7)
            .build(),
    );

    // Granular FX — re-granulation (gfx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::GranularFx)
            .position(1650.0, 400.0)
            .param_f("buffer", 3.0)
            .param_f("grain size", 60.0)
            .param_f("density", 0.5)
            .param_f("position", 0.5)
            .param_f("pos spread", 0.4)
            .param_f("pitch spread", 0.1)
            .param_f("pan spread", 0.6)
            .param_b("freeze", false)
            .param_f("mix", 0.5)
            .build(),
    );

    // Convolver — cathedral hall with dynamic convolution (cnv-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Convolver)
            .position(2050.0, 50.0)
            .param_choice("ir type", "hall")
            .param_f("mix", 0.45)
            .param_f("pre-delay", 30.0)
            .param_f("decay", 0.9)
            .param_f("brightness", 0.6)
            .param_f("dynamic", 0.6)
            .build(),
    );

    // Limiter — tame the output (lmt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Limiter)
            .position(2050.0, 400.0)
            .param_f("ceiling", -1.0)
            .param_f("look-ahead", 3.0)
            .param_f("release", 150.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // === Connections ===
    // GranularOsc → Filter → Amp → Output
    patch.add_connection("grn-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // LFO → Filter cutoff for slow timbral sweep
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");

    // Groups
    patch.add_group(
        "Granular Voice",
        Some("#6B8E8E"),
        &["grn-1", "flt-1", "lfo-1"],
    );
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group(
        "Effects",
        Some("#8B6BAE"),
        &["sbl-1", "gfx-1", "cnv-1", "lmt-1"],
    );

    patch
}
