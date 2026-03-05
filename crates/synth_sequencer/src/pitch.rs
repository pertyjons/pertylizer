//! Pitch and velocity types for the sequencer.

use serde::{Deserialize, Serialize};

use synth_core::{Hertz, Semitones};

/// MIDI-compatible pitch (0-127).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pitch(u8);

impl Pitch {
    pub const MIN: Self = Self(0);
    pub const MAX: Self = Self(127);
    pub const MIDDLE_C: Self = Self(60);

    /// Create a new pitch from a MIDI note number.
    pub fn new(midi_note: u8) -> Option<Self> {
        (midi_note <= 127).then_some(Self(midi_note))
    }

    /// Create a pitch from octave and note name.
    /// Octave -1 contains MIDI notes 0-11, octave 4 contains middle C (60).
    pub fn from_octave_note(octave: i8, note: NoteName) -> Option<Self> {
        let midi = (octave + 1) as i16 * 12 + note as i16;
        (0..=127).contains(&midi).then_some(Self(midi as u8))
    }

    /// Get the octave (-1 to 9).
    pub fn octave(&self) -> i8 {
        (self.0 / 12) as i8 - 1
    }

    /// Get the note name within the octave.
    pub fn note_name(&self) -> NoteName {
        NoteName::from_midi(self.0 % 12)
    }

    /// Calculate the frequency in Hz.
    /// Uses A4 = 440 Hz as reference by default.
    pub fn frequency(&self, a4_hz: Hertz) -> Hertz {
        Hertz(a4_hz.0 * 2.0_f32.powf((self.0 as f32 - 69.0) / 12.0))
    }

    /// Get the raw MIDI note number.
    pub fn as_midi(&self) -> u8 {
        self.0
    }

    /// Transpose by semitones.
    ///
    /// Returns `None` if the result would be outside valid MIDI range (0-127).
    #[allow(clippy::cast_possible_truncation)]
    /// Saturating subtraction of a raw offset (clamped to valid MIDI range).
    #[must_use]
    pub fn saturating_sub(self, offset: u8) -> Self {
        Self(self.0.saturating_sub(offset))
    }

    /// Saturating addition of a raw offset (clamped to 127).
    #[must_use]
    pub fn saturating_add(self, offset: u8) -> Self {
        Self(self.0.saturating_add(offset).min(127))
    }

    pub fn transpose(&self, semitones: Semitones) -> Option<Self> {
        let transposed = self.0 as i16 + semitones.as_f32().round() as i16;
        if (0..=127).contains(&transposed) {
            Some(Self(transposed as u8))
        } else {
            None
        }
    }
}

impl std::fmt::Display for Pitch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.note_name(), self.octave())
    }
}

impl Default for Pitch {
    fn default() -> Self {
        Self::MIDDLE_C
    }
}

/// Note names within an octave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum NoteName {
    C = 0,
    Cs = 1,
    D = 2,
    Ds = 3,
    E = 4,
    F = 5,
    Fs = 6,
    G = 7,
    Gs = 8,
    A = 9,
    As = 10,
    B = 11,
}

impl NoteName {
    /// Create from MIDI note number mod 12.
    pub fn from_midi(value: u8) -> Self {
        match value % 12 {
            0 => Self::C,
            1 => Self::Cs,
            2 => Self::D,
            3 => Self::Ds,
            4 => Self::E,
            5 => Self::F,
            6 => Self::Fs,
            7 => Self::G,
            8 => Self::Gs,
            9 => Self::A,
            10 => Self::As,
            _ => Self::B,
        }
    }

    /// Check if this is a black key (sharp/flat).
    pub fn is_black_key(&self) -> bool {
        matches!(self, Self::Cs | Self::Ds | Self::Fs | Self::Gs | Self::As)
    }
}

impl std::fmt::Display for NoteName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::C => "C",
            Self::Cs => "C#",
            Self::D => "D",
            Self::Ds => "D#",
            Self::E => "E",
            Self::F => "F",
            Self::Fs => "F#",
            Self::G => "G",
            Self::Gs => "G#",
            Self::A => "A",
            Self::As => "A#",
            Self::B => "B",
        };
        write!(f, "{s}")
    }
}

// Velocity type is now in crate::types::pitch - re-export for backwards compatibility
pub use synth_core::Velocity;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_pitch_creation() {
        assert!(Pitch::new(60).is_some());
        assert!(Pitch::new(128).is_none());
    }

    #[test]
    fn test_pitch_octave_note() {
        let c4 = Pitch::from_octave_note(4, NoteName::C).unwrap();
        assert_eq!(c4.as_midi(), 60);

        let a4 = Pitch::from_octave_note(4, NoteName::A).unwrap();
        assert_eq!(a4.as_midi(), 69);
    }

    #[test]
    fn test_pitch_note_name() {
        let pitch = Pitch::new(60).unwrap();
        assert_eq!(pitch.note_name(), NoteName::C);
        assert_eq!(pitch.octave(), 4);
    }

    #[test]
    fn test_pitch_frequency() {
        let a4 = Pitch::new(69).unwrap();
        assert!((a4.frequency(Hertz(440.0)).0 - 440.0).abs() < 0.01);

        let a5 = Pitch::new(81).unwrap();
        assert!((a5.frequency(Hertz(440.0)).0 - 880.0).abs() < 0.01);
    }

    #[test]
    fn test_pitch_transpose() {
        let c4 = Pitch::new(60).unwrap();
        assert_eq!(c4.transpose(Semitones::new(12.0)).unwrap().as_midi(), 72); // C5
        assert_eq!(c4.transpose(Semitones::new(-12.0)).unwrap().as_midi(), 48); // C3

        // Test out-of-range returns None
        let high = Pitch::new(120).unwrap();
        assert!(high.transpose(Semitones::new(20.0)).is_none());

        let low = Pitch::new(10).unwrap();
        assert!(low.transpose(Semitones::new(-20.0)).is_none());
    }

    #[test]
    fn test_velocity_creation() {
        // New Velocity type uses normalized f32 (0.0-1.0)
        let vel = Velocity::new(0.5);
        assert!((vel.as_f32() - 0.5).abs() < 0.01);

        // Values are clamped to 0.0-1.0
        let vel_clamped = Velocity::new(2.0);
        assert!((vel_clamped.as_f32() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_velocity_conversion() {
        let vel = Velocity::from_midi(127);
        assert!((vel.as_f32() - 1.0).abs() < 0.01);

        let vel2 = Velocity::new(0.5);
        assert_eq!(vel2.to_midi(), 64); // 0.5 * 127 rounded = 64
    }

    #[test]
    fn test_note_name_black_keys() {
        assert!(NoteName::Cs.is_black_key());
        assert!(NoteName::Fs.is_black_key());
        assert!(!NoteName::C.is_black_key());
        assert!(!NoteName::E.is_black_key());
    }
}
