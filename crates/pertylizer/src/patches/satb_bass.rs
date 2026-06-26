//! SATB Bass - long-tract low voice (VocalTract Length control).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// SATB Bass - the darkest SATB voice: a long vocal tract drops all formants,
/// giving the deep, chesty resonance of a bass.
pub fn patch_satb_bass() -> Patch {
    let mut patch = Patch::new("SATB Bass");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Bass voice — a long Kelly–Lochbaum tract (low formants) with a rounded, dark vowel for a deep, chesty bottom voice."
            .into(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Vocal Tract (mono out) -> Amplifier (env on cv) -> Reverb -> Output
LFO -> Vocal Tract tongue_cv (gentle vowel shimmer)

One of four SATB presets that differ mainly by the Length control (the
physical tract length). A LONG tract (Length 0.9) scales every formant
DOWN — the deep, dark resonance that makes a bass. Paired with a rounded
back vowel and rounded lips for extra weight.

Play it low (~E1–E3). A slow attack and a darker, drier room keep the
bottom of the choir solid rather than boomy.

TRY: shorten Length toward the Tenor preset for a baritone; round the Lips
further (lower) for a more covered /o u/ tone; lower Breathiness for a
firmer, more pressed bass.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "bass".into(),
        "satb".into(),
        "choir".into(),
    ];

    // Vocal Tract — long tract, dark bass (vtr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VocalTract)
            .position(50.0, 50.0)
            .param_f("tongue", 0.42)
            .param_f("constriction", 0.45)
            .param_f("lips", 0.3)
            .param_f("length", 0.9)
            .param_f("nasality", 0.0)
            .param_f("breathiness", 0.08)
            .param_f("level", 0.85)
            .build(),
    );

    // LFO — gentle vowel shimmer on the tongue (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 350.0)
            .waveform("sine")
            .param_f("rate", 0.22)
            .param_f("depth", 0.09)
            .build(),
    );

    // Amp Envelope — slower attack for a settled bottom voice (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.06)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.85)
            .param_f("release", 0.7)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 1.0)
            .build(),
    );

    // Reverb — darker, drier room (rev-1, effect chain)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 300.0)
            .param_f("room_size", 0.55)
            .param_f("damping", 0.5)
            .param_f("mix", 0.22)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(800.0, 50.0)
            .param_f("master", 0.85)
            .build(),
    );

    patch.add_connection("lfo-1", "out", "vtr-1", "tongue_cv");
    patch.add_connection("vtr-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");
    patch
}
