//! Analog Dream Machine — complex MSEG envelope modulating filter with vintage effects chain.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Analog Dream Machine — MSEG-driven filter sweeps through Phaser, Flanger, BBD Delay, and Compressor.
pub fn patch_analog_dream_machine() -> Patch {
    let mut patch = Patch::new("Analog Dream Machine");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Complex MSEG envelope creates evolving filter sweeps on detuned saws, \
         processed through vintage phaser, flanger, BBD delay, and compressor."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Two detuned sawtooth oscillators create a thick analog base. The mixer
blends them and feeds a resonant lowpass filter. The MSEG (Multi-Stage
Envelope Generator) creates a complex, looping modulation pattern that
sweeps the filter cutoff in interesting shapes — far more expressive
than a simple LFO or standard ADSR.

MSEG CONFIGURATION:
- 6 segments with looping enabled
- Sustain at segment 4 (holds during key press)
- Time Scale = 1.5 (slightly slower evolution)
- The default segments create a complex rising/falling pattern

EFFECTS CHAIN (auto-routed):
1. Phaser — classic 4-stage phase shifting for spatial movement
2. Flanger — subtle flanging for metallic shimmer
3. BBD Delay — warm analog bucket-brigade delay with wow/flutter
4. Compressor — glues everything together

MODULATION:
- MSEG → Filter Cutoff (via ModMatrix) — complex envelope shapes the filter
- Envelope → Amplifier CV — standard amplitude shaping

TRY: Adjust MSEG Time Scale for faster/slower filter patterns.
Increase Phaser feedback for more dramatic sweeps.
Set BBD Delay feedback to 0.8 for dub-style echoes.
"#
        .to_string(),
    );
    patch.tags = vec![
        "ambient".into(),
        "analog".into(),
        "mseg".into(),
        "vintage".into(),
        "evolving".into(),
    ];

    // Oscillator 1 - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.7)
            .build(),
    );

    // Oscillator 2 - Sawtooth detuned (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("level", 0.6)
            .param_f("detune", 8.0)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .build(),
    );

    // MSEG - complex modulation envelope (msg-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mseg)
            .position(450.0, 400.0)
            .param_f("segments", 6.0)
            .param_f("sustain seg", 4.0)
            .param_f("loop", 1.0)
            .param_f("time scale", 1.5)
            .build(),
    );

    // Filter - resonant lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 800.0)
            .param_f("resonance", 0.5)
            .param_f("key_track", 0.6)
            .param_f("cv_amt", 4000.0)
            .build(),
    );

    // Mod Matrix - MSEG → Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(850.0, 400.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "mseg1")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.7)
            .build(),
    );

    // Amplitude envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 400.0)
            .param_f("attack", 0.8)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.75)
            .param_f("release", 2.0)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1650.0, 50.0)
            .param_f("master", 0.75)
            .build(),
    );

    // === Effects (auto-routed) ===

    // Phaser — classic sweep (phs-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Phaser)
            .position(2050.0, 50.0)
            .param_f("rate", 0.3)
            .param_f("depth", 0.6)
            .param_f("feedback", 0.65)
            .param_f("center freq", 1200.0)
            .param_f("mix", 0.4)
            .build(),
    );

    // Flanger — subtle metallic shimmer (fln-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Flanger)
            .position(2050.0, 400.0)
            .param_f("rate", 0.2)
            .param_f("depth", 0.5)
            .param_f("feedback", 0.5)
            .param_f("delay", 3.0)
            .param_f("mix", 0.3)
            .build(),
    );

    // BBD Delay — warm analog echoes (bbd-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::BbdDelay)
            .position(2450.0, 50.0)
            .param_f("time", 0.375)
            .param_f("feedback", 0.55)
            .param_f("tone", 0.4)
            .param_f("wow/flutter", 0.4)
            .param_f("clock noise", 0.15)
            .param_f("mix", 0.35)
            .build(),
    );

    // Compressor — glue compression (cmp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Compressor)
            .position(2450.0, 400.0)
            .param_f("threshold", -18.0)
            .param_f("ratio", 3.0)
            .param_f("attack", 15.0)
            .param_f("release", 150.0)
            .param_f("makeup", 4.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // === Connections ===
    // Oscillators → Mixer → Filter → Amp → Output
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("msg-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group("Oscillators", Some("#D96A4A"), &["osc-1", "osc-2", "mix-1"]);
    patch.add_group("MSEG Filter", Some("#7B68EE"), &["msg-1", "flt-1", "mmx-1"]);
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group(
        "Vintage FX",
        Some("#D4A94A"),
        &["phs-1", "fln-1", "bbd-1", "cmp-1"],
    );

    patch
}
