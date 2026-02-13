//! Digital Chime - Crystalline chime with envelope-scanned wavetable.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Digital Chime - Sparkling digital chime with position sweep on attack.
pub fn patch_digital_chime() -> Patch {
    let mut patch = Patch::new("Digital Chime");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Crystalline chime using the Digital wavetable with envelope-driven position sweep."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (Digital) -> Filter -> Amplifier -> Chorus -> Reverb -> Output

The Digital wavetable contains FM, hard sync, bitcrush and ring mod-like
waveforms. An envelope sweeps through these on each note attack, creating
a complex timbral burst that settles into a pure tone.

The chorus adds stereo width and the reverb creates a shimmering tail.

MODULATION:
- Env 2 -> Wavetable Position CV (timbral sweep on attack)
- Env 1 -> Amplifier (percussive chime envelope)

TRY: Play high notes for glass-like chimes.
Adjust the position envelope decay for longer/shorter timbral sweeps.
"#
        .to_string(),
    );
    patch.tags = vec![
        "wavetable".into(),
        "digital".into(),
        "chime".into(),
        "percussive".into(),
        "experimental".into(),
    ];

    // Wavetable Oscillator - Digital bank (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "digital")
            .param_f("position", 0.0)
            .param_f("level", 0.75)
            .build(),
    );

    // Filter - Highpass to keep it sparkly (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(250.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 6000.0)
            .param_f("resonance", 0.2)
            .build(),
    );

    // Amp Envelope - Percussive chime (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 1.2)
            .param_f("sustain", 0.0)
            .param_f("release", 1.5)
            .build(),
    );

    // Position Envelope - Fast sweep through wavetable (env-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Envelope)
            .position(250.0, 300.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.4)
            .param_f("sustain", 0.0)
            .param_f("release", 0.2)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.6)
            .build(),
    );

    // Chorus - Stereo shimmer (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Chorus)
            .position(650.0, 50.0)
            .param_f("rate", 1.5)
            .param_f("depth", 0.4)
            .param_f("mix", 0.3)
            .build(),
    );

    // Reverb - Bright space (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Reverb)
            .position(650.0, 200.0)
            .param_f("room_size", 0.75)
            .param_f("damping", 0.15)
            .param_f("mix", 0.45)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(850.0, 50.0)
            .param_f("master", 0.75)
            .build(),
    );

    // Connections
    patch.add_connection("wtb-1", "out", "flt-1", "in");
    patch.add_connection("env-2", "out", "wtb-1", "pos_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
