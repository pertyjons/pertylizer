//! Resonant Percussion — hammer noise through modal resonators with reverse/gate reverb.

use crate::patch::{Author, ModuleBuilder, Patch};
use synth_core::ModuleType;

/// Resonant Percussion — MechanicalNoise excites ModalResonator with ReverseGateReverb drama.
pub fn patch_resonant_percussion() -> Patch {
    let mut patch = Patch::new("Resonant Percussion");
    patch.author = Some(Author::from("Pertylizer"));
    patch.description = Some(
        "Hammer-style mechanical noise bursts excite modal resonant frequencies, \
         processed through EQ shaping and dramatic reverse/gate reverb."
            .to_string(),
    );
    patch.notes = Some(
        r#"
SIGNAL FLOW:
A Mechanical Noise module generates short hammer-strike bursts — brief,
broadband noise impulses that simulate physical excitation. This feeds
through a fast percussive envelope into the amplifier.

EFFECTS CHAIN (auto-routed):
1. EQ — shapes the excitation spectrum, boosting mids for body
2. Modal Resonator — 12 resonant modes tuned to base note, creating
   pitched metallic/wooden percussion tones from the noise
3. Reverse/Gate Reverb — dramatic reverse reverb swells that build
   up before each hit, creating an otherworldly effect

MECHANICAL NOISE:
- Type = Hammer (short broadband impact)
- Duration = 15ms (crisp transient)
- Cutoff = 5000 Hz (bright excitation)
- Velocity Sensitivity = 0.8 (dynamic response)

MODAL RESONATOR:
- Base Note = 60 (middle C, tracks keyboard)
- 12 modes with moderate spread and decay
- Brightness = 0.6 for clear harmonics

TRY: Change MechanicalNoise type to KeyDown for softer attacks.
Lower ModalResonator base note for deeper, more bell-like tones.
Switch ReverseGateReverb mode to Gate for rhythmic gating.
"#
        .to_string(),
    );
    patch.tags = vec![
        "drums".into(),
        "percussion".into(),
        "modal".into(),
        "mechanical".into(),
        "experimental".into(),
    ];

    patch.settings.octave_offset = -1;

    // Mechanical Noise - hammer strikes (mec-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MechanicalNoise)
            .position(50.0, 50.0)
            .param_choice("type", "hammer")
            .param_f("duration", 15.0)
            .param_f("cutoff", 5000.0)
            .param_f("vel sens", 0.8)
            .param_f("level", 0.7)
            .build(),
    );

    // Fast percussive envelope (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(450.0, 400.0)
            .param_f("attack", 0.001)
            .param_f("decay", 0.3)
            .param_f("sustain", 0.0)
            .param_f("release", 0.2)
            .param_f("atk_curve", -0.5)
            .param_f("dec_curve", -0.4)
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(450.0, 50.0)
            .param_f("level", 0.8)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(850.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // === Effects (auto-routed) ===

    // EQ — shape excitation spectrum (equ-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Eq)
            .position(1250.0, 50.0)
            .param_f("low freq", 100.0)
            .param_f("low gain", -2.0)
            .param_f("mid freq", 1200.0)
            .param_f("mid gain", 4.0)
            .param_f("mid q", 2.0)
            .param_f("high freq", 8000.0)
            .param_f("high gain", 2.0)
            .build(),
    );

    // Modal Resonator — pitched resonance from noise (mdr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModalResonator)
            .position(1250.0, 400.0)
            .param_f("base note", 60.0)
            .param_f("spread", 0.35)
            .param_f("modes", 12.0)
            .param_f("decay", 0.65)
            .param_f("brightness", 0.6)
            .param_f("mix", 0.85)
            .build(),
    );

    // Reverse/Gate Reverb — dramatic reverse swells (rgr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ReverseGateReverb)
            .position(1650.0, 50.0)
            .param_f("window", 800.0)
            .param_choice("mode", "reverse")
            .param_choice("trigger", "threshold")
            .param_f("threshold", -30.0)
            .param_f("gate time", 200.0)
            .param_f("mix", 0.5)
            .build(),
    );

    // === Connections ===
    // MechanicalNoise → Amp → Output
    patch.add_connection("mec-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group("Exciter", Some("#D96A4A"), &["mec-1", "env-1"]);
    patch.add_group("Output", Some("#4A9D8F"), &["amp-1", "out-1"]);
    patch.add_group("Effects", Some("#8B6BAE"), &["equ-1", "mdr-1", "rgr-1"]);

    patch
}
