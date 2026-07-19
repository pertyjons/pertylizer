//! Spatial geometry newtypes shared across DSP crates.
//!
//! These wrap raw `f32` values for room-acoustics and spatialisation code
//! (early reflections, binaural spatialisers). Keeping them in `synth_core`
//! lets `synth_modules` use real newtypes for room dimensions and positions
//! instead of raw meters-as-`f32`.

use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// Length in meters.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
pub struct Meters(f32);

impl Meters {
    /// Create a new length.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// Zero meters.
    pub const ZERO: Self = Self(0.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Clamp to a range.
    #[inline]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }

    /// Max with another value.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    /// Min with another value.
    #[inline]
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }
}

impl From<f32> for Meters {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<Meters> for f32 {
    fn from(value: Meters) -> Self {
        value.0
    }
}

impl Add for Meters {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Meters {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul<f32> for Meters {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Div<f32> for Meters {
    type Output = Self;

    #[inline]
    fn div(self, rhs: f32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl AddAssign for Meters {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Meters {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

impl MulAssign<f32> for Meters {
    #[inline]
    fn mul_assign(&mut self, rhs: f32) {
        self.0 *= rhs;
    }
}

impl DivAssign<f32> for Meters {
    #[inline]
    fn div_assign(&mut self, rhs: f32) {
        self.0 /= rhs;
    }
}

/// Speed in meters per second.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
pub struct MetersPerSecond(f32);

impl MetersPerSecond {
    /// Create a new speed.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

/// Fractional sample offset (for interpolated delays).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
pub struct SampleOffset(f32);

impl SampleOffset {
    /// Create a new offset.
    #[inline]
    pub const fn new(value: f32) -> Self {
        Self(value)
    }

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

/// 3D position in meters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
#[repr(transparent)]
#[must_use]
pub struct Position3([Meters; 3]);

impl Position3 {
    /// Create a new position.
    #[inline]
    pub const fn new(x: Meters, y: Meters, z: Meters) -> Self {
        Self([x, y, z])
    }

    /// X component.
    #[inline]
    pub const fn x(self) -> Meters {
        self.0[0]
    }

    /// Y component.
    #[inline]
    pub const fn y(self) -> Meters {
        self.0[1]
    }

    /// Z component.
    #[inline]
    pub const fn z(self) -> Meters {
        self.0[2]
    }

    /// Convert to raw f32 array.
    #[inline]
    pub fn as_f32(self) -> [f32; 3] {
        [self.0[0].as_f32(), self.0[1].as_f32(), self.0[2].as_f32()]
    }

    /// Safe access by index. Returns `None` if index is out of range (0..3).
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Meters> {
        self.0.get(index)
    }
}

impl Default for Position3 {
    fn default() -> Self {
        Self([Meters::ZERO; 3])
    }
}

impl From<[Meters; 3]> for Position3 {
    fn from(value: [Meters; 3]) -> Self {
        Self(value)
    }
}

impl From<[f32; 3]> for Position3 {
    fn from(value: [f32; 3]) -> Self {
        Self([
            Meters::new(value[0]),
            Meters::new(value[1]),
            Meters::new(value[2]),
        ])
    }
}

impl From<Position3> for [Meters; 3] {
    fn from(value: Position3) -> Self {
        value.0
    }
}

impl std::ops::Index<usize> for Position3 {
    type Output = Meters;

    #[inline]
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index.min(2)]
    }
}

impl std::ops::IndexMut<usize> for Position3 {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.0[index.min(2)]
    }
}
