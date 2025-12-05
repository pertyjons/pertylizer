//! Pitch-related types for type-safe audio processing.

use std::ops::{Add, Div, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

use super::Hertz;

/// Pitch offset in cents (1/100th of a semitone).
///
/// 100 cents = 1 semitone
/// 1200 cents = 1 octave
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Cents(pub f32);

impl Cents {
    /// Create a new cents value.
    #[inline]
    pub const fn new(cents: f32) -> Self {
        Self(cents)
    }

    /// Zero cents.
    pub const ZERO: Self = Self(0.0);

    /// One semitone.
    pub const SEMITONE: Self = Self(100.0);

    /// One octave.
    pub const OCTAVE: Self = Self(1200.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to semitones.
    #[inline]
    pub fn to_semitones(self) -> Semitones {
        Semitones::new(self.0 / 100.0)
    }

    /// Convert to frequency ratio.
    #[inline]
    pub fn to_ratio(self) -> f32 {
        (self.0 / 1200.0).exp2()
    }

    /// Apply to a frequency.
    #[inline]
    pub fn apply(self, freq: Hertz) -> Hertz {
        Hertz::new(freq.0 * self.to_ratio())
    }

    /// Clamp to a typical detune range.
    #[inline]
    pub fn clamp_detune(self) -> Self {
        Self(self.0.clamp(-100.0, 100.0))
    }
}

impl Add for Cents {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Cents {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Cents {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Mul<f32> for Cents {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl Div<f32> for Cents {
    type Output = Self;

    #[inline]
    fn div(self, rhs: f32) -> Self::Output {
        Self(self.0 / rhs)
    }
}

impl From<f32> for Cents {
    fn from(cents: f32) -> Self {
        Self(cents)
    }
}

impl From<Cents> for f32 {
    fn from(cents: Cents) -> Self {
        cents.0
    }
}

impl std::fmt::Display for Cents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+.0} cents", self.0)
    }
}

/// Pitch offset in semitones.
///
/// 1 semitone = 100 cents
/// 12 semitones = 1 octave
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Semitones(pub f32);

impl Semitones {
    /// Create a new semitones value.
    #[inline]
    pub const fn new(semitones: f32) -> Self {
        Self(semitones)
    }

    /// Zero semitones.
    pub const ZERO: Self = Self(0.0);

    /// One octave.
    pub const OCTAVE: Self = Self(12.0);

    /// Get the raw value.
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to cents.
    #[inline]
    pub fn to_cents(self) -> Cents {
        Cents::new(self.0 * 100.0)
    }

    /// Convert to frequency ratio.
    #[inline]
    pub fn to_ratio(self) -> f32 {
        (self.0 / 12.0).exp2()
    }

    /// Apply to a frequency.
    #[inline]
    pub fn apply(self, freq: Hertz) -> Hertz {
        Hertz::new(freq.0 * self.to_ratio())
    }

    /// Round to nearest semitone.
    #[inline]
    pub fn round(self) -> Self {
        Self(self.0.round())
    }

    /// Get the interval name (approximation).
    pub fn interval_name(&self) -> &'static str {
        let rounded = self.0.round() as i32;
        match rounded.rem_euclid(12) {
            0 => "unison/octave",
            1 => "minor 2nd",
            2 => "major 2nd",
            3 => "minor 3rd",
            4 => "major 3rd",
            5 => "perfect 4th",
            6 => "tritone",
            7 => "perfect 5th",
            8 => "minor 6th",
            9 => "major 6th",
            10 => "minor 7th",
            11 => "major 7th",
            _ => "unknown",
        }
    }
}

impl Add for Semitones {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Semitones {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Semitones {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl Mul<f32> for Semitones {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self::Output {
        Self(self.0 * rhs)
    }
}

impl From<f32> for Semitones {
    fn from(semitones: f32) -> Self {
        Self(semitones)
    }
}

impl From<Semitones> for f32 {
    fn from(semitones: Semitones) -> Self {
        semitones.0
    }
}

impl From<Cents> for Semitones {
    fn from(cents: Cents) -> Self {
        cents.to_semitones()
    }
}

impl From<Semitones> for Cents {
    fn from(semitones: Semitones) -> Self {
        semitones.to_cents()
    }
}

impl std::fmt::Display for Semitones {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+.1} st", self.0)
    }
}

/// Octave offset.
///
/// 1 octave = 12 semitones = 1200 cents
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Octaves(pub i32);

impl Octaves {
    /// Create a new octave offset.
    #[inline]
    pub const fn new(octaves: i32) -> Self {
        Self(octaves)
    }

    /// Zero octaves.
    pub const ZERO: Self = Self(0);

    /// Get the raw value.
    #[inline]
    pub const fn as_i32(self) -> i32 {
        self.0
    }

    /// Convert to semitones.
    #[inline]
    pub fn to_semitones(self) -> Semitones {
        Semitones::new(self.0 as f32 * 12.0)
    }

    /// Convert to cents.
    #[inline]
    pub fn to_cents(self) -> Cents {
        Cents::new(self.0 as f32 * 1200.0)
    }

    /// Convert to frequency ratio.
    #[inline]
    pub fn to_ratio(self) -> f32 {
        2.0f32.powi(self.0)
    }

    /// Apply to a frequency.
    #[inline]
    pub fn apply(self, freq: Hertz) -> Hertz {
        Hertz::new(freq.0 * self.to_ratio())
    }
}

impl Add for Octaves {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Octaves {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Neg for Octaves {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

impl From<i32> for Octaves {
    fn from(octaves: i32) -> Self {
        Self(octaves)
    }
}

impl From<Octaves> for i32 {
    fn from(octaves: Octaves) -> Self {
        octaves.0
    }
}

impl std::fmt::Display for Octaves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+} oct", self.0)
    }
}

/// MIDI note number (0-127).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MidiNote(pub u8);

impl MidiNote {
    /// Create a new MIDI note.
    #[inline]
    pub const fn new(note: u8) -> Self {
        Self(note)
    }

    // === Standard note constants ===

    /// Lowest note on 88-key piano (A0, MIDI 21)
    pub const A0: Self = Self(21);

    /// Middle C (C4, MIDI 60)
    pub const C4: Self = Self(60);

    /// Concert pitch A (A4, MIDI 69, 440Hz)
    pub const A4: Self = Self(69);

    /// Highest note on 88-key piano (C8, MIDI 108)
    pub const C8: Self = Self(108);

    /// Lowest MIDI note
    pub const MIN: Self = Self(0);

    /// Highest MIDI note
    pub const MAX: Self = Self(127);

    /// Get the raw value.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Convert to frequency.
    #[inline]
    pub fn to_frequency(self) -> Hertz {
        Hertz::from_midi(self.0)
    }

    /// Transpose by semitones.
    #[inline]
    pub fn transpose(self, semitones: i8) -> Self {
        Self((self.0 as i16 + semitones as i16).clamp(0, 127) as u8)
    }

    /// Get the note name (e.g., "C4", "F#5").
    pub fn name(&self) -> String {
        let note_names = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let note = self.0 as usize;
        let name = note_names[note % 12];
        let octave = (note / 12) as i32 - 1;
        format!("{}{}", name, octave)
    }

    /// Get the octave number.
    #[inline]
    pub fn octave(&self) -> i32 {
        (self.0 / 12) as i32 - 1
    }

    /// Get the pitch class (0-11).
    #[inline]
    pub fn pitch_class(&self) -> u8 {
        self.0 % 12
    }
}

impl From<u8> for MidiNote {
    fn from(note: u8) -> Self {
        Self(note)
    }
}

impl From<MidiNote> for u8 {
    fn from(note: MidiNote) -> Self {
        note.0
    }
}

impl std::fmt::Display for MidiNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cents_to_ratio() {
        let octave = Cents::OCTAVE;
        assert!((octave.to_ratio() - 2.0).abs() < 0.001);

        let fifth = Cents::new(700.0);
        assert!((fifth.to_ratio() - 1.498).abs() < 0.01);
    }

    #[test]
    fn test_semitones_interval() {
        assert_eq!(Semitones::new(0.0).interval_name(), "unison/octave");
        assert_eq!(Semitones::new(7.0).interval_name(), "perfect 5th");
        assert_eq!(Semitones::new(4.0).interval_name(), "major 3rd");
    }

    #[test]
    fn test_octave_ratio() {
        let oct = Octaves::new(1);
        assert!((oct.to_ratio() - 2.0).abs() < 0.001);

        let oct = Octaves::new(-1);
        assert!((oct.to_ratio() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_midi_note_name() {
        assert_eq!(MidiNote::C4.name(), "C4");
        assert_eq!(MidiNote::A4.name(), "A4");
        assert_eq!(MidiNote::new(72).name(), "C5");
    }

    #[test]
    fn test_midi_note_frequency() {
        let a4 = MidiNote::A4;
        assert!((a4.to_frequency().0 - 440.0).abs() < 0.001);
    }
}
