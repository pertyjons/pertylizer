//! Vintage Electric Piano — Rhodes-style piano with mechanical key sounds and vintage effects.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Vintage Electric Piano — MechanicalNoise, Convolver, EQ, BBD Delay, EnsembleChorus, Compressor.
pub fn patch_vintage_electric_piano() -> Patch {
    let mut patch = Patch::new("Vintage Electric Piano");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Detailed Rhodes-style electric piano combining sine oscillator tine sounds \
         with mechanical key noise, plate reverb, vintage chorus, BBD delay, and gentle compression."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A sine oscillator generates the pure tine-like fundamental of a Rhodes
piano. A second sine oscillator (slightly detuned, octave up) provides
the upper partial. A Mechanical Noise module adds realistic key-down
click sounds for physical authenticity. All three are mixed together.

The mix passes through a lowpass filter with velocity-sensitive envelope
modulation — brighter when played harder, darker when soft — mimicking
the real Rhodes behavior where harder tine strikes produce more harmonics.

EFFECTS CHAIN (auto-routed):
1. EQ — gentle bass boost at 200 Hz, slight mid cut, sparkle at 5 kHz
2. Ensemble Chorus — Juno-style 2-voice chorus for classic Rhodes width
3. BBD Delay — warm vintage delay for spaciousness
4. Convolver (Plate) — subtle plate reverb for studio polish
5. Compressor — gentle compression for even dynamics

MECHANICAL NOISE:
- Type = KeyDown (realistic key mechanism sound)
- Duration = 8ms (very short click)
- Cutoff = 4000 Hz (bright but not harsh)
- Level = 0.08 (subtle, barely audible layer of realism)
- Velocity Sensitivity = 0.7 (louder clicks on harder playing)

TRY: Increase MechanicalNoise level to 0.2 for more pronounced clicks.
Change EnsembleChorus voices to 3 for thicker chorus effect.
Increase BBD Delay feedback for more prominent echoes.
"#
        .to_string(),
    );
    patch.tags = vec![
        "keys".into(),
        "piano".into(),
        "rhodes".into(),
        "vintage".into(),
        "electric_piano".into(),
    ];

    // Oscillator 1 - Sine tine fundamental (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sine")
            .param_f("level", 0.8)
            .build(),
    );

    // Oscillator 2 - Upper partial (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sine")
            .param_f("level", 0.3)
            .param_f("detune", 3.0)
            .param_f("octave", 1.0)
            .build(),
    );

    // Mechanical Noise - key click (mec-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MechanicalNoise)
            .position(250.0, 400.0)
            .param_choice("type", "key_down")
            .param_f("duration", 8.0)
            .param_f("cutoff", 4000.0)
            .param_f("vel sens", 0.7)
            .param_f("level", 0.08)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .build(),
    );

    // Filter - velocity-sensitive brightness (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2500.0)
            .param_f("resonance", 0.1)
            .param_f("key_track", 0.6)
            .param_f("cv_amt", 3000.0)
            .build(),
    );

    // Filter envelope - velocity-sensitive brightness (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(850.0, 400.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.6)
            .param_f("sustain", 0.2)
            .param_f("release", 0.3)
            .param_f("vel_sens", 0.8)
            .param_f("dec_curve", -0.5)
            .build(),
    );

    // Amplitude envelope - piano-like decay (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 400.0)
            .param_f("attack", 0.002)
            .param_f("decay", 1.5)
            .param_f("sustain", 0.3)
            .param_f("release", 0.5)
            .param_f("vel_sens", 0.6)
            .param_f("dec_curve", -0.3)
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

    // EQ — Rhodes-style frequency shaping (equ-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Eq)
            .position(2050.0, 50.0)
            .param_f("low freq", 200.0)
            .param_f("low gain", 2.0)
            .param_f("mid freq", 1000.0)
            .param_f("mid gain", -1.5)
            .param_f("mid q", 1.2)
            .param_f("high freq", 5000.0)
            .param_f("high gain", 2.5)
            .build(),
    );

    // Ensemble Chorus — classic Rhodes chorus (enc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::EnsembleChorus)
            .position(2050.0, 400.0)
            .param_f("rate", 0.7)
            .param_f("depth", 0.8)
            .param_f("base delay", 8.0)
            .param_f("tone", 0.5)
            .param_f("noise", 0.05)
            .param_f("stereo width", 0.8)
            .param_f("voices", 2.0)
            .param_f("mix", 0.35)
            .build(),
    );

    // BBD Delay — warm vintage echo (bbd-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::BbdDelay)
            .position(2450.0, 50.0)
            .param_f("time", 0.35)
            .param_f("feedback", 0.3)
            .param_f("tone", 0.45)
            .param_f("wow/flutter", 0.2)
            .param_f("clock noise", 0.1)
            .param_f("mix", 0.2)
            .build(),
    );

    // Convolver — subtle plate reverb (cnv-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Convolver)
            .position(2450.0, 400.0)
            .param_choice("ir type", "plate")
            .param_f("mix", 0.2)
            .param_f("pre-delay", 10.0)
            .param_f("decay", 0.7)
            .param_f("brightness", 0.7)
            .build(),
    );

    // Compressor — gentle leveling (cmp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Compressor)
            .position(2850.0, 50.0)
            .param_f("threshold", -12.0)
            .param_f("ratio", 2.5)
            .param_f("attack", 12.0)
            .param_f("release", 200.0)
            .param_f("makeup", 2.0)
            .param_f("mix", 1.0)
            .build(),
    );

    // === Connections ===
    // Oscillators + MechanicalNoise → Mixer → Filter → Amp → Output
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    patch.add_connection("mec-1", "out", "mix-1", "in3");
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group(
        "Tine Voice",
        Some("#D4A94A"),
        &["osc-1", "osc-2", "mec-1", "mix-1"],
    );
    patch.add_group("Filter", Some("#7B68EE"), &["flt-1", "env-2"]);
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "env-1", "out-1"]);
    patch.add_group(
        "Vintage FX",
        Some("#D96A4A"),
        &["equ-1", "enc-1", "bbd-1", "cnv-1", "cmp-1"],
    );

    patch
}
