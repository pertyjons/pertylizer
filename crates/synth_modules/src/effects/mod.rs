//! Audio effects modules.
//!
//! This module contains effect processors that operate on the mixed
//! audio output:
//! - BBD Delay (analog bucket-brigade emulation)
//! - Delay (mono, stereo, ping-pong)
//! - Reverb (Schroeder algorithm)
//! - Distortion (soft clip, hard clip, foldback, bitcrush)
//! - Chorus
//! - Phaser (cascaded all-pass filters)
//! - Flanger (modulated delay with feedback)
//! - Compressor (dynamics processor)
//! - EQ (3-band parametric equalizer)
//! - Mid/Side (stereo width and mid/side gain processing)

pub mod bbd_delay;
pub mod chorus;
pub mod compressor;
pub mod convolver;
pub mod crossover;
pub mod delay;
pub mod distortion;
pub mod ensemble_chorus;
pub mod eq;
pub mod flanger;
pub mod frequency_shifter;
pub mod granular_fx;
pub mod limiter;
pub mod mid_side;
pub mod modal_resonator;
pub mod phase_vocoder;
pub mod phaser;
pub mod reverb;
pub mod reverse_gate_reverb;
pub mod shimmer_reverb;
pub mod spectral_blur;
pub mod tilt_eq;
pub mod univibe;
pub mod vocoder;
pub mod waveshaper;

pub use bbd_delay::BbdDelay;
pub use chorus::Chorus;
pub use compressor::Compressor;
pub use convolver::Convolver;
pub use crossover::CrossoverSplitter;
pub use delay::Delay;
pub use distortion::{Distortion, DistortionType};
pub use ensemble_chorus::EnsembleChorus;
pub use eq::Eq;
pub use flanger::Flanger;
pub use frequency_shifter::FrequencyShifter;
pub use granular_fx::GranularFx;
pub use limiter::Limiter;
pub use mid_side::MidSide;
pub use modal_resonator::ModalResonator;
pub use phase_vocoder::PhaseVocoder;
pub use phaser::Phaser;
pub use reverb::Reverb;
pub use reverse_gate_reverb::ReverseGateReverb;
pub use shimmer_reverb::ShimmerReverb;
pub use spectral_blur::SpectralBlur;
pub use tilt_eq::TiltEq;
pub use univibe::Univibe;
pub use vocoder::Vocoder;
pub use waveshaper::Waveshaper;
