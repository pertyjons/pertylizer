//! FOF Choir - granular CHANT-style vocal ensemble (Fof unison).

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// FOF Choir - a shimmering vocal ensemble from one FOF generator's unison.
///
/// The companion to the `Choir` preset: where that one filters a glottal source
/// (VoiceSynth), this builds the vowels in the time domain from overlapping FOF
/// grains (CHANT), giving the characteristic granular "shimmer" and cleaner
/// pitch/vowel decoupling.
pub fn patch_fof_choir() -> Patch {
    let mut patch = Patch::new("FOF Choir");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("Granular CHANT-style vocal ensemble — 16 decorrelated FOF voices.".into());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
FOF (out_l/out_r unison) -> Amplifier (in_l/in_r, env on cv)
  -> MidSide (width) -> Reverb -> Output

FOF synthesizes each vowel directly in the time domain from overlapping
formant-wave-function grains (one per formant, fired once per pitch period).
16 internal sub-voices each get their own detune, vibrato, formant jitter,
onset and pan; their amplitude beating is the choir. The Skirt knob is unique
to FOF — it shapes the grain attack (0 = sharp & bright, 1 = soft & dull).
A slow LFO drifts the vowel for an evolving "aah -> ooh" texture.

VOICE TYPE (SATB):
Formant Shift sets vocal-tract length — sweep it for the section you want:
~0.30 bass, ~0.45 tenor, ~0.60 alto, ~0.75 soprano, combined with the note
range you play.

TRY: lower Unison Voices for a small group; raise Skirt for a softer, airier
tone; lower Bandwidth for a sharper, more vocal-formant ring.
"#
        .into(),
    );
    patch.tags = vec![
        "voice".into(),
        "vocal".into(),
        "choir".into(),
        "fof".into(),
        "chant".into(),
        "ensemble".into(),
        "stereo".into(),
    ];

    // FOF — full choir (fof-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Fof)
            .position(50.0, 50.0)
            .param_f("vowel", 0.15)
            .param_f("formant_shift", 0.5)
            .param_f("skirt", 0.35)
            .param_f("bandwidth", 0.5)
            .param_f("breathiness", 0.18)
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
            .param_f("rate", 0.1)
            .param_f("depth", 0.12)
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
            .param_f("mix", 0.42)
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
    patch.add_connection("fof-1", "out_l", "amp-1", "in_l");
    patch.add_connection("fof-1", "out_r", "amp-1", "in_r");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("lfo-1", "out", "fof-1", "vowel_cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
