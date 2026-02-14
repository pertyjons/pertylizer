//! Microtonal tuning support.
//!
//! Provides alternative tuning systems beyond standard 12-TET, including
//! just intonation, Pythagorean tuning, and arbitrary Scala file import.

use serde::{Deserialize, Serialize};

use crate::types::{Hertz, MidiNote};

// ============================================================================
// TUNING ERROR
// ============================================================================

/// Errors that can occur when parsing or constructing tuning tables.
#[derive(Debug, thiserror::Error)]
pub enum TuningError {
    /// Invalid Scala .scl file format.
    #[error("Invalid Scala file: {0}")]
    InvalidScala(String),

    /// Invalid keyboard mapping (.kbm) format.
    #[error("Invalid keyboard mapping: {0}")]
    InvalidMapping(String),
}

// ============================================================================
// TUNING TABLE
// ============================================================================

/// A lookup table mapping MIDI notes (0-127) to frequencies.
///
/// Supports standard 12-TET as well as microtonal tuning systems
/// such as just intonation, Pythagorean tuning, and arbitrary scales
/// loaded from Scala .scl files.
///
/// # Example
///
/// ```ignore
/// use synth_core::tuning::TuningTable;
/// use synth_core::types::MidiNote;
///
/// let table = TuningTable::equal_temperament();
/// let a4 = table.note_to_freq(MidiNote::A4);
/// assert!((a4.as_f32() - 440.0).abs() < 0.01);
/// ```
#[derive(Debug, Clone)]
pub struct TuningTable {
    /// Frequency for each MIDI note (0-127).
    frequencies: [Hertz; 128],
    /// Name of the tuning.
    name: &'static str,
}

impl TuningTable {
    /// Standard 12-tone equal temperament (A4 = 440 Hz).
    ///
    /// Each semitone is exactly `2^(1/12)` apart. This is the standard
    /// tuning used in virtually all Western music since the 20th century.
    #[must_use]
    pub fn equal_temperament() -> Self {
        let mut frequencies = [Hertz::new(0.0); 128];
        for i in 0..128u8 {
            frequencies[i as usize] = Hertz::new(440.0 * 2.0f32.powf((i as f32 - 69.0) / 12.0));
        }
        Self {
            frequencies,
            name: "12-TET",
        }
    }

    /// Just intonation based on A4 = 440 Hz.
    ///
    /// Uses small-integer frequency ratios for pure intervals.
    /// The scale is built relative to C, with these ratios per degree:
    ///
    /// | Degree | Ratio | Interval       |
    /// |--------|-------|----------------|
    /// | 0      | 1/1   | Unison         |
    /// | 1      | 16/15 | Minor 2nd      |
    /// | 2      | 9/8   | Major 2nd      |
    /// | 3      | 6/5   | Minor 3rd      |
    /// | 4      | 5/4   | Major 3rd      |
    /// | 5      | 4/3   | Perfect 4th    |
    /// | 6      | 45/32 | Tritone        |
    /// | 7      | 3/2   | Perfect 5th    |
    /// | 8      | 8/5   | Minor 6th      |
    /// | 9      | 5/3   | Major 6th      |
    /// | 10     | 9/5   | Minor 7th      |
    /// | 11     | 15/8  | Major 7th      |
    #[must_use]
    pub fn just_intonation() -> Self {
        let ratios: [f32; 12] = [
            1.0,
            16.0 / 15.0,
            9.0 / 8.0,
            6.0 / 5.0,
            5.0 / 4.0,
            4.0 / 3.0,
            45.0 / 32.0,
            3.0 / 2.0,
            8.0 / 5.0,
            5.0 / 3.0,
            9.0 / 5.0,
            15.0 / 8.0,
        ];

        Self::from_ratios(&ratios, "Just Intonation")
    }

