//! Snap pitches to the nearest scale degree of a given key/scale.
//!
//! The scale table is shared with [`crate::harmony`] so chord identification
//! and scale-snap stay in lock-step (the same `"dorian"` template name).

use crate::harmony::{ScaleTemplate, scale_by_name};
pub use synth_sequencer::ScaleTieBreak;
use synth_sequencer::{Pitch, PitchClass, ScaleMask, ScaleQuantize};

/// Scale + tonic pair used by [`snap_pitch_to_scale`] and
/// [`crate::composition::transpose`].
pub struct ScaleConstraint {
    pub tonic: u8,
    /// Name of the matched scale template (`"major"`, `"dorian"`, …).
    /// `scale_by_name` falls back to major on unknown input, so this name
    /// may differ from what the caller passed in.
    pub scale_name: &'static str,
    template: &'static ScaleTemplate,
    quantizer: ScaleQuantize,
}

impl ScaleConstraint {
    /// Build a constraint from a tonic pitch class and a scale name. Unknown
    /// names fall back to major (`scale_by_name` already does the fallback).
    #[must_use]
    pub fn new(tonic: u8, scale_name: &str) -> Self {
        let template = scale_by_name(scale_name);
        let tonic = tonic % 12;
        Self {
            tonic,
            scale_name: template.name,
            template,
            quantizer: ScaleQuantize {
                root: PitchClass::new(tonic),
                mask: ScaleMask::from_intervals(template.intervals),
            },
        }
    }

    /// Total number of pitch classes the scale uses (5 = pentatonic, 7 =
    /// diatonic, 12 = chromatic).
    #[must_use]
    pub fn degree_count(&self) -> usize {
        self.template.intervals.len()
    }

    pub fn contains(&self, midi: u8) -> bool {
        Pitch::new(midi).is_some_and(|pitch| self.quantizer.contains(pitch))
    }
}

/// Options passed to [`quantize_pitches_to_scale`].
pub struct ScaleQuantizeOptions<'a> {
    pub scale: &'a ScaleConstraint,
    pub tie_break: ScaleTieBreak,
}

/// Cross-note diagnostics for the scale quantizer.
#[must_use]
#[derive(Debug, Clone, Copy, Default)]
pub struct ScaleQuantizeResult {
    pub notes_in: u32,
    pub notes_already_in_scale: u32,
    pub notes_moved: u32,
    /// Sum of absolute corrections in semitones — divide by `notes_moved`
    /// for the mean shift size, divide by `notes_in` for "drift per note".
    pub total_correction_semitones: u32,
    pub max_correction_semitones: u8,
}

/// Snap each pitch to the nearest in-scale pitch (within one octave of the
/// original). In-place mutation; returns aggregate stats.
pub fn quantize_pitches_to_scale(
    pitches: &mut [u8],
    options: &ScaleQuantizeOptions<'_>,
) -> ScaleQuantizeResult {
    let mut out = ScaleQuantizeResult {
        notes_in: pitches.len() as u32,
        ..ScaleQuantizeResult::default()
    };
    for pitch in pitches.iter_mut() {
        if options.scale.contains(*pitch) {
            out.notes_already_in_scale += 1;
            continue;
        }
        let snapped = snap_pitch_to_scale(*pitch, options.scale, options.tie_break);
        if snapped == *pitch {
            out.notes_already_in_scale += 1;
            continue;
        }
        let delta = (i32::from(snapped) - i32::from(*pitch)).unsigned_abs();
        out.notes_moved += 1;
        out.total_correction_semitones += delta;
        if delta as u8 > out.max_correction_semitones {
            #[allow(clippy::cast_possible_truncation)]
            {
                out.max_correction_semitones = delta as u8;
            }
        }
        *pitch = snapped;
    }
    out
}

