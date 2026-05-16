//! Snap pitches to the nearest scale degree of a given key/scale.
//!
//! The scale table is shared with [`crate::harmony`] so chord identification
//! and scale-snap stay in lock-step (the same `"dorian"` template name).

use crate::harmony::{ScaleTemplate, scale_by_name};

/// Tie-break direction used when two scale degrees are equidistant from the
/// input pitch.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleTieBreak {
    /// Prefer the higher pitch on ties (default — biases towards the
    /// melody side of dyads).
    NearestUp,
    /// Prefer the lower pitch on ties.
    NearestDown,
    /// Same as [`ScaleTieBreak::NearestUp`] but the helper documents intent
    /// when no tie is possible.
    Nearest,
}

impl ScaleTieBreak {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "up" | "nearest_up" | "" => Some(Self::NearestUp),
            "down" | "nearest_down" => Some(Self::NearestDown),
            "nearest" => Some(Self::Nearest),
            _ => None,
        }
    }
}

/// Scale + tonic pair used by [`snap_pitch_to_scale`] and
/// [`crate::composition::transpose`].
pub struct ScaleConstraint {
    pub tonic: u8,
    /// Name of the matched scale template (`"major"`, `"dorian"`, …).
    /// `scale_by_name` falls back to major on unknown input, so this name
    /// may differ from what the caller passed in.
    pub scale_name: &'static str,
    template: &'static ScaleTemplate,
    in_set: [bool; 12],
}

impl ScaleConstraint {
    /// Build a constraint from a tonic pitch class and a scale name. Unknown
    /// names fall back to major (`scale_by_name` already does the fallback).
    #[must_use]
    pub fn new(tonic: u8, scale_name: &str) -> Self {
        let template = scale_by_name(scale_name);
        let in_set = template.pitch_classes(tonic % 12);
        Self {
            tonic: tonic % 12,
            scale_name: template.name,
            template,
            in_set,
        }
    }

    /// Total number of pitch classes the scale uses (5 = pentatonic, 7 =
    /// diatonic, 12 = chromatic).
    #[must_use]
    pub fn degree_count(&self) -> usize {
        self.template.intervals.len()
    }

    pub fn contains(&self, midi: u8) -> bool {
        self.in_set[(midi % 12) as usize]
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
    if scale.contains(pitch) {
        return pitch;
    }
    let prefer_up = matches!(tie_break, ScaleTieBreak::NearestUp | ScaleTieBreak::Nearest);
    let to_match = |v: i32| -> Option<u8> {
        (0..=127)
            .contains(&v)
            .then_some({
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                {
                    v as u8
                }
            })
            .filter(|&p| scale.contains(p))
    };
    for delta in 1..=6_i32 {
        let up = to_match(i32::from(pitch) + delta);
        let down = to_match(i32::from(pitch) - delta);
        match (up, down) {
            (Some(u), Some(d)) => return if prefer_up { u } else { d },
            (Some(u), None) => return u,
            (None, Some(d)) => return d,
            (None, None) => continue,
        }
    }
    pitch
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
}
