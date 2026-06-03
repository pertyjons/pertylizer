//! SATB Soprano - short-tract high voice (VocalTract Length control).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// SATB Soprano - the brightest SATB voice: a short vocal tract pushes all
/// formants up, the soprano register's defining colour.
pub fn patch_satb_soprano() -> Patch {
    let mut patch = Patch::new("SATB Soprano");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Soprano voice — a short Kelly–Lochbaum tract (high Length) lifts every formant for a bright, ringing top voice."
            .into(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Vocal Tract (mono out) -> Amplifier (env on cv) -> Reverb -> Output
LFO -> Vocal Tract tongue_cv (gentle vowel shimmer)

One of four SATB presets that differ almost entirely by the new Length
control — the physical tract length. A SHORT tract (Length 0.18) scales
the formants up, which is exactly what makes a soprano sound like a
soprano: same vowel, smaller resonator, higher formants.

Play it in the soprano range (~C4–C6). Bright vowel, a little extra
breath, and a long hall to sit on top of the ensemble.

TRY: nudge Length up for a boy-soprano/child timbre, or down toward the
Alto preset; sweep Vowel for a sung line.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "soprano".into(),
        "satb".into(),
        "choir".into(),
    ];

    // Vocal Tract — short tract, bright soprano (vtr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VocalTract)
            .position(50.0, 50.0)
            .param_f("tongue", 0.55)
            .param_f("constriction", 0.35)
            .param_f("lips", 0.6)
            .param_f("length", 0.18)
            .param_f("nasality", 0.0)
            .param_f("breathiness", 0.16)
            .param_f("level", 0.85)
            .build(),
    );

    // LFO — gentle vowel shimmer on the tongue (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 350.0)
            .waveform("sine")
            .param_f("rate", 0.3)
            .param_f("depth", 0.1)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.08)
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

    // Reverb — bright hall (rev-1, effect chain)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 300.0)
            .param_f("room_size", 0.75)
            .param_f("damping", 0.3)
            .param_f("mix", 0.3)
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
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
