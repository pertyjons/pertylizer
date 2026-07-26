//! Chord-symbol → MIDI notes.
//!
//! Parses chord symbols like `"Cm7"`, `"F#maj7"`, `"Bbsus4"` against the
//! same suffix table that [`crate::harmony::identify_chord`] emits, so a
//! round-trip (`identify` → `generate` → `identify`) stays stable. Adds a
//! voicing pass (close / drop2 / drop3 / open) on top of the raw interval
//! set.
//!
//! Pure symbolic — output is just MIDI numbers; placement into a pattern is
//! left to the caller.

use std::str::FromStr;

use crate::harmony::{CHORD_TEMPLATES, ChordTemplate};

/// Voicing applied to the close-position chord.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChordVoicing {
    /// Notes stacked above the root, all within one octave. Default.
    #[default]
    Close,
    /// Take the 2nd-highest note of the close voicing and drop it an octave.
    /// Standard jazz/keyboard voicing for 4-note chords.
    Drop2,
    /// Take the 3rd-highest note of the close voicing and drop it an octave.
    /// Useful for tetrads — degenerates to dropping the root on triads.
    Drop3,
    /// Spread voicing: drop2 + drop3 combined. Produces wide voicings well
    /// suited to pad / piano left-hand parts.
    Open,
}

/// Error returned when parsing a chord voicing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown voicing {0:?}; expected one of close, drop2, drop3, open")]
pub struct ParseChordVoicingError(String);

impl FromStr for ChordVoicing {
    type Err = ParseChordVoicingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "close" | "" => Ok(Self::Close),
            "drop2" | "drop-2" => Ok(Self::Drop2),
            "drop3" | "drop-3" => Ok(Self::Drop3),
            "open" | "spread" => Ok(Self::Open),
            _ => Err(ParseChordVoicingError(s.to_string())),
        }
    }
}

impl ChordVoicing {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Drop2 => "drop2",
            Self::Drop3 => "drop3",
            Self::Open => "open",
        }
    }
}

/// Result of [`generate_chord`].
#[must_use]
#[derive(Debug, Clone)]
pub struct GeneratedChord {
    /// MIDI notes in ascending order after voicing has been applied.
    pub notes: Vec<u8>,
    /// Root pitch class echoed back (0..12, C = 0).
    pub root_pitch_class: u8,
    /// Quality name from the matched template (e.g. `"minor7"`).
    pub quality: &'static str,
    /// Suffix that matched (e.g. `"m7"`).
    pub suffix: &'static str,
    /// Voicing that was actually applied.
    pub voicing: ChordVoicing,
    /// Non-fatal warnings (voicings that degraded to close, notes clamped to
    /// the valid MIDI range, etc.).
    pub warnings: Vec<String>,
}

/// Error returned by [`generate_chord`] for malformed input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenerateChordError {
    #[error("chord symbol is empty")]
    EmptySymbol,
    #[error("unknown chord root in {0:?}")]
    UnknownRoot(String),
    #[error("unknown chord quality suffix {remainder:?} after root {root:?}")]
    UnknownQuality { root: String, remainder: String },
    #[error("octave {0} is out of MIDI range (valid -1..=9)")]
    OctaveOutOfRange(i32),
}

/// Parse `symbol` (e.g. `"Cm7"`), build a close-position chord rooted at
/// `octave` in scientific pitch notation (octave 4 = middle-C octave), then
/// apply `voicing`.
///
/// The resulting MIDI notes are clamped into the valid 0..=127 range. If
/// clamping happens, the warnings list grows; the chord is still returned
/// so the caller can decide whether to retry one octave lower.
pub fn generate_chord(
    symbol: &str,
    octave: i32,
    voicing: ChordVoicing,
) -> Result<GeneratedChord, GenerateChordError> {
    if symbol.trim().is_empty() {
        return Err(GenerateChordError::EmptySymbol);
    }
    if !(-1..=9).contains(&octave) {
        return Err(GenerateChordError::OctaveOutOfRange(octave));
    }

    let (root_pc, remainder) = parse_root(symbol)?;
    let template = match_template(remainder).ok_or_else(|| GenerateChordError::UnknownQuality {
        root: symbol[..symbol.len() - remainder.len()].to_string(),
        remainder: remainder.to_string(),
    })?;

    let mut warnings = Vec::new();

    // Close voicing: stack intervals above the root in the requested octave.
    // octave -1 corresponds to MIDI 0..11, so root MIDI = (octave + 1) * 12 + root_pc.
    let root_midi = (octave + 1) * 12 + i32::from(root_pc);
    let mut notes: Vec<i32> = template
        .intervals
        .iter()
        .map(|&i| root_midi + i32::from(i))
        .collect();

    let actual_voicing = apply_voicing(&mut notes, template, voicing, &mut warnings);
    notes.sort_unstable();

    let clamped: Vec<u8> = notes
        .iter()
        .map(|&n| {
            if n < 0 {
                warnings.push(format!("note {n} clamped up to 0 (below MIDI range)"));
                0
            } else if n > 127 {
                warnings.push(format!("note {n} clamped down to 127 (above MIDI range)"));
                127
            } else {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                {
                    n as u8
                }
            }
        })
        .collect();

    Ok(GeneratedChord {
        notes: clamped,
        root_pitch_class: root_pc,
        quality: template.quality,
        suffix: template.suffix,
        voicing: actual_voicing,
        warnings,
    })
}

