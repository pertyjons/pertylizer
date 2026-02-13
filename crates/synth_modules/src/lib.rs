//! Synth modules and audio effects for modular synthesizer.
//!
//! This crate provides ready-to-use audio modules:
//! - Oscillators (sine, saw, square, etc.)
//! - Sub-oscillator (bass reinforcement)
//! - Noise generator (textures and percussion)
//! - Filters (lowpass, highpass, bandpass)
//! - Envelopes (ADSR)
//! - LFOs (modulation sources)
//! - Amplifiers/VCAs (level control)
//! - Output (final stereo output)
//! - Physical modeling (strings, resonators, body)
//! - Effects (delay, reverb, distortion, chorus)

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]

pub mod amplifier;
pub mod envelope;
pub mod envelope_follower;
pub mod filter;
pub mod lfo;
pub mod math_oscillator;
pub mod mod_matrix;
pub mod noise;
pub mod oscillator;
pub mod output;
pub mod ring_mod;
pub mod sub_osc;
pub mod wavetable_data;
pub mod wavetable_osc;

// Physical modeling modules
pub mod body_resonance;
pub mod keyboard_panner;
pub mod mechanical_noise;

// Effects
pub mod effects;

// Module exports
pub use amplifier::{Amplifier, Mixer};
pub use envelope::{Envelope, EnvelopePositionBuffer, EnvelopeStage};
pub use envelope_follower::EnvelopeFollower;
pub use filter::{Filter, LadderFilter};
pub use lfo::Lfo;
pub use math_oscillator::MathOscillator;
pub use mod_matrix::ModMatrix;
pub use noise::NoiseGenerator;
pub use oscillator::Oscillator;
pub use output::StereoOutput;
pub use ring_mod::RingMod;
pub use sub_osc::SubOscillator;
pub use wavetable_osc::WavetableOsc;

// Physical modeling exports
pub use body_resonance::BodyResonance;
pub use keyboard_panner::KeyboardPanner;
pub use mechanical_noise::MechanicalNoise;

// Effects exports
pub use effects::{
    Chorus, Compressor, Delay, Distortion, DistortionType, Eq, Flanger, Phaser, Reverb, Waveshaper,
};

// Re-export param types from synth_core for convenience
pub use synth_core::{NoiseType, SubOscOctave, SubOscWaveform};
