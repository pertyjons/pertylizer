//! Solo Voice - expressive single singing voice (VoiceSynth).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Solo Voice - a single expressive sung vowel with vibrato.
pub fn patch_solo_voice() -> Patch {
    let mut patch = Patch::new("Solo Voice");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("Expressive solo singing voice — glottal source through formant resonators.".into());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Voice Synth (mono out) -> Amplifier (env on cv) -> Reverb -> Output

The Voice Synth is a physically-inspired source–filter model: a glottal
pulse excites a bank of formant resonators tuned to the Vowel parameter
(A→E→I→O→U). Internal vibrato and a touch of breathiness give it life;
the host envelope shapes the overall amplitude.

VOICE TYPE:
- Formant Shift sets vocal-tract length: <0.5 = bass/baritone (darker),
  >0.5 = alto/soprano (brighter). Default 0.5 is a neutral tenor. For
  dedicated, physically-modelled SATB voices, see the SATB Soprano/Alto/
  Tenor/Bass presets (the articulatory Vocal Tract engine + its Length
  control).

TRY: sweep Vowel for a "talking" line; raise Breathiness for a whispery
tone; lower Open Quotient for a pressed, belted character.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "solo".into(),
        "formant".into(),
    ];

    // Voice Synth — solo (vox-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VoiceSynth)
            .position(50.0, 50.0)
            .param_f("vowel", 0.0)
            .param_f("formant_shift", 0.5)
            .param_f("breathiness", 0.12)
            .param_f("open_quotient", 0.5)
            .param_f("tilt", 0.1)
            .param_f("vibrato_rate", 5.5)
            .param_f("vibrato_depth", 35.0)
            .param_f("unison_voices", 0.0)
            .param_f("level", 0.85)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.06)
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

    // Reverb - small room (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 300.0)
            .param_f("room_size", 0.5)
            .param_f("damping", 0.4)
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

    // Connections — mono voice through the VCA, reverb in the effect chain.
    patch.add_connection("vox-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
