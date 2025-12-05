//! Pitch and velocity types for the sequencer.

use serde::{Deserialize, Serialize};

/// MIDI-compatible pitch (0-127).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Pitch(u8);

impl Pitch {
    pub const MIN: Pitch = Pitch(0);
    pub const MAX: Pitch = Pitch(127);
    pub const MIDDLE_C: Pitch = Pitch(60);

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
    pub fn frequency(&self, a4_hz: f32) -> f32 {
        a4_hz * 2.0_f32.powf((self.0 as f32 - 69.0) / 12.0)
    }

    /// Get the raw MIDI note number.
    pub fn as_midi(&self) -> u8 {
        self.0
    }

    /// Transpose by semitones, clamping to valid range.
    pub fn transpose(&self, semitones: i8) -> Self {
        let new_pitch = (self.0 as i16 + semitones as i16).clamp(0, 127) as u8;
        Pitch(new_pitch)
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

/// Velocity (0-127).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Velocity(u8);

impl Velocity {
    /// Note off / silent.
    pub const OFF: Velocity = Velocity(0);
    /// Pianissississimo.
    pub const PPP: Velocity = Velocity(16);
    /// Pianissimo.
    pub const PP: Velocity = Velocity(32);
    /// Piano.
    pub const P: Velocity = Velocity(48);
    /// Mezzo-piano.
    pub const MP: Velocity = Velocity(64);
    /// Mezzo-forte.
    pub const MF: Velocity = Velocity(80);
    /// Forte.
    pub const F: Velocity = Velocity(96);
    /// Fortissimo.
    pub const FF: Velocity = Velocity(112);
    /// Maximum velocity.
    pub const MAX: Velocity = Velocity(127);

    /// Create a new velocity value.
    pub fn new(vel: u8) -> Option<Self> {
        (vel <= 127).then_some(Self(vel))
    }

    /// Get the raw velocity value (0-127).
    pub fn as_u8(&self) -> u8 {
        self.0
    }

    /// Get the velocity as a normalized float (0.0-1.0).
    pub fn as_f32(&self) -> f32 {
        self.0 as f32 / 127.0
    }

    /// Create from a normalized float (0.0-1.0).
    pub fn from_f32(value: f32) -> Self {
        Velocity((value.clamp(0.0, 1.0) * 127.0) as u8)
    }

    /// Get the dynamics name.
    pub fn dynamics_name(&self) -> &'static str {
        match self.0 {
            0 => "off",
            1..=24 => "ppp",
            25..=40 => "pp",
            41..=56 => "p",
            57..=72 => "mp",
            73..=88 => "mf",
            89..=104 => "f",
            105..=120 => "ff",
            _ => "fff",
        }
    }
}

impl Default for Velocity {
    fn default() -> Self {
        Self::MF
    }
}

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
        assert!((a4.frequency(440.0) - 440.0).abs() < 0.01);

        let a5 = Pitch::new(81).unwrap();
        assert!((a5.frequency(440.0) - 880.0).abs() < 0.01);
    }

    #[test]
    fn test_pitch_transpose() {
        let c4 = Pitch::new(60).unwrap();
        assert_eq!(c4.transpose(12).as_midi(), 72); // C5
        assert_eq!(c4.transpose(-12).as_midi(), 48); // C3

        // Test clamping
        let high = Pitch::new(120).unwrap();
        assert_eq!(high.transpose(20).as_midi(), 127);

        let low = Pitch::new(10).unwrap();
        assert_eq!(low.transpose(-20).as_midi(), 0);
    }

    #[test]
    fn test_velocity_creation() {
        assert!(Velocity::new(64).is_some());
        assert!(Velocity::new(128).is_none());
    }

    #[test]
    fn test_velocity_conversion() {
        let vel = Velocity::new(127).unwrap();
        assert!((vel.as_f32() - 1.0).abs() < 0.01);

        let vel2 = Velocity::from_f32(0.5);
        assert_eq!(vel2.as_u8(), 63);
    }

    #[test]
    fn test_note_name_black_keys() {
        assert!(NoteName::Cs.is_black_key());
        assert!(NoteName::Fs.is_black_key());
        assert!(!NoteName::C.is_black_key());
        assert!(!NoteName::E.is_black_key());
    }
}
