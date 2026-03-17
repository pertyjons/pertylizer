//! Low-level DSP primitives for Pertylizer.
//!
//! This module contains reusable DSP building blocks that are used
//! by multiple modules and effects.
//!
//! ## Modules
//!
//! - [`oscillators`] - Band-limited waveform generation (PolyBLEP)
//! - [`filters`] - Filter algorithms (SVF, Biquad, Ladder)
//! - [`delay`] - Generic delay line with interpolation

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::suboptimal_flops)]

pub mod delay;
pub mod fdn;
pub mod filters;
pub mod oscillators;
pub mod oversampling;
pub mod spectral;

pub use delay::{DelayLine, InterpolatedDelayLine};
pub use fdn::{FdnCore, FdnStereoOutput};
pub use filters::{
    AcidFilter, BiquadCoeffs, FluidFilter, ScreamerFilter, StereoBiquad, StereoSvf, SvfCoeffs,
    SvfFilterType,
};
pub use oscillators::poly_blep;
pub use oversampling::{Downsampler, OversamplingFactor};
pub use spectral::{FftProcessor, PartitionedConvolver, StftProcessor, WindowType, fill_window};

// Re-export Complex for use by downstream crates (e.g. phase vocoder)
pub use realfft::num_complex::Complex;
