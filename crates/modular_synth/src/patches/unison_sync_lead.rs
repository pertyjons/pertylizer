//! Unison Sync Lead - Aggressive sync lead with tight unison for cutting solos.

use crate::patch::{ModuleBuilder, Patch, PatchModuleType};

/// Unison Sync Lead - Hard sync combined with unison for a cutting, aggressive lead.
pub fn patch_unison_sync_lead() -> Patch {
    let mut patch = Patch::new("Unison Sync Lead");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Aggressive hard-sync lead with 3-voice tight unison and waveshaper distortion."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Osc 1 (Slave, 3-voice unison saw) --sync from--> Osc 2 (Master, square)
  -> Waveshaper (Soft Clip) -> Filter (Lowpass) -> Amplifier -> Output

TECHNIQUE:
Hard sync locks the slave oscillator's phase to the master. The slave
(osc-1) has 3-voice unison with tight detune (6 cents) and zero spread
(mono). This creates a thick, focused sync tone without stereo widening
— perfect for a cutting mono lead.

The waveshaper adds soft-clip saturation for extra grit, and the
envelope-modulated filter sweeps the sync harmonic content.

Osc 2 runs at a slightly different frequency to create the characteristic
sync sweep. The Mod Matrix routes the filter envelope to osc-1 frequency
for timbral movement.

KEY DESIGN:
- Uni Spread = 0: Mono unison for focused, centered lead
- Uni Phase = 0: Coherent phase on note-on for punchy, consistent attack
- Tight detune (6 cents): Thickens without muddying the sync harmonics

TRY: Modulate osc-1 frequency with an LFO for evolving sync tones.
Increase unison detune for a rougher, more aggressive texture.
"#
        .to_string(),
    );
    patch.tags = vec![
        "lead".into(),
        "sync".into(),
        "unison".into(),
        "aggressive".into(),
        "mono".into(),
    ];

    // Slave Oscillator - 3-voice unison saw with sync input (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Oscillator)
            .position(50.0, 50.0)
            .waveform("sawtooth")
            .param_f("level", 0.85)
            .param_f("unison", 3.0)
            .param_f("uni detune", 6.0)
            .param_f("uni spread", 0.0) // Mono — focused center
            .param_f("uni phase", 0.0) // No randomization — punchy attack
            .build(),
    );

    // Master Oscillator - Square wave for sync (osc-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Oscillator)
            .position(50.0, 200.0)
            .waveform("square")
            .param_f("level", 0.0) // Silent — only used as sync source
            .build(),
    );

    // Waveshaper - Soft clip saturation (wsh-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Waveshaper)
            .position(250.0, 50.0)
            .waveshaper_curve("soft_clip")
            .param_f("drive", 0.5)
            .param_f("mix", 0.7)
            .param_f("bias", 0.0)
            .param_f("symmetry", 0.0)
            .build(),
    );

    // Filter - Lowpass, envelope-controlled (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Filter)
            .position(450.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 1200.0)
            .param_f("resonance", 0.25)
            .build(),
    );

    // Amp Envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Envelope)
            .position(50.0, 400.0)
            .param_f("attack", 0.002)
            .param_f("decay", 0.2)
            .param_f("sustain", 0.75)
            .param_f("release", 0.15)
            .build(),
    );

    // Filter Envelope - Opens filter aggressively on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, PatchModuleType::Envelope)
            .position(250.0, 400.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.1)
            .param_f("release", 0.1)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::Amplifier)
            .position(650.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Mod Matrix - Env2 -> Filter Cutoff (mmx-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::ModMatrix)
            .position(450.0, 400.0)
            .param_choice("grid_size", "1x1")
            .param_choice("slot_0_source", "env2")
            .param_choice("slot_0_destination", "filter1_cutoff")
            .param_f("slot_0_amount", 0.8)
            .param_b("slot_0_enabled", true)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, PatchModuleType::StereoOutput)
            .position(850.0, 50.0)
            .param_f("master", 0.7)
            .build(),
    );

    // Connections
    patch.add_connection("osc-2", "out", "osc-1", "sync"); // Hard sync
    patch.add_connection("osc-1", "out", "wsh-1", "in");
    patch.add_connection("wsh-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
