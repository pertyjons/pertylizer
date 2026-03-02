//! String Ensemble - Lush, orchestral strings.

use crate::patch::{ModuleBuilder, Patch};
use synth_core::ModuleType;

/// String Ensemble - Lush, orchestral strings using SuperSaw algorithm.
pub fn patch_string_ensemble() -> Patch {
    let mut patch = Patch::new("String Ensemble");
    patch.author = Some("Pertylizer".to_string());
    patch.description =
        Some("Rich, orchestral string section with vibrato and hall reverb.".to_string());
    patch.notes = Some(
        r#"
SIGNAL FLOW:
To simulate a section of violins playing together (an ensemble), we need multiple
sound sources slightly detuned from each other.

CORE SOUND:
1. Math Oscillator (SuperSaw): This algorithm generates multiple detuned sawtooth
   waves simultaneously.
   - Param A (Spread): Controls how detuned they are.
   - Param B (Count): Controls how many virtual oscillators are playing.

2. Sub Oscillator (Sawtooth): Adds body and warmth one octave down, acting
   like the cellos/basses in the orchestra.

FILTERING:
A Lowpass filter gently rolls off the harsh high frequencies of the sawtooth
waves. The cutoff is modulated by an envelope to give a slight "bowing"
articulation at the start of the note.

MODULATION:
- LFO1 creates a classic vibrato (pitch modulation) at ~5-6 Hz.
- Amp Envelope has a slow attack (bowing in) and long release (natural hall decay).

EFFECTS:
- Chorus: Crucial for the "ensemble" feel, widening the stereo image.
- Reverb: Simulates a large concert hall.

TRY: Play chords! This patch shines with slow, sustained harmonies.
"#
        .to_string(),
    );
    patch.tags = vec![
        "strings".into(),
        "orchestral".into(),
        "pad".into(),
        "ensemble".into(),
    ];

    // OSC1 - SuperSaw for the main ensemble sound (mth-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::MathOscillator)
            .position(50.0, 50.0)
            .algorithm("super_saw")
            .param_f("param_a", 0.3) // Spread (Detune amount)
            .param_f("param_b", 0.8) // Count (High density)
            .param_f("param_c", 0.5)
            .param_f("level", 0.7)
            .build(),
    );

    // OSC2 - Regular Sawtooth for body/center definition (osc-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 300.0)
            .waveform("sawtooth")
            .param_f("level", 0.4)
            .param_f("detune", -5.0) // Slight detune against the supersaw
            .build(),
    );

    // Mixer (mix-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(450.0, 50.0)
            .param_f("master", 0.8)
            .build(),
    );

    // Filter - Lowpass to tame brightness (flt-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(850.0, 50.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2500.0) // Open enough for brilliance
            .param_f("resonance", 0.1) // Low resonance for natural sound
            .build(),
    );

    // Amp Envelope - Slow bowing attack (env-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(1250.0, 350.0)
            .param_f("attack", 0.6) // Slow attack (bowing)
            .param_f("decay", 0.5)
            .param_f("sustain", 0.8)
            .param_f("release", 1.2) // Long release
            .build(),
    );

    // Filter Envelope - Subtle timbral movement (env-2)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(850.0, 350.0)
            .param_f("attack", 0.4)
            .param_f("decay", 1.0)
            .param_f("sustain", 0.5)
            .param_f("release", 1.0)
            .build(),
    );

    // Vibrato LFO (lfo-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .position(50.0, 650.0)
            .waveform("sine")
            .param_f("rate", 5.5) // Classic vibrato speed
            .param_f("depth", 0.03) // Subtle depth
            .build(),
    );

    // Amplifier (amp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(1250.0, 50.0)
            .param_f("level", 0.7)
            .build(),
    );

    // Chorus - The "Ensemble" effect (chr-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Chorus)
            .position(2000.0, 50.0)
            .param_f("rate", 0.4)
            .param_f("depth", 0.5)
            .param_f("mix", 0.5)
            .build(),
    );

    // Reverb - Concert Hall (rev-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(2000.0, 300.0)
            .param_f("room_size", 0.85) // Large hall
            .param_f("damping", 0.4)
            .param_f("mix", 0.4)
            .build(),
    );

    // Oscilloscope (scp-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscilloscope)
            .position(2000.0, 600.0)
            .param_f("time", 1.0)
            .param_f("gain", 1.0)
            .build(),
    );

    // Stereo Output (out-1)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1600.0, 50.0)
            .param_f("master level", 0.8)
            .build(),
    );

    // Connections
    patch.add_connection("mth-1", "out", "mix-1", "in1");
    patch.add_connection("osc-1", "out", "mix-1", "in2");

    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");

    // Modulation
    patch.add_connection("env-1", "out", "amp-1", "cv"); // Volume envelope
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv"); // Filter envelope
    patch.add_connection("lfo-1", "out", "mth-1", "fm"); // Vibrato to SuperSaw
    patch.add_connection("lfo-1", "out", "osc-1", "fm"); // Vibrato to Saw

    // Voice output: amp -> stereo output (effects handled via effect chain)
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");

    // Groups
    patch.add_group(
        "Voice",
        Some("#4A90D9"),
        &["mth-1", "osc-1", "mix-1", "flt-1", "amp-1"],
    );
    patch.add_group("Modulation", Some("#50C878"), &["env-1", "env-2", "lfo-1"]);

    patch
}