/// Move `pitch` to the closest pitch in `scale`. Searches up to ±6 semitones
/// (one tritone in each direction) — a 12-pitch-class scale guarantees at
/// least one member within 6 semitones of any input.
pub fn snap_pitch_to_scale(pitch: u8, scale: &ScaleConstraint, tie_break: ScaleTieBreak) -> u8 {
    Pitch::new(pitch).map_or(pitch, |pitch| {
        scale
            .quantizer
            .snap_with_tie_break(pitch, tie_break)
            .as_midi()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_scale_pitch_is_unchanged() {
        let scale = ScaleConstraint::new(0, "major");
        assert!(scale.contains(60)); // C
        assert_eq!(snap_pitch_to_scale(60, &scale, ScaleTieBreak::Nearest), 60);
    }

    #[test]
    fn tie_break_names_parse_consistently() {
        assert_eq!("up".parse(), Ok(ScaleTieBreak::NearestUp));
        assert_eq!("down".parse(), Ok(ScaleTieBreak::NearestDown));
        assert_eq!("nearest".parse(), Ok(ScaleTieBreak::Nearest));
        assert!("sideways".parse::<ScaleTieBreak>().is_err());
    }

    #[test]
    fn off_scale_pitch_snaps_to_nearest() {
        // C# major scale C, D, E, F, G, A, B. C# (61) is 1 away from C (60)
        // and D (62). NearestUp picks D.
        let scale = ScaleConstraint::new(0, "major");
        assert_eq!(
            snap_pitch_to_scale(61, &scale, ScaleTieBreak::NearestUp),
            62
        );
        assert_eq!(
            snap_pitch_to_scale(61, &scale, ScaleTieBreak::NearestDown),
            60
        );
    }

    #[test]
    fn quantize_collection_counts_moves() {
        // C major scale; input contains C, C#, D, F#, G.
        let scale = ScaleConstraint::new(0, "major");
        let mut pitches = [60_u8, 61, 62, 66, 67];
        let r = quantize_pitches_to_scale(
            &mut pitches,
            &ScaleQuantizeOptions {
                scale: &scale,
                tie_break: ScaleTieBreak::NearestUp,
            },
        );
        // 60 in, 61 → 62, 62 in, 66 → 67 (tie, prefer up), 67 in.
        assert_eq!(pitches, [60, 62, 62, 67, 67]);
        assert_eq!(r.notes_moved, 2);
        assert_eq!(r.notes_already_in_scale, 3);
        assert_eq!(r.total_correction_semitones, 2);
        assert_eq!(r.max_correction_semitones, 1);
    }

    #[test]
    fn pentatonic_minor_drops_more_notes() {
        // Pentatonic minor on A (9): A, C, D, E, G — so 0, 3, 5, 7, 10.
        let scale = ScaleConstraint::new(9, "pentatonic_minor");
        // Input: A, A#, B, C, C#, D, D#, E, F, F#, G, G#.
        let mut pitches: Vec<u8> = (57..69).collect();
        let r = quantize_pitches_to_scale(
            &mut pitches,
            &ScaleQuantizeOptions {
                scale: &scale,
                tie_break: ScaleTieBreak::NearestUp,
            },
        );
        // Pentatonic minor scale: 5 of 12 are in-scale, 7 are out.
        assert_eq!(r.notes_already_in_scale, 5);
        assert_eq!(r.notes_moved, 7);
        for p in &pitches {
            assert!(scale.contains(*p));
        }
    }

    #[test]
    fn composition_wrapper_matches_sequencer_for_every_scale_and_pitch() {
        let tie_breaks = [
            ScaleTieBreak::Nearest,
            ScaleTieBreak::NearestUp,
            ScaleTieBreak::NearestDown,
        ];

        for template in crate::harmony::SCALES {
            for tonic in 0..12 {
                let scale = ScaleConstraint::new(tonic, template.name);
                for tie_break in tie_breaks {
                    for midi in 0..=127 {
                        let pitch = Pitch::new(midi).expect("MIDI range is valid");
                        let expected = scale
                            .quantizer
                            .snap_with_tie_break(pitch, tie_break)
                            .as_midi();
                        assert_eq!(
                            snap_pitch_to_scale(midi, &scale, tie_break),
                            expected,
                            "{} tonic {tonic}, pitch {midi}, tie {tie_break:?}",
                            template.name
                        );
                    }
                }
            }
        }
    }
}
