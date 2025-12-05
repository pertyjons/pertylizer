//! Aggressive Bass - Punchy, distorted bass with filter movement.

use crate::patch::{ModuleBuilder, ModuleType, Patch};

/// Aggressive Bass - Punchy, distorted bass with filter movement.
pub fn patch_aggressive_bass() -> Patch {
    let mut patch = Patch::new("Aggressive Bass");
    patch.author = Some("Modular Synth".to_string());
    patch.description =
        Some("Punchy, aggressive bass with distortion and filter sweep.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A square wave provides the fundamental punch and harmonic content. The square
wave's odd harmonics give the bass its characteristic hollow, powerful sound.

The signal passes through a resonant lowpass filter that sweeps down from a
higher cutoff on each note attack, creating the classic "zap" bass sound.
High resonance adds edge and aggression.

DISTORTION:
The tube distortion after the filter adds warmth and harmonics, making the
bass cut through a mix. The drive is set moderately to add grit without
losing the low-end punch.

ENVELOPE DESIGN:
- Fast attack (2ms) for immediate punch
- Short decay to a moderate sustain keeps energy
- Filter envelope has longer decay for the "wah" sweep effect

TRY: Play single notes with space between them. Works great for EDM/dubstep
style bass lines. Try different octaves for different characters.
"#
        .to_string(),
    );
    patch.tags = vec![
        "bass".into(),
        "aggressive".into(),
        "edm".into(),
        "punchy".into(),
    ];

    // OSC - Square wave for punch (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("square")
            .param_f("level", 0.7) // Reduced to make room for sub
            .build(),
    );

    // Sub-Oscillator for bass weight (sub-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SubOscillator)
            .position(50.0, 150.0)
            .param_choice("waveform", "sine") // Pure sine for clean sub
            .param_choice("octave", "minus1") // One octave down
            .param_f("level", 0.6)
            .build(),
    );

    // Filter - Resonant lowpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 400.0)
            .param_f("resonance", 0.6)
            .build(),
    );

    // Amp Envelope - Punchy with curves (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.15)
            .param_f("sustain", 0.6)
            .param_f("release", 0.1)
            .param_f("attack_curve", -0.8) // Fast punch
            .param_f("decay_curve", -0.4) // Quick initial drop
            .param_f("release_curve", -0.3)
            .build(),
    );

    // Filter Envelope - Sweep with curves (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.25)
            .param_f("sustain", 0.2)
            .param_f("release", 0.1)
            .param_f("attack_curve", -0.9) // Instant filter open
            .param_f("decay_curve", -0.5) // Quick sweep down
            .param_f("release_curve", -0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Distortion - Tube warmth (dst-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Distortion)
            .position(650.0, 50.0)
            .distortion_mode("tube")
            .param_f("drive", 0.5)
            .param_f("tone", 0.4)
            .param_f("mix", 0.6)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(850.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1050.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Connections (using string IDs: type-instance)
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("sub-1", "out", "flt-1", "in"); // Sub-osc adds bass weight
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = -2;
    patch
}
