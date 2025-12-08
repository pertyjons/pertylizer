//! Grand Piano - Acoustic piano simulation with velocity response.

use crate::patch::{ModuleBuilder, ModuleType, Patch};

/// Grand Piano - Rich acoustic piano with velocity-sensitive dynamics and key tracking.
pub fn patch_grand_piano() -> Patch {
    let mut patch = Patch::new("Grand Piano");
    patch.author = Some("Modular Synth".to_string());
    patch.description = Some(
        "Dynamic acoustic piano with velocity-sensitive hammer attack, \
         key tracking, and body resonance."
            .to_string(),
    );
    patch.tags = vec![
        "piano".into(),
        "keys".into(),
        "acoustic".into(),
        "classic".into(),
        "velocity".into(),
    ];

    patch.notes = Some(
        r#"
GRAND PIANO - Acoustic Piano Simulation

SIGNAL FLOW:
Emulates a grand piano using layered oscillators, velocity-sensitive filtering,
and subtle hammer noise for realistic attack.

1. SOURCES:
   - Osc 1 (Sawtooth): Main string body with slight detune (-3 cents)
   - Osc 2 (Sawtooth): Secondary string, detuned opposite (+3 cents)
   - Sub Osc (Triangle): Low-end body resonance (-1 octave, subtle)
   - Noise: Hammer strike transient (very short burst)

2. FILTERING:
   - Lowpass filter with fast envelope for "hammer" sound
   - Key tracking: higher notes are brighter (like real piano)
   - Filter envelope with velocity sensitivity for dynamics

3. DYNAMICS:
   - Velocity sensitivity on both amp and filter envelopes
   - Natural decay envelope (no sustain plateau)
   - Quick release when key lifted

4. EFFECTS:
   - Reverb: Simulates piano body and room acoustics

PLAYING TIPS:
- Play softly for mellow jazz tones
- Play hard for bright, percussive attack
- Higher notes will naturally sound brighter
- Works great with sustain pedal
"#
        .to_string(),
    );

    // ========================================================================
    // SOUND SOURCES
    // ========================================================================

    // OSC 1: Main String (Sawtooth - rich harmonics)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .position(50.0, 100.0)
            .waveform("sawtooth")
            .param_f("detune", -3.0) // Slight detune left
            .param_f("level", 0.65)
            .build(),
    );

    // OSC 2: Secondary String (Sawtooth - detuned for chorus)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Oscillator)
            .position(50.0, 250.0)
            .waveform("sawtooth")
            .param_f("detune", 3.0) // Slight detune right
            .param_f("level", 0.55)
            .build(),
    );

    // SUB OSC: Body resonance (Triangle -1 octave, subtle)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::SubOscillator)
            .position(50.0, 400.0)
            .param_choice("waveform", "triangle")
            .param_choice("octave", "minus1")
            .param_f("level", 0.25) // Subtle, not boomy
            .build(),
    );

    // NOISE: Hammer strike transient
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .position(50.0, 550.0)
            .param_choice("type", "white")
            .param_f("level", 0.4) // Will be shaped by very short envelope
            .build(),
    );

    // ========================================================================
    // ENVELOPES
    // ========================================================================

    // ENV 1: Amplitude (main volume envelope with velocity)
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .position(250.0, 50.0)
            .param_f("attack", 0.003) // Near-instant attack
            .param_f("decay", 2.5)    // Long natural decay
            .param_f("sustain", 0.0)  // No sustain plateau (piano-like)
            .param_f("release", 0.35) // Release when key up
            .param_f("velocity_sensitivity", 0.8) // Velocity affects amplitude
            .param_f("attack_curve", -0.3)
            .param_f("decay_curve", -0.4)  // Natural exponential decay
            .param_f("release_curve", -0.3)
            .build(),
    );

    // ENV 2: Filter envelope (hammer brightness with velocity)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Envelope)
            .position(250.0, 250.0)
            .param_f("attack", 0.001)  // Instant
            .param_f("decay", 0.2)     // Quick decay for percussive feel
            .param_f("sustain", 0.0)
            .param_f("release", 0.15)
            .param_f("velocity_sensitivity", 0.7) // Velocity opens filter more
            .param_f("attack_curve", -0.5)
            .param_f("decay_curve", -0.5)
            .param_f("release_curve", -0.3)
            .build(),
    );

    // ENV 3: Noise envelope (hammer click - very short)
    patch.add_module(
        ModuleBuilder::new(3, ModuleType::Envelope)
            .position(250.0, 450.0)
            .param_f("attack", 0.001)  // Instant
            .param_f("decay", 0.012)   // ~12ms click
            .param_f("sustain", 0.0)
            .param_f("release", 0.008)
            .param_f("velocity_sensitivity", 0.6) // Harder = more click
            .param_f("attack_curve", -0.8)
            .param_f("decay_curve", -0.7)
            .param_f("release_curve", -0.5)
            .build(),
    );

    // ========================================================================
    // FILTER
    // ========================================================================

    // FILTER: Lowpass with key tracking
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .position(450.0, 200.0)
            .filter_mode("lowpass")
            .param_f("cutoff", 2000.0)     // Base cutoff
            .param_f("resonance", 0.05)    // Very low resonance (piano is not resonant)
            .param_f("env_amount", 0.7)    // Envelope opens filter for attack
            .param_f("key_tracking", 0.6)  // Higher notes = brighter
            .build(),
    );

    // ========================================================================
    // AMPLIFIERS & MIXER
    // ========================================================================

    // NOISE VCA: Controlled by ENV 3 (hammer click)
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Amplifier)
            .position(150.0, 550.0)
            .param_f("level", 0.15) // Subtle click level
            .build(),
    );

    // MIXER: Combine all oscillator sources
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Mixer)
            .position(350.0, 350.0)
            .param_f("level", 1.0)
            .build(),
    );

    // MAIN VCA: Final volume
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .position(650.0, 200.0)
            .param_f("level", 0.8)
            .build(),
    );

    // ========================================================================
    // EFFECTS
    // ========================================================================

    // REVERB: Piano body and room
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Reverb)
            .position(850.0, 200.0)
            .param_f("room_size", 0.4)  // Medium room
            .param_f("damping", 0.35)   // Slightly bright
            .param_f("mix", 0.18)       // Subtle, not washy
            .build(),
    );

    // OUTPUT
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .position(1050.0, 200.0)
            .param_f("master", 0.85)
            .build(),
    );

    // ========================================================================
    // CONNECTIONS
    // ========================================================================

    // --- Noise path (hammer click) ---
    patch.add_connection("nse-1", "out", "amp-2", "in");
    patch.add_connection("env-3", "out", "amp-2", "cv");

    // --- Mix oscillators into mixer ---
    patch.add_connection("osc-1", "out", "mix-1", "in1");
    patch.add_connection("osc-2", "out", "mix-1", "in2");
    patch.add_connection("sub-1", "out", "mix-1", "in3");
    patch.add_connection("amp-2", "out", "mix-1", "in4"); // Noise via its VCA

    // --- Main signal chain ---
    patch.add_connection("mix-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch.add_connection("amp-1", "out", "rev-1", "in");

    // --- Stereo output from reverb ---
    patch.add_connection("rev-1", "out_l", "out-1", "in_l");
    patch.add_connection("rev-1", "out_r", "out-1", "in_r");

    // --- Modulation ---
    patch.add_connection("env-1", "out", "amp-1", "cv");        // Volume envelope
    patch.add_connection("env-2", "out", "flt-1", "cutoff_cv"); // Filter envelope

    patch
}