    /// Pythagorean tuning based on pure 3:2 fifths.
    ///
    /// All intervals are derived from stacking perfect fifths (ratio 3/2)
    /// and reducing by octaves. This produces very pure fifths and fourths
    /// but noticeably impure thirds.
    #[must_use]
    pub fn pythagorean() -> Self {
        // Build 12 notes by stacking fifths: F C G D A E B F# C# G# D# A#
        // Fifths order from C: 0, +1(G), +2(D), +3(A), +4(E), +5(B),
        //                      +6(F#), -5(Gb/F#)... or -1(F), -2(Bb), etc.
        // Using the circle of fifths: F(-1), C(0), G(1), D(2), A(3), E(4), B(5)
        // Mapping: fifth_index -> pitch_class
        // -1->F(5), 0->C(0), 1->G(7), 2->D(2), 3->A(9), 4->E(4), 5->B(11)
        // 6->F#(6), -5->Db(1), -4->Ab(8), -3->Eb(3), -2->Bb(10)

        let fifth_indices: [(i32, usize); 12] = [
            (0, 0),   // C
            (-5, 1),  // Db
            (2, 2),   // D
            (-3, 3),  // Eb
            (4, 4),   // E
            (-1, 5),  // F
            (6, 6),   // F#
            (1, 7),   // G
            (-4, 8),  // Ab
            (3, 9),   // A
            (-2, 10), // Bb
            (5, 11),  // B
        ];

        let mut ratios = [0.0f32; 12];
        for &(fifth_count, pitch_class) in &fifth_indices {
            // ratio = (3/2)^fifth_count, then reduce to one octave
            let raw = (3.0f32 / 2.0).powi(fifth_count);
            // Normalize to range [1.0, 2.0)
            let normalized = if raw >= 1.0 {
                raw / 2.0f32.powi(raw.log2().floor() as i32)
            } else {
                raw * 2.0f32.powi((-raw.log2()).ceil() as i32)
            };
            ratios[pitch_class] = normalized;
        }

        Self::from_ratios(&ratios, "Pythagorean")
    }

    /// 19-tone equal temperament.
    ///
    /// Divides the octave into 19 equal steps. Provides better approximations
    /// of just major thirds (5/4) and minor thirds (6/5) compared to 12-TET,
    /// at the cost of slightly less pure fifths.
    #[must_use]
    pub fn equal_temperament_19() -> Self {
        Self::equal_temperament_n(19, "19-TET")
    }

    /// 31-tone equal temperament.
    ///
    /// Divides the octave into 31 equal steps. Provides excellent
    /// approximations of many just intervals, including 7-limit intervals.
    #[must_use]
    pub fn equal_temperament_31() -> Self {
        Self::equal_temperament_n(31, "31-TET")
    }

    /// Parse a Scala .scl file and optional .kbm keyboard mapping.
    ///
    /// The .scl format specifies pitch values as either cents or ratios:
    /// - Lines starting with `!` are comments
    /// - First non-comment line: description
    /// - Second non-comment line: number of notes per octave
    /// - Remaining lines: pitch values (cents if contains `.`, ratio if contains `/`,
    ///   otherwise integer cents)
    ///
    /// # Errors
    ///
    /// Returns `TuningError::InvalidScala` if the file format is invalid,
    /// or `TuningError::InvalidMapping` if the keyboard mapping is invalid.
    pub fn from_scala(scl_data: &str, kbm_data: Option<&str>) -> Result<Self, TuningError> {
        let pitches = parse_scala(scl_data)?;
        let mapping = if let Some(kbm) = kbm_data {
            parse_kbm(kbm)?
        } else {
            KeyboardMapping::default()
        };

        let mut frequencies = [Hertz::new(0.0); 128];
        let reference_freq = mapping.reference_frequency;
        let reference_note = mapping.reference_note as i32;
        let scale_size = pitches.len();

        if scale_size == 0 {
            return Err(TuningError::InvalidScala(
                "Scale must have at least one pitch".to_string(),
            ));
        }

        // The last pitch in the scale defines the "equave" (usually 2/1 = octave)
        let equave_ratio = pitches[scale_size - 1];

        for midi_note in 0..128i32 {
            let steps_from_ref = midi_note - reference_note;

            // How many complete equaves and remaining steps
            let octave_offset = steps_from_ref.div_euclid(scale_size as i32);
            let scale_degree = steps_from_ref.rem_euclid(scale_size as i32) as usize;

            // Get the ratio for this scale degree (degree 0 = unison = ratio 1.0)
            let degree_ratio = if scale_degree == 0 {
                1.0
            } else {
                pitches[scale_degree - 1]
            };

            let octave_ratio = equave_ratio.powi(octave_offset);
            frequencies[midi_note as usize] =
                Hertz::new(reference_freq * degree_ratio * octave_ratio);
        }

        Ok(Self {
            frequencies,
            // Scala tunings get a static name since we can't store the dynamic description
            name: "Scala Tuning",
        })
    }

