//! Pitch-interval motif detection.
//!
//! Powers `find_motifs` and `analyze_hook_strength` (Tier-2 #15, Group C).
//! Operates on the same scope-relative [`MelodicNote`] stream that
//! `bar_features` uses. The motif representation is transposition-invariant
//! by construction: each motif is a sequence of signed semitone deltas
//! between consecutive notes within the same track, so the same melodic
//! shape rooted at C or at F# collapses to one entry.

use std::collections::HashMap;

use crate::analysis::bar_features::MelodicNote;

/// One occurrence of a motif (track-relative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotifHit {
    pub track_id: u16,
    /// Scope-relative tick of the first note in the matched n-gram.
    pub start_tick: u32,
    /// MIDI pitch of the first note.
    pub first_pitch: u8,
}

/// One motif candidate.
#[derive(Debug, Clone)]
pub struct Motif {
    /// Length in intervals (number of semitone deltas). `notes = length + 1`.
    pub length: u8,
    /// Signed semitone deltas, ordered.
    pub intervals: Vec<i16>,
    /// Sorted by (track_id, start_tick).
    pub occurrences: Vec<MotifHit>,
}

impl Motif {
    #[must_use]
    pub fn count(&self) -> u32 {
        self.occurrences.len() as u32
    }

    /// Sort key for ranking motif candidates: longer motifs and higher
    /// repeat counts both raise the score; log-scaling on count avoids a
    /// 50× repeat trivially burying a 4-note phrase under a 3-note one.
    #[must_use]
    pub fn score(&self) -> f32 {
        let count = self.count() as f32;
        f32::from(self.length) * (1.0 + count).log2()
    }
}

/// Per-track pitch sequence in time order. Notes that start at the same
/// tick are ordered by ascending pitch so simultaneous chord tones produce
/// a stable interval stream.
fn track_pitch_sequences(notes: &[MelodicNote]) -> HashMap<u16, Vec<(u32, u8)>> {
    let mut by_track: HashMap<u16, Vec<(u32, u8)>> = HashMap::new();
    for n in notes {
        by_track
            .entry(n.track_id)
            .or_default()
            .push((n.tick, n.pitch.as_midi()));
    }
    for v in by_track.values_mut() {
        v.sort_unstable();
    }
    by_track
}

/// Find recurring pitch-interval n-grams across the scope.
///
/// `min_len` and `max_len` are motif lengths in intervals (i.e. one less
/// than the number of notes). `min_count` is the minimum occurrence count
/// for a motif to be reported. Returns the top `top_n` motifs sorted by
/// descending [`Motif::score`].
#[must_use]
pub fn find_motifs(
    notes: &[MelodicNote],
    min_len: u8,
    max_len: u8,
    min_count: u32,
    top_n: u32,
) -> Vec<Motif> {
    if min_len < 2 || max_len < min_len {
        return Vec::new();
    }
    let by_track = track_pitch_sequences(notes);

    // Key = interval sequence; value = list of (track_id, start_tick,
    // first_pitch).
    let mut buckets: HashMap<Vec<i16>, Vec<MotifHit>> = HashMap::new();

    for (track_id, seq) in &by_track {
        if seq.len() < (min_len as usize) + 1 {
            continue;
        }
        // Pre-compute the full interval stream for this track.
        let intervals: Vec<i16> = seq
            .windows(2)
            .map(|w| i16::from(w[1].1) - i16::from(w[0].1))
            .collect();
        for len in min_len..=max_len {
            let len_usize = len as usize;
            if intervals.len() < len_usize {
                continue;
            }
            for (i, window) in intervals.windows(len_usize).enumerate() {
                let key: Vec<i16> = window.to_vec();
                buckets.entry(key).or_default().push(MotifHit {
                    track_id: *track_id,
                    start_tick: seq[i].0,
                    first_pitch: seq[i].1,
                });
            }
        }
    }

    let mut motifs: Vec<Motif> = buckets
        .into_iter()
        .filter_map(|(intervals, mut occurrences)| {
            if (occurrences.len() as u32) < min_count {
                return None;
            }
            occurrences.sort_unstable_by_key(|h| (h.track_id, h.start_tick));
            let length = intervals.len() as u8;
            Some(Motif {
                length,
                intervals,
                occurrences,
            })
        })
        .collect();

    // Filter out motifs that are strict prefixes of longer motifs with the
    // same count — the longer motif is always more informative.
    motifs.sort_by_key(|m| std::cmp::Reverse(m.length));
    let mut keep: Vec<Motif> = Vec::with_capacity(motifs.len());
    for m in motifs {
        let mut covered = false;
        for k in &keep {
            if k.length > m.length
                && k.count() == m.count()
                && k.intervals.starts_with(&m.intervals)
            {
                covered = true;
                break;
            }
        }
        if !covered {
            keep.push(m);
        }
    }

    keep.sort_by(|a, b| {
        b.score()
            .partial_cmp(&a.score())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.length.cmp(&a.length))
            .then(b.count().cmp(&a.count()))
    });
    keep.truncate(top_n as usize);
    keep
}

