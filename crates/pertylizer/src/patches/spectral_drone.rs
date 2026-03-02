//! Spectral Drone — detuned saws through Frequency Shifter for inharmonic shimmer.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Spectral Drone — two detuned saws processed by a frequency shifter for metallic, evolving textures.
pub fn patch_spectral_drone() -> Patch {
    let mut patch = Patch::new("Spectral Drone");
    patch.author = Some("Pertylizer".to_string());
    patch.description = Some(
        "Evolving drone using detuned saws and Frequency Shifter for inharmonic spectral content."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Two detuned sawtooth oscillators are mixed together, creating a thick,
chorused base tone. This passes through a lowpass filter with moderate
resonance for warmth, then through the amp with a slow pad envelope.

The Frequency Shifter effect shifts all harmonics by a fixed Hz amount
(3 Hz), making them inharmonic — the overtones are no longer integer
multiples of the fundamental. This creates a metallic, bell-like,
continuously beating quality unique to frequency shifting.

FREQUENCY SHIFTER:
- Shift = 3 Hz (subtle beating between harmonics)
- Mode = stereo (up-shift on left, down-shift on right for wide image)
- Mix = 0.6 (blend shifted and original for depth)

Unlike pitch shifting, frequency shifting adds a CONSTANT Hz offset to
every harmonic. A 440 Hz fundamental becomes 443 Hz, but the 880 Hz
second harmonic becomes 883 Hz (not 886 Hz). This breaks the harmonic
series, creating the characteristic "barber pole" effect.

ENVELOPE DESIGN:
- Very slow attack (2s) for gradual swell
- Long decay (2s) to high sustain (0.85)
- Very long release (3s) for ambient fade

TRY: Increase Shift to 10-50 Hz for more extreme metallic effects.
Try Mode = 0 (up only) or 0.5 (down only) for different characters.
Set Shift to very small values (0.5-2 Hz) for subtle phasing/beating.
"#
        .to_string(),
    );
    patch.tags = vec![
        "drone".into(),
        "ambient".into(),
        "freq_shifter".into(),
        "experimental".into(),
    ];

    // Oscillator 1 - Sawtooth (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.7)
            .build(),
    );

    // Oscillator 2 - Sawtooth detuned (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 400.0)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .param_f("detune", -7.0)
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .build(),
    );

    // Filter - Lowpass for warmth (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1800.0)
            .param_f("resonance", 0.55)
            .build(),
    );

    // Amp Envelope - Slow drone envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 350.0)
            .param_f("attack", 2.0)
            .param_f("decay", 2.0)
            .param_f("sustain", 0.85)
            .param_f("release", 3.0)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1600.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Frequency Shifter effect — auto-routed, no manual connections
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::FrequencyShifter)
            .position(2000.0, 350.0)
            .param_f("shift", 3.0) // 3 Hz shift for subtle beating
            .param_f("mix", 0.6) // Blend with dry
            .param_f("mode", 1.0) // Stereo mode (up L, down R)
            .build(),
    );

    // Reverb for space
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(2000.0, 50.0)
            .param_f("mix", 0.4)
            .param_f("decay", 4.0)
            .build(),
    );

    // Connections
    // Both oscillators → Mixer
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    // Mixer → Filter → Amp → Output
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    // Note: FrequencyShifter and Reverb are auto-routed effects

    patch
}