    /// Look up the frequency for a MIDI note.
    #[inline]
    #[must_use]
    pub fn note_to_freq(&self, note: MidiNote) -> Hertz {
        self.frequencies[note.as_u8() as usize]
    }

    /// Get the tuning name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.name
    }

    // === Internal helpers ===

    /// Build a tuning table from 12-note ratios relative to C, with A4 = 440 Hz.
    fn from_ratios(ratios: &[f32], name: &'static str) -> Self {
        // A4 = MIDI 69, pitch class 9 (A), octave 5 in MIDI terms (69/12 = 5 remainder 9)
        // C in the same octave as A4 is C4 = MIDI 60
        // We define ratios relative to C within an octave.
        // Frequency of C4 = 440.0 / ratios[9] (since A is pitch class 9)
        let c4_freq = 440.0 / ratios[9];

        let mut frequencies = [Hertz::new(0.0); 128];
        for i in 0..128u8 {
            let pitch_class = (i % 12) as usize;
            let octave = (i as i32 / 12) - 5; // relative to octave containing C4 (MIDI 60)
            let ratio = ratios[pitch_class];
            let freq = c4_freq * ratio * 2.0f32.powi(octave);
            frequencies[i as usize] = Hertz::new(freq);
        }

        Self { frequencies, name }
    }

    /// Build an N-tone equal temperament mapped onto 128 MIDI notes.
    ///
    /// Maps MIDI note 69 to 440 Hz (A4), then spaces notes by `2^(1/n)`.
    fn equal_temperament_n(divisions: u32, name: &'static str) -> Self {
        let mut frequencies = [Hertz::new(0.0); 128];
        let n = divisions as f32;
        for i in 0..128u8 {
            // Map linearly: MIDI note difference from 69 maps directly to
            // steps in the N-TET system
            frequencies[i as usize] = Hertz::new(440.0 * 2.0f32.powf((i as f32 - 69.0) / n));
        }
        Self { frequencies, name }
    }
}

impl Default for TuningTable {
    fn default() -> Self {
        Self::equal_temperament()
    }
}

// ============================================================================
// TUNING PRESET ENUM
// ============================================================================

/// Preset tuning systems.
///
/// Provides a serializable enum for selecting between built-in tuning tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TuningPreset {
    /// Standard 12-tone equal temperament (A4 = 440 Hz).
    #[default]
    EqualTemperament,
    /// Just intonation with pure integer ratios.
    JustIntonation,
    /// Pythagorean tuning based on stacked 3:2 fifths.
    Pythagorean,
    /// 19-tone equal temperament.
    ET19,
    /// 31-tone equal temperament.
    ET31,
}

impl TuningPreset {
    /// All available tuning presets.
    pub const ALL: [Self; 5] = [
        Self::EqualTemperament,
        Self::JustIntonation,
        Self::Pythagorean,
        Self::ET19,
        Self::ET31,
    ];

