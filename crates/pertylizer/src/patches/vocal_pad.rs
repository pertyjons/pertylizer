//! Vocal Pad - Ethereal vowel pad using a warm wavetable shaped by a FormantFilter.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Vocal Pad - Lush choir-like pad with a warm source shaped by a FormantFilter.
pub fn patch_vocal_pad() -> Patch {
    let mut patch = Patch::new("Vocal Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Ethereal vowel pad using the Warm wavetable as a harmonically-rich source, shaped by a FormantFilter with slow LFO vowel sweeping."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (Warm bank) -> Filter (Fluid) -> FormantFilter -> Amplifier -> Reverb -> Output

The Warm wavetable provides a rich harmonic source with a strong
fundamental. The FormantFilter shapes it into vowel-like resonances
(a/e/i/o/u), with a slow LFO sweeping the vowel position to create
an evolving choir-like texture.

The Fluid filter adds warm Oberheim-style character with gentle resonance.

MODULATION:
- LFO 1 -> FormantFilter Vowel CV (slow vowel sweep)
- LFO 1 -> Wavetable Position CV (subtle timbral motion)
- Env 1 -> Amplifier (slow pad envelope)

TRY: Change LFO rate for faster/slower vowel morphing.
Increase FormantFilter resonance for more vocal character.
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

    // Wavetable Oscillator - Warm bank (wtb-1)
    // Note: the "Formant" wavetable bank bakes in formant emphasis around 800/1200 Hz
    // relative to a 130 Hz reference, which suppresses the fundamental at C5 and above.
    // Use the "Warm" bank as the source and let the FormantFilter shape the vowel character.
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "warm")
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

    // Amplifier (amp-1).
    // Level 1.2 → 1.8, master 0.7 → 0.9 below — peak had drifted back to
    // 0.033 (re-audit 2026-05-10), this clears the 0.05 `low_output`
    // threshold without saturating the formant filter character.
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(800.0, 50.0)
            .param_f("level", 1.8)
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
            .param_f("master", 0.9)
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
