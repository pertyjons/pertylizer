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

// Re-export param types from engine for convenience
pub use crate::engine::typed_params::{NoiseType, SubOscOctave, SubOscWaveform};