fn parse_root(symbol: &str) -> Result<(u8, &str), GenerateChordError> {
    let mut chars = symbol.chars();
    let letter = chars.next().ok_or(GenerateChordError::EmptySymbol)?;
    let base = match letter {
        'C' | 'c' => 0_u8,
        'D' | 'd' => 2,
        'E' | 'e' => 4,
        'F' | 'f' => 5,
        'G' | 'g' => 7,
        'A' | 'a' => 9,
        'B' | 'b' => 11,
        _ => return Err(GenerateChordError::UnknownRoot(symbol.to_string())),
    };
    let after_letter = &symbol[letter.len_utf8()..];
    // 'b' after the root letter is always the flat accidental — we already
    // consumed the root, so "Bb" parses as B + flat, "Bbm7" as Bb + "m7".
    let (offset, remainder) = if let Some(rest) = after_letter.strip_prefix('#') {
        (1_i8, rest)
    } else if let Some(rest) = after_letter.strip_prefix('b') {
        (-1, rest)
    } else {
        (0, after_letter)
    };
    #[allow(clippy::cast_sign_loss)]
    let pc = ((i16::from(base) + i16::from(offset)).rem_euclid(12)) as u8;
    Ok((pc, remainder))
}

fn match_template(remainder: &str) -> Option<&'static ChordTemplate> {
    let key = remainder.trim();
    if key.is_empty() {
        return CHORD_TEMPLATES.iter().find(|t| t.suffix.is_empty());
    }
    let canon = canonicalize_suffix(key).unwrap_or(key);
    if canon.is_empty() {
        return CHORD_TEMPLATES.iter().find(|t| t.suffix.is_empty());
    }
    // Longest matching suffix wins so e.g. "m7b5" beats "m".
    CHORD_TEMPLATES
        .iter()
        .filter(|t| !t.suffix.is_empty() && t.suffix.eq_ignore_ascii_case(canon))
        .max_by_key(|t| t.suffix.len())
}

/// Map user-friendly synonyms onto canonical `CHORD_TEMPLATES` suffixes.
/// Returns `None` when the input already matches a canonical form.
fn canonicalize_suffix(s: &str) -> Option<&'static str> {
    const SYNONYMS: &[(&str, &str)] = &[
        ("min", "m"),
        ("minor", "m"),
        ("maj", ""),
        ("major", ""),
        ("min7", "m7"),
        ("minor7", "m7"),
        ("maj7", "maj7"),
        ("major7", "maj7"),
        ("dim", "dim"),
        ("diminished", "dim"),
        ("aug", "aug"),
        ("augmented", "aug"),
        ("ø", "m7b5"),
        ("halfdim", "m7b5"),
        ("half-dim", "m7b5"),
        ("half_diminished", "m7b5"),
    ];
    SYNONYMS
        .iter()
        .find(|(syn, _)| syn.eq_ignore_ascii_case(s))
        .map(|(_, canon)| *canon)
}

fn apply_voicing(
    notes: &mut [i32],
    template: &ChordTemplate,
    requested: ChordVoicing,
    warnings: &mut Vec<String>,
) -> ChordVoicing {
    let n = notes.len();
    if requested == ChordVoicing::Close || n < 3 {
        return ChordVoicing::Close;
    }
    notes.sort_unstable();
    match requested {
        ChordVoicing::Close => ChordVoicing::Close,
        ChordVoicing::Drop2 => {
            // 2nd-highest. For a triad this is the middle note (the 3rd),
            // producing an open-position triad.
            drop_index(notes, n - 2);
            ChordVoicing::Drop2
        }
        ChordVoicing::Drop3 => {
            if n < 4 {
                warnings.push(format!(
                    "{} has only {n} notes; drop3 needs >= 4, falling back to drop2",
                    template.quality
                ));
                drop_index(notes, n - 2);
                ChordVoicing::Drop2
            } else {
                drop_index(notes, n - 3);
                ChordVoicing::Drop3
            }
        }
        ChordVoicing::Open => {
            // drop2 + drop3 — touch each index once before re-sorting so the
            // two drops don't move the same note twice.
            if n < 4 {
                warnings.push(format!(
                    "{} has only {n} notes; open voicing falls back to drop2",
                    template.quality
                ));
                drop_index(notes, n - 2);
                ChordVoicing::Drop2
            } else {
                notes[n - 2] -= 12;
                notes[n - 3] -= 12;
                ChordVoicing::Open
            }
        }
    }
}

