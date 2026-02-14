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

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::suboptimal_flops)]

pub mod delay;
pub mod filters;
pub mod oscillators;
pub mod spectral;

pub use delay::{DelayLine, InterpolatedDelayLine};
pub use filters::{
    AcidFilter, BiquadCoeffs, FluidFilter, ScreamerFilter, SvfCoeffs, SvfFilterType,
};
pub use oscillators::{poly_blep, poly_blep_integrated};
pub use spectral::{FftProcessor, PartitionedConvolver, StftProcessor, WindowType, fill_window};

// Re-export Complex for use by downstream crates (e.g. phase vocoder)
pub use realfft::num_complex::Complex;
