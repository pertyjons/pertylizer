//! Pitch-related types for type-safe audio processing.

use std::ops::{Add, Div, Mul, Neg, Sub};

use serde::{Deserialize, Serialize};

use super::{Hertz, ValueRange};

/// Pitch offset in cents (1/100th of a semitone).
///
/// 100 cents = 1 semitone
/// 1200 cents = 1 octave
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Cents(f32);

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

    /// Bounds for the oscillator **fine** detune control (±1 semitone).
    ///
    /// Single source of truth shared by the descriptor `.value_range(...)`, the
    /// GUI/MCP `with_f32` apply clamp, and [`Self::clamp_detune`] — so the three
    /// can never drift. Default `0.0` matches [`Self::ZERO`], the `Detune` param
    /// variant's default.
    pub const DETUNE_RANGE: ValueRange = ValueRange::new(-100.0, 100.0, 0.0);

    /// Bounds for the oscillator **unison** detune spread (0–100 ¢, default 10).
    ///
    /// Distinct from [`Self::DETUNE_RANGE`]: unison spread is unipolar and
    /// defaults to 10 ¢. Single source of truth shared by the `uni_detune`
    /// descriptor, the `with_f32` apply clamp, and the `set_param` clamp.
    pub const UNISON_DETUNE_RANGE: ValueRange = ValueRange::new(0.0, 100.0, 10.0);

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
        Hertz::new(freq.as_f32() * self.to_ratio())
    }

    /// Clamp to the oscillator fine-detune range ([`Self::DETUNE_RANGE`]).
    #[inline]
    pub fn clamp_detune(self) -> Self {
        Self::new(Self::DETUNE_RANGE.clamp(self.0))
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
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Semitones(f32);

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
        Hertz::new(freq.as_f32() * self.to_ratio())
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
pub struct Octaves(i32);

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
        Hertz::new(freq.as_f32() * self.to_ratio())
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, schemars::JsonSchema,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MidiNote(#[schemars(range(min = 0, max = 127))] u8);

impl MidiNote {
    /// Create a new MIDI note (clamped to 0-127).
    #[inline]
    pub const fn new(note: u8) -> Self {
        Self(if note > 127 { 127 } else { note })
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
    ///
    /// Returns `None` if the result would be outside valid MIDI range (0-127).
    #[inline]
    #[allow(clippy::cast_possible_truncation)]
    pub fn transpose(self, semitones: Semitones) -> Option<Self> {
        let transposed = self.0 as i16 + semitones.as_f32().round() as i16;
        if (0..=127).contains(&transposed) {
            Some(Self(transposed as u8))
        } else {
            None
        }
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
        Self::new(note)
    }
}

impl From<MidiNote> for u8 {
    fn from(note: MidiNote) -> Self {
        note.0
    }
}

impl<'de> Deserialize<'de> for MidiNote {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let note = u8::deserialize(deserializer)?;
        if note <= Self::MAX.as_u8() {
            Ok(Self(note))
        } else {
            Err(serde::de::Error::custom("MIDI note must be in 0..=127"))
        }
    }
}

impl std::fmt::Display for MidiNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// MIDI velocity (0-127, normalized to 0.0-1.0).
///
/// Represents how hard a note was played.
/// Stored internally as normalized f32 for smooth processing.
#[derive(
    Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(from = "f32", into = "f32")]
#[repr(transparent)]
pub struct Velocity(f32);

impl Velocity {
    /// Create a new velocity from normalized value (0.0-1.0).
    #[inline]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    /// Create from MIDI velocity (0-127).
    #[inline]
    pub fn from_midi(midi_velocity: u8) -> Self {
        Self(midi_velocity as f32 / 127.0)
    }

    /// Create without clamping (for performance in hot paths).
    #[inline]
    pub const fn new_unchecked(value: f32) -> Self {
        debug_assert!(
            value >= 0.0 && value <= 1.0,
            "Velocity::new_unchecked: value outside [0, 1]"
        );
        Self(value)
    }

    /// Zero velocity (note off).
    pub const ZERO: Self = Self(0.0);

    /// Note off / silent (alias for ZERO).
    pub const OFF: Self = Self(0.0);

    /// Maximum velocity.
    pub const MAX: Self = Self(1.0);

    /// Default velocity (mezzo-forte, MIDI 100).
    pub const DEFAULT: Self = Self(100.0 / 127.0);

    // === Musical dynamics constants ===

    /// Pianissississimo (MIDI 16).
    pub const PPP: Self = Self(16.0 / 127.0);

    /// Pianissimo (MIDI 32).
    pub const PP: Self = Self(32.0 / 127.0);

    /// Piano (soft, MIDI 48).
    pub const P: Self = Self(48.0 / 127.0);

    /// Mezzo-piano (MIDI 64).
    pub const MP: Self = Self(64.0 / 127.0);

    /// Mezzo-forte (MIDI 80).
    pub const MF: Self = Self(80.0 / 127.0);

    /// Forte (loud, MIDI 96).
    pub const F: Self = Self(96.0 / 127.0);

    /// Fortissimo (MIDI 112).
    pub const FF: Self = Self(112.0 / 127.0);

    /// Fortississimo (MIDI 127, alias for MAX).
    pub const FFF: Self = Self(1.0);

    /// Get the normalized value (0.0-1.0).
    #[inline]
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// Convert to MIDI velocity (0-127).
    #[inline]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn to_midi(self) -> u8 {
        (self.0 * 127.0).round() as u8
    }

    /// Apply velocity curve (power function).
    ///
    /// curve = 1.0: linear
    /// curve < 1.0: more sensitive to soft playing
    /// curve > 1.0: more sensitive to hard playing
    #[inline]
    pub fn curve(self, exponent: f32) -> Self {
        Self(self.0.powf(exponent))
    }

    /// Scale an amplitude by this velocity.
    #[inline]
    pub fn scale(self, value: f32) -> f32 {
        value * self.0
    }

    /// Linear interpolation between two velocities.
    #[inline]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.0 + (other.0 - self.0) * t)
    }

    /// Check if this is effectively zero (note off).
    #[inline]
    pub fn is_zero(self) -> bool {
        self.0 < 0.001
    }

    /// Get the dynamics name for this velocity.
    #[must_use]
    pub fn dynamics_name(&self) -> &'static str {
        let midi = self.to_midi();
        match midi {
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
        Self::DEFAULT
    }
}

