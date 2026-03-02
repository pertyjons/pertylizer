//! Harmonic Lead - Expressive lead using harmonics wavetable scanning.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Harmonic Lead - Bright lead with envelope-driven harmonic sweep.
pub fn patch_harmonic_lead() -> Patch {
    let mut patch = Patch::new("Harmonic Lead");
    patch.author = Some("Pertylizer".to_string());
    patch.description = Some(
        "Expressive lead using the Harmonics wavetable with envelope-driven position sweep."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (Harmonics) -> Filter (Screamer) -> Amplifier -> Delay -> Output

The Harmonics wavetable morphs from a single sine harmonic through
increasingly complex additive waveforms (up to 32 harmonics).
An envelope sweeps through these on each note, creating a bright
attack that settles into a simpler sustain tone.

The Screamer filter adds aggressive MS-20-style character with
diod clipping resonance — perfect for cutting lead lines.

MODULATION:
- Env 2 -> Wavetable Position CV (harmonic sweep on attack)
- Env 2 -> Filter Cutoff CV (brightness follows harmonics)
- Env 1 -> Amplifier (lead envelope)

TRY: Play expressive melodies. The harmonic sweep gives each note
a bright, evolving attack. Increase filter resonance for screaming leads.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "wavetable".into(),
        "harmonics".into(),
        "expressive".into(),
        "bright".into(),
    ];

    // Wavetable Oscillator - Harmonics bank (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "harmonics")
            .param_f("position", 0.1)
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Screamer for aggressive character (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(400.0, 50.0)
            .filter_model("screamer")
            .param_f("cutoff", 3000.0)
            .param_f("resonance", 0.45)
            .param_f("drive", 2.5)
            .build(),
    );

    // Amp Envelope - Lead with bite (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(800.0, 350.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.7)
            .param_f("release", 0.3)
            .build(),
    );

    // Position/Filter Envelope - Harmonic sweep (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(50.0, 250.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.15)
            .param_f("release", 0.2)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(800.0, 50.0)
            .param_f("level", 0.65)
            .build(),
    );

    // Delay - Rhythmic echo (dly-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Delay)
            .position(1550.0, 50.0)
            .delay_mode("stereo")
            .param_f("time", 0.35)
            .param_f("feedback", 0.35)
            .param_f("mix", 0.25)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1150.0, 50.0)
            .param_f("master level", 0.75)
            .build(),
    );

    // Connections
    patch.add_connection("wtb-1", "out", "flt-1", "in");
    patch.add_connection("env-2", "out", "wtb-1", "pos_cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
