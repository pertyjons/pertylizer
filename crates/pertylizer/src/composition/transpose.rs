//! Shift pitches by a fixed number of semitones, optionally snapping the
//! result onto a scale.
//!
//! Operates on plain `u8` MIDI pitches so it can be reused over different
//! note containers (the bridge feeds it `Vec<Pitch>` from a pattern; tests
//! and one-shot callers may use raw arrays).

use crate::composition::quantize_scale::{ScaleConstraint, ScaleTieBreak, snap_pitch_to_scale};

/// Per-note bookkeeping returned by [`transpose_pitches`].
///
/// Reports the count of notes that landed off-scale before correction so
/// callers can warn ("you transposed -5 semitones in C major and 4 notes
/// drifted outside the scale; auto-snapped").
#[must_use]
#[derive(Debug, Clone, Copy, Default)]
pub struct TransposeResult {
    pub notes_in: u32,
    pub notes_transposed: u32,
    /// Notes whose transposed pitch would have fallen outside 0..=127 and
    /// were therefore left untouched.
    pub notes_out_of_range: u32,
    /// Notes whose raw transposition landed outside the scale and were
    /// snapped (only counted when a scale constraint is supplied).
    pub notes_snapped_to_scale: u32,
}

/// Transpose `pitches` in place by `semitones`. When `scale` is set, snap
/// any result that lands off-scale to the nearest in-scale pitch using the
/// supplied tie-break direction.
pub fn transpose_pitches(
    pitches: &mut [u8],
    semitones: i32,
    scale: Option<&ScaleConstraint>,
    scale_tie_break: ScaleTieBreak,
) -> TransposeResult {
    let mut out = TransposeResult {
        notes_in: pitches.len() as u32,
        ..TransposeResult::default()
    };
    for pitch in pitches.iter_mut() {
        let target = i32::from(*pitch) + semitones;
        if !(0..=127).contains(&target) {
            out.notes_out_of_range += 1;
            continue;
        }
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let mut new_pitch = target as u8;
        if let Some(scale) = scale
            && !scale.contains(new_pitch)
        {
            new_pitch = snap_pitch_to_scale(new_pitch, scale, scale_tie_break);
            out.notes_snapped_to_scale += 1;
        }
        *pitch = new_pitch;
        out.notes_transposed += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::quantize_scale::ScaleConstraint;

    #[test]
    fn unconstrained_transpose_shifts_every_note() {
        let mut pitches = [60_u8, 62, 64, 67];
        let r = transpose_pitches(&mut pitches, 5, None, ScaleTieBreak::Nearest);
        assert_eq!(pitches, [65, 67, 69, 72]);
        assert_eq!(r.notes_transposed, 4);
        assert_eq!(r.notes_out_of_range, 0);
        assert_eq!(r.notes_snapped_to_scale, 0);
    }

    #[test]
    fn out_of_range_notes_are_skipped() {
        let mut pitches = [120_u8, 60];
        let r = transpose_pitches(&mut pitches, 12, None, ScaleTieBreak::Nearest);
        // 120 + 12 = 132 > 127 → skipped, original kept.
        assert_eq!(pitches, [120, 72]);
        assert_eq!(r.notes_out_of_range, 1);
        assert_eq!(r.notes_transposed, 1);
    }

    #[test]
    fn scale_constraint_snaps_off_key_results() {
        // Shift Bb (10) → C (12) by +2 semitones in C major (no snap needed),
        // and shift E (4) → F# (6) which is OUT of C major → snaps to F (5)
        // or G (7), nearest-up = F at distance 1.
        let mut pitches = [58_u8, 64];
        let scale = ScaleConstraint::new(0, "major");
        let r = transpose_pitches(&mut pitches, 2, Some(&scale), ScaleTieBreak::NearestUp);
        // 58 + 2 = 60 (C) — already in scale.
        // 64 + 2 = 66 (F#) — not in scale, snap to nearest at distance 1.
        //   Candidates: F=65 (down 1) or G=67 (up 1). NearestUp prefers up.
        assert_eq!(pitches[0], 60);
        assert_eq!(pitches[1], 67);
        assert_eq!(r.notes_snapped_to_scale, 1);
        assert_eq!(r.notes_transposed, 2);
    }
}
