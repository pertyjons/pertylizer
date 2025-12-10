//! Value range types for parameter bounds.
//!
//! This module provides the `ValueRange` type that encapsulates
//! min, max, and default values for parameters, ensuring they
//! travel together as a single unit.

use serde::{Deserialize, Serialize};

/// A range of values with min, max, and default.
///
/// Used to define parameter bounds and ensure that min/max/default
/// values are always consistent and travel together.
///
/// # Example
///
/// ```
/// use modular_synth::types::ValueRange;
///
/// // Create a frequency range
/// let freq_range = ValueRange::new(20.0, 20000.0, 440.0);
///
/// // Clamp a value to the range
/// let clamped = freq_range.clamp(50000.0); // Returns 20000.0
///
/// // Get normalized position (0.0-1.0)
/// let normalized = freq_range.normalize(440.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[must_use]
pub struct ValueRange {
    /// Minimum allowed value.
    pub min: f32,
    /// Maximum allowed value.
    pub max: f32,
    /// Default value (must be within min..=max).
    pub default: f32,
}

impl ValueRange {
    /// Create a new value range.
    ///
    /// # Panics
    /// In debug builds, panics if default is outside min..=max range
    /// or if min > max.
    #[inline]
    pub const fn new(min: f32, max: f32, default: f32) -> Self {
        debug_assert!(min <= max, "min must be <= max");
        debug_assert!(
            default >= min && default <= max,
            "default must be within range"
        );
        Self { min, max, default }
    }

    /// Create a unit range (0.0 to 1.0) with default at 0.5.
    pub const UNIT: Self = Self {
        min: 0.0,
        max: 1.0,
        default: 0.5,
    };

    /// Create a unit range (0.0 to 1.0) with default at 0.0.
    pub const UNIT_ZERO: Self = Self {
        min: 0.0,
        max: 1.0,
        default: 0.0,
    };

    /// Create a unit range (0.0 to 1.0) with default at 1.0.
    pub const UNIT_ONE: Self = Self {
        min: 0.0,
        max: 1.0,
        default: 1.0,
    };

    /// Create a bipolar range (-1.0 to 1.0) with default at 0.0.
    pub const BIPOLAR: Self = Self {
        min: -1.0,
        max: 1.0,
        default: 0.0,
    };

    /// Create a percentage range (0.0 to 100.0) with default at 50.0.
    pub const PERCENT: Self = Self {
        min: 0.0,
        max: 100.0,
        default: 50.0,
    };

    /// Create a toggle/boolean range (0.0 or 1.0) with default off.
    pub const TOGGLE: Self = Self {
        min: 0.0,
        max: 1.0,
        default: 0.0,
    };

    /// Create a range from min to max with default at the midpoint.
    #[inline]
    pub const fn symmetric(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            default: (min + max) / 2.0,
        }
    }

    /// Create a range from min to max with default at min.
    #[inline]
    pub const fn from_min(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            default: min,
        }
    }

    /// Create a range from min to max with default at max.
    #[inline]
    pub const fn from_max(min: f32, max: f32) -> Self {
        Self {
            min,
            max,
            default: max,
        }
    }

    /// Get the span (difference between max and min).
    #[inline]
    #[must_use]
    pub const fn span(&self) -> f32 {
        self.max - self.min
    }

    /// Check if a value is within the range.
    #[inline]
    #[must_use]
    pub fn contains(&self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }

    /// Clamp a value to the range.
    #[inline]
    #[must_use]
    pub fn clamp(&self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }

    /// Normalize a value to 0.0..1.0 (linear).
    ///
    /// Values outside the range are clamped first.
    #[inline]
    #[must_use]
    pub fn normalize(&self, value: f32) -> f32 {
        let span = self.span();
        if span.abs() < f32::EPSILON {
            return 0.0;
        }
        let clamped = self.clamp(value);
        (clamped - self.min) / span
    }

    /// Denormalize a 0.0..1.0 value to the range (linear).
    #[inline]
    #[must_use]
    pub fn denormalize(&self, normalized: f32) -> f32 {
        let n = normalized.clamp(0.0, 1.0);
        self.min + n * self.span()
    }

    /// Linear interpolation between min and max.
    #[inline]
    #[must_use]
    pub fn lerp(&self, t: f32) -> f32 {
        self.denormalize(t)
    }

    /// Get the default value's normalized position.
    #[inline]
    #[must_use]
    pub fn default_normalized(&self) -> f32 {
        self.normalize(self.default)
    }

    /// Create a new range with a different default.
    #[inline]
    pub const fn with_default(self, default: f32) -> Self {
        Self {
            min: self.min,
            max: self.max,
            default,
        }
    }
}

impl Default for ValueRange {
    fn default() -> Self {
        Self::UNIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_range() {
        let range = ValueRange::new(0.0, 100.0, 50.0);
        assert_eq!(range.min, 0.0);
        assert_eq!(range.max, 100.0);
        assert_eq!(range.default, 50.0);
    }

    #[test]
    fn test_span() {
        let range = ValueRange::new(10.0, 50.0, 30.0);
        assert_eq!(range.span(), 40.0);
    }

    #[test]
    fn test_contains() {
        let range = ValueRange::new(0.0, 100.0, 50.0);
        assert!(range.contains(50.0));
        assert!(range.contains(0.0));
        assert!(range.contains(100.0));
        assert!(!range.contains(-1.0));
        assert!(!range.contains(101.0));
    }

    #[test]
    fn test_clamp() {
        let range = ValueRange::new(0.0, 100.0, 50.0);
        assert_eq!(range.clamp(50.0), 50.0);
        assert_eq!(range.clamp(-10.0), 0.0);
        assert_eq!(range.clamp(200.0), 100.0);
    }

    #[test]
    fn test_normalize() {
        let range = ValueRange::new(0.0, 100.0, 50.0);
        assert!((range.normalize(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((range.normalize(50.0) - 0.5).abs() < f32::EPSILON);
        assert!((range.normalize(100.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_denormalize() {
        let range = ValueRange::new(0.0, 100.0, 50.0);
        assert!((range.denormalize(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((range.denormalize(0.5) - 50.0).abs() < f32::EPSILON);
        assert!((range.denormalize(1.0) - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_symmetric() {
        let range = ValueRange::symmetric(-10.0, 10.0);
        assert_eq!(range.default, 0.0);
    }

    #[test]
    fn test_preset_ranges() {
        assert_eq!(ValueRange::UNIT.min, 0.0);
        assert_eq!(ValueRange::UNIT.max, 1.0);
        assert_eq!(ValueRange::BIPOLAR.min, -1.0);
        assert_eq!(ValueRange::BIPOLAR.default, 0.0);
    }
}
