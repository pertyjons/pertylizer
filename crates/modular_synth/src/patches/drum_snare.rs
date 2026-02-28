//! Snare Drum - Punchy electronic snare with noise.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Snare Drum - Punchy electronic snare with noise.
pub fn patch_drum_snare() -> Patch {
    let mut patch = Patch::new("Snare Drum");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Electronic snare with tuned body and noise snap.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A snare drum has two components: the tuned "body" (from the drum head)
and the "snare" rattle (from the snare wires). This patch creates both.

BODY (OSC1 - Triangle):
The triangle wave provides a tuned, punchy body. A pitch envelope sweeps
from higher to lower, similar to a kick but faster, giving that characteristic
snare "pop".

SNARE/NOISE (OSC2 - Noise):
White noise through a bandpass filter creates the snare wire sound.
The bandpass centers around 5kHz with moderate resonance for a
metallic, snappy character.

MIXING:
Both sounds are mixed together and share the same amp envelope.
The balance between body and noise determines the snare character:
- More body = deeper, more tonal
- More noise = brighter, snappier

TRY: Adjust the noise filter cutoff for different snare characters.
Higher cutoff = brighter, crisper. Lower = darker, thicker.
"#
        .to_string(),
    );
    patch.tags = vec!["drum".into(), "snare".into(), "percussion".into()];

    // OSC1 - Triangle for body (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 250.0)
            .waveform("triangle")
            .param_f("level", 0.6)
            .build(),
    );

    // Noise Generator for snare rattle (nse-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .position(50.0, 50.0)
            .param_choice("type", "white") // Crisp white noise for snare
            .param_f("level", 0.7)
            .build(),
    );

    // Noise Filter - Bandpass (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("bandpass")
            .param_f("cutoff", 5000.0)
            .param_f("resonance", 0.3)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(850.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Pitch Envelope with punchy curves (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(50.0, 600.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.03)
            .param_f("sustain", 0.0)
            .param_f("release", 0.01)
            .param_f("atk_curve", -0.9) // Instant snap
            .param_f("dec_curve", -0.8) // Quick pitch drop
            .param_f("rel_curve", -0.5)
            .build(),
    );

    // Amp Envelope with punchy curves (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(1250.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.12)
            .param_f("sustain", 0.0)
            .param_f("release", 0.08)
            .param_f("atk_curve", -1.0) // Instant transient
            .param_f("dec_curve", -0.6) // Punchy body
            .param_f("rel_curve", -0.4)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.75)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(2000.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1600.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections (using string IDs: type-instance)
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("nse-1", "out", "flt-1", "in"); // Noise generator to filter
    patch.add_connection("flt-1", "out", "mix-1", "in2");
    patch.add_connection("mix-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "osc-1", "fm");
    patch.add_connection("env-2", "out", "amp-1", "cv");
    // Voice output: amp -> stereo output
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch.settings.octave_offset = -1;
    patch
}
