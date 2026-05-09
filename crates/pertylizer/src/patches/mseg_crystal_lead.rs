//! MSEG Crystal Lead — sharp crystalline lead with complex MSEG shaping additive brightness.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// MSEG Crystal Lead — AdditiveOsc + standard Osc with MSEG modulation, Flanger, BBD Delay, Compressor.
pub fn patch_mseg_crystal_lead() -> Patch {
    let mut patch = Patch::new("MSEG Crystal Lead");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Sharp crystalline lead combining additive and standard oscillators, \
         with complex MSEG envelope shaping filter movement, flanged and delayed."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
An Additive Oscillator provides bright, crystalline harmonics while a
standard sawtooth oscillator adds analog warmth. Both are mixed and
fed through a resonant lowpass filter. The MSEG creates a complex,
looping modulation pattern that shapes the filter cutoff — much more
interesting than a simple LFO.

EFFECTS CHAIN (auto-routed):
1. Flanger — jet-like sweeping for metallic shimmer
2. BBD Delay — warm analog echoes with character
3. Compressor — even dynamics for consistent lead presence

ADDITIVE OSCILLATOR:
- Brightness = 0.9 (very bright, crystal-like)
- Stretch = 0.15 (inharmonic bell quality)
- Odd/Even = 0.3 (odd-harmonic emphasis, hollow character)

MSEG:
- 5 segments, no sustain hold (runs through completely)
- Loop enabled for continuous filter animation
- Fast time scale for rhythmic filter effects

TRY: Adjust MSEG time scale for faster/slower filter patterns.
Increase Flanger feedback for more dramatic jet sweeps.
Set BBD Delay time to match tempo for rhythmic echoes.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "crystal".into(),
        "mseg".into(),
        "additive".into(),
        "bright".into(),
    ];

    // Additive Oscillator - bright crystal harmonics (add-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::AdditiveOsc)
            .position(50.0, 50.0)
            .param_f("tilt", 0.55)
            .param_f("odd/even", 0.3)
            .param_f("brightness", 0.9)
            .param_f("stretch", 0.15)
            .param_f("randomize", 0.05)
            .param_f("level", 0.6)
            .build(),
    );

    // Standard Oscillator - analog warmth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .param_f("detune", 5.0)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .build(),
    );

    // MSEG - complex filter modulation (msg-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mseg)
            .position(450.0, 400.0)
            .param_f("segments", 5.0)
            .param_f("sustain seg", 3.0)
            .param_f("loop", 1.0)
            .param_f("time scale", 0.8)
            .build(),
    );

    // Filter - resonant sweep target (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1200.0)
            .param_f("resonance", 0.45)
            .param_f("key_track", 0.7)
            .param_f("cv_amt", 5000.0)
            .build(),
    );

    // Amplitude envelope - snappy lead (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 400.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.4)
            .param_f("sustain", 0.6)
            .param_f("release", 0.3)
            .param_f("vel_sens", 0.6)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.75)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1650.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // === Effects (auto-routed) ===

    // Flanger — jet-like metallic shimmer (fln-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Flanger)
            .position(2050.0, 50.0)
            .param_f("rate", 0.35)
            .param_f("depth", 0.6)
            .param_f("feedback", 0.6)
            .param_f("delay", 2.5)
            .param_f("mix", 0.35)
            .build(),
    );

    // BBD Delay — warm analog echoes (bbd-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::BbdDelay)
            .position(2050.0, 400.0)
            .param_f("time", 0.3)
            .param_f("feedback", 0.45)
            .param_f("tone", 0.5)
            .param_f("wow/flutter", 0.25)
            .param_f("clock noise", 0.1)
            .param_f("mix", 0.3)
            .build(),
    );

    // Compressor — consistent lead dynamics (cmp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Compressor)
            .position(2450.0, 50.0)
            .param_f("threshold", -15.0)
            .param_f("ratio", 4.0)
            .param_f("attack", 8.0)
            .param_f("release", 120.0)
            .param_f("makeup", 3.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // === Connections ===
    // Both oscillators → Mixer → Filter → Amp → Output
    patch.add_connection("add-1", "out", "mix-1", "in1");
    patch.add_connection("osc-1", "out", "mix-1", "in2");
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("msg-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group(
        "Crystal Voice",
        Some("#7B68EE"),
        &["add-1", "osc-1", "mix-1"],
    );
    patch.add_group("MSEG Filter", Some("#D96A4A"), &["msg-1", "flt-1"]);
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group("Lead FX", Some("#D4A94A"), &["fln-1", "bbd-1", "cmp-1"]);

    patch
}