impl From<f32> for Velocity {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<Velocity> for f32 {
    fn from(vel: Velocity) -> Self {
        vel.0
    }
}

impl From<u8> for Velocity {
    fn from(midi_velocity: u8) -> Self {
        Self::from_midi(midi_velocity)
    }
}

impl std::fmt::Display for Velocity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_midi())
    }
}

/// MIDI channel (1-16).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, schemars::JsonSchema,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MidiChannel(#[schemars(range(min = 1, max = 16))] u8);

impl MidiChannel {
    /// Create a new MIDI channel (1-16).
    #[inline]
    pub const fn new(channel: u8) -> Self {
        Self(if channel == 0 {
            1
        } else if channel > 16 {
            16
        } else {
            channel
        })
    }

    /// Channel 1 (default).
    pub const CH1: Self = Self(1);

    /// Channel 10 (drums in GM).
    pub const DRUMS: Self = Self(10);

    /// Get the channel number (1-16).
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Get as zero-indexed (0-15) for MIDI messages.
    #[inline]
    pub const fn as_index(self) -> u8 {
        self.0 - 1
    }
}

impl Default for MidiChannel {
    fn default() -> Self {
        Self::CH1
    }
}

impl From<u8> for MidiChannel {
    fn from(channel: u8) -> Self {
        Self::new(channel)
    }
}

impl From<MidiChannel> for u8 {
    fn from(channel: MidiChannel) -> Self {
        channel.0
    }
}

impl<'de> Deserialize<'de> for MidiChannel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let channel = u8::deserialize(deserializer)?;
        if (Self::CH1.as_u8()..=16).contains(&channel) {
            Ok(Self(channel))
        } else {
            Err(serde::de::Error::custom("MIDI channel must be in 1..=16"))
        }
    }
}

impl std::fmt::Display for MidiChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ch {}", self.0)
    }
}

/// MIDI controller number (0–127 for standard CCs, 128+ for pseudo-CCs).
///
/// Pseudo-CCs map non-CC controllers to a unified CC space for telemetry:
/// - 128 = Pitch Bend
/// - 129 = Channel Aftertouch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CcNumber(u8);

