//! Tracker-specific types for XM/IT compatibility.
//!
//! These types provide type-safe wrappers for tracker format values,
//! enabling accurate import and playback of XM/IT/S3M modules.

use serde::{Deserialize, Serialize};

use super::{Clampable, Hertz, Seconds};

// ============================================================================
// Envelope Types
// ============================================================================

/// Envelope frame position (0-65535 ticks).
///
/// XM/IT envelopes use frame-based timing where frames advance
/// at a rate of `BPM * 2 / 5` Hz (50 Hz at 125 BPM).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EnvelopeFrame(pub u16);

impl EnvelopeFrame {
    /// Minimum frame (0).
    pub const MIN: Self = Self(0);

    /// Maximum frame (65535).
    pub const MAX: Self = Self(65535);

    /// Create a new envelope frame.
    #[inline]
    pub const fn new(frame: u16) -> Self {
        Self(frame)
    }

    /// Get the raw frame value.
    #[inline]
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Convert to seconds given a tick rate.
    ///
    /// Tick rate is typically `BPM * 2 / 5` (50 Hz at 125 BPM).
    #[inline]
    #[must_use]
    pub fn to_seconds(self, tick_rate: f32) -> Seconds {
        Seconds::new(self.0 as f32 / tick_rate.max(1.0))
    }
}

impl From<u16> for EnvelopeFrame {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for EnvelopeFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}f", self.0)
    }
}

/// Envelope value (0.0-1.0).
///
/// XM uses 0-64 internally, IT uses 0-64 for volume and 0-32 for pan.
/// This type normalizes to 0.0-1.0 for consistent processing.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EnvelopeValue(pub f32);

impl EnvelopeValue {
    /// Minimum value (0.0).
    pub const MIN: Self = Self(0.0);

    /// Maximum value (1.0).
    pub const MAX: Self = Self(1.0);

    /// Center value (0.5).
    pub const CENTER: Self = Self(0.5);

    /// Create a new envelope value, clamping to [0.0, 1.0].
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Create without clamping (for performance in hot paths).
    #[inline]
    pub const fn new_unchecked(value: f32) -> Self {
        Self(value)
    }

    /// Get the raw value.
    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Create from tracker 0-64 value (XM volume envelope).
    #[inline]
    #[must_use]
    pub fn from_tracker_64(value: u8) -> Self {
        Self::new(value as f32 / 64.0)
    }

    /// Create from tracker 0-32 value (IT pan envelope).
    #[inline]
    #[must_use]
    pub fn from_tracker_32(value: u8) -> Self {
        Self::new(value as f32 / 32.0)
    }

    /// Linear interpolation to another value.
    #[inline]
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.0 + (other.0 - self.0) * t)
    }
}

impl Clampable for EnvelopeValue {
    fn clamp(self) -> Self {
        Self(self.0.clamp(0.0, 1.0))
    }

    fn is_valid(&self) -> bool {
        self.0.is_finite() && (0.0..=1.0).contains(&self.0)
    }
}

impl From<f32> for EnvelopeValue {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for EnvelopeValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.0}%", self.0 * 100.0)
    }
}

/// Envelope point index (0-24).
///
/// IT format supports up to 25 envelope points, XM supports 12.
/// Using IT's limit as the maximum.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct EnvelopePointIndex(pub u8);

impl EnvelopePointIndex {
    /// Minimum index (0).
    pub const MIN: Self = Self(0);

    /// Maximum index (24, IT limit of 25 points).
    pub const MAX: Self = Self(24);

    /// Create a new point index, clamping to [0, 24].
    #[inline]
    pub const fn new(index: u8) -> Self {
        Self(if index > 24 { 24 } else { index })
    }

    /// Get the raw index value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get the index as usize for array indexing.
    #[inline]
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl From<u8> for EnvelopePointIndex {
    fn from(value: u8) -> Self {
        Self::new(value)
    }
}

impl From<usize> for EnvelopePointIndex {
    fn from(value: usize) -> Self {
        Self::new(value.min(24) as u8)
    }
}

impl std::fmt::Display for EnvelopePointIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pt{}", self.0)
    }
}

/// Fadeout rate (0-65535).
///
/// Controls how quickly volume fades after note-off.
/// Higher values = faster fade. 0 = no fadeout.
/// XM fadeout: `volume -= fadeout / 32768` per tick.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct FadeoutRate(pub u16);

