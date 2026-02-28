//! Unison Supersaw - Classic trance/EDM supersaw lead using intra-voice unison.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Unison Supersaw - Massive, wide supersaw lead.
pub fn patch_unison_supersaw() -> Patch {
    let mut patch = Patch::new("Unison Supersaw");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Classic trance supersaw with 7-voice unison, full stereo spread and chorus.".to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Saw oscillator (7-voice unison) -> Filter (Lowpass) -> Amplifier -> Chorus -> Output

The oscillator's intra-voice unison generates 7 detuned sawtooth copies with
full stereo spread. The mono sum goes through a lowpass filter for warmth,
then chorus adds extra width.

KEY PARAMETERS:
- Unison: 7 voices for maximum thickness
- Uni Detune: 25 cents for classic supersaw spread
- Uni Spread: Full stereo (1.0)
- Filter cutoff: Modulated by envelope for timbral movement

TRY: Lower unison to 3 for a tighter sound. Increase detune for a more
aggressive, detuned character. Play chords for classic trance stabs.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "supersaw".into(),
        "unison".into(),
        "trance".into(),
        "stereo".into(),
    ];

    // Oscillator - 7-voice unison saw (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .param_f("unison", 7.0)
            .param_f("uni detune", 25.0)
            .param_f("uni spread", 1.0)
            .param_f("uni phase", 1.0)
            .build(),
    );

    // Filter - Lowpass with envelope modulation (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1800.0)
            .param_f("resonance", 0.15)
            .build(),
    );

    // Amp Envelope - Snappy attack, moderate sustain (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.4)
            .param_f("sustain", 0.7)
            .param_f("release", 0.3)
            .build(),
    );

    // Filter Envelope - Bright attack that decays (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.2)
            .param_f("release", 0.2)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Mod Matrix - Env2 -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(450.0, 300.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "env2")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.7)
            .build(),
    );

    // Chorus - Extra width (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(650.0, 50.0)
            .param_f("rate", 0.6)
            .param_f("depth", 0.3)
            .param_f("mix", 0.35)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(850.0, 50.0)
            .param_f("master level", 0.7)
            .build(),
    );

    // Connections
    // Note: env-2 -> filter cutoff is routed via Mod Matrix, no cable needed.
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
