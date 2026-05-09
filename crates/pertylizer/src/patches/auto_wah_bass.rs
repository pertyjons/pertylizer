//! Auto-Wah Bass - Dynamic filter bass using envelope follower.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Auto-Wah Bass - Funky bass with envelope follower driving filter cutoff.
pub fn patch_auto_wah_bass() -> Patch {
    let mut patch = Patch::new("Auto-Wah Bass");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Funky auto-wah bass where playing dynamics control filter brightness via envelope follower."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Sawtooth Osc -> Filter (Acid) -> Amp -> Output
                                   \-> Envelope Follower
                                          \-> Mod Matrix -> Filter Cutoff

The follower side-chains the AMP output, so it tracks the actual played
amplitude — env-1's contour, velocity scaling, and any release tail all
show up in the EFL's CV signal. The matrix routes that CV to the filter
cutoff, sidestepping the graph cycle that a direct cable would form.

Result: a true auto-wah. Hard hits open the filter wider. The wah swells
with each note's attack and closes back during decay/release. Staccato
gives a funky quack; legato gives a smooth sweep.

MODULATION:
- Amp.left -> EFL -> Mod Matrix slot 1 -> Filter Cutoff (auto-wah)
- Env 1 -> Amplifier (snappy bass envelope)

TRY: Raise Mod Matrix slot 1 amount for a wider wah range. Drop EFL
sensitivity for subtler tracking, raise it for more dramatic swells.
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

    // Envelope Follower - tracks the post-amp signal via Mod Matrix routing
    // (a direct cable would form a graph cycle). Fast attack catches each
    // note's onset; release ~30 ms lets the follower mirror env-1's decay
    // shape so the wah closes as the note settles.
    //
    // Sensitivity is high (1.0 → ×4 internal multiplier) because the amp
    // output is already attenuated by the lowpass filter ahead of it; the
    // raw EFL value lands around 0.15-0.30, which we want amplified well
    // before it hits the matrix's amount knob.
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::EnvelopeFollower)
            .position(1600.0, 50.0)
            .param_f("attack", 2.0)
            .param_f("release", 30.0)
            .param_f("sensitivity", 1.0)
            .build(),
    );

    // Filter - Acid model for squelchy resonance (flt-1).
    // Env Amt = 0.5 gives a 2-octave wah range (24 semitones at full
    // follower output), which is more expressive than the 1-octave default
    // for a patch whose entire personality is the filter sweep.
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_model("acid")
            .filter_mode("lowpass")
            .param_f("cutoff", 400.0)
            .param_f("resonance", 0.6)
            .param_f("drive", 2.0)
            .param_f("env_amt", 0.5)
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
            .param_f("level", 1.0)
            .build(),
    );

    // Mod Matrix - Routes the EFL output to filter cutoff. Going through
    // the matrix instead of a direct cable lets the follower side-chain
    // the post-amp signal: a direct cable would form flt → amp → efl → flt
    // and be rejected by the graph's cycle detector. Amount 1.0 gives the
    // EFL output its full 48-semitone (4-octave) range; the EFL itself
    // typically delivers 0.2-0.5, so usable range lands at 1-2 octaves.
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 350.0)
            .param_choice("grid size", "1x1")
            .param_choice("slot 1 source", "efl1")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1200.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // EFL side-chains the amp output. No direct cable to flt-1.cutoff_cv —
    // the Mod Matrix slot above carries the EFL signal there instead, so
    // the topology stays acyclic.
    patch.add_connection("amp-1", "left", "efl-1", "in");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch.settings.octave_offset = -1;
    patch
}