    /// Get the display name.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::EqualTemperament => "12-TET",
            Self::JustIntonation => "Just Intonation",
            Self::Pythagorean => "Pythagorean",
            Self::ET19 => "19-TET",
            Self::ET31 => "31-TET",
        }
    }

    /// Get a unique string identifier for serialization.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::EqualTemperament => "equal_temperament",
            Self::JustIntonation => "just_intonation",
            Self::Pythagorean => "pythagorean",
            Self::ET19 => "et19",
            Self::ET31 => "et31",
        }
    }

    /// Look up a preset by its string identifier.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "equal_temperament" => Some(Self::EqualTemperament),
            "just_intonation" => Some(Self::JustIntonation),
            "pythagorean" => Some(Self::Pythagorean),
            "et19" => Some(Self::ET19),
            "et31" => Some(Self::ET31),
            _ => None,
        }
    }

    /// Look up a preset by its index in `ALL`.
    #[must_use]
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// Get the index of this preset in `ALL`.
    #[must_use]
    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|p| p == self).unwrap_or(0)
    }

    /// Build the corresponding `TuningTable`.
    #[must_use]
    pub fn to_table(self) -> TuningTable {
        match self {
            Self::EqualTemperament => TuningTable::equal_temperament(),
            Self::JustIntonation => TuningTable::just_intonation(),
            Self::Pythagorean => TuningTable::pythagorean(),
            Self::ET19 => TuningTable::equal_temperament_19(),
            Self::ET31 => TuningTable::equal_temperament_31(),
        }
    }
}

// ============================================================================
// SCALA PARSER INTERNALS
// ============================================================================

/// Internal keyboard mapping representation.
struct KeyboardMapping {
    reference_note: u8,
    reference_frequency: f32,
}

impl Default for KeyboardMapping {
    fn default() -> Self {
        Self {
            reference_note: 69, // A4
            reference_frequency: 440.0,
        }
    }
}

/// Parse a Scala .scl file into a list of pitch ratios (as linear ratios).
///
/// Returns one ratio per scale degree (excluding the implicit 1/1 unison).
/// The last entry is the interval of equivalence (typically 2/1 for an octave).
fn parse_scala(scl_data: &str) -> Result<Vec<f32>, TuningError> {
    let mut non_comment_lines = scl_data
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('!') && !line.is_empty());

    // First non-comment line: description (ignored but must be present)
    let _description = non_comment_lines
        .next()
        .ok_or_else(|| TuningError::InvalidScala("Missing description line".to_string()))?;

    // Second non-comment line: number of notes
    let count_line = non_comment_lines
        .next()
        .ok_or_else(|| TuningError::InvalidScala("Missing note count line".to_string()))?;

    let note_count: usize = count_line
        .parse()
        .map_err(|_| TuningError::InvalidScala(format!("Invalid note count: {count_line}")))?;

    // Remaining lines: pitch values
    let mut pitches = Vec::with_capacity(note_count);
    for line in non_comment_lines.take(note_count) {
        // Take only the pitch value (ignore trailing comments/whitespace)
        let token = line
            .split_whitespace()
            .next()
            .ok_or_else(|| TuningError::InvalidScala(format!("Empty pitch line: {line}")))?;

        let ratio = parse_pitch_value(token)?;
        pitches.push(ratio);
    }

    if pitches.len() != note_count {
        return Err(TuningError::InvalidScala(format!(
            "Expected {note_count} pitches, found {}",
            pitches.len()
        )));
    }

    Ok(pitches)
}

