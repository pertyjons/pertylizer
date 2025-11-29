//! Type-safe audio domain types (Newtype Pattern).
//!
//! This module provides strongly-typed wrappers for primitive values
//! to prevent "primitive obsession" and catch unit mismatches at compile time.
//!
//! # Zero-Cost Abstractions
//!
//! All types implement `Copy`, `Clone`, and necessary arithmetic traits,
//! ensuring no runtime overhead compared to raw `f32`.
//!
//! # Examples
//!
//! ```ignore
//! use modular_synth::types::{Hertz, Seconds, NormalizedValue};
//!
//! let freq = Hertz::new(440.0);
//! let time = Seconds::new(0.5);
//!
//! // Compile error: can't mix units!
//! // let wrong = freq + time;
//!
//! // Convert between related types
//! let period = freq.period(); // Returns Seconds
//! ```

// Load modules in dependency order (time first, then frequency which depends on it)
mod time;
mod frequency;
mod amplitude;
mod normalized;
mod samples;
mod pitch;

pub use frequency::*;
pub use time::*;
pub use amplitude::*;
pub use normalized::*;
pub use samples::*;
pub use pitch::*;

/// Trait for types that can be clamped to a valid range.
pub trait Clampable: Sized {
    /// Clamp the value to its valid range.
    fn clamp(self) -> Self;

    /// Check if the value is within the valid range.
    fn is_valid(&self) -> bool;
}

/// Trait for types that can be interpolated.
pub trait Interpolate: Sized {
    /// Linear interpolation between self and other.
    fn lerp(self, other: Self, t: f32) -> Self;

    /// Exponential interpolation (useful for frequencies).
    fn exp_lerp(self, other: Self, t: f32) -> Self;
}
