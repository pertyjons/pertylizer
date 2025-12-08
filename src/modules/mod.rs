//! Synthesis modules.
//!
//! This module contains all the building blocks for the modular synthesizer:
//! - Oscillators (sound sources)
//! - Sub-oscillator (bass reinforcement)
//! - Noise generator (textures and percussion)
//! - Filters (tone shaping)
//! - Envelopes (amplitude/modulation control)
//! - LFOs (modulation sources)
//! - Amplifiers/VCAs (level control)
//! - Output (final stereo output)
//! - Physical modeling (strings, resonators, body)

pub mod amplifier;
pub mod core;
pub mod envelope;
pub mod filter;
pub mod lfo;
pub mod math_oscillator;
pub mod noise;
pub mod oscillator;
pub mod output;
pub mod sub_osc;

// Physical modeling modules
pub mod body_resonance;
pub mod keyboard_panner;
pub mod mechanical_noise;
pub mod resonator_bank;
pub mod string_resonator;
pub mod velocity_mapper;

pub use amplifier::{Amplifier, Mixer};
pub use core::*;
pub use envelope::{Envelope, EnvelopeStage};
pub use filter::{Filter, LadderFilter};
pub use lfo::Lfo;
pub use math_oscillator::MathOscillator;
pub use noise::NoiseGenerator;
pub use oscillator::Oscillator;
pub use output::StereoOutput;
pub use sub_osc::SubOscillator;

// Physical modeling exports
pub use body_resonance::BodyResonance;
pub use keyboard_panner::KeyboardPanner;
pub use mechanical_noise::MechanicalNoise;
pub use resonator_bank::ResonatorBank;
pub use string_resonator::StringResonator;
pub use velocity_mapper::VelocityMapper;

// Re-export param types from engine for convenience
pub use crate::engine::typed_params::{NoiseType, SubOscOctave, SubOscWaveform};
