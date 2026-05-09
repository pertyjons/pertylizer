//! Velocity Pad - Expressive pad using Mod Matrix for velocity-sensitive filtering.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Velocity Pad - Warm pad with velocity-controlled brightness and dynamics.
pub fn patch_velocity_pad() -> Patch {
    let mut patch = Patch::new("Velocity Pad");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description =
        Some("Expressive pad with velocity-controlled filter and dynamics via Mod Matrix.".into());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Two detuned sawtooth oscillators create a warm, wide pad foundation. A triangle
sub oscillator adds body. The mixed signal passes through a lowpass filter into
an amplifier with envelope control.

MOD MATRIX ROUTING (the star of this patch):
- Slot 1: Velocity → Filter 1 Cutoff (amount +0.6)
    Harder playing opens the filter for brighter tones
- Slot 2: Velocity → Amp Level (amount +0.4)
    Dynamics respond naturally to playing intensity
- Slot 3: LFO 1 → Filter 1 Cutoff (amount +0.2)
    Slow filter sweep adds gentle movement
- Slot 4: Envelope 2 → Filter 1 Cutoff (amount +0.5)
    Attack envelope shapes the filter contour

Unlike cable-based modulation, the Mod Matrix allows multiple sources to
control the filter cutoff simultaneously without a separate mixer module.

ENVELOPE DESIGN:
- Amp envelope: Slow attack (150ms) for pad-like swell, long release (1.5s)
- Filter envelope: Faster attack (50ms), medium decay for initial brightness

TRY: Play soft for dark, mellow tones. Play hard for bright, present sound.
Use the mod wheel for additional expression.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pad".into(),
        "velocity".into(),
        "mod_matrix".into(),
        "expressive".into(),
    ];

    // OSC1 - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.45)
            .param_f("detune", -5.0)
            .build(),
    );

    // OSC2 - Sawtooth, detuned (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("level", 0.45)
            .param_f("detune", 5.0)
            .build(),
    );

    // Filter - Lowpass, starts fairly closed (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 300.0)
            .param_f("resonance", 0.25)
            .build(),
    );

    // Amp Envelope - Slow pad attack (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.15)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.7)
            .param_f("release", 1.5)
            .build(),
    );

    // Filter Envelope - Faster attack for brightness contour (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.05)
            .param_f("decay", 0.8)
            .param_f("sustain", 0.3)
            .param_f("release", 1.0)
            .build(),
    );

    // LFO - Slow sine for gentle movement (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(450.0, 650.0)
            .waveform("sine")
            .param_f("rate", 0.25)
            .param_f("depth", 0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(850.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Mod Matrix - The key module! (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .position(1600.0, 50.0)
            .param_choice("grid size", "2x2")
            // Slot 1: Velocity → Filter Cutoff, amount +0.6
            .param_choice("slot 1 source", "velocity")
            .param_choice("slot 1 dest", "flt1_cutoff")
            .param_f("slot 1 amount", 0.6)
            // Slot 2: Velocity → Amp Level, amount +0.4
            .param_choice("slot 2 source", "velocity")
            .param_choice("slot 2 dest", "amp_level")
            .param_f("slot 2 amount", 0.4)
            // Slot 3: LFO 1 → Filter Cutoff, amount +0.2 (gentle movement)
            .param_choice("slot 3 source", "lfo1")
            .param_choice("slot 3 dest", "flt1_cutoff")
            .param_f("slot 3 amount", 0.2)
            // Slot 4: Env 2 → Filter Cutoff, amount +0.5
            .param_choice("slot 4 source", "env2")
            .param_choice("slot 4 dest", "flt1_cutoff")
            .param_f("slot 4 amount", 0.5)
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
    patch.add_connection("osc-2", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    // Note: Filter envelope and LFO are routed via Mod Matrix, no cables needed!

    patch
}
