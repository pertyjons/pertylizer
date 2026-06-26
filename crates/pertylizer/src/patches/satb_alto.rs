//! SATB Alto - medium-short tract (VocalTract Length control).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// SATB Alto - a slightly shortened tract sits the formants between tenor and
/// soprano: the warm, full lower female / counter-tenor voice.
pub fn patch_satb_alto() -> Patch {
    let mut patch = Patch::new("SATB Alto");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Alto voice — a medium-short Kelly–Lochbaum tract (Length 0.4) places the formants between tenor and soprano."
            .into(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Vocal Tract (mono out) -> Amplifier (env on cv) -> Reverb -> Output
LFO -> Vocal Tract tongue_cv (gentle vowel shimmer)

One of four SATB presets distinguished by the Length control (the physical
tract length). The alto sits a touch shorter than nominal (Length 0.4),
lifting the formants moderately for a warm but bright lower voice.

Play it in the alto range (~F3–F5). A rounder vowel than the soprano and a
shorter reverb keep it grounded in the middle of the choir.

TRY: raise Length toward the Soprano preset, or lower it toward the Tenor;
add a little Nasality for an earthier colour.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "alto".into(),
        "satb".into(),
        "choir".into(),
    ];

    // Vocal Tract — medium-short tract, alto (vtr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VocalTract)
            .position(50.0, 50.0)
            .param_f("tongue", 0.5)
            .param_f("constriction", 0.4)
            .param_f("lips", 0.5)
            .param_f("length", 0.4)
            .param_f("nasality", 0.0)
            .param_f("breathiness", 0.12)
            .param_f("level", 0.85)
            .build(),
    );

    // LFO — gentle vowel shimmer on the tongue (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 350.0)
            .waveform("sine")
            .param_f("rate", 0.28)
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
            .param_f("release", 0.6)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 1.0)
            .build(),
    );

    // Reverb — medium hall (rev-1, effect chain)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 300.0)
            .param_f("room_size", 0.65)
            .param_f("damping", 0.35)
            .param_f("mix", 0.28)
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