/// Parse a single pitch value from a Scala file.
///
/// - Contains `/` -> ratio (e.g. "3/2" -> 1.5)
/// - Contains `.` -> cents (e.g. "701.955" -> 2^(701.955/1200))
/// - Otherwise -> integer cents (e.g. "700" -> 2^(700/1200))
fn parse_pitch_value(token: &str) -> Result<f32, TuningError> {
    if token.contains('/') {
        // Parse as ratio
        let parts: Vec<&str> = token.split('/').collect();
        if parts.len() != 2 {
            return Err(TuningError::InvalidScala(format!("Invalid ratio: {token}")));
        }
        let numerator: f32 = parts[0]
            .trim()
            .parse()
            .map_err(|_| TuningError::InvalidScala(format!("Invalid ratio numerator: {token}")))?;
        let denominator: f32 = parts[1].trim().parse().map_err(|_| {
            TuningError::InvalidScala(format!("Invalid ratio denominator: {token}"))
        })?;
        if denominator == 0.0 {
            return Err(TuningError::InvalidScala(
                "Division by zero in ratio".to_string(),
            ));
        }
        Ok(numerator / denominator)
    } else if token.contains('.') {
        // Parse as cents
        let cents: f32 = token
            .parse()
            .map_err(|_| TuningError::InvalidScala(format!("Invalid cents value: {token}")))?;
        Ok(2.0f32.powf(cents / 1200.0))
    } else {
        // Parse as integer cents
        let cents: i32 = token
            .parse()
            .map_err(|_| TuningError::InvalidScala(format!("Invalid integer cents: {token}")))?;
        Ok(2.0f32.powf(cents as f32 / 1200.0))
    }
}

