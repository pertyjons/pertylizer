//! Vocal Tract - articulatory Kelly–Lochbaum speech voice (VocalTract).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Vocal Tract - a talking/singing voice whose vowels come from tube physics.
pub fn patch_vocal_tract() -> Patch {
    let mut patch = Patch::new("Vocal Tract");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Articulatory voice — a glottal pulse driven through a Kelly–Lochbaum vocal-tract waveguide."
            .into(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Vocal Tract (mono out) -> Amplifier (env on cv) -> Reverb -> Output
LFO -> Vocal Tract tongue_cv (slow vowel morph / "talking" motion)

Unlike the source–filter Voice Synth, the Vocal Tract derives its
formants from the *physics* of a 44-section tube: a glottal pulse is
scattered down the tract and radiated at the lips. The vowel is the
tube's shape, set by three articulators:
- Tongue / Constriction: position and tightness of the tongue hump
  (the main vowel control).
- Lips: aperture/rounding (0 = rounded /o u/, 1 = spread /i e/).
- Nasality: opens the velum into a nasal cavity for /m n ŋ/ and
  nasalised vowels (0 here — raise it for a nasal colour).

The LFO slowly sweeps the tongue for a vowel-morphing, almost-talking
character; the host envelope shapes the overall amplitude.

TRY: raise Nasality for a nasal/"blocked nose" tone; lower Lips for
rounded vowels; increase Constriction for a tighter, more vowel-like
resonance; play low notes for a darker, chest-like voice.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "speech".into(),
        "physical".into(),
        "waveguide".into(),
    ];

    // Vocal Tract — articulatory voice (vtr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VocalTract)
            .position(50.0, 50.0)
            .param_f("tongue", 0.5)
            .param_f("constriction", 0.4)
            .param_f("lips", 0.5)
            .param_f("nasality", 0.0)
            .param_f("breathiness", 0.12)
            .param_f("level", 0.85)
            .build(),
    );

    // LFO — slow vowel morph on the tongue (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 350.0)
            .waveform("sine")
            .param_f("rate", 0.25)
            .param_f("depth", 0.5)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.05)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.8)
            .param_f("release", 0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 1.0)
            .build(),
    );

    // Reverb - small room (rev-1, effect chain)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 300.0)
            .param_f("room_size", 0.5)
            .param_f("damping", 0.4)
            .param_f("mix", 0.2)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(800.0, 50.0)
            .param_f("master", 0.85)
            .build(),
    );

    // Connections — mono tract through the VCA; LFO morphs the vowel.
    patch.add_connection("lfo-1", "out", "vtr-1", "tongue_cv");
    patch.add_connection("vtr-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
