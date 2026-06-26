//! SATB Tenor - near-nominal tract (VocalTract Length control).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// SATB Tenor - a near-nominal tract length, the neutral male upper voice that
/// the other SATB presets scale away from.
pub fn patch_satb_tenor() -> Patch {
    let mut patch = Patch::new("SATB Tenor");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Tenor voice — a near-nominal Kelly–Lochbaum tract (Length 0.6) for a full, slightly bright male upper voice."
            .into(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Vocal Tract (mono out) -> Amplifier (env on cv) -> Reverb -> Output
LFO -> Vocal Tract tongue_cv (gentle vowel shimmer)

One of four SATB presets set apart by the Length control (the physical
tract length). The tenor sits just longer than neutral (Length 0.6) —
formants close to the tract's natural placement, the reference the
soprano/alto raise and the bass lowers.

Play it in the tenor range (~C3–C5). A neutral "ah" vowel and a moderate
hall make it the workhorse middle-upper voice.

TRY: raise Length toward Alto/Soprano for a lighter timbre, or lower it
toward the Bass preset; open the Vowel for a brighter, more forward tone.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "tenor".into(),
        "satb".into(),
        "choir".into(),
    ];

    // Vocal Tract — near-nominal tract, tenor (vtr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VocalTract)
            .position(50.0, 50.0)
            .param_f("tongue", 0.48)
            .param_f("constriction", 0.42)
            .param_f("lips", 0.45)
            .param_f("length", 0.6)
            .param_f("nasality", 0.0)
            .param_f("breathiness", 0.1)
            .param_f("level", 0.85)
            .build(),
    );

    // LFO — gentle vowel shimmer on the tongue (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 350.0)
            .waveform("sine")
            .param_f("rate", 0.26)
            .param_f("depth", 0.1)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.07)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.85)
            .param_f("release", 0.65)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 1.0)
            .build(),
    );

    // Reverb — moderate hall (rev-1, effect chain)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 300.0)
            .param_f("room_size", 0.6)
            .param_f("damping", 0.4)
            .param_f("mix", 0.25)
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