impl FadeoutRate {
    /// No fadeout (infinite sustain after release).
    pub const NONE: Self = Self(0);

    /// Default fadeout rate.
    pub const DEFAULT: Self = Self(256);

    /// Fast fadeout.
    pub const FAST: Self = Self(4096);

    /// Instant fadeout.
    pub const INSTANT: Self = Self(65535);

    /// Create a new fadeout rate.
    #[inline]
    pub const fn new(rate: u16) -> Self {
        Self(rate)
    }

    /// Get the raw rate value.
    #[inline]
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Convert to per-tick decay multiplier.
    ///
    /// Returns a value slightly less than 1.0 to multiply the volume each tick.
    #[inline]
    #[must_use]
    pub fn to_decay_per_tick(self) -> f32 {
        if self.0 == 0 {
            1.0
        } else {
            1.0 - (self.0 as f32 / 65536.0)
        }
    }

    /// Estimate time to silence in seconds at given tick rate.
    #[inline]
    #[must_use]
    pub fn estimated_duration(self, tick_rate: f32) -> Seconds {
        if self.0 == 0 {
            Seconds::new(f32::INFINITY)
        } else {
            // Approximate: -60dB in N ticks where N = ln(0.001) / ln(decay)
            let decay = self.to_decay_per_tick();
            if decay >= 1.0 {
                Seconds::new(f32::INFINITY)
            } else {
                let ticks = (-6.9078) / decay.ln(); // ln(0.001) ≈ -6.9078
                Seconds::new(ticks / tick_rate.max(1.0))
            }
        }
    }
}

impl From<u16> for FadeoutRate {
    fn from(value: u16) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for FadeoutRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 == 0 {
            write!(f, "none")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

// ============================================================================
// Filter Types
// ============================================================================

/// Tracker filter cutoff (0-127, Zxx effect).
///
/// IT/MPTM use Zxx for filter control where:
/// - Z00-Z7F: Set cutoff (0-127)
/// - Z80-Z8F: Set resonance (0-15)
///
/// Cutoff maps approximately logarithmically from ~130 Hz to ~8 kHz.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct TrackerCutoff(pub u8);

impl TrackerCutoff {
    /// Minimum cutoff (0, ~130 Hz).
    pub const MIN: Self = Self(0);

    /// Maximum cutoff (127, ~8 kHz).
    pub const MAX: Self = Self(127);

    /// Default/open cutoff (127).
    pub const OPEN: Self = Self(127);

    /// Create a new cutoff value, clamping to [0, 127].
    #[inline]
    pub const fn new(cutoff: u8) -> Self {
        Self(if cutoff > 127 { 127 } else { cutoff })
    }

    /// Get the raw cutoff value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Convert to frequency in Hertz.
    ///
    /// Uses IT's formula: 110 Hz * 2^(cutoff * 6.5 / 127)
    /// This gives approximately 130 Hz at 0 and 8 kHz at 127.
    #[inline]
    #[must_use]
    pub fn to_hertz(self) -> Hertz {
        Hertz::new(110.0 * 2.0_f32.powf(self.0 as f32 * 6.5 / 127.0))
    }

    /// Create from Zxx effect byte (Z00-Z7F range).
    #[inline]
    #[must_use]
    pub fn from_zxx(value: u8) -> Self {
        Self::new(value & 0x7F)
    }

    /// Create from frequency (inverse of `to_hertz`).
    #[inline]
    #[must_use]
    pub fn from_hertz(freq: Hertz) -> Self {
        let hz = freq.as_f32().max(110.0);
        let cutoff = ((hz / 110.0).log2() * 127.0 / 6.5).clamp(0.0, 127.0);
        Self::new(cutoff as u8)
    }
}

impl Clampable for TrackerCutoff {
    fn clamp(self) -> Self {
        Self(self.0.min(127))
    }

    fn is_valid(&self) -> bool {
        self.0 <= 127
    }
}

impl From<u8> for TrackerCutoff {
    fn from(value: u8) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for TrackerCutoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Z{:02X}", self.0)
    }
}

/// Tracker filter resonance (0-127).
///
/// IT uses Z80-Z8F for 4-bit resonance (0-15), but we support full 0-127
/// range for extended formats and smoother control.
///
/// Resonance maps from Q=0.5 (no resonance) to Q=12 (high resonance).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct TrackerResonance(pub u8);

impl TrackerResonance {
    /// Minimum resonance (0, Q=0.5).
    pub const MIN: Self = Self(0);