/// Output of [`hook_strength`].
#[derive(Debug, Clone)]
pub struct HookAnalysis {
    /// [0, 1] hook score (longest motif × repeat count × coverage). 0 when
    /// no motif meets the threshold.
    pub score: f32,
    /// Fraction of melodic notes that participate in at least one motif of
    /// `length >= min_len && count >= min_count`. Always reported, even
    /// when `strongest` is `None`.
    pub coverage: f32,
    /// Longest motif (ties broken by count) above the threshold. `None`
    /// when nothing qualifies.
    pub strongest: Option<Motif>,
}

/// Compute coverage + hook score in one pass over `motifs`. Returns
/// `coverage` and `strongest` separately so callers can use either.
#[must_use]
pub fn hook_strength(
    notes: &[MelodicNote],
    motifs: &[Motif],
    min_len: u8,
    min_count: u32,
) -> HookAnalysis {
    if notes.is_empty() {
        return HookAnalysis {
            score: 0.0,
            coverage: 0.0,
            strongest: None,
        };
    }
    let by_track = track_pitch_sequences(notes);
    let mut covered: std::collections::HashSet<(u16, u32)> = std::collections::HashSet::new();
    let mut best: Option<&Motif> = None;
    for m in motifs {
        if m.length < min_len || m.count() < min_count {
            continue;
        }
        best = match best {
            None => Some(m),
            Some(b)
                if (m.length, m.count()).cmp(&(b.length, b.count()))
                    == std::cmp::Ordering::Greater =>
            {
                Some(m)
            }
            other => other,
        };
        for occ in &m.occurrences {
            let Some(seq) = by_track.get(&occ.track_id) else {
                continue;
            };
            let Some(idx) = seq.iter().position(|(t, _)| *t == occ.start_tick) else {
                continue;
            };
            for k in 0..=(m.length as usize) {
                if let Some((tick, _)) = seq.get(idx + k) {
                    covered.insert((occ.track_id, *tick));
                }
            }
        }
    }
    let coverage = covered.len() as f32 / notes.len() as f32;
    let (score, strongest) = match best {
        None => (0.0, None),
        Some(b) => {
            // Cap motif length at 8 intervals (9 notes) for the score —
            // longer is bonus territory but doesn't pay more.
            let length_term = (f32::from(b.length) / 8.0).min(1.0) * 0.5;
            let count_term = ((1.0 + b.count() as f32).log2()
                / (1.0 + notes.len() as f32).log2().max(0.1))
            .min(1.0)
                * 0.3;
            let coverage_term = coverage * 0.2;
            let s = (length_term + count_term + coverage_term).clamp(0.0, 1.0);
            (s, Some(b.clone()))
        }
    };
    HookAnalysis {
        score,
        coverage,
        strongest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_sequencer::Pitch;

    fn note(track: u16, tick: u32, pitch: u8) -> MelodicNote {
        MelodicNote {
            track_id: track,
            tick,
            duration_ticks: 240,
            pitch: Pitch::new(pitch).unwrap(),
            velocity: 0.7,
        }
    }

    #[test]
    fn no_motifs_when_below_min_count() {
        // C D E F G — only 4 intervals (+2 +2 +1 +2), all distinct, no repeat.
        let notes = [
            note(0, 0, 60),
            note(0, 240, 62),
            note(0, 480, 64),
            note(0, 720, 65),
            note(0, 960, 67),
        ];
        let m = find_motifs(&notes, 3, 6, 3, 10);
        assert!(m.is_empty());
    }

    #[test]
    fn finds_simple_repeating_motif() {
        // Three reps of C D E F (intervals +2 +2 +1) on one track.
        let mut notes = Vec::new();
        for rep in 0..3u32 {
            let base = 60 + rep * 5; // each rep starts higher → transposition
            let t0 = rep * 2000;
            notes.push(note(0, t0, base as u8));
            notes.push(note(0, t0 + 240, (base + 2) as u8));
            notes.push(note(0, t0 + 480, (base + 4) as u8));
            notes.push(note(0, t0 + 720, (base + 5) as u8));
        }
        let motifs = find_motifs(&notes, 3, 6, 3, 10);
        assert!(!motifs.is_empty(), "expected at least one motif");
        // The 3-interval motif [+2, +2, +1] should be present with count 3.
        let target: Vec<i16> = vec![2, 2, 1];
        let hit = motifs.iter().find(|m| m.intervals == target);
        assert!(hit.is_some(), "did not find expected motif: {motifs:?}");
        assert_eq!(hit.unwrap().count(), 3);
    }

    #[test]
    fn transposition_invariant_collapses_repeats() {
        // C-D-E-F at root 60, then same shape at root 67. Two reps of [+2,+2,+1].
        let notes = [
            note(0, 0, 60),
            note(0, 240, 62),
            note(0, 480, 64),
            note(0, 720, 65),
            note(0, 1000, 67),
            note(0, 1240, 69),
            note(0, 1480, 71),
            note(0, 1720, 72),
        ];
        let motifs = find_motifs(&notes, 3, 3, 2, 10);
        let target: Vec<i16> = vec![2, 2, 1];
        let hit = motifs.iter().find(|m| m.intervals == target);
        assert!(
            hit.is_some(),
            "transposed reps did not collapse: {motifs:?}"
        );
        assert_eq!(hit.unwrap().count(), 2);
    }

    #[test]
    fn coverage_ratio_counts_motif_notes() {
        // 8 notes total: first 4 + last 4 form two reps of the same motif.
        // Motif [+2,+2,+1] of length 3 covers 4 notes per occurrence → 8/8.
        let notes = [
            note(0, 0, 60),
            note(0, 240, 62),
            note(0, 480, 64),
            note(0, 720, 65),
            note(0, 1000, 70),
            note(0, 1240, 72),
            note(0, 1480, 74),
            note(0, 1720, 75),
        ];
        let motifs = find_motifs(&notes, 3, 3, 2, 10);
        let analysis = hook_strength(&notes, &motifs, 3, 2);
        assert!(
            (analysis.coverage - 1.0).abs() < 1e-4,
            "expected full coverage, got {}",
            analysis.coverage
        );
    }

    #[test]
    fn hook_strength_zero_without_qualifying_motif() {
        // Random non-repeating melody.
        let notes = [
            note(0, 0, 60),
            note(0, 240, 64),
            note(0, 480, 67),
            note(0, 720, 72),
        ];
        let motifs = find_motifs(&notes, 3, 6, 3, 10);
        let analysis = hook_strength(&notes, &motifs, 3, 3);
        assert_eq!(analysis.score, 0.0);
        assert!(analysis.strongest.is_none());
    }

    #[test]
    fn hook_strength_positive_with_three_reps() {
        // Three reps of the same shape — should produce a non-zero hook.
        let mut notes = Vec::new();
        for rep in 0..3u32 {
            let t0 = rep * 2000;
            notes.push(note(0, t0, 60));
            notes.push(note(0, t0 + 240, 62));
            notes.push(note(0, t0 + 480, 64));
            notes.push(note(0, t0 + 720, 65));
        }
        let motifs = find_motifs(&notes, 3, 6, 3, 10);
        let analysis = hook_strength(&notes, &motifs, 3, 3);
        assert!(
            analysis.score > 0.0,
            "expected positive hook score, got {}",
            analysis.score
        );
        let best = analysis.strongest.expect("expected a strongest motif");
        assert!(best.length >= 3);
        assert!(best.count() >= 3);
    }

    #[test]
    fn long_motif_does_not_hide_unique_short_motif() {
        // A long motif [+2,+2,+1,+2,+2,+1] with 3 reps shouldn't suppress
        // a different short motif [+5,+5] that appears 3 times in
        // unrelated locations.
        let mut notes = Vec::new();
        for rep in 0..3u32 {
            let t0 = rep * 4000;
            notes.extend([
                note(0, t0, 60),
                note(0, t0 + 240, 62),
                note(0, t0 + 480, 64),
                note(0, t0 + 720, 65),
                note(0, t0 + 960, 67),
                note(0, t0 + 1200, 69),
                note(0, t0 + 1440, 70),
            ]);
        }
        // Add separate +5,+5 reps on track 1.
        for rep in 0..3u32 {
            let t0 = rep * 1000 + 12_000;
            notes.extend([
                note(1, t0, 50),
                note(1, t0 + 240, 55),
                note(1, t0 + 480, 60),
            ]);
        }
        let motifs = find_motifs(&notes, 2, 6, 3, 10);
        let short = motifs.iter().find(|m| m.intervals == vec![5, 5]);
        assert!(short.is_some(), "short distinct motif was lost: {motifs:?}");
    }
}
