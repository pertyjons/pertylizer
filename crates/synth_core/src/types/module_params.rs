//! Newtypes for parameter values that don't fit `NormalizedValue` or
//! `BipolarValue` cleanly — the descriptor range is non-unit and the
//! semantic value must be stored directly so JSON saves match the
//! advertised range.

use serde::{Deserialize, Serialize};

/// Bitcrusher bit depth (1.0–16.0 bits).
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "f32", into = "f32")]
#[repr(transparent)]
pub struct BitDepth(f32);

impl BitDepth {
    pub const MIN: Self = Self(1.0);
    pub const MAX: Self = Self(16.0);
    pub const DEFAULT: Self = Self(8.0);

    #[inline]
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(1.0, 16.0))
    }

    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for BitDepth {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Time-scaling multiplier (0.01–10.0×). Used by MSEG and similar
/// envelope-time controls.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "f32", into = "f32")]
#[repr(transparent)]
pub struct TimeScale(f32);

impl TimeScale {
    pub const MIN: Self = Self(0.01);
    pub const MAX: Self = Self(10.0);
    pub const UNITY: Self = Self(1.0);

    #[inline]
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.01, 10.0))
    }

    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for TimeScale {
    fn default() -> Self {
        Self::UNITY
    }
}

/// Pulse-width / duty cycle (0.01–0.99). Bounds keep pulse oscillators
/// out of degenerate full-on / full-off regions.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "f32", into = "f32")]
#[repr(transparent)]
pub struct PulseWidth(f32);

impl PulseWidth {
    pub const MIN: Self = Self(0.01);
    pub const MAX: Self = Self(0.99);
    pub const SQUARE: Self = Self(0.5);

    #[inline]
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.01, 0.99))
    }

    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for PulseWidth {
    fn default() -> Self {
        Self::SQUARE
    }
}

/// Convolver impulse-response decay trim (0.1–1.0× of the source
/// duration). 1.0 keeps the full tail; lower values truncate.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "f32", into = "f32")]
#[repr(transparent)]
pub struct DecayTrim(f32);

impl DecayTrim {
    pub const MIN: Self = Self(0.1);
    pub const MAX: Self = Self(1.0);
    pub const FULL: Self = Self(1.0);

    #[inline]
    #[must_use]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.1, 1.0))
    }

    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl Default for DecayTrim {
    fn default() -> Self {
        Self::FULL
    }
}

macro_rules! impl_f32_conversion {
    ($type:ty) => {
        impl From<f32> for $type {
            fn from(value: f32) -> Self {
                Self::new(value)
            }
        }

        impl From<$type> for f32 {
            fn from(value: $type) -> Self {
                value.as_f32()
            }
        }
    };
}

impl_f32_conversion!(BitDepth);
impl_f32_conversion!(TimeScale);
impl_f32_conversion!(PulseWidth);
impl_f32_conversion!(DecayTrim);
