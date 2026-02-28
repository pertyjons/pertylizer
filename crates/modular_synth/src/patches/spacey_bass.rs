//! Spacey Bass - Classic subtractive bass with detuned oscillators.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Spacey Bass - Classic subtractive bass with two detuned oscillators.
pub fn patch_spacey_bass() -> Patch {
    let mut patch = Patch::new("Spacey Bass");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Classic subtractive bass with two detuned oscillators.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Two sawtooth oscillators create a thick, detuned sound. The second oscillator is
slightly detuned (+7 cents) to create movement and width in the sound.

Both oscillators feed into a lowpass filter with moderate resonance. The filter
cutoff is modulated by both an envelope (for attack pluck) and an LFO (for
slow movement).

MODULATION:
- Filter envelope opens the filter on attack for that classic "wow" sound
- Slow LFO adds subtle movement to the filter cutoff
- LFO also modulates OSC1 for gentle vibrato

ENVELOPE DESIGN:
- Fast attack (5ms) for immediate response
- Short decay (200ms) to moderate sustain for playability
- Moderate release (300ms) for smooth note endings

TRY: Works great for bass lines and pads. Experiment with filter cutoff
and resonance for different tonal characters.
"#
        .to_string(),
    );
    patch.tags = vec!["bass".into(), "subtractive".into(), "default".into()];

    // Oscillator 1 - Sawtooth, main sound (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.6)
            .build(),
    );

    // Oscillator 2 - Sawtooth, detuned for thickness (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 150.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .param_f("detune", 7.0) // +7 cents for thickness
            .build(),
    );

    // Filter - Low pass with moderate resonance (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 400.0)
            .param_f("resonance", 0.4)
            .build(),
    );

    // Amp Envelope - Punchy bass envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.6)
            .param_f("release", 0.3)
            .build(),
    );

    // Filter Envelope - Opens filter on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.2)
            .param_f("release", 0.4)
            .build(),
    );

    // LFO - Slow sine for movement (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(450.0, 300.0)
            .param_f("rate", 0.3)
            .param_f("depth", 0.25)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(650.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    // Osc1 -> Filter, Osc2 -> Filter (both mixed into filter)
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("osc-2", "out", "flt-1", "in");
    // Filter -> Amp
    patch.add_connection("flt-1", "out", "amp-1", "in");
    // Amp Env -> Amp CV
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Filter Env -> Filter Cutoff CV
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    // LFO -> Filter Cutoff CV (mixed with envelope)
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    // LFO -> Osc1 FM (vibrato)
    patch.add_connection("lfo-1", "out", "osc-1", "fm");
    // Voice output: amp -> stereo output
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
