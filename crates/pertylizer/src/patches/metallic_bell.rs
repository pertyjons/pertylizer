//! Metallic Bell - Shimmering bell using wavetable through ring modulation.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Metallic Bell - Inharmonic, shimmering bell with wavetable + ring mod.
pub fn patch_metallic_bell() -> Patch {
    let mut patch = Patch::new("Metallic Bell");
    patch.author = Some("Pertylizer".to_string());
    patch.description = Some(
        "Shimmering metallic bell combining Digital wavetable with Ring Modulation.".to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
Wavetable Osc (Digital) -> Ring Mod (sine carrier, key tracking) -> Filter -> Amp -> Reverb -> Output

The Digital wavetable provides complex harmonics. The Ring Mod multiplies
this with a sine carrier at a non-integer frequency ratio, creating the
inharmonic sidebands that give bells their characteristic metallic shimmer.

Keyboard tracking on the Ring Mod ensures the timbre scales with pitch.
A percussive envelope with long release mimics a struck bell.

MODULATION:
- Env 1 -> Amplifier (percussive bell envelope)
- Env 2 -> Filter Cutoff (brightness decay)

TRY: Change freq_ratio for different metallic timbres.
Lower notes sound gong-like, higher notes chime-like.
"#
        .to_string(),
    );
    patch.tags = vec![
        "bell".into(),
        "metallic".into(),
        "ring_mod".into(),
        "wavetable".into(),
        "percussive".into(),
    ];

    // Wavetable Oscillator - Digital bank (wtb-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::WavetableOsc)
            .position(50.0, 50.0)
            .param_choice("table", "digital")
            .param_f("position", 0.3)
            .param_f("level", 0.8)
            .build(),
    );

    // Ring Mod - Sine carrier with keyboard tracking (rng-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::RingMod)
            .position(400.0, 50.0)
            .param_choice("carrier_wave", "sine")
            .param_f("freq_ratio", 0.65) // Non-integer ratio for inharmonics
            .param_f("key_track", 0.9)
            .param_f("mix", 0.7)
            .build(),
    );

    // Filter - Gentle lowpass to tame harsh highs (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(750.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 4000.0)
            .param_f("resonance", 0.15)
            .build(),
    );

    // Amp Envelope - Percussive with long release (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1150.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 1.8)
            .param_f("sustain", 0.05)
            .param_f("release", 2.5)
            .build(),
    );

    // Filter Envelope - Brightness on attack (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(750.0, 350.0)
            .param_f("attack", 0.001)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.1)
            .param_f("release", 0.5)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1150.0, 50.0)
            .param_f("level", 0.55)
            .build(),
    );

    // Reverb - Big space for bell (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(1900.0, 50.0)
            .param_f("room_size", 0.9)
            .param_f("damping", 0.2)
            .param_f("mix", 0.5)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1500.0, 50.0)
            .param_f("master level", 0.75)
            .build(),
    );

    // Connections
    patch.add_connection("wtb-1", "out", "rng-1", "in");
    patch.add_connection("rng-1", "out", "flt-1", "in");
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    patch
}