fn drop_index(notes: &mut [i32], idx: usize) {
    notes[idx] -= 12;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::assert_matches;

    #[test]
    fn major_triad_close_at_c4() {
        let chord = generate_chord("C", 4, ChordVoicing::Close).unwrap();
        assert_eq!(chord.notes, vec![60, 64, 67]);
        assert_eq!(chord.quality, "major");
        assert_eq!(chord.suffix, "");
        assert!(chord.warnings.is_empty());
    }

    #[test]
    fn minor_seventh_close_at_c4() {
        let chord = generate_chord("Cm7", 4, ChordVoicing::Close).unwrap();
        assert_eq!(chord.notes, vec![60, 63, 67, 70]);
        assert_eq!(chord.quality, "minor7");
    }

    #[test]
    fn parses_flat_root() {
        let chord = generate_chord("Bbmaj7", 4, ChordVoicing::Close).unwrap();
        // Bb4 = MIDI 70.
        assert_eq!(chord.root_pitch_class, 10);
        assert_eq!(chord.notes, vec![70, 74, 77, 81]);
    }

    #[test]
    fn parses_sharp_root() {
        let chord = generate_chord("F#m7", 4, ChordVoicing::Close).unwrap();
        // F#4 = MIDI 66.
        assert_eq!(chord.root_pitch_class, 6);
        assert_eq!(chord.notes, vec![66, 69, 73, 76]);
    }

    #[test]
    fn voicing_names_round_trip_through_from_str() {
        for voicing in [
            ChordVoicing::Close,
            ChordVoicing::Drop2,
            ChordVoicing::Drop3,
            ChordVoicing::Open,
        ] {
            assert_eq!(voicing.as_str().parse(), Ok(voicing));
        }
        assert!("unknown".parse::<ChordVoicing>().is_err());
    }

    #[test]
    fn synonym_min_resolves_to_minor() {
        let a = generate_chord("Cmin", 4, ChordVoicing::Close).unwrap();
        let b = generate_chord("Cm", 4, ChordVoicing::Close).unwrap();
        assert_eq!(a.notes, b.notes);
        assert_eq!(a.quality, "minor");
    }

    #[test]
    fn drop2_on_minor_seventh_drops_second_highest() {
        let close = generate_chord("Cm7", 4, ChordVoicing::Close).unwrap();
        let drop2 = generate_chord("Cm7", 4, ChordVoicing::Drop2).unwrap();
        // Close: 60, 63, 67, 70. drop2 drops the G (67) an octave → 55.
        assert_eq!(close.notes, vec![60, 63, 67, 70]);
        assert_eq!(drop2.notes, vec![55, 60, 63, 70]);
        assert_eq!(drop2.voicing, ChordVoicing::Drop2);
    }

    #[test]
    fn drop3_on_triad_falls_back_to_drop2_with_warning() {
        let chord = generate_chord("C", 4, ChordVoicing::Drop3).unwrap();
        assert_eq!(chord.voicing, ChordVoicing::Drop2);
        assert!(!chord.warnings.is_empty());
    }

    #[test]
    fn open_voicing_on_tetrad_spreads_two_octaves() {
        let chord = generate_chord("Cmaj7", 4, ChordVoicing::Open).unwrap();
        // Close: 60, 64, 67, 71. Open drops indices 1 and 2 (E and G) by 12:
        // → 52, 55, 60, 71.
        assert_eq!(chord.notes, vec![52, 55, 60, 71]);
    }

    #[test]
    fn empty_symbol_errors() {
        assert_matches!(
            generate_chord("", 4, ChordVoicing::Close),
            Err(GenerateChordError::EmptySymbol)
        );
    }

    #[test]
    fn unknown_quality_errors() {
        assert_matches!(
            generate_chord("Czzz", 4, ChordVoicing::Close),
            Err(GenerateChordError::UnknownQuality { .. })
        );
    }

    #[test]
    fn octave_out_of_range_errors() {
        assert_matches!(
            generate_chord("C", 12, ChordVoicing::Close),
            Err(GenerateChordError::OctaveOutOfRange(12))
        );
    }

    #[test]
    fn high_octave_clamps_and_warns() {
        let chord = generate_chord("Cmaj7", 9, ChordVoicing::Close).unwrap();
        // C9 = MIDI 120, then 124, 127, and B9 = 131 → clamped to 127.
        assert_eq!(chord.notes.last(), Some(&127));
        assert!(chord.warnings.iter().any(|w| w.contains("clamped")));
    }

    #[test]
    fn power_chord_close() {
        let chord = generate_chord("C5", 4, ChordVoicing::Close).unwrap();
        assert_eq!(chord.notes, vec![60, 67]);
        assert_eq!(chord.quality, "power");
    }

    #[test]
    fn sus4_round_trips_through_identifier() {
        use crate::harmony::identify_chord;
        let chord = generate_chord("Csus4", 4, ChordVoicing::Close).unwrap();
        let pcs: Vec<u8> = chord.notes.iter().map(|n| n % 12).collect();
        let identified = identify_chord(&pcs, Some(chord.root_pitch_class)).unwrap();
        assert_eq!(identified.symbol, "Csus4");
    }
}
