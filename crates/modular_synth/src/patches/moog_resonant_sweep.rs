//! Moog Resonant Sweep - Fat analog lead/bass with filter envelope sweep.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Moog Resonant Sweep - Fat analog lead/bass with filter envelope sweep.
pub fn patch_moog_resonant_sweep() -> Patch {
    let mut patch = Patch::new("Moog Resonant Sweep");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Fat Moog-style lead with resonant filter sweep, sub bass, and LFO wobble.".to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Two detuned sawtooth oscillators create the classic thick Moog sound.
OSC2 is detuned +12 cents from OSC1 for beating/chorus between them.
A square sub-oscillator one octave down adds massive low-end weight.

All three are mixed and fed through a Ladder filter (Moog-style 24dB/oct
lowpass). The filter starts at 500 Hz with 0.55 resonance for a pronounced
"peak" at the cutoff frequency.

FILTER SWEEP ("WOOOW"):
The filter envelope sweeps the cutoff up by 6000 Hz with a 700ms decay,
creating the signature "wooow" attack sound. Low sustain (0.15) means
the filter closes back down after each note's initial burst.

A triangle LFO at 1.8 Hz adds periodic wobble to the cutoff, creating
the rhythmic "wow wow wow" movement between notes.

Drive at 1.8 adds warm Moog-style saturation through the filter.

EFFECTS:
Chorus thickens the sound further. A subtle reverb (12% mix) adds space
without washing out the tight filter character.

TRY: Play single notes and hold them to hear the filter sweep. Low notes
(C2-C3) sound massive. Upper register (C4-C5) gets the classic Moog lead
bite. Works great for both bass lines and lead melodies.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "bass".into(),
        "moog".into(),
        "analog".into(),
        "filter".into(),
        "resonant".into(),
    ];

    // OSC1 - Sawtooth, main (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.85)
            .build(),
    );

    // OSC2 - Sawtooth, detuned +12 cents (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 200.0)
            .waveform("sawtooth")
            .param_f("level", 0.75)
            .param_f("detune", 12.0)
            .build(),
    );

    // Sub Oscillator - Square, -1 octave (sub-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SubOscillator)
            .position(50.0, 350.0)
            .waveform("square")
            .param_f("octave", -1.0)
            .param_f("level", 0.5)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(250.0, 100.0)
            .param_f("master", 0.9)
            .param_f("input_1", 0.8)
            .param_f("input_2", 0.7)
            .param_f("input_3", 0.5)
            .build(),
    );

    // Filter - Ladder LP (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 100.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 500.0)
            .param_f("resonance", 0.55)
            .param_f("key_track", 0.8)
            .param_f("drive", 1.8)
            .param_f("cv_amt", 6000.0)
            .build(),
    );

    // Filter Envelope - slow sweep (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(250.0, 350.0)
            .param_f("attack", 0.005)
            .param_f("decay", 0.7)
            .param_f("sustain", 0.15)
            .param_f("release", 0.5)
            .param_f("vel_sens", 0.6)
            .build(),
    );

    // Amp Envelope (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(450.0, 350.0)
            .param_f("attack", 0.01)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.85)
            .param_f("release", 0.5)
            .param_f("vel_sens", 0.7)
            .build(),
    );

    // LFO - Triangle wobble (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(650.0, 350.0)
            .waveform("triangle")
            .param_f("rate", 1.8)
            .param_f("depth", 0.3)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(650.0, 100.0)
            .param_f("level", 0.95)
            .build(),
    );

    // Chorus (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(850.0, 100.0)
            .param_f("rate", 0.8)
            .param_f("depth", 0.35)
            .param_f("mix", 0.25)
            .build(),
    );

    // Reverb (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1050.0, 100.0)
            .param_f("decay", 2.0)
            .param_f("mix", 0.15)
            .param_f("damping", 0.6)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1250.0, 100.0)
            .param_f("master level", 0.75)
            .build(),
    );

    // Connections
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    patch.add_connection("sub-1", "out", "mix-1", "in3");
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("env-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("lfo-1", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-2", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