impl CcNumber {
    /// Mod wheel (CC1).
    pub const MOD_WHEEL: Self = Self(1);
    /// Pitch bend (pseudo-CC, not a real MIDI CC).
    pub const PITCH_BEND: Self = Self(128);
    /// Channel aftertouch (pseudo-CC, not a real MIDI CC).
    pub const AFTERTOUCH: Self = Self(129);

    /// Create a new CC number.
    #[inline]
    #[must_use]
    pub const fn new(cc: u8) -> Self {
        Self(cc)
    }

    /// Get the raw CC number.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl From<u8> for CcNumber {
    fn from(cc: u8) -> Self {
        Self(cc)
    }
}

impl From<CcNumber> for u8 {
    fn from(cc: CcNumber) -> Self {
        cc.0
    }
}

/// Standard MIDI continuous-controller number (`0..=127`).
///
/// Unlike [`CcNumber`], this type excludes the pseudo-CC values used by the
/// telemetry subsystem.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct MidiCcNumber(#[schemars(range(min = 0, max = 127))] u8);

impl MidiCcNumber {
    pub const MIN: u8 = 0;
    pub const MAX: u8 = 127;
    pub const MOD_WHEEL: Self = Self(1);

    /// Create a standard MIDI CC number.
    pub const fn new(cc: u8) -> Option<Self> {
        if cc <= Self::MAX {
            Some(Self(cc))
        } else {
            None
        }
    }

    /// Return the raw MIDI CC number.
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for MidiCcNumber {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let cc = u8::deserialize(deserializer)?;
        Self::new(cc).ok_or_else(|| serde::de::Error::custom("MIDI CC must be in 0..=127"))
    }
}

impl From<MidiCcNumber> for u8 {
    fn from(cc: MidiCcNumber) -> Self {
        cc.0
    }
}

impl std::fmt::Display for MidiCcNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_velocity_from_midi() {
        let vel = Velocity::from_midi(127);
        assert!((vel.as_f32() - 1.0).abs() < 0.01);

        let vel = Velocity::from_midi(0);
        assert!((vel.as_f32() - 0.0).abs() < 0.01);

        let vel = Velocity::from_midi(64);
        assert!((vel.as_f32() - 0.504).abs() < 0.01);
    }

    #[test]
    fn test_velocity_to_midi() {
        assert_eq!(Velocity::MAX.to_midi(), 127);
        assert_eq!(Velocity::ZERO.to_midi(), 0);
    }

    #[test]
    fn test_midi_channel() {
        let ch = MidiChannel::new(10);
        assert_eq!(ch.as_u8(), 10);
        assert_eq!(ch.as_index(), 9);
    }

    #[test]
    fn midi_channel_serde_is_one_indexed_and_validated() {
        assert!(matches!(
            serde_json::to_string(&MidiChannel::CH1).as_deref(),
            Ok("1")
        ));
        assert!(matches!(
            serde_json::from_str::<MidiChannel>("16").map(MidiChannel::as_u8),
            Ok(16)
        ));
        assert!(serde_json::from_str::<MidiChannel>("0").is_err());
        assert!(serde_json::from_str::<MidiChannel>("17").is_err());
    }

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
        assert!((a4.to_frequency().as_f32() - 440.0).abs() < 0.001);
    }

    #[test]
    fn midi_note_serde_rejects_values_above_127() {
        assert!(matches!(
            serde_json::to_string(&MidiNote::C4).as_deref(),
            Ok("60")
        ));
        assert!(matches!(
            serde_json::from_str::<MidiNote>("127").map(MidiNote::as_u8),
            Ok(127)
        ));
        assert!(serde_json::from_str::<MidiNote>("128").is_err());
    }

    #[test]
    fn midi_cc_number_serde_rejects_pseudo_controllers() {
        assert!(matches!(
            serde_json::from_str::<MidiCcNumber>("127").map(MidiCcNumber::as_u8),
            Ok(127)
        ));
        assert!(serde_json::from_str::<MidiCcNumber>("128").is_err());
        let schema = serde_json::to_value(schemars::schema_for!(MidiCcNumber)).unwrap_or_default();
        assert_eq!(schema["maximum"], 127);
    }
}
