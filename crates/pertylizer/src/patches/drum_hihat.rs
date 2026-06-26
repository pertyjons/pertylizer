//! Hi-Hat - Metallic electronic hi-hat.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Hi-Hat - Metallic electronic hi-hat.
pub fn patch_drum_hihat() -> Patch {
    let mut patch = Patch::new("Hi-Hat");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some("Metallic electronic hi-hat with variable decay.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Hi-hats are essentially filtered noise with very fast envelopes.
The metallic quality comes from highpass filtering that removes
the low frequencies, leaving only the bright, shimmery content.

FILTER:
A highpass filter at around 7-8kHz removes the "body" of the noise,
leaving the bright, metallic character. Higher resonance adds a
slight ring/shimmer.

ENVELOPE:
The amp envelope is extremely fast:
- Instant attack (1ms)
- Very short decay (30-50ms for closed, 200ms+ for open)
- No sustain - hi-hats are purely percussive

CLOSED vs OPEN:
Adjust the decay time to switch between closed and open hi-hats:
- Closed: 30-50ms decay
- Open: 150-300ms decay

TRY: Play rapid 16th notes for closed hi-hat patterns. Longer notes
for open hi-hats. The filter cutoff affects brightness.
"#
        .to_string(),
    );
    patch.tags = vec![
        "drum".into(),
        "hihat".into(),
        "percussion".into(),
        "cymbal".into(),
    ];

    // Noise Generator - White noise for metallic character (nse-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .position(50.0, 50.0)
            .param_choice("type", "white") // Crisp white noise for hi-hat
            .param_f("level", 0.8)
            .build(),
    );

    // Filter - Highpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(350.0, 50.0)
            .filter_mode("highpass")
            .param_f("cutoff", 7500.0)
            .param_f("resonance", 0.4)
            .build(),
    );

    // Amp Envelope - Very short with punchy curves (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(750.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.05)
            .param_f("sustain", 0.0)
            .param_f("release", 0.03)
            .param_f("atk_curve", -1.0) // Instant snap
            .param_f("dec_curve", -0.5) // Quick fade
            .param_f("rel_curve", -0.6) // Tight cutoff
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(750.0, 50.0)
            .param_f("level", 0.5)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(1500.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1100.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Connections (using string IDs: type-instance)
    patch.add_connection("nse-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");
    patch
}
