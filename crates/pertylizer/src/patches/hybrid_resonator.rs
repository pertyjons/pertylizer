//! Hybrid Resonator - Layered hybrid voice with ring-modulated wavetable and body resonance.

use crate::patch::{ModuleBuilder, Patch, PatchAuthor};
use synth_core::ModuleType;

/// Hybrid Resonator - Spiky, resonant hybrid with motion in the mids.
pub fn patch_hybrid_resonator() -> Patch {
    let mut patch = Patch::new("Hybrid Resonator");
    patch.author = Some(PatchAuthor::from("Pertylizer"));
    patch.description = Some(
        "Layered hybrid voice with ring-modulated wavetable, bitwise math edge, and resonant body tone."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Osc (Unison Saw) + Sub + Math (Bitwise) + Noise + (Wavetable -> Ring Mod)
    -> Mixer -> Filter -> Body Resonance -> Amp -> Keyboard Panner -> Output

The wavetable is ring-modulated for inharmonic shimmer, while a bitwise
math oscillator adds brittle digital edge. Everything is glued by a lowpass
filter and then "re-boxed" through a body resonance model for a woody midrange.

MODULATION:
- Env 1 -> Amp CV (articulation)
- Env 2 -> Filter Resonance + Mod Mixer
- LFO -> Wavetable Position + Mod Mixer
- Mod Mixer -> Filter Cutoff (blend of envelope + LFO motion)

TRY: Increase ring mod mix for more metallic bite, or lower filter cutoff
for a darker, more percussive tone.
"#
        .to_string(),
    );
    patch.tags = vec![
        "hybrid".into(),
        "ring_mod".into(),
        "bitwise".into(),
        "resonator".into(),
        "textured".into(),
        "experimental".into(),
    ];

    // Oscillator - Unison saw (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 250.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .param_f("unison", 4.0)
            .param_f("uni detune", 18.0)
            .param_f("uni spread", 0.7)
            .param_f("uni phase", 1.0)
            .build(),
    );

    // Wavetable Oscillator - Basic table (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "basic")
            .param_f("position", 0.25)
            .param_f("level", 0.7)
            .build(),
    );

    // Math Oscillator - Bitwise edge (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MathOscillator)
            .position(50.0, 350.0)
            .algorithm("bitwise")
            .param_f("param_a", 0.5)
            .param_f("param_b", 0.5)
            .param_f("param_c", 0.5)
            .param_f("level", 0.6)
            .build(),
    );

    // Sub Oscillator - Square, -1 octave (sub-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SubOscillator)
            .position(50.0, 1050.0)
            .waveform("square")
            .param_f("octave", -1.0)
            .param_f("level", 0.4)
            .build(),
    );

    // Noise - White, subtle (nse-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .position(50.0, 600.0)
            .param_choice("type", "white")
            .param_f("level", 0.2)
            .build(),
    );

    // Ring Mod - Wavetable shimmer (rng-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::RingMod)
            .position(450.0, 50.0)
            .param_choice("carrier_wave", "sine")
            .param_f("carrier_freq", 220.0)
            .param_f("freq_ratio", 1.0)
            .param_f("key_track", 0.5)
            .param_f("mix", 0.45)
            .build(),
    );

    // Mixer - Blend sources (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(800.0, 50.0)
            .param_f("master", 0.9)
            .param_f("input_1", 0.8)
            .param_f("input_2", 0.5)
            .param_f("input_3", 0.6)
            .param_f("input_4", 0.7)
            .param_f("input_5", 0.2)
            .build(),
    );

    // Filter - Lowpass, mild drive (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(1200.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1200.0)
            .param_f("resonance", 0.35)
            .param_f("drive", 1.2)
            .build(),
    );

    // Body Resonance - Adds woody midrange (bdy-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::BodyResonance)
            .position(1600.0, 50.0)
            .param_f("freq", 280.0)
            .param_f("resonance", 0.7)
            .param_f("size", 0.6)
            .param_f("bright", 0.65)
            .param_f("mix", 0.35)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(2000.0, 50.0)
            .param_f("level", 0.9)
            .build(),
    );

    // Keyboard Panner - Note-based stereo spread (kbp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::KeyboardPanner)
            .position(2350.0, 50.0)
            .param_f("spread", 0.7)
            .param_f("center", 60.0)
            .param_f("curve", 0.4)
            .param_f("invert", 0.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(2650.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(2000.0, 350.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.25)
            .param_f("sustain", 0.6)
            .param_f("release", 0.4)
            .build(),
    );

    // Filter/Resonance Envelope (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(50.0, 1950.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.18)
            .param_f("sustain", 0.2)
            .param_f("release", 0.25)
            .build(),
    );

    // LFO - Wavetable drift + cutoff motion (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 1750.0)
            .waveform("triangle")
            .param_f("rate", 4.0)
            .param_f("depth", 0.35)
            .build(),
    );

    // Mod Mixer - Blend env + LFO for cutoff (mix-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Mixer)
            .position(50.0, 1350.0)
            .param_f("input_1", 0.8)
            .param_f("input_2", 0.35)
            .build(),
    );

    // Connections - sources
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("sub-1", "out", "mix-1", "in2");
    patch.add_connection("mth-1", "out", "mix-1", "in3");
    patch.add_connection("rng-1", "out", "mix-1", "in4");
    patch.add_connection("nse-1", "out", "mix-1", "in5");
    patch.add_connection("wtb-1", "out", "rng-1", "in");

    // Connections - main chain
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "bdy-1", "in");
    patch.add_connection("bdy-1", "out", "amp-1", "in");
    patch.add_connection("amp-1", "out", "kbp-1", "in");
    patch.add_connection("kbp-1", "out_l", "out-1", "in_l");
    patch.add_connection("kbp-1", "out_r", "out-1", "in_r");

    // Connections - modulation
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "mix-2", "in1");
    patch.add_connection("lfo-1", "out", "mix-2", "in2");
    patch.add_connection("mix-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("env-2", "out", "flt-1", "res_cv");
    patch.add_connection("lfo-1", "out", "wtb-1", "pos_cv");

    patch
}
