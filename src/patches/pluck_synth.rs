//! Pluck Synth - Short, plucky synthesizer sound.

use crate::patch::{ModuleBuilder, ModuleType, Patch};

/// Pluck Synth - Short, plucky synthesizer sound.
pub fn patch_pluck_synth() -> Patch {
    let mut patch = Patch::new("Pluck Synth");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Short, plucky synthesizer for arpeggios and sequences.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A sawtooth wave provides rich harmonics that will be sculpted by the
filter. Sawtooth is ideal for plucks because it has all harmonics,
giving the filter plenty of material to shape.

FILTER CHARACTER:
The resonant lowpass filter is key to the pluck sound. The envelope
rapidly opens and closes the filter:
- Fast attack opens instantly
- Quick decay (100ms) creates the "pluck"
- Moderate resonance adds a slight "ping"

This mimics how physical plucked strings have bright attacks that
quickly mellow out.

AMP ENVELOPE:
Similarly shaped to the filter but with slightly longer decay,
allowing the filter's "pluck" to be heard before the volume fades.

EFFECTS:
The ping-pong delay adds rhythmic interest, perfect for arpeggios.
It turns single notes into cascading patterns.

TRY: Play arpeggiated patterns or sequences. The delay creates
instant complexity. Great for EDM, synthwave, and electronic pop.
"#
        .to_string(),
    );
    patch.tags = vec![
        "pluck".into(),
        "synth".into(),
        "arpeggio".into(),
        "sequence".into(),
    ];

    // OSC - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );

    // Filter (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1000.0)
            .param_f("resonance", 0.45)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.0)
            .param_f("release", 0.15)
            .build(),
    );

    // Filter Envelope (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.1)
            .param_f("sustain", 0.1)
            .param_f("release", 0.1)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Delay (dly-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Delay)
            .position(650.0, 50.0)
            .delay_mode("ping_pong")
            .param_f("time", 0.25)
            .param_f("feedback", 0.45)
            .param_f("mix", 0.35)
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
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}
