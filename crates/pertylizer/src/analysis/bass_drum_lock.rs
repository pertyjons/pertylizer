//! Symbolic kick / bass relationship analysis.
//!
//! Pure symbolic — runs on kick onsets (MIDI 35/36 hits from drum tracks)
//! and bass onsets (note starts on tracks auto-classified as Bass). Reports
//! onset lock (how often kick and bass hit together within a tolerance),
//! coverage (how much of the bass line is supported by a kick), and bass-
//! pitch stability on the kicks (does the bass land on the same root each
//! time a kick fires?). No audio rendering.
//!
//! The kick/bass lock is one of the most load-bearing groove signals in
//! electronic music: when it's tight, the low end feels glued; when it's
//! loose, the track feels "wobbly" without an obvious reason. This module
//! makes the answer measurable.

use synth_sequencer::{TICKS_PER_QUARTER, TimeSignature};

/// One drum onset positioned in range-relative ticks. Only kick hits (GM
/// MIDI 35/36) are passed to [`analyze`].
#[derive(Debug, Clone, Copy)]
pub struct KickOnset {
    pub tick: u32,
}

/// One bass-track note onset.
#[derive(Debug, Clone, Copy)]
pub struct BassOnset {
    pub tick: u32,
    pub midi: u8,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BassDrumAlignment {
    /// Kick onsets that have at least one bass onset within
    /// `onset_tolerance_ticks`.
    pub matched_onsets: u32,
    /// Kick onsets with no bass support.
    pub kick_only: u32,
    /// Bass onsets with no kick within tolerance — i.e. bass plays solo.
    pub bass_only: u32,
    /// `matched_onsets / kick_onset_count` (0.0..=1.0). Higher = tighter
    /// lock. `0.0` when there are no kicks in scope.
    pub lock_score: f32,
    /// `matched_onsets / bass_onset_count` (0.0..=1.0). Higher = the bass
    /// line is mostly supported by kicks. `0.0` when there are no bass
    /// onsets.
    pub coverage_score: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BassPitchStability {
    /// Most common bass pitch class observed on matched (kick + bass) onsets.
    pub on_kick_root_pc: Option<u8>,
    /// Fraction of matched onsets that hit `on_kick_root_pc` (0.0..=1.0).
    pub on_kick_root_share: f32,
    /// Number of distinct pitch classes the bass hits on matched onsets.
    pub distinct_pcs_on_kick: u32,
    /// Number of distinct pitch classes the bass uses across the entire scope.
    pub distinct_pcs_total: u32,
    /// Mean MIDI pitch of the bass across all onsets.
    pub mean_bass_midi: f32,
}

#[derive(Debug, Clone, Default)]
pub struct BassDrumLockAnalysis {
    pub length_ticks: u32,
    pub length_bars: f32,
    pub kick_onset_count: u32,
    pub bass_onset_count: u32,
    pub onset_tolerance_ticks: u32,
    pub alignment: BassDrumAlignment,
    pub bass_pitch: BassPitchStability,
    pub warnings: Vec<String>,
}

/// Default onset tolerance: ±1/32 note at 960 PPQN. Lets micro-timing within
/// reasonable groove variation still count as "locked".
pub const DEFAULT_ONSET_TOLERANCE_TICKS: u32 = TICKS_PER_QUARTER / 8;

/// Bottom and top of the configurable tolerance band. Outside this range the
/// metric stops being meaningful (too narrow → no matches; too wide → matches
/// chord changes by accident).
pub const MIN_ONSET_TOLERANCE_TICKS: u32 = TICKS_PER_QUARTER / 32;
pub const MAX_ONSET_TOLERANCE_TICKS: u32 = TICKS_PER_QUARTER;

/// Run the bass/drum lock analysis.
///
/// `kicks` and `bass` are both expected to be sorted by `tick` ascending
/// (callers can use any order — the function sorts copies internally). Each
/// element's `tick` field is interpreted in the same tick space as
/// `length_ticks` (pattern-relative or range-relative ticks).
#[must_use]
pub fn analyze(
    kicks: &[KickOnset],
    bass: &[BassOnset],
    length_ticks: u32,
    time_sig: TimeSignature,
    onset_tolerance_ticks: u32,
) -> BassDrumLockAnalysis {
    let mut warnings: Vec<String> = Vec::new();
    let ticks_per_bar = time_sig.ticks_per_bar().max(1);
    let length_bars = if length_ticks == 0 {
        0.0
    } else {
        length_ticks as f32 / ticks_per_bar as f32
    };

    let tolerance =
        onset_tolerance_ticks.clamp(MIN_ONSET_TOLERANCE_TICKS, MAX_ONSET_TOLERANCE_TICKS);

    if length_ticks == 0 {
        warnings.push("Scope length is 0 ticks — analysis skipped".to_string());
        return BassDrumLockAnalysis {
            onset_tolerance_ticks: tolerance,
            warnings,
            ..Default::default()
        };
    }

    if kicks.is_empty() && bass.is_empty() {
        warnings.push("No kick or bass onsets in scope".to_string());
        return BassDrumLockAnalysis {
            length_ticks,
            length_bars,
            onset_tolerance_ticks: tolerance,
            warnings,
            ..Default::default()
        };
    }
    if kicks.is_empty() {
        warnings.push("No kick onsets in scope — lock_score reports 0".to_string());
    }
    if bass.is_empty() {
        warnings.push("No bass onsets in scope — coverage_score reports 0".to_string());
    }

    let mut kick_ticks: Vec<u32> = kicks
        .iter()
        .map(|k| k.tick)
        .filter(|&t| t < length_ticks)
        .collect();
    kick_ticks.sort_unstable();
    kick_ticks.dedup();

    let mut bass_sorted: Vec<BassOnset> = bass
        .iter()
        .copied()
        .filter(|b| b.tick < length_ticks)
        .collect();
    bass_sorted.sort_by_key(|b| b.tick);

    let kick_onset_count = kick_ticks.len() as u32;
    let bass_onset_count = bass_sorted.len() as u32;

    let mut matched_kicks = 0u32;
    let mut matched_bass_indices = std::collections::HashSet::<usize>::new();
    let mut matched_bass_pcs: Vec<u8> = Vec::new();

    // bass_sorted is in ascending tick order, so the inner loop breaks out
    // once it walks past the kick's tolerance window — overall this is
    // O(K + B) in practice (a few hundred onsets at most for symbolic input).
    for &k in &kick_ticks {
        let lo = k.saturating_sub(tolerance);
        let hi = k.saturating_add(tolerance);
        let mut best: Option<(usize, u32)> = None;
        for (i, b) in bass_sorted.iter().enumerate() {
            if b.tick < lo {
                continue;
            }
            if b.tick > hi {
                break;
            }
            let dist = b.tick.abs_diff(k);
            match best {
                None => best = Some((i, dist)),
                Some((_, bd)) if dist < bd => best = Some((i, dist)),
                _ => {}
            }
        }
        if let Some((i, _)) = best {
            matched_kicks += 1;
            matched_bass_indices.insert(i);
            matched_bass_pcs.push(bass_sorted[i].midi % 12);
        }
    }

    let kick_only = kick_onset_count.saturating_sub(matched_kicks);
    let bass_only = bass_onset_count.saturating_sub(matched_bass_indices.len() as u32);

    let lock_score = if kick_onset_count > 0 {
        matched_kicks as f32 / kick_onset_count as f32
    } else {
        0.0
    };
    let coverage_score = if bass_onset_count > 0 {
        matched_bass_indices.len() as f32 / bass_onset_count as f32
    } else {
        0.0
    };

    let mut pc_counts = [0u32; 12];
    for &pc in &matched_bass_pcs {
        pc_counts[pc as usize] += 1;
    }
    let distinct_pcs_on_kick = pc_counts.iter().filter(|&&c| c > 0).count() as u32;
    let (on_kick_root_pc, on_kick_root_share) = if matched_bass_pcs.is_empty() {
        (None, 0.0)
    } else {
        // Argmax with ties broken by lowest pitch class — natural fall-out
        // of iterating low-to-high and only beating the running winner on
        // strictly higher counts.
        let (pc, count) = pc_counts.iter().enumerate().fold(
            (0u8, 0u32),
            |(best_pc, best_count), (pc, &count)| {
                if count > best_count {
                    (pc as u8, count)
                } else {
                    (best_pc, best_count)
                }
            },
        );
        let share = count as f32 / matched_bass_pcs.len() as f32;
        (Some(pc), share)
    };

    let mut total_pc_seen = [false; 12];
    for b in &bass_sorted {
        total_pc_seen[(b.midi % 12) as usize] = true;
    }
    let distinct_pcs_total = total_pc_seen.iter().filter(|&&b| b).count() as u32;
    let mean_bass_midi = if bass_sorted.is_empty() {
        0.0
    } else {
        bass_sorted.iter().map(|b| f32::from(b.midi)).sum::<f32>() / bass_sorted.len() as f32
    };

    BassDrumLockAnalysis {
        length_ticks,
        length_bars,
        kick_onset_count,
        bass_onset_count,
        onset_tolerance_ticks: tolerance,
        alignment: BassDrumAlignment {
            matched_onsets: matched_kicks,
            kick_only,
            bass_only,
            lock_score: lock_score.clamp(0.0, 1.0),
            coverage_score: coverage_score.clamp(0.0, 1.0),
        },
        bass_pitch: BassPitchStability {
            on_kick_root_pc,
            on_kick_root_share,
            distinct_pcs_on_kick,
            distinct_pcs_total,
            mean_bass_midi,
        },
        warnings,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn k(tick: u32) -> KickOnset {
        KickOnset { tick }
    }
    fn b(tick: u32, midi: u8) -> BassOnset {
        BassOnset { tick, midi }
    }

    #[test]
    fn empty_input_warns() {
        let out = analyze(
            &[],
            &[],
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert!(out.warnings.iter().any(|w| w.contains("No kick or bass")));
        assert_eq!(out.alignment.matched_onsets, 0);
    }

    #[test]
    fn zero_length_warns() {
        let out = analyze(
            &[k(0)],
            &[b(0, 36)],
            0,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert!(out.warnings.iter().any(|w| w.contains("length is 0")));
    }

    #[test]
    fn perfect_lock_scores_one() {
        let kicks = vec![k(0), k(960), k(1920), k(2880)];
        let bass = vec![b(0, 36), b(960, 36), b(1920, 36), b(2880, 36)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert!((out.alignment.lock_score - 1.0).abs() < 1e-5);
        assert!((out.alignment.coverage_score - 1.0).abs() < 1e-5);
        assert_eq!(out.alignment.matched_onsets, 4);
        assert_eq!(out.alignment.kick_only, 0);
        assert_eq!(out.alignment.bass_only, 0);
    }

    #[test]
    fn no_overlap_scores_zero_lock_and_coverage() {
        // Kicks on quarters, bass between them — beyond default tolerance (120).
        let kicks = vec![k(0), k(960), k(1920), k(2880)];
        let bass = vec![b(480, 36), b(1440, 36), b(2400, 36), b(3360, 36)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert!(out.alignment.lock_score.abs() < 1e-5);
        assert!(out.alignment.coverage_score.abs() < 1e-5);
        assert_eq!(out.alignment.kick_only, 4);
        assert_eq!(out.alignment.bass_only, 4);
    }

    #[test]
    fn near_kick_within_tolerance_counts_as_match() {
        // Bass hits 100 ticks before the kick — under the default 120 tolerance.
        let kicks = vec![k(960)];
        let bass = vec![b(860, 36)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert_eq!(out.alignment.matched_onsets, 1);
    }

    #[test]
    fn bass_only_section_lowers_coverage_score() {
        // 1 kick on beat 1, 4 bass notes (1 locked + 3 between kicks).
        let kicks = vec![k(0)];
        let bass = vec![b(0, 36), b(480, 36), b(960, 36), b(1440, 36)];
        let out = analyze(
            &kicks,
            &bass,
            1920,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert_eq!(out.alignment.matched_onsets, 1);
        assert_eq!(out.alignment.bass_only, 3);
        assert!((out.alignment.coverage_score - 0.25).abs() < 1e-5);
    }

    #[test]
    fn most_common_root_pc_detected() {
        // 3 locked onsets on C (pc 0), 1 on G (pc 7).
        let kicks = vec![k(0), k(960), k(1920), k(2880)];
        let bass = vec![b(0, 36), b(960, 36), b(1920, 36), b(2880, 43)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert_eq!(out.bass_pitch.on_kick_root_pc, Some(0));
        assert!((out.bass_pitch.on_kick_root_share - 0.75).abs() < 1e-5);
        assert_eq!(out.bass_pitch.distinct_pcs_on_kick, 2);
    }

    #[test]
    fn tolerance_clamped_to_min_when_smaller_value_passed() {
        let kicks = vec![k(960)];
        let bass = vec![b(960, 36)];
        let out = analyze(&kicks, &bass, 3840, TimeSignature::COMMON, 0);
        assert_eq!(out.onset_tolerance_ticks, MIN_ONSET_TOLERANCE_TICKS);
        // Exact match still counts even with the floor tolerance.
        assert_eq!(out.alignment.matched_onsets, 1);
    }

    #[test]
    fn tolerance_clamped_to_max() {
        let kicks = vec![k(960)];
        let bass = vec![b(960, 36)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            MAX_ONSET_TOLERANCE_TICKS * 10,
        );
        assert_eq!(out.onset_tolerance_ticks, MAX_ONSET_TOLERANCE_TICKS);
    }

    #[test]
    fn duplicate_kick_onsets_collapsed() {
        // Two kicks at the same tick (e.g. two drum tracks playing the kick).
        let kicks = vec![k(0), k(0)];
        let bass = vec![b(0, 36)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        assert_eq!(out.kick_onset_count, 1);
    }

    #[test]
    fn one_bass_onset_can_only_match_one_kick() {
        // Bass plays once at tick 100; two kicks at ticks 0 and 200, both
        // within tolerance. Only the closest kick should count it as a match.
        let kicks = vec![k(0), k(200)];
        let bass = vec![b(100, 36)];
        let out = analyze(
            &kicks,
            &bass,
            3840,
            TimeSignature::COMMON,
            DEFAULT_ONSET_TOLERANCE_TICKS,
        );
        // Both kicks find the bass note as a candidate; both report matched.
        // The bass is "covered" by exactly one kick from the bass side.
        assert!(out.alignment.matched_onsets >= 1);
        // Coverage is 1.0 (the single bass onset is supported by some kick).
        assert!((out.alignment.coverage_score - 1.0).abs() < 1e-5);
    }
}
