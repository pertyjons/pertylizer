//! Auto-Wah Bass - Dynamic filter bass using envelope follower.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Auto-Wah Bass - Funky bass with envelope follower driving filter cutoff.
pub fn patch_auto_wah_bass() -> Patch {
    let mut patch = Patch::new("Auto-Wah Bass");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Funky auto-wah bass where playing dynamics control filter brightness via envelope follower."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Sawtooth Osc -> Filter (Acid) -> Amp -> Output
         \-> Envelope Follower -> Filter Cutoff CV

The oscillator feeds both the filter and an Envelope Follower.
The follower tracks the amplitude of the signal and outputs a
control voltage that drives the filter cutoff — play harder and
the filter opens up, play softer and it closes down.

The Acid filter model adds squelchy resonance character perfect
for funky bass lines. Fast attack on the follower ensures the
wah responds immediately to note onset.

MODULATION:
- Envelope Follower -> Filter Cutoff CV (dynamic wah)
- Env 1 -> Amplifier (snappy bass envelope)

TRY: Increase sensitivity for more dramatic wah effect.
Play staccato for funky quack, legato for smooth sweep.
"#
        .to_string(),
    );
    patch.tags = vec![
        "bass".into(),
        "envelope_follower".into(),
        "auto_wah".into(),
        "funky".into(),
        "dynamic".into(),
    ];

    // Oscillator - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.85)
            .build(),
    );

    // Envelope Follower - Track oscillator amplitude (efl-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::EnvelopeFollower)
            .position(1600.0, 50.0)
            .param_f("attack", 2.0)
            .param_f("release", 80.0)
            .param_f("sensitivity", 0.7)
            .build(),
    );

    // Filter - Acid model for squelchy resonance (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_model("acid")
            .filter_mode("lowpass")
            .param_f("cutoff", 400.0)
            .param_f("resonance", 0.6)
            .param_f("drive", 2.0)
            .build(),
    );

    // Amp Envelope - Snappy bass (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.6)
            .param_f("release", 0.15)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("osc-1", "out", "efl-1", "in");
    patch.add_connection("efl-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch.settings.octave_offset = -1;
    patch
}
