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

pub mod core;
pub mod oscillator;
pub mod math_oscillator;
pub mod sub_osc;
pub mod noise;
pub mod filter;
pub mod envelope;
pub mod lfo;
pub mod amplifier;
pub mod output;

pub use core::*;
pub use oscillator::Oscillator;
pub use math_oscillator::MathOscillator;
pub use sub_osc::{SubOscillator, SubOscWaveform, SubOscOctave};
pub use noise::{NoiseGenerator, NoiseType};
pub use filter::{Filter, LadderFilter};
pub use envelope::{Envelope, EnvelopeStage};
pub use lfo::Lfo;
pub use amplifier::{Amplifier, Mixer};
pub use output::StereoOutput;
