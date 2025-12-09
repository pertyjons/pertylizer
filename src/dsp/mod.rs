//! Low-level DSP primitives for the modular synthesizer.
//!
//! This module contains reusable DSP building blocks that are used
//! by multiple modules and effects.
//!
//! ## Modules
//!
//! - [`oscillators`] - Band-limited waveform generation (PolyBLEP)
//! - [`filters`] - Filter algorithms (SVF, Biquad, Ladder)
//! - [`delay`] - Generic delay line with interpolation

pub mod delay;
pub mod filters;
pub mod oscillators;

pub use delay::{DelayLine, InterpolatedDelayLine};
pub use filters::{BiquadCoeffs, FilterType, SvfCoeffs};
pub use oscillators::{poly_blep, poly_blep_integrated};