/// Parse a Scala .kbm keyboard mapping file.
///
/// Extracts the reference note and reference frequency for mapping
/// scale degrees to MIDI notes.
fn parse_kbm(kbm_data: &str) -> Result<KeyboardMapping, TuningError> {
    let non_comment_lines: Vec<&str> = kbm_data
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('!') && !line.is_empty())
        .collect();

    // KBM format:
    // Line 0: Size of map (number of notes to retune)
    // Line 1: First MIDI note number to retune
    // Line 2: Last MIDI note number to retune
    // Line 3: Middle note where scale degree 0 is mapped
    // Line 4: Reference note for frequency
    // Line 5: Reference frequency (Hz)
    // Line 6: Scale degree to consider as formal octave
    // Remaining: key mapping entries

    if non_comment_lines.len() < 6 {
        return Err(TuningError::InvalidMapping(
            "KBM file too short, need at least 6 lines".to_string(),
        ));
    }

    let reference_note: u8 = non_comment_lines[4].parse().map_err(|_| {
        TuningError::InvalidMapping(format!("Invalid reference note: {}", non_comment_lines[4]))
    })?;

    let reference_frequency: f32 = non_comment_lines[5].parse().map_err(|_| {
        TuningError::InvalidMapping(format!(
            "Invalid reference frequency: {}",
            non_comment_lines[5]
        ))
    })?;

    Ok(KeyboardMapping {
        reference_note,
        reference_frequency,
    })
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Equal temperament tests ---

    #[test]
    fn equal_temperament_a4_is_440() {
        let table = TuningTable::equal_temperament();
        let freq = table.note_to_freq(MidiNote::A4);
        assert!(
            (freq.as_f32() - 440.0).abs() < 0.01,
            "A4 should be 440 Hz, got {}",
            freq.as_f32()
        );
    }

    #[test]
    fn equal_temperament_middle_c() {
        let table = TuningTable::equal_temperament();
        let freq = table.note_to_freq(MidiNote::C4);
        assert!(
            (freq.as_f32() - 261.63).abs() < 0.1,
            "C4 should be ~261.63 Hz, got {}",
            freq.as_f32()
        );
    }

    #[test]
    fn equal_temperament_octave_doubling() {
        let table = TuningTable::equal_temperament();
        let a3 = table.note_to_freq(MidiNote::new(57)); // A3
        let a4 = table.note_to_freq(MidiNote::A4); // A4
        let a5 = table.note_to_freq(MidiNote::new(81)); // A5
        assert!(
            (a4.as_f32() / a3.as_f32() - 2.0).abs() < 0.001,
            "Octave should double frequency"
        );
        assert!(
            (a5.as_f32() / a4.as_f32() - 2.0).abs() < 0.001,
            "Octave should double frequency"
        );
    }

    #[test]
    fn equal_temperament_name() {
        let table = TuningTable::equal_temperament();
        assert_eq!(table.name(), "12-TET");
    }

    // --- Just intonation tests ---

    #[test]
    fn just_intonation_a4_is_440() {
        let table = TuningTable::just_intonation();
        let freq = table.note_to_freq(MidiNote::A4);
        assert!(
            (freq.as_f32() - 440.0).abs() < 0.1,
            "JI A4 should be 440 Hz, got {}",
            freq.as_f32()
        );
    }

    #[test]
    fn just_intonation_perfect_fifth() {
        let table = TuningTable::just_intonation();
        let c4 = table.note_to_freq(MidiNote::C4);
        let g4 = table.note_to_freq(MidiNote::new(67)); // G4
        let ratio = g4.as_f32() / c4.as_f32();
        assert!(
            (ratio - 1.5).abs() < 0.001,
            "JI perfect fifth should be 3/2, got {}",
            ratio
        );
    }

    #[test]
    fn just_intonation_major_third() {
        let table = TuningTable::just_intonation();
        let c4 = table.note_to_freq(MidiNote::C4);
        let e4 = table.note_to_freq(MidiNote::new(64)); // E4
        let ratio = e4.as_f32() / c4.as_f32();
        assert!(
            (ratio - 1.25).abs() < 0.001,
            "JI major third should be 5/4, got {}",
            ratio
        );
    }

    #[test]
    fn just_intonation_octave() {
        let table = TuningTable::just_intonation();
        let c4 = table.note_to_freq(MidiNote::C4);
        let c5 = table.note_to_freq(MidiNote::new(72)); // C5
        let ratio = c5.as_f32() / c4.as_f32();
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "JI octave should be 2/1, got {}",
            ratio
        );
    }

    // --- Pythagorean tests ---

    #[test]
    fn pythagorean_perfect_fifth() {
        let table = TuningTable::pythagorean();
        let c4 = table.note_to_freq(MidiNote::C4);
        let g4 = table.note_to_freq(MidiNote::new(67));
        let ratio = g4.as_f32() / c4.as_f32();
        assert!(
            (ratio - 1.5).abs() < 0.001,
            "Pythagorean fifth should be 3/2, got {}",
            ratio
        );
    }

    #[test]
    fn pythagorean_octave() {
        let table = TuningTable::pythagorean();
        let c4 = table.note_to_freq(MidiNote::C4);
        let c5 = table.note_to_freq(MidiNote::new(72));
        let ratio = c5.as_f32() / c4.as_f32();
        assert!(
            (ratio - 2.0).abs() < 0.01,
            "Pythagorean octave should be 2/1, got {}",
            ratio
        );
    }

    // --- N-TET tests ---

    #[test]
    fn et19_a4_is_440() {
        let table = TuningTable::equal_temperament_19();
        let freq = table.note_to_freq(MidiNote::A4);
        assert!(
            (freq.as_f32() - 440.0).abs() < 0.01,
            "19-TET A4 should be 440 Hz, got {}",
            freq.as_f32()
        );
    }

    #[test]
    fn et31_a4_is_440() {
        let table = TuningTable::equal_temperament_31();
        let freq = table.note_to_freq(MidiNote::A4);
        assert!(
            (freq.as_f32() - 440.0).abs() < 0.01,
            "31-TET A4 should be 440 Hz, got {}",
            freq.as_f32()
        );
    }

    // --- Scala parser tests ---

    #[test]
    fn parse_scala_12tet() {
        let scl = "\
! 12-TET
12-tone equal temperament
12
100.0
200.0
300.0
400.0
500.0
600.0
700.0
800.0
900.0
1000.0
1100.0
1200.0
";
        let table = TuningTable::from_scala(scl, None).unwrap();
        let a4 = table.note_to_freq(MidiNote::A4);
        assert!(
            (a4.as_f32() - 440.0).abs() < 0.5,
            "Scala 12-TET A4 should be ~440 Hz, got {}",
            a4.as_f32()
        );
    }

    #[test]
    fn parse_scala_just_ratios() {
        let scl = "\
! Just intonation
Just intonation scale
7
9/8
5/4
4/3
3/2
5/3
15/8
2/1
";
        let table = TuningTable::from_scala(scl, None).unwrap();
        // Check that the fifth (degree 4, ratio 3/2) is correct
        // Reference note = 69 (A4). Scale has 7 notes.
        // Note 69+4 = 73 should be 440.0 * 3/2 = 660.0 approximately
        let ref_freq = table.note_to_freq(MidiNote::A4);
        let fifth_up = table.note_to_freq(MidiNote::new(73));
        let ratio = fifth_up.as_f32() / ref_freq.as_f32();
        assert!(
            (ratio - 1.5).abs() < 0.01,
            "Scala JI fifth should be 3/2, got {}",
            ratio
        );
    }

    #[test]
    fn parse_scala_invalid_empty() {
        let result = TuningTable::from_scala("", None);
        assert!(result.is_err());
    }

    #[test]
    fn parse_scala_invalid_count() {
        let scl = "\
! test
description
abc
";
        let result = TuningTable::from_scala(scl, None);
        assert!(result.is_err());
    }

    #[test]
    fn parse_scala_with_integer_cents() {
        let scl = "\
! Whole tone
Whole tone scale
6
200
400
600
800
1000
1200
";
        let table = TuningTable::from_scala(scl, None).unwrap();
        // Each step should be 200 cents = 2^(200/1200) ~= 1.1225
        let n69 = table.note_to_freq(MidiNote::A4);
        let n70 = table.note_to_freq(MidiNote::new(70));
        let ratio = n70.as_f32() / n69.as_f32();
        let expected = 2.0f32.powf(200.0 / 1200.0);
        assert!(
            (ratio - expected).abs() < 0.01,
            "200 cent step ratio should be {expected}, got {ratio}"
        );
    }

    #[test]
    fn parse_scala_with_kbm() {
        let scl = "\
! test
test scale
12
100.0
200.0
300.0
400.0
500.0
600.0
700.0
800.0
900.0
1000.0
1100.0
1200.0
";
        let kbm = "\
! test mapping
12
0
127
60
60
261.6256
12
";
        let table = TuningTable::from_scala(scl, Some(kbm)).unwrap();
        let c4 = table.note_to_freq(MidiNote::C4);
        assert!(
            (c4.as_f32() - 261.6256).abs() < 0.1,
            "C4 with kbm should be ~261.63 Hz, got {}",
            c4.as_f32()
        );
    }

    // --- TuningPreset tests ---

    #[test]
    fn preset_all_variants() {
        assert_eq!(TuningPreset::ALL.len(), 5);
    }

    #[test]
    fn preset_default_is_equal_temperament() {
        assert_eq!(TuningPreset::default(), TuningPreset::EqualTemperament);
    }

    #[test]
    fn preset_id_roundtrip() {
        for preset in &TuningPreset::ALL {
            let id = preset.id();
            let recovered = TuningPreset::from_id(id);
            assert_eq!(
                recovered,
                Some(*preset),
                "Roundtrip failed for {:?} with id {:?}",
                preset,
                id
            );
        }
    }

    #[test]
    fn preset_index_roundtrip() {
        for (i, preset) in TuningPreset::ALL.iter().enumerate() {
            assert_eq!(preset.index(), i);
            assert_eq!(TuningPreset::from_index(i), Some(*preset));
        }
    }

    #[test]
    fn preset_from_id_unknown() {
        assert_eq!(TuningPreset::from_id("unknown"), None);
    }

    #[test]
    fn preset_from_index_out_of_bounds() {
        assert_eq!(TuningPreset::from_index(99), None);
    }

    #[test]
    fn preset_to_table() {
        for preset in &TuningPreset::ALL {
            let table = preset.to_table();
            assert_eq!(table.name(), preset.name());
        }
    }

    #[test]
    fn default_tuning_table_is_equal_temperament() {
        let table = TuningTable::default();
        let freq = table.note_to_freq(MidiNote::A4);
        assert!(
            (freq.as_f32() - 440.0).abs() < 0.01,
            "Default tuning should be 12-TET"
        );
    }
}
