//! Choir - wide decorrelated vocal ensemble (VoiceSynth unison).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Choir - a lush, wide vocal ensemble from one Voice Synth's unison.
pub fn patch_choir() -> Patch {
    let mut patch = Patch::new("Choir");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("Wide vocal ensemble — 16 decorrelated voices with stereo spread.".into());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Voice Synth (out_l/out_r unison) -> Amplifier (in_l/in_r, env on cv)
  -> MidSide (width) -> Reverb -> Output

The Voice Synth runs 16 internal sub-voices, each with its own detune,
vibrato, formant jitter, onset and pan. The pseudo-random amplitude
beating between them is what the ear hears as a choir — the stereo
out_l/out_r are routed directly to preserve the full ensemble width.
A slow LFO drifts the vowel for an evolving "aah → ooh" texture.

VOICE TYPE (SATB):
Formant Shift sets vocal-tract length — sweep it for the section you want:
~0.30 bass, ~0.45 tenor, ~0.60 alto, ~0.75 soprano. Combine with the
note range you play. For solo, physically-modelled section voices, see the
dedicated SATB Soprano/Alto/Tenor/Bass presets (Vocal Tract + Length).

TRY: lower Unison Voices for a small group; raise Unison Detune for a
more dissonant, massed sound; set Unison Spread to 0 for a mono collapse.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "choir".into(),
        "ensemble".into(),
        "stereo".into(),
    ];

    // Voice Synth — full choir (vox-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VoiceSynth)
            .position(50.0, 50.0)
            .param_f("vowel", 0.15)
            .param_f("formant_shift", 0.5)
            .param_f("breathiness", 0.2)
            .param_f("open_quotient", 0.6)
            .param_f("tilt", 0.15)
            .param_f("vibrato_rate", 5.5)
            .param_f("vibrato_depth", 22.0)
            .param_f("unison_voices", 1.0)
            .param_f("unison_detune", 18.0)
            .param_f("unison_spread", 0.9)
            .param_f("level", 0.8)
            .build(),
    );

    // Amp Envelope - slow, choral swell (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.4)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.85)
            .param_f("release", 1.2)
            .build(),
    );

    // LFO - slow vowel drift (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 400.0)
            .param_choice("waveform", "sine")
            .param_f("rate", 0.12)
            .param_f("depth", 0.1)
            .build(),
    );

    // Amplifier - stereo input (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 1.0)
            .build(),
    );

    // MidSide - widen the ensemble (mds-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MidSide)
            .position(1200.0, 300.0)
            .param_f("width", 0.7)
            .param_f("mid_gain", 0.0)
            .param_f("side_gain", 1.5)
            .param_f("rotation", 0.0)
            .param_f("mix", 0.8)
            .build(),
    );

    // Reverb - choral hall (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 50.0)
            .param_f("room_size", 0.85)
            .param_f("damping", 0.35)
            .param_f("mix", 0.4)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(800.0, 50.0)
            .param_f("master", 0.85)
            .build(),
    );

    // Connections — stereo unison straight into the VCA, effects in the chain.
    patch.add_connection("vox-1", "out_l", "amp-1", "in_l");
    patch.add_connection("vox-1", "out_r", "amp-1", "in_r");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("lfo-1", "out", "vox-1", "vowel_cv");
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");
    patch
}
