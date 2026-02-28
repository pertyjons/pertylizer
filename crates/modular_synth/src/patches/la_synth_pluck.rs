//! LA Synth Pluck — Linear Arithmetic synthesis with percussive attack transient.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// LA Synth Pluck — sharp attack transient crossfades into a filtered sustain tone.
pub fn patch_la_synth_pluck() -> Patch {
    let mut patch = Patch::new("LA Synth Pluck");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Linear Arithmetic pluck using LA Synth's attack transient crossfading into filtered saw."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
An oscillator provides a sustained sawtooth tone that feeds into the LA Synth
module. The LA Synth generates a percussive "pluck" attack transient and
crossfades it into the sustained input signal over a short time window.

The LA Synth output goes through a lowpass filter with moderate resonance,
giving the tone warmth. A slow LFO subtly modulates the filter cutoff for
gentle movement.

LA SYNTH PARAMETERS:
- Attack Type = 0.66 (pluck transient — short, bright burst)
- Attack Time = 25 ms (very short for snappy pluck)
- Attack Level = 0.85 (prominent transient)
- X-Fade Time = 40 ms (quick crossfade to sustain)
- Brightness = 0.7 (bright attack, not harsh)

This demonstrates the core idea of LA synthesis (Roland D-50 style):
a sampled/synthesized attack transient blended with a sustained waveform,
giving each note a natural, evolving character.

ENVELOPE DESIGN:
- Medium attack (10ms) for smooth onset
- Long decay (800ms) to sustain for pad-like sustain
- High sustain (0.7) for held notes
- Long release (1.5s) for smooth fade

TRY: Adjust Attack Type to hear different transients (0=click, 0.33=noise,
0.66=pluck, 1.0=hammer). Lower X-Fade Time for more percussive character.
"#
        .to_string(),
    );
    patch.tags = vec!["la_synth".into(), "pluck".into(), "experimental".into()];

    // Oscillator - Sawtooth sustain signal (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.7)
            .build(),
    );

    // LA Synth - Attack transient + crossfade (las-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::LaSynth)
            .position(450.0, 50.0)
            .param_f("attack type", 0.66) // pluck
            .param_f("attack time", 25.0) // 25ms
            .param_f("attack level", 0.85)
            .param_f("x-fade time", 40.0) // 40ms crossfade
            .param_f("brightness", 0.7)
            .build(),
    );

    // Filter - Lowpass for warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2500.0)
            .param_f("resonance", 0.3)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 350.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.8)
            .param_f("sustain", 0.7)
            .param_f("release", 1.5)
            .build(),
    );

    // LFO - Slow filter movement (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(850.0, 350.0)
            .param_f("rate", 0.4)
            .param_f("depth", 0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1600.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Reverb effect for space
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(2000.0, 50.0)
            .param_f("mix", 0.25)
            .param_f("decay", 2.5)
            .build(),
    );

    // Connections
    // Osc -> LA Synth (sustain input)
    patch.add_connection("osc-1", "out", "las-1", "in");
    // LA Synth -> Filter
    patch.add_connection("las-1", "out", "flt-1", "in");
    // Filter -> Amp
    patch.add_connection("flt-1", "out", "amp-1", "in");
    // Envelope -> Amp CV
    patch.add_connection("env-1", "out", "amp-1", "cv");
    // LFO -> Filter cutoff modulation
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    // Amp -> Stereo Output
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
