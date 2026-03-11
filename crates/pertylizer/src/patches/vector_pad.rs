//! Vector Pad — four-oscillator pad with LFO-driven vector crossfading.

use crate::patch::{ModuleBuilder, Patch, PatchAuthor};
use synth_core::ModuleType;

/// Vector Pad — four distinct waveforms blended via XY vector mixer with slow LFO sweep.
pub fn patch_vector_pad() -> Patch {
    let mut patch = Patch::new("Vector Pad");
    patch.author = Some(PatchAuthor::from("Pertylizer"));
    patch.description = Some(
        "Evolving pad using Vector Mixer to crossfade four oscillators via dual LFO modulation."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Four oscillators with different waveforms feed into the Vector Mixer's
four corners (A=saw, B=triangle, C=square, D=triangle+detune). Two slow LFOs
modulate the X and Y position, creating a continuously evolving timbre as
the mixer sweeps between the four sources.

VECTOR MIXER:
- Input A (top-left): Sawtooth — bright, buzzy
- Input B (top-right): Triangle — soft, warm
- Input C (bottom-left): Square — hollow, reedy
- Input D (bottom-right): Triangle detuned — chorused, ethereal

The two LFOs run at different rates (0.15 Hz and 0.23 Hz) creating
a non-repeating Lissajous-like pattern through the timbral space.

After the vector mixer, the signal passes through a lowpass filter
with moderate resonance for warmth.

MODULATION:
- LFO 1 (sine, 0.15 Hz) → Vector X axis
- LFO 2 (sine, 0.23 Hz) → Vector Y axis
- The irrational ratio means the pattern never exactly repeats

ENVELOPE DESIGN:
- Slow attack (800ms) for pad-like swell
- Long decay (1s) to high sustain (0.8)
- Very long release (2s) for ambient tail

TRY: Change oscillator waveforms for wildly different timbral journeys.
Speed up the LFOs for more dramatic morphing. Add reverb for lush ambience.
"#
        .to_string(),
    );
    patch.tags = vec!["pad".into(), "vector".into(), "evolving".into()];

    // Oscillator A - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 350.0)
            .waveform("sawtooth")
            .param_f("level", 0.8)
            .build(),
    );

    // Oscillator B - Triangle (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 700.0)
            .waveform("triangle")
            .param_f("level", 0.8)
            .build(),
    );

    // Oscillator C - Square (osc-3)
    patch.add_module(
        ModuleBuilder::new(3, ModuleType::Oscillator)
            .position(50.0, 1400.0)
            .waveform("square")
            .param_f("detune", -5.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Oscillator D - Triangle detuned (osc-4)
    patch.add_module(
        ModuleBuilder::new(4, ModuleType::Oscillator)
            .position(50.0, 1050.0)
            .waveform("triangle")
            .param_f("detune", 2.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Vector Mixer (vec-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::VectorMixer)
            .position(1200.0, 350.0)
            .param_f("x", 0.5)
            .param_f("y", 0.5)
            .build(),
    );

    // LFO 1 - Slow sine for X axis (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 1950.0)
            .param_f("rate", 0.15)
            .param_f("depth", 1.0)
            .build(),
    );

    // LFO 2 - Slow sine for Y axis (lfo-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Lfo)
            .position(50.0, 1750.0)
            .param_f("rate", 0.23)
            .param_f("depth", 1.0)
            .build(),
    );

    // Filter (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(50.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 3000.0)
            .param_f("resonance", 0.25)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.8)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.8)
            .param_f("release", 2.0)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(800.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Reverb for lush ambience
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1200.0, 50.0)
            .param_f("mix", 0.35)
            .param_f("decay", 3.0)
            .build(),
    );

    // Connections
    // Four oscillators → Vector Mixer corners
    patch.add_connection("osc-1", "out", "vec-1", "in_a");
    patch.add_connection("osc-2", "out", "vec-1", "in_b");
    patch.add_connection("osc-3", "out", "vec-1", "in_c");
    patch.add_connection("osc-4", "out", "vec-1", "in_d");
    // LFOs → Vector Mixer XY
    patch.add_connection("lfo-1", "out", "vec-1", "x_cv");
    patch.add_connection("lfo-2", "out", "vec-1", "y_cv");
    // Vector Mixer → Filter → Amp → Output
    patch.add_connection("vec-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
