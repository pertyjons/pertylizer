//! FM Bell - Bright, bell-like FM synthesis sound.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// FM Bell - Bright, bell-like FM synthesis sound.
pub fn patch_fm_bell() -> Patch {
    let mut patch = Patch::new("FM Bell");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some("Bright, metallic bell sound using FM synthesis.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
This patch creates a bell-like tone using FM (Frequency Modulation)
synthesis. In FM, one oscillator (the modulator) modulates the
frequency of another (the carrier), creating complex harmonics.

FM BASICS:
- Carrier (OSC1): Sine wave - the sound you actually hear
- Modulator (OSC2): Sine wave - modulates OSC1's frequency via AMP1
- AMP1 envelopes the modulator signal (ENV1 controls FM depth decay)
- When the enveloped modulator connects to OSC1's FM input,
  the carrier's frequency wobbles, creating sidebands (new harmonics)

BELL CHARACTER:
Bells have inharmonic partials (frequencies that aren't simple
multiples of the fundamental). FM naturally creates these when
the modulator and carrier have non-integer frequency ratios.

The amp envelope has instant attack and long decay, mimicking
how a struck bell rings and slowly fades.

REVERB:
Heavy reverb simulates the acoustic space where bells typically
exist (churches, towers) and extends the decay naturally.

TRY: Play single notes and let them ring. Higher notes sound
more "chime-like", lower notes more "gong-like".
"#
        .to_string(),
    );
    patch.tags = vec![
        "bell".into(),
        "fm".into(),
        "metallic".into(),
        "chime".into(),
    ];

    // OSC1 - Carrier (sine) (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscillator)
            .position(250.0, 50.0)
            .waveform("sine")
            .param_f("level", 0.7)
            .build(),
    );

    // OSC2 - Modulator (sine) (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sine")
            .param_f("level", 0.5)
            .param_f("detune", 2.0) // Slightly detuned for inharmonic partials
            .build(),
    );

    // Modulator Envelope - Controls FM depth (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.1)
            .param_f("release", 0.3)
            .build(),
    );

    // Amp Envelope - Bell-like (env-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 2.0)
            .param_f("sustain", 0.0)
            .param_f("release", 1.0)
            .build(),
    );

    // Modulator Amplifier - Envelopes FM depth (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(150.0, 50.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Carrier Amplifier (amp-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Reverb - Large space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Reverb)
            .position(650.0, 50.0)
            .param_f("room_size", 0.8)
            .param_f("damping", 0.2)
            .param_f("mix", 0.45)
            .build(),
    );

    // Oscilloscope - Waveform visualization (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscilloscope)
            .position(850.0, 50.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output - Final destination (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(1050.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections (using string IDs: type-instance)
    // FM routing: osc-2 -> amp-1 (envelope controls FM depth) -> osc-1 fm input
    patch.add_connection("osc-2", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out", "osc-1", "fm");
    // Carrier -> output amp -> stereo output (effects handled via effect chain)
    patch.add_connection("osc-1", "out", "amp-2", "in");
    patch.add_connection("env-2", "out", "amp-2", "cv");
    patch.add_connection("amp-2", "left", "out-1", "in_l");
    patch.add_connection("amp-2", "right", "out-1", "in_r");
    patch
}
