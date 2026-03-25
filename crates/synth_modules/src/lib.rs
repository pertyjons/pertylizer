//! Synth modules and audio effects for Pertylizer.
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

pub mod math;

pub mod additive_osc;
pub mod amplifier;
pub mod audio_input;
pub mod envelope;
pub mod envelope_follower;
pub mod euclidean;
pub mod filter;
pub mod fractal_osc;
pub mod granular_osc;
pub mod kinetic_modulator;
pub mod la_synth;
pub mod lfo;
pub mod math_oscillator;
pub mod mod_matrix;
pub mod mseg;
pub mod noise;
pub mod oscillator;
pub mod output;
pub mod pitch_tracker;
pub mod random_gates;
pub mod ring_mod;
pub mod sampler;
pub mod signal_monitor;
pub mod sub_osc;
pub mod turing_machine;
pub mod vector_mixer;
pub mod wavetable_data;
pub mod wavetable_osc;

// Physical modeling modules
pub mod body_resonance;
pub mod keyboard_panner;
pub mod mechanical_noise;

// Effects
pub mod effects;

// Module exports
pub use additive_osc::AdditiveOsc;
pub use amplifier::{Amplifier, Mixer};
pub use audio_input::AudioInput;
pub use envelope::{Envelope, EnvelopePositionBuffer, EnvelopeStage};
pub use envelope_follower::EnvelopeFollower;
pub use euclidean::Euclidean;
pub use filter::{Filter, LadderFilter};
pub use fractal_osc::FractalOscillator;
pub use granular_osc::GranularOsc;
pub use kinetic_modulator::KineticModulator;
pub use la_synth::LaSynth;
pub use lfo::Lfo;
pub use math_oscillator::MathOscillator;
pub use mod_matrix::ModMatrix;
pub use mseg::Mseg;
pub use noise::NoiseGenerator;
pub use oscillator::Oscillator;
pub use output::StereoOutput;
pub use pitch_tracker::PitchTracker;
pub use random_gates::RandomGates;
pub use ring_mod::RingMod;
pub use sampler::Sampler;
pub use signal_monitor::SignalMonitor;
pub use sub_osc::SubOscillator;
pub use turing_machine::TuringMachine;
pub use vector_mixer::VectorMixer;
pub use wavetable_osc::WavetableOsc;

// Physical modeling exports
pub use body_resonance::BodyResonance;
pub use keyboard_panner::KeyboardPanner;
pub use mechanical_noise::MechanicalNoise;

// Effects exports
pub use effects::{
    BbdDelay, Chorus, Compressor, Convolver, Delay, Distortion, DistortionType, Eq, Flanger,
    FrequencyShifter, Limiter, MidSide, PhaseVocoder, Phaser, Reverb, Waveshaper,
};

// Re-export param types from synth_core for convenience
pub use synth_core::{NoiseType, SubOscOctave, SubOscWaveform};