    /// Maximum resonance (127, Q≈12).
    pub const MAX: Self = Self(127);

    /// Default resonance (0, no resonance).
    pub const DEFAULT: Self = Self(0);

    /// Create a new resonance value, clamping to [0, 127].
    #[inline]
    pub const fn new(res: u8) -> Self {
        Self(if res > 127 { 127 } else { res })
    }

    /// Get the raw resonance value.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Convert to Q factor for filter.
    ///
    /// Maps 0 → 0.5 (Butterworth), 127 → 12 (high resonance).
    #[inline]
    #[must_use]
    pub fn to_q(self) -> f32 {
        0.5 + (self.0 as f32 / 127.0) * 11.5
    }

    /// Create from IT Z8x effect (4-bit, 0-15).
    #[inline]
    #[must_use]
    pub fn from_z8x(value: u8) -> Self {
        Self::new((value & 0x0F) * 8)
    }

    /// Create from Q factor (inverse of `to_q`).
    #[inline]
    #[must_use]
    pub fn from_q(q: f32) -> Self {
        let res = ((q.clamp(0.5, 12.0) - 0.5) * 127.0 / 11.5) as u8;
        Self::new(res)
    }
}

impl Clampable for TrackerResonance {
    fn clamp(self) -> Self {
        Self(self.0.min(127))
    }

    fn is_valid(&self) -> bool {
        self.0 <= 127
    }
}

impl From<u8> for TrackerResonance {
    fn from(value: u8) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for TrackerResonance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Q{:.1}", self.to_q())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_frame_conversions() {
        let frame = EnvelopeFrame::new(100);
        assert_eq!(frame.as_u16(), 100);

        // At 50 Hz tick rate, 100 frames = 2 seconds
        let secs = frame.to_seconds(50.0);
        assert!((secs.as_f32() - 2.0).abs() < 0.001);
    }

    #[test]
    fn envelope_value_clamping() {
        assert_eq!(EnvelopeValue::new(1.5).as_f32(), 1.0);
        assert_eq!(EnvelopeValue::new(-0.5).as_f32(), 0.0);
        assert_eq!(EnvelopeValue::new(0.5).as_f32(), 0.5);
    }

    #[test]
    fn envelope_value_from_tracker() {
        assert!((EnvelopeValue::from_tracker_64(64).as_f32() - 1.0).abs() < 0.001);
        assert!((EnvelopeValue::from_tracker_64(32).as_f32() - 0.5).abs() < 0.001);
        assert!((EnvelopeValue::from_tracker_64(0).as_f32()).abs() < 0.001);
    }

    #[test]
    fn fadeout_rate_decay() {
        // No fadeout should return 1.0
        assert_eq!(FadeoutRate::NONE.to_decay_per_tick(), 1.0);

        // Non-zero should be less than 1.0
        let decay = FadeoutRate::DEFAULT.to_decay_per_tick();
        assert!(decay < 1.0);
        assert!(decay > 0.99);
    }

    #[test]
    fn tracker_cutoff_to_hertz() {
        let low = TrackerCutoff::MIN.to_hertz();
        let high = TrackerCutoff::MAX.to_hertz();

        // Should be approximately 130 Hz to 8 kHz
        assert!(low.as_f32() > 100.0 && low.as_f32() < 200.0);
        assert!(high.as_f32() > 7000.0 && high.as_f32() < 10000.0);
    }

    #[test]
    fn tracker_cutoff_roundtrip() {
        for cutoff in [0, 32, 64, 96, 127] {
            let tc = TrackerCutoff::new(cutoff);
            let hz = tc.to_hertz();
            let back = TrackerCutoff::from_hertz(hz);
            assert!((back.as_u8() as i16 - cutoff as i16).abs() <= 1);
        }
    }

    #[test]
    fn tracker_resonance_to_q() {
        assert!((TrackerResonance::MIN.to_q() - 0.5).abs() < 0.01);
        assert!((TrackerResonance::MAX.to_q() - 12.0).abs() < 0.01);
    }

    #[test]
    fn tracker_resonance_from_z8x() {
        // Z80 = resonance 0, Z8F = resonance 15
        assert_eq!(TrackerResonance::from_z8x(0x80).as_u8(), 0);
        assert_eq!(TrackerResonance::from_z8x(0x8F).as_u8(), 120);
    }
}
