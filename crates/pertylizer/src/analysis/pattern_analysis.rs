//! Symbolic pattern analysis.
//!
//! Operates directly on a `synth_sequencer::Pattern`'s notes — no audio
//! rendering, no engine snapshot needed. Reports density, pitch shape,
//! velocity dynamics, rhythmic structure, and bar-level repetition so an AI
//! agent can verify "is this pattern interesting?" without listening.

use std::collections::HashSet;

use synth_sequencer::{Note, Pitch, TICKS_PER_QUARTER, TimeSignature};

use crate::harmony::weighted_pitch_class_histogram;

/// Numeric outcome of `analyze`.
#[derive(Debug, Clone)]
pub struct PatternAnalysis {
    pub length_ticks: u32,
    pub length_bars: f32,
    pub note_count: u32,

    pub density: DensityStats,
    pub pitch: PitchStats,
    pub velocity: VelocityStats,
    pub rhythm: RhythmStats,
    pub repetition: RepetitionStats,

    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DensityStats {
    pub notes_per_bar: f32,
    pub notes_per_beat: f32,
    /// Fraction of the pattern's tick span covered by at least one note (0..1).
    pub active_ratio: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PitchStats {
    pub low: u8,
    pub high: u8,
    pub range_semitones: u8,
    pub mean: f32,
    pub distinct_count: u32,
    /// Duration-weighted, sums to 1.0 when notes exist (else all zero).
    /// Index 0 = C.
    pub class_histogram: [f32; 12],
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VelocityStats {
    pub min: f32,
    pub max: f32,
    pub mean: f32,
    pub std_dev: f32,
    pub range: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RhythmStats {
    pub max_polyphony: u32,
    /// Mean simultaneous-voice count weighted by the time at least one note
    /// was sounding. `0.0` for empty patterns; `1.0` for strictly monophonic.
    pub mean_polyphony: f32,
    pub is_monophonic: bool,
    pub distinct_onset_count: u32,
    pub distinct_duration_count: u32,
    pub mean_ioi_ticks: f32,
    pub ioi_std_ticks: f32,
    /// 0..1. 1.0 = perfectly regular onset grid (zero IOI std); 0 with fewer
    /// than 2 distinct onsets (no IOIs to measure).
    pub regularity_score: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RepetitionStats {
    pub distinct_bars: u32,
    pub total_bars: u32,
    /// 0..1. `1.0` = every bar carries the same notes; `0.0` = every bar
    /// unique. `0.0` for single-bar patterns (no repetition to measure).
    pub bar_repetition_score: f32,
}

/// 32nd-note quantization step for the bar-repetition canonical hash.
const REPETITION_GRID_TICKS: u32 = TICKS_PER_QUARTER / 8;

#[derive(Debug, Clone, Copy)]
struct Kept {
    start: u32,
    end: u32,
    pitch: Pitch,
    velocity: f32,
}

/// Standard deviation over `xs`. Returns 0 for fewer than 2 samples.
fn std_dev(xs: &[f32], mean: f32) -> f32 {
    if xs.len() < 2 {
        return 0.0;
    }
    let var: f32 = xs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / xs.len() as f32;
    var.sqrt()
}

/// Sum of disjoint covered tick spans across `intervals`. Overlapping intervals
/// collapse into a single span — used to derive `DensityStats::active_ratio`.
fn merged_coverage_ticks(mut intervals: Vec<(u32, u32)>) -> u64 {
    if intervals.is_empty() {
        return 0;
    }
    intervals.sort_unstable();
    let mut total: u64 = 0;
    let mut cur_start = intervals[0].0;
    let mut cur_end = intervals[0].1;
    for &(s, e) in &intervals[1..] {
        if s <= cur_end {
            cur_end = cur_end.max(e);
        } else {
            total += u64::from(cur_end - cur_start);
            cur_start = s;
            cur_end = e;
        }
    }
    total += u64::from(cur_end - cur_start);
    total
}

/// Run the full pattern analysis.
///
/// `notes` are the pattern's notes. `length_ticks` is the pattern's authored
/// length; notes that start past the end are dropped and reported in
/// `warnings`. `time_sig` controls `notes_per_bar` / `notes_per_beat`
/// normalization and the bar-repetition window.
#[must_use]
pub fn analyze(notes: &[Note], length_ticks: u32, time_sig: TimeSignature) -> PatternAnalysis {
    let mut warnings: Vec<String> = Vec::new();
    let ticks_per_bar = time_sig.ticks_per_bar().max(1);
    let total_bars = length_ticks.div_ceil(ticks_per_bar).max(1);

    if length_ticks == 0 {
        warnings.push("Pattern length is 0 ticks — analysis skipped".to_string());
        return PatternAnalysis {
            length_ticks: 0,
            length_bars: 0.0,
            note_count: 0,
            density: DensityStats::default(),
            pitch: PitchStats::default(),
            velocity: VelocityStats::default(),
            rhythm: RhythmStats::default(),
            repetition: RepetitionStats::default(),
            warnings,
        };
    }

    let ticks_per_beat = time_sig.ticks_per_beat().max(1);
    let length_bars = length_ticks as f32 / ticks_per_bar as f32;
    let synth_dur = ticks_per_beat;

    let mut kept: Vec<Kept> = Vec::with_capacity(notes.len());
    let mut dropped_out_of_bounds = 0u32;
    for n in notes {
        let start = n.start.0;
        if start >= length_ticks {
            dropped_out_of_bounds += 1;
            continue;
        }
        let dur = n.duration.map(|d| d.0).unwrap_or(synth_dur).max(1);
        let end = start.saturating_add(dur).min(length_ticks);
        if end <= start {
            continue;
        }
        kept.push(Kept {
            start,
            end,
            pitch: n.pitch,
            velocity: n.velocity.as_f32(),
        });
    }
    if dropped_out_of_bounds > 0 {
        warnings.push(format!(
            "{dropped_out_of_bounds} note(s) start past the pattern length and were dropped"
        ));
    }

    let note_count = kept.len() as u32;
    if note_count == 0 {
        warnings.push("Pattern contains no notes".to_string());
        return PatternAnalysis {
            length_ticks,
            length_bars,
            note_count: 0,
            density: DensityStats::default(),
            pitch: PitchStats::default(),
            velocity: VelocityStats::default(),
            rhythm: RhythmStats::default(),
            repetition: RepetitionStats {
                total_bars,
                ..RepetitionStats::default()
            },
            warnings,
        };
    }

    let mut pitch_low = u8::MAX;
    let mut pitch_high = u8::MIN;
    let mut total_weight = 0.0f32;
    let mut weighted_pitch_sum = 0.0f32;
    let mut distinct_pitches = HashSet::new();
    let mut vel_min = f32::INFINITY;
    let mut vel_max = f32::NEG_INFINITY;
    let mut vel_sum = 0.0f32;
    let mut intervals: Vec<(u32, u32)> = Vec::with_capacity(kept.len());
    let mut hist_input: Vec<(Pitch, f32)> = Vec::with_capacity(kept.len());
    for k in &kept {
        let midi = k.pitch.as_midi();
        let weight = (k.end - k.start) as f32;
        pitch_low = pitch_low.min(midi);
        pitch_high = pitch_high.max(midi);
        total_weight += weight;
        weighted_pitch_sum += f32::from(midi) * weight;
        distinct_pitches.insert(midi);
        vel_min = vel_min.min(k.velocity);
        vel_max = vel_max.max(k.velocity);
        vel_sum += k.velocity;
        intervals.push((k.start, k.end));
        hist_input.push((k.pitch, weight));
    }

    let length_in_beats = length_ticks as f32 / ticks_per_beat as f32;
    let covered = merged_coverage_ticks(intervals);
    let density = DensityStats {
        notes_per_bar: note_count as f32 / length_bars.max(f32::EPSILON),
        notes_per_beat: note_count as f32 / length_in_beats.max(f32::EPSILON),
        active_ratio: (covered as f32 / length_ticks as f32).clamp(0.0, 1.0),
    };

    let mut hist = weighted_pitch_class_histogram(&hist_input);
    if total_weight > 0.0 {
        for bin in &mut hist {
            *bin /= total_weight;
        }
    }
    let pitch = PitchStats {
        low: pitch_low,
        high: pitch_high,
        range_semitones: pitch_high - pitch_low,
        mean: weighted_pitch_sum / total_weight.max(f32::EPSILON),
        distinct_count: distinct_pitches.len() as u32,
        class_histogram: hist,
    };

    let vel_mean = vel_sum / note_count as f32;
    let vel_std = if note_count < 2 {
        0.0
    } else {
        let var = kept
            .iter()
            .map(|k| (k.velocity - vel_mean).powi(2))
            .sum::<f32>()
            / note_count as f32;
        var.sqrt()
    };
    let velocity = VelocityStats {
        min: vel_min,
        max: vel_max,
        mean: vel_mean,
        std_dev: vel_std,
        range: vel_max - vel_min,
    };

    // Sweep-line for polyphony. Tiebreak: ends (-1) before starts (+1) at the
    // same tick so a note that ends exactly when another starts doesn't get
    // counted as an overlap.
    let mut events: Vec<(u32, i32)> = Vec::with_capacity(kept.len() * 2);
    for k in &kept {
        events.push((k.start, 1));
        events.push((k.end, -1));
    }
    events.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let mut current: i32 = 0;
    let mut prev_tick: u32 = 0;
    let mut max_poly: i32 = 0;
    let mut poly_time_weighted: u64 = 0;
    let mut active_ticks: u64 = 0;
    for (tick, delta) in &events {
        if current > 0 {
            let span = u64::from(tick.saturating_sub(prev_tick));
            poly_time_weighted += span * current as u64;
            active_ticks += span;
        }
        current += delta;
        max_poly = max_poly.max(current);
        prev_tick = *tick;
    }
    let mean_polyphony = if active_ticks > 0 {
        poly_time_weighted as f32 / active_ticks as f32
    } else {
        0.0
    };

    let mut onsets: Vec<u32> = kept.iter().map(|k| k.start).collect();
    onsets.sort_unstable();
    onsets.dedup();
    let distinct_durations: HashSet<u32> = kept.iter().map(|k| k.end - k.start).collect();
    let (mean_ioi, ioi_std, regularity) = if onsets.len() < 2 {
        (0.0, 0.0, 0.0)
    } else {
        let iois: Vec<f32> = onsets.windows(2).map(|w| (w[1] - w[0]) as f32).collect();
        let mean = iois.iter().sum::<f32>() / iois.len() as f32;
        let std = std_dev(&iois, mean);
        let cv = if mean > 0.0 { std / mean } else { 0.0 };
        let regularity = (1.0 - cv.min(1.0)).clamp(0.0, 1.0);
        (mean, std, regularity)
    };
    let rhythm = RhythmStats {
        max_polyphony: max_poly.max(0) as u32,
        mean_polyphony,
        is_monophonic: max_poly <= 1,
        distinct_onset_count: onsets.len() as u32,
        distinct_duration_count: distinct_durations.len() as u32,
        mean_ioi_ticks: mean_ioi,
        ioi_std_ticks: ioi_std,
        regularity_score: regularity,
    };

    let repetition = if total_bars < 2 {
        warnings.push(
            "Pattern is shorter than 2 bars — bar_repetition_score is not meaningful".to_string(),
        );
        RepetitionStats {
            distinct_bars: 1,
            total_bars,
            bar_repetition_score: 0.0,
        }
    } else {
        let grid = REPETITION_GRID_TICKS.max(1);
        let mut sigs: Vec<Vec<(u32, u8)>> = vec![Vec::new(); total_bars as usize];
        for k in &kept {
            let bar = (k.start / ticks_per_bar) as usize;
            if bar >= sigs.len() {
                continue;
            }
            let quantized = (k.start % ticks_per_bar) / grid * grid;
            sigs[bar].push((quantized, k.pitch.as_midi()));
        }
        for sig in &mut sigs {
            sig.sort_unstable();
        }
        let distinct = sigs.iter().collect::<HashSet<_>>().len() as u32;
        let denom = (total_bars - 1) as f32;
        let score = if denom > 0.0 {
            (1.0 - (distinct.saturating_sub(1)) as f32 / denom).clamp(0.0, 1.0)
        } else {
            0.0
        };
        RepetitionStats {
            distinct_bars: distinct,
            total_bars,
            bar_repetition_score: score,
        }
    };

    PatternAnalysis {
        length_ticks,
        length_bars,
        note_count,
        density,
        pitch,
        velocity,
        rhythm,
        repetition,
        warnings,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use synth_sequencer::{Duration, NoteId, PatternTick, Pitch, SeqInstrumentId, Velocity};

    fn note_at(tick: u32, midi: u8, dur: Option<u32>, vel: f32) -> Note {
        let mut n = Note::new(
            NoteId(tick as u64 * 128 + midi as u64),
            PatternTick(tick),
            Pitch::new(midi).unwrap(),
            Velocity::new(vel),
            SeqInstrumentId(0),
        );
        if let Some(d) = dur {
            n = n.with_duration(Duration(d));
        }
        n
    }

    #[test]
    fn empty_pattern_emits_warning_and_zero_stats() {
        let out = analyze(&[], 3840, TimeSignature::COMMON);
        assert_eq!(out.note_count, 0);
        assert!(out.warnings.iter().any(|w| w.contains("no notes")));
        assert_eq!(out.density.notes_per_beat, 0.0);
        assert_eq!(out.pitch.distinct_count, 0);
        assert_eq!(out.velocity.mean, 0.0);
        assert_eq!(out.rhythm.max_polyphony, 0);
        assert_eq!(out.repetition.total_bars, 1);
    }

    #[test]
    fn zero_length_pattern_returns_skipped_marker() {
        let out = analyze(&[], 0, TimeSignature::COMMON);
        assert!(out.warnings.iter().any(|w| w.contains("length is 0")));
        assert_eq!(out.length_bars, 0.0);
    }

    #[test]
    fn single_note_pattern_reports_distinct_pitch_one_zero_range() {
        // One C4 quarter note in a 1-bar 4/4 pattern.
        let notes = vec![note_at(0, 60, Some(960), 0.8)];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.note_count, 1);
        assert_eq!(out.pitch.low, 60);
        assert_eq!(out.pitch.high, 60);
        assert_eq!(out.pitch.range_semitones, 0);
        assert_eq!(out.pitch.distinct_count, 1);
        assert!((out.pitch.class_histogram[0] - 1.0).abs() < 1e-5);
        // Only one note → no IOI to measure.
        assert_eq!(out.rhythm.distinct_onset_count, 1);
        assert_eq!(out.rhythm.regularity_score, 0.0);
    }

    #[test]
    fn repeated_bar_yields_high_repetition_score() {
        // Same note on every bar at the downbeat, 4 bars long.
        let notes: Vec<Note> = (0..4)
            .map(|bar| note_at(bar * 3840, 60, Some(960), 0.8))
            .collect();
        let out = analyze(&notes, 4 * 3840, TimeSignature::COMMON);
        assert_eq!(out.repetition.total_bars, 4);
        assert_eq!(out.repetition.distinct_bars, 1);
        assert!((out.repetition.bar_repetition_score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn unique_bars_yield_zero_repetition_score() {
        // Different pitch every bar.
        let notes: Vec<Note> = (0..4)
            .map(|bar| note_at(bar * 3840, 60 + bar as u8, Some(960), 0.8))
            .collect();
        let out = analyze(&notes, 4 * 3840, TimeSignature::COMMON);
        assert_eq!(out.repetition.total_bars, 4);
        assert_eq!(out.repetition.distinct_bars, 4);
        assert!(out.repetition.bar_repetition_score.abs() < 1e-5);
    }

    #[test]
    fn regular_grid_scores_high_regularity() {
        // 16 notes on a steady 16th-note grid (240 ticks apart in 4/4).
        let notes: Vec<Note> = (0..16)
            .map(|i| note_at(i * 240, 60, Some(120), 0.8))
            .collect();
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.rhythm.distinct_onset_count, 16);
        // Perfectly regular grid → std = 0 → regularity = 1.
        assert!(out.rhythm.ioi_std_ticks.abs() < 1e-5);
        assert!((out.rhythm.regularity_score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn velocity_stats_reflect_dynamic_range() {
        let notes = vec![
            note_at(0, 60, Some(120), 0.2),
            note_at(240, 60, Some(120), 0.4),
            note_at(480, 60, Some(120), 0.6),
            note_at(720, 60, Some(120), 0.8),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert!((out.velocity.min - 0.2).abs() < 1e-5);
        assert!((out.velocity.max - 0.8).abs() < 1e-5);
        assert!((out.velocity.mean - 0.5).abs() < 1e-5);
        assert!((out.velocity.range - 0.6).abs() < 1e-5);
        // Non-zero std for varied velocities.
        assert!(out.velocity.std_dev > 0.0);
    }

    #[test]
    fn polyphonic_chord_reports_max_polyphony_three() {
        // C major triad held for a beat.
        let notes = vec![
            note_at(0, 60, Some(960), 0.8),
            note_at(0, 64, Some(960), 0.8),
            note_at(0, 67, Some(960), 0.8),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.rhythm.max_polyphony, 3);
        assert!(!out.rhythm.is_monophonic);
        // All three notes share one onset.
        assert_eq!(out.rhythm.distinct_onset_count, 1);
    }

    #[test]
    fn monophonic_run_flagged_as_monophonic() {
        // Four sequential quarter notes, never overlapping.
        let notes = vec![
            note_at(0, 60, Some(960), 0.8),
            note_at(960, 62, Some(960), 0.8),
            note_at(1920, 64, Some(960), 0.8),
            note_at(2880, 65, Some(960), 0.8),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.rhythm.max_polyphony, 1);
        assert!(out.rhythm.is_monophonic);
        assert_eq!(out.pitch.distinct_count, 4);
    }

    #[test]
    fn density_handles_dense_sixteenth_pattern() {
        // 16 sixteenths in one 4/4 bar → 4 notes per beat, 16 per bar.
        let notes: Vec<Note> = (0..16)
            .map(|i| note_at(i * 240, 60, Some(120), 0.8))
            .collect();
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert!((out.density.notes_per_bar - 16.0).abs() < 1e-3);
        assert!((out.density.notes_per_beat - 4.0).abs() < 1e-3);
        // 16 × 120 / 3840 = 0.5
        assert!((out.density.active_ratio - 0.5).abs() < 1e-5);
    }

    #[test]
    fn notes_past_pattern_length_are_dropped_with_warning() {
        let notes = vec![
            note_at(0, 60, Some(960), 0.8),
            note_at(4000, 62, Some(120), 0.8), // past 1-bar length
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.note_count, 1);
        assert!(out.warnings.iter().any(|w| w.contains("past the pattern")));
    }

    #[test]
    fn open_ended_note_uses_synthetic_duration() {
        // No duration → falls back to one beat.
        let notes = vec![note_at(0, 60, None, 0.8)];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        // Synthetic duration = ticks_per_beat = 960 in 4/4.
        // active_ratio = 960 / 3840 = 0.25
        assert!((out.density.active_ratio - 0.25).abs() < 1e-5);
    }

    #[test]
    fn ioi_irregularity_lowers_regularity_score() {
        // Onsets at 0, 100, 1000 — wildly irregular.
        let notes = vec![
            note_at(0, 60, Some(50), 0.8),
            note_at(100, 60, Some(50), 0.8),
            note_at(1000, 60, Some(50), 0.8),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert!(out.rhythm.regularity_score < 0.6);
    }

    #[test]
    fn pitch_class_histogram_normalizes_to_one() {
        // C major triad, equal durations.
        let notes = vec![
            note_at(0, 60, Some(960), 0.8),
            note_at(960, 64, Some(960), 0.8),
            note_at(1920, 67, Some(960), 0.8),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        let sum: f32 = out.pitch.class_histogram.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!((out.pitch.class_histogram[0] - 1.0 / 3.0).abs() < 1e-5);
        assert!((out.pitch.class_histogram[4] - 1.0 / 3.0).abs() < 1e-5);
        assert!((out.pitch.class_histogram[7] - 1.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn merged_coverage_handles_overlap() {
        // Union: 0..200 (200) + 300..400 (100) = 300.
        let intervals = vec![(0, 100), (50, 200), (300, 400)];
        assert_eq!(merged_coverage_ticks(intervals), 300);
    }
}
