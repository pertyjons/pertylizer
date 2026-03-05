//! Time types for the sequencer.
//!
//! All time is represented in ticks with 960 PPQN (pulses per quarter note).

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Sub, SubAssign};
use synth_core::Bpm;

/// Ticks per quarter note (PPQN).
pub const TICKS_PER_QUARTER: u32 = 960;

/// Absolute position on the song timeline.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Tick(pub u64);

impl Tick {
    pub const ZERO: Self = Self(0);

    /// Create a Tick from a pattern-local tick and the pattern's start position.
    pub fn from_pattern_tick(pattern_start: Self, offset: PatternTick) -> Self {
        Self(pattern_start.0 + offset.0 as u64)
    }

    /// Convert to seconds at a given tempo.
    pub fn to_seconds(&self, tempo_bpm: Bpm) -> f64 {
        let beats = self.0 as f64 / TICKS_PER_QUARTER as f64;
        beats * 60.0 / tempo_bpm.as_f32() as f64
    }

    /// Create from seconds at a given tempo.
    pub fn from_seconds(seconds: f64, tempo_bpm: Bpm) -> Self {
        let beats = seconds * tempo_bpm.as_f32() as f64 / 60.0;
        Self((beats * TICKS_PER_QUARTER as f64) as u64)
    }

    /// Convert to bar, beat, tick representation.
    pub fn to_bar_beat_tick(&self, time_sig: TimeSignature) -> (u32, u32, u32) {
        let ticks_per_bar = time_sig.ticks_per_bar();
        let bar = (self.0 / ticks_per_bar as u64) as u32;
        let remaining = self.0 % ticks_per_bar as u64;
        let ticks_per_beat = TICKS_PER_QUARTER * 4 / time_sig.denominator as u32;
        let beat = (remaining / ticks_per_beat as u64) as u32;
        let tick = (remaining % ticks_per_beat as u64) as u32;
        (bar, beat, tick)
    }

    /// Create from bar, beat, tick representation.
    pub fn from_bar_beat_tick(bar: u32, beat: u32, tick: u32, time_sig: TimeSignature) -> Self {
        let ticks_per_bar = time_sig.ticks_per_bar();
        let ticks_per_beat = TICKS_PER_QUARTER * 4 / time_sig.denominator as u32;
        Self(bar as u64 * ticks_per_bar as u64 + beat as u64 * ticks_per_beat as u64 + tick as u64)
    }
}

impl Add for Tick {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Tick {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0.saturating_sub(rhs.0);
    }
}

impl Add<Duration> for Tick {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0 + rhs.0 as u64)
    }
}

/// Relative position within a pattern (0 = pattern start).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct PatternTick(pub u32);

impl PatternTick {
    pub const ZERO: Self = Self(0);
}

impl Add for PatternTick {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for PatternTick {
    type Output = Duration;
    fn sub(self, rhs: Self) -> Self::Output {
        Duration(self.0.saturating_sub(rhs.0))
    }
}

impl AddAssign for PatternTick {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for PatternTick {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0.saturating_sub(rhs.0);
    }
}

impl Add<Duration> for PatternTick {
    type Output = Self;
    fn add(self, rhs: Duration) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

/// Duration in ticks.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Duration(pub u32);

impl Duration {
    /// Whole note (4 beats).
    pub const WHOLE: Self = Self(3840);
    /// Half note (2 beats).
    pub const HALF: Self = Self(1920);
    /// Quarter note (1 beat).
    pub const QUARTER: Self = Self(960);
    /// Eighth note (1/2 beat).
    pub const EIGHTH: Self = Self(480);
    /// Sixteenth note (1/4 beat).
    pub const SIXTEENTH: Self = Self(240);
    /// 32nd note.
    pub const THIRTY_SECOND: Self = Self(120);
    /// Triplet quarter note.
    pub const TRIPLET_QUARTER: Self = Self(640);
    /// Triplet eighth note.
    pub const TRIPLET_EIGHTH: Self = Self(320);

    /// Create a dotted duration (1.5x length).
    pub fn dotted(self) -> Self {
        Self(self.0 + self.0 / 2)
    }

    /// Convert this duration to a pattern-relative tick position.
    #[must_use]
    pub fn as_pattern_tick(self) -> PatternTick {
        PatternTick(self.0)
    }
}

impl Add for Duration {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Duration {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0.saturating_sub(rhs.0))
    }
}

impl AddAssign for Duration {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl std::ops::Mul<u32> for Duration {
    type Output = Self;
    fn mul(self, rhs: u32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl SubAssign for Duration {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0.saturating_sub(rhs.0);
    }
}

/// Time signature (e.g., 4/4, 3/4, 6/8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

impl TimeSignature {
    /// Common time (4/4).
    pub const COMMON: Self = Self {
        numerator: 4,
        denominator: 4,
    };
    /// Waltz time (3/4).
    pub const WALTZ: Self = Self {
        numerator: 3,
        denominator: 4,
    };
    /// 6/8 time.
    pub const SIX_EIGHT: Self = Self {
        numerator: 6,
        denominator: 8,
    };

    /// Create a new time signature.
    pub fn new(numerator: u8, denominator: u8) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Ticks per bar.
    pub fn ticks_per_bar(&self) -> u32 {
        TICKS_PER_QUARTER * self.numerator as u32 * 4 / self.denominator as u32
    }

    /// Ticks per beat.
    pub fn ticks_per_beat(&self) -> u32 {
        TICKS_PER_QUARTER * 4 / self.denominator as u32
    }
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self::COMMON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_to_seconds() {
        // 120 BPM = 2 beats/sec = 1920 ticks/sec
        let tick = Tick(1920);
        let seconds = tick.to_seconds(Bpm::new(120.0));
        assert!((seconds - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_tick_from_seconds() {
        let tick = Tick::from_seconds(1.0, Bpm::new(120.0));
        assert_eq!(tick.0, 1920);
    }

    #[test]
    fn test_bar_beat_tick_conversion() {
        let time_sig = TimeSignature::COMMON;
        let tick = Tick::from_bar_beat_tick(1, 2, 480, time_sig);
        let (bar, beat, t) = tick.to_bar_beat_tick(time_sig);
        assert_eq!((bar, beat, t), (1, 2, 480));
    }

    #[test]
    fn test_time_signature_ticks_per_bar() {
        assert_eq!(TimeSignature::COMMON.ticks_per_bar(), 3840);
        assert_eq!(TimeSignature::WALTZ.ticks_per_bar(), 2880);
    }

    #[test]
    fn test_duration_constants() {
        assert_eq!(Duration::QUARTER.0, TICKS_PER_QUARTER);
        assert_eq!(Duration::HALF.0, TICKS_PER_QUARTER * 2);
        assert_eq!(Duration::WHOLE.0, TICKS_PER_QUARTER * 4);
    }

    #[test]
    fn test_dotted_duration() {
        let dotted_quarter = Duration::QUARTER.dotted();
        assert_eq!(dotted_quarter.0, 1440); // 960 + 480
    }

    #[test]
    fn test_pattern_tick_sub_returns_duration() {
        let a = PatternTick(1000);
        let b = PatternTick(400);
        let d: Duration = a - b;
        assert_eq!(d.0, 600);
    }

    #[test]
    fn test_pattern_tick_sub_saturates() {
        let a = PatternTick(100);
        let b = PatternTick(500);
        let d: Duration = a - b;
        assert_eq!(d.0, 0);
    }

    #[test]
    fn test_duration_as_pattern_tick() {
        let d = Duration::QUARTER;
        let pt = d.as_pattern_tick();
        assert_eq!(pt.0, 960);
    }
}
