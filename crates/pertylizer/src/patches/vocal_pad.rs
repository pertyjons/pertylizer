//! Vocal Pad - Ethereal vowel pad using formant wavetable scanning.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Vocal Pad - Lush choir-like pad with formant wavetable morphing.
pub fn patch_vocal_pad() -> Patch {
    let mut patch = Patch::new("Vocal Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Ethereal vowel pad using the Formant wavetable with slow LFO position scanning and FormantFilter for vowel shaping."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (Formant bank) -> Filter (Fluid) -> Amplifier -> Reverb -> Output

The Formant wavetable contains vowel shapes (a/e/i/o/u) that morph
smoothly as the position scans through them. A slow LFO sweeps the
position, creating an evolving choir-like texture.

The Fluid filter adds warm Oberheim-style character with gentle resonance.

MODULATION:
- LFO 1 -> Wavetable Position CV (slow vowel sweep)
- Env 1 -> Amplifier (slow pad envelope)

TRY: Change LFO rate for faster/slower vowel morphing.
Increase filter resonance for more vocal character.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "wavetable".into(),
        "formant".into(),
        "vocal".into(),
        "evolving".into(),
    ];

    // Wavetable Oscillator - Formant bank (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "formant")
            .param_f("position", 0.2)
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Fluid model for warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(400.0, 50.0)
            .filter_model("fluid")
            .param_f("cutoff", 2500.0)
            .param_f("resonance", 0.3)
            .param_f("morph", 0.1)
            .param_f("drive", 1.5)
            .build(),
    );

    // Amp Envelope - Slow pad (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(800.0, 350.0)
            .param_f("attack", 0.8)
            .param_f("decay", 0.4)
            .param_f("sustain", 0.75)
            .param_f("release", 2.0)
            .build(),
    );

    // LFO - Slow position sweep (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 250.0)
            .param_choice("waveform", "triangle")
            .param_f("rate", 0.08)
            .param_f("depth", 0.6)
            .build(),
    );

    // Formant Filter - Vowel shaping (fmt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::FormantFilter)
            .position(600.0, 50.0)
            .param_f("vowel", 0.3)
            .param_f("cutoff", 1200.0)
            .param_f("resonance", 0.6)
            .param_f("mix", 0.4)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(800.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // Reverb - Large lush space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1550.0, 50.0)
            .param_f("room_size", 0.85)
            .param_f("damping", 0.3)
            .param_f("mix", 0.5)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1150.0, 50.0)
            .param_f("master", 0.7)
            .build(),
    );

    // Connections
    patch.add_connection("wtb-1", "out", "flt-1", "in");
    patch.add_connection("lfo-1", "out", "wtb-1", "pos_cv");
    patch.add_connection("flt-1", "out", "fmt-1", "in");
    patch.add_connection("fmt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("lfo-1", "out", "fmt-1", "vowel_cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
