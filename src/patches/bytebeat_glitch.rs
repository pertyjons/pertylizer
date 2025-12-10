//! Bytebeat Glitch - Retro digital music formula.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Bytebeat Glitch - Retro digital music formula.
pub fn patch_bytebeat_glitch() -> Patch {
    let mut patch = Patch::new("Bytebeat Glitch");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Algorithmic music using bytebeat formula synthesis.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Bytebeat is a style of algorithmic music where sound is generated from
simple mathematical formulas. The classic formula is:
  t * ((t >> A) | (t >> B))

where t is a time counter and A, B are bitshift amounts.

PARAMETERS:
- Param A: First bitshift amount (creates rhythm/melody)
- Param B: Second bitshift amount (creates harmony/texture)
- Param C: Not actively used

Different parameter combinations create completely different "songs"!
Some sound melodic, others chaotic, many sound retro and 8-bit.

TRY: Play notes and experiment with different Param A/B values.
Small changes can create vastly different musical results!
"#
        .to_string(),
    );
    patch.tags = vec![
        "math".into(),
        "bytebeat".into(),
        "glitch".into(),
        "8bit".into(),
        "retro".into(),
        "experimental".into(),
    ];

    // Math Oscillator - Bytebeat (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("bytebeat")
            .param_f("param_a", 0.4) // Bitshift A
            .param_f("param_b", 0.6) // Bitshift B
            .param_f("param_c", 0.5)
            .param_f("level", 0.6)
            .build(),
    );

    // Filter - Tame harsh highs (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3000.0)
            .param_f("resonance", 0.2)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.1)
            .param_f("sustain", 0.7)
            .param_f("release", 0.2)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.5)
            .build(),
    );

    // Delay - Add rhythm (dly-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Delay)
            .position(650.0, 50.0)
            .delay_mode("ping_pong")
            .param_f("time", 0.15)
            .param_f("feedback", 0.3)
            .param_f("mix", 0.3)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscilloscope)
            .position(850.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(1050.0, 50.0)
            .param_f("master", 0.7)
            .build(),
    );

    // Connections
    patch.add_connection("mth-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
