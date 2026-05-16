//! Shared bar-level feature extraction + section clustering.
//!
//! Used by both `analyze_arrangement` / `analyze_form_map` (Tier-2 #20) and
//! is consumed indirectly by `find_motifs` / `analyze_hook_strength` (Tier-2
//! #15) so all four Group C tools agree on what a "bar" looks like.
//!
//! The pipeline is:
//!   1. Caller hands us a flat note stream in scope-relative ticks
//!      ([`MelodicNote`]).
//!   2. [`build_bar_features`] bins notes into bars under the scope time
//!      signature and produces one [`BarFeature`] per bar.
//!   3. [`cluster_sections`] walks the per-bar features, merges adjacent
//!      similar bars (cosine ≥ `similarity_threshold`), enforces a minimum
//!      section length, and assigns labels (`"A"`, `"B"`, `"A'"`, …).
//!
//! No audio rendering, no engine snapshot needed.

use std::collections::HashSet;

use synth_sequencer::{Pitch, TimeSignature};

use crate::harmony::weighted_pitch_class_histogram;

/// One melodic note inside the analyzed scope. Ticks are scope-relative —
/// `tick = 0` is the first bar of the analyzed window regardless of whether
/// the caller is in pattern or arrangement scope.
#[derive(Debug, Clone, Copy)]
pub struct MelodicNote {
    /// Track that owns the note. `0` for pattern scope.
    pub track_id: u16,
    /// Scope-relative start tick.
    pub tick: u32,
    /// Note duration in ticks (`>= 1`).
    pub duration_ticks: u32,
    /// MIDI pitch.
    pub pitch: Pitch,
    /// Velocity (`0..=1`).
    pub velocity: f32,
}

/// Per-bar feature vector. The pitch-class histogram is duration-weighted
/// and normalized to sum to 1 when the bar has any notes (else all zero).
/// `note_count` is preserved unnormalized so density-aware similarity can
/// still pick out empty bars cleanly.
#[derive(Debug, Clone)]
pub struct BarFeature {
    /// 1-indexed bar number in scope.
    pub bar: u32,
    /// Number of melodic notes that started inside the bar.
    pub note_count: u32,
    /// Distinct pitch classes touched (0..=12).
    pub distinct_pitch_classes: u8,
    /// Dominant pitch class (0..12) — argmax of the histogram. `None` for
    /// empty bars.
    pub dominant_pitch_class: Option<u8>,
    /// Mean velocity across notes that started in the bar.
    pub mean_velocity: f32,
    /// Duration-weighted pitch-class histogram, normalized to sum = 1 when
    /// non-empty (else all zeros).
    pub class_histogram: [f32; 12],
    /// Track IDs that contributed at least one note to the bar. Sorted.
    pub active_track_ids: Vec<u16>,
}

impl BarFeature {
    /// True when no melodic notes started in this bar.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.note_count == 0
    }
}

/// Bucket `notes` into bars under `time_sig` and produce one [`BarFeature`]
/// per bar. `total_bars` is the bar count the caller wants — notes that
/// fall past the last bar are silently clipped (the caller is expected to
/// have already truncated to scope).
#[must_use]
pub fn build_bar_features(
    notes: &[MelodicNote],
    time_sig: TimeSignature,
    total_bars: u32,
) -> Vec<BarFeature> {
    let ticks_per_bar = time_sig.ticks_per_bar().max(1);
    let mut hist_pairs_per_bar: Vec<Vec<(Pitch, f32)>> =
        (0..total_bars).map(|_| Vec::new()).collect();
    let mut counts: Vec<u32> = vec![0; total_bars as usize];
    let mut velocity_sums: Vec<f32> = vec![0.0; total_bars as usize];
    let mut active_tracks: Vec<HashSet<u16>> = (0..total_bars).map(|_| HashSet::new()).collect();

    for note in notes {
        let bar_idx = (note.tick / ticks_per_bar) as usize;
        if bar_idx >= total_bars as usize {
            continue;
        }
        counts[bar_idx] += 1;
        velocity_sums[bar_idx] += note.velocity;
        active_tracks[bar_idx].insert(note.track_id);
        hist_pairs_per_bar[bar_idx].push((note.pitch, note.duration_ticks.max(1) as f32));
    }

    let mut out = Vec::with_capacity(total_bars as usize);
    for (i, pairs) in hist_pairs_per_bar.iter().enumerate() {
        let hist = weighted_pitch_class_histogram(pairs);
        let total: f32 = hist.iter().sum();
        let mut normalized = [0.0f32; 12];
        let mut dominant: Option<u8> = None;
        let mut distinct: u8 = 0;
        if total > 0.0 {
            for (j, h) in hist.iter().enumerate() {
                normalized[j] = h / total;
                if *h > 0.0 {
                    distinct += 1;
                }
            }
            if let Some((idx, _)) = normalized
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            {
                dominant = Some(idx as u8);
            }
        }
        let mean_vel = if counts[i] > 0 {
            velocity_sums[i] / counts[i] as f32
        } else {
            0.0
        };
        let mut tracks: Vec<u16> = active_tracks[i].iter().copied().collect();
        tracks.sort_unstable();
        out.push(BarFeature {
            bar: (i as u32) + 1,
            note_count: counts[i],
            distinct_pitch_classes: distinct,
            dominant_pitch_class: dominant,
            mean_velocity: mean_vel,
            class_histogram: normalized,
            active_track_ids: tracks,
        });
    }
    out
}

/// Cosine similarity between two bar features. The pitch-class histogram
/// dominates because that's what carries section identity; density and
/// track activity are folded in as small extra dimensions so two bars with
/// identical histograms but wildly different densities don't collapse.
///
/// Returns 0 when either bar is empty. Empty-vs-empty returns 1 so
/// clustering can group all-rest bars together.
#[must_use]
pub fn bar_similarity(a: &BarFeature, b: &BarFeature) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    // Build extended feature vectors:
    //   12 pitch-class buckets (already normalized) +
    //   1 density bucket (notes / 32, capped at 1.0) +
    //   1 mean-velocity bucket
    // 14 dims, dot/norm computed directly. The density / velocity bins are
    // scaled down so they nudge similarity rather than dominate it.
    const DENSITY_SCALE: f32 = 0.2;
    const VELOCITY_SCALE: f32 = 0.1;

    let density_a = (a.note_count as f32 / 32.0).min(1.0) * DENSITY_SCALE;
    let density_b = (b.note_count as f32 / 32.0).min(1.0) * DENSITY_SCALE;
    let vel_a = a.mean_velocity * VELOCITY_SCALE;
    let vel_b = b.mean_velocity * VELOCITY_SCALE;

    let mut dot = density_a * density_b + vel_a * vel_b;
    let mut na = density_a * density_a + vel_a * vel_a;
    let mut nb = density_b * density_b + vel_b * vel_b;
    for i in 0..12 {
        dot += a.class_histogram[i] * b.class_histogram[i];
        na += a.class_histogram[i] * a.class_histogram[i];
        nb += b.class_histogram[i] * b.class_histogram[i];
    }
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(0.0, 1.0)
}

/// One contiguous run of bars assigned the same label by clustering.
#[derive(Debug, Clone)]
pub struct ClusteredSection {
    /// Section letter (`"A"`, `"B"`, `"A'"`, …).
    pub label: String,
    /// 1-indexed inclusive start bar.
    pub start_bar: u32,
    /// 1-indexed inclusive end bar.
    pub end_bar: u32,
}

impl ClusteredSection {
    #[must_use]
    pub fn length_bars(&self) -> u32 {
        self.end_bar - self.start_bar + 1
    }
}

/// Centroid of a section — used to label later sections that match an
/// earlier one. Average of the bar-level histograms, average density and
/// velocity.
fn section_centroid(bars: &[BarFeature]) -> BarFeature {
    let n = bars.len().max(1) as f32;
    let mut hist = [0.0f32; 12];
    let mut note_total: u32 = 0;
    let mut vel_total: f32 = 0.0;
    let mut tracks: HashSet<u16> = HashSet::new();
    for b in bars {
        for i in 0..12 {
            hist[i] += b.class_histogram[i];
        }
        note_total += b.note_count;
        vel_total += b.mean_velocity;
        for t in &b.active_track_ids {
            tracks.insert(*t);
        }
    }
    for h in &mut hist {
        *h /= n;
    }
    let mean_vel = vel_total / n;
    let mut active: Vec<u16> = tracks.into_iter().collect();
    active.sort_unstable();
    BarFeature {
        bar: 0,
        note_count: ((note_total as f32) / n).round() as u32,
        distinct_pitch_classes: hist.iter().filter(|h| **h > 0.0).count() as u8,
        dominant_pitch_class: hist
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .filter(|(_, h)| **h > 0.0)
            .map(|(i, _)| i as u8),
        mean_velocity: mean_vel,
        class_histogram: hist,
        active_track_ids: active,
    }
}

/// Merge adjacent similar bars into sections and label them by first
/// appearance. Sections shorter than `section_min_bars` are merged into the
/// longer of their neighbours (or kept if they're at a boundary).
///
/// `bars` is the full per-bar feature stream (1-indexed bars in
/// `bars[i].bar`). `similarity_threshold` is the cosine cutoff used both for
/// adjacent-merge and for matching a section against the catalogue of
/// previously labeled section centroids.
#[must_use]
pub fn cluster_sections(
    bars: &[BarFeature],
    similarity_threshold: f32,
    section_min_bars: u32,
) -> Vec<ClusteredSection> {
    if bars.is_empty() {
        return Vec::new();
    }

    // Step 1: split into runs of adjacent similar bars.
    let mut raw_runs: Vec<(u32, u32)> = Vec::new();
    let mut run_start = bars[0].bar;
    for w in bars.windows(2) {
        let sim = bar_similarity(&w[0], &w[1]);
        if sim < similarity_threshold {
            raw_runs.push((run_start, w[0].bar));
            run_start = w[1].bar;
        }
    }
    raw_runs.push((run_start, bars[bars.len() - 1].bar));

    // Step 2: enforce minimum section length. A short run gets absorbed
    // into whichever neighbour is longer (or the only neighbour at a
    // boundary). Single-section input is left alone.
    let min = section_min_bars.max(1);
    loop {
        if raw_runs.len() < 2 {
            break;
        }
        let mut merged = false;
        for i in 0..raw_runs.len() {
            let len = raw_runs[i].1 - raw_runs[i].0 + 1;
            if len >= min {
                continue;
            }
            // Pick a neighbour to merge into.
            let prev_len = if i > 0 {
                Some(raw_runs[i - 1].1 - raw_runs[i - 1].0 + 1)
            } else {
                None
            };
            let next_len = if i + 1 < raw_runs.len() {
                Some(raw_runs[i + 1].1 - raw_runs[i + 1].0 + 1)
            } else {
                None
            };
            let absorb_prev = match (prev_len, next_len) {
                (Some(p), Some(n)) => p >= n,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if absorb_prev {
                raw_runs[i - 1].1 = raw_runs[i].1;
                raw_runs.remove(i);
            } else {
                raw_runs[i + 1].0 = raw_runs[i].0;
                raw_runs.remove(i);
            }
            merged = true;
            break;
        }
        if !merged {
            break;
        }
    }

    // Step 3: label each run. Compare against the running catalogue of
    // section centroids. Exact match (sim >= threshold + 0.05) → reuse
    // label; soft match (>= threshold) → prime the label; else a new
    // letter.
    let mut catalogue: Vec<(String, BarFeature)> = Vec::new();
    let mut sections: Vec<ClusteredSection> = Vec::with_capacity(raw_runs.len());

    for (start, end) in raw_runs {
        let run_bars: Vec<BarFeature> = bars
            .iter()
            .filter(|b| b.bar >= start && b.bar <= end)
            .cloned()
            .collect();
        if run_bars.is_empty() {
            continue;
        }
        let centroid = section_centroid(&run_bars);
        let mut best: Option<(usize, f32)> = None;
        for (idx, (_, c)) in catalogue.iter().enumerate() {
            let sim = bar_similarity(c, &centroid);
            if best.is_none_or(|(_, b)| sim > b) {
                best = Some((idx, sim));
            }
        }
        let label = match best {
            Some((idx, sim)) if sim >= similarity_threshold + 0.05 => catalogue[idx].0.clone(),
            Some((idx, sim)) if sim >= similarity_threshold => {
                let base = strip_prime(&catalogue[idx].0).to_string();
                let existing: HashSet<&str> = catalogue.iter().map(|c| c.0.as_str()).collect();
                let mut candidate = format!("{base}'");
                while existing.contains(candidate.as_str()) {
                    candidate.push('\'');
                }
                catalogue.push((candidate.clone(), centroid.clone()));
                candidate
            }
            _ => {
                let label = next_label(catalogue.len());
                catalogue.push((label.clone(), centroid.clone()));
                label
            }
        };
        sections.push(ClusteredSection {
            label,
            start_bar: start,
            end_bar: end,
        });
    }
    sections
}

fn next_label(existing: usize) -> String {
    // 0 -> A, 25 -> Z, 26 -> AA, 27 -> AB, …
    let mut n = existing;
    let mut s = String::new();
    loop {
        let rem = n % 26;
        s.insert(0, (b'A' + rem as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s
}

pub(crate) fn strip_prime(label: &str) -> &str {
    label.trim_end_matches('\'')
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_sequencer::Pitch;

    fn note(track: u16, tick: u32, pitch: u8, vel: f32, dur: u32) -> MelodicNote {
        MelodicNote {
            track_id: track,
            tick,
            duration_ticks: dur,
            pitch: Pitch::new(pitch).unwrap(),
            velocity: vel,
        }
    }

    fn ts_4_4() -> TimeSignature {
        TimeSignature::new(4, 4)
    }

    #[test]
    fn build_bar_features_empty_input() {
        let bars = build_bar_features(&[], ts_4_4(), 2);
        assert_eq!(bars.len(), 2);
        assert!(bars.iter().all(|b| b.is_empty()));
    }

    #[test]
    fn build_bar_features_pitch_histogram_normalizes() {
        // One bar = 4 * 960 ticks at 960 PPQN.
        let bar_ticks = ts_4_4().ticks_per_bar();
        let notes = [
            note(0, 0, 60, 0.8, bar_ticks / 2),             // C
            note(0, bar_ticks / 2, 64, 0.8, bar_ticks / 2), // E
        ];
        let bars = build_bar_features(&notes, ts_4_4(), 1);
        let total: f32 = bars[0].class_histogram.iter().sum();
        assert!((total - 1.0).abs() < 1e-5);
        assert!(bars[0].class_histogram[0] > 0.0); // C
        assert!(bars[0].class_histogram[4] > 0.0); // E
        assert_eq!(bars[0].distinct_pitch_classes, 2);
        assert_eq!(bars[0].note_count, 2);
    }

    #[test]
    fn similarity_self_is_one() {
        let bar_ticks = ts_4_4().ticks_per_bar();
        let notes = [
            note(0, 0, 60, 0.7, bar_ticks / 2),
            note(0, bar_ticks / 2, 64, 0.7, bar_ticks / 2),
        ];
        let bars = build_bar_features(&notes, ts_4_4(), 1);
        let sim = bar_similarity(&bars[0], &bars[0]);
        assert!((sim - 1.0).abs() < 1e-4);
    }

    #[test]
    fn similarity_disjoint_histograms_low() {
        let bar_ticks = ts_4_4().ticks_per_bar();
        let a_notes = [note(0, 0, 60, 0.7, bar_ticks)]; // C
        let b_notes = [note(0, 0, 62, 0.7, bar_ticks)]; // D
        let a = build_bar_features(&a_notes, ts_4_4(), 1).remove(0);
        let b = build_bar_features(&b_notes, ts_4_4(), 1).remove(0);
        let sim = bar_similarity(&a, &b);
        assert!(sim < 0.5, "C-only vs D-only got sim {sim}");
    }

    #[test]
    fn cluster_single_section_when_all_similar() {
        let bar_ticks = ts_4_4().ticks_per_bar();
        let mut notes = Vec::new();
        for b in 0..4 {
            notes.push(note(0, b * bar_ticks, 60, 0.7, bar_ticks / 2));
            notes.push(note(
                0,
                b * bar_ticks + bar_ticks / 2,
                64,
                0.7,
                bar_ticks / 2,
            ));
        }
        let bars = build_bar_features(&notes, ts_4_4(), 4);
        let sections = cluster_sections(&bars, 0.85, 2);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].label, "A");
        assert_eq!(sections[0].start_bar, 1);
        assert_eq!(sections[0].end_bar, 4);
    }

    #[test]
    fn cluster_detects_aaba_pattern() {
        // 8 bars: AABA-like. Bars 1,2,4,5,8 use a C/E pad; bars 3,6 use a
        // contrasting F#/A pad; bar 7 returns to C/E.
        let bar_ticks = ts_4_4().ticks_per_bar();
        let mut notes = Vec::new();
        let a_pitches = [60u8, 64];
        let b_pitches = [66u8, 69];
        for b in 0..8u32 {
            let p = if matches!(b, 2 | 5) {
                b_pitches
            } else {
                a_pitches
            };
            for (i, midi) in p.iter().enumerate() {
                notes.push(note(
                    0,
                    b * bar_ticks + (i as u32) * (bar_ticks / 2),
                    *midi,
                    0.7,
                    bar_ticks / 2,
                ));
            }
        }
        let bars = build_bar_features(&notes, ts_4_4(), 8);
        let sections = cluster_sections(&bars, 0.85, 1);
        // Expect at least two distinct labels (A and B).
        let labels: HashSet<String> = sections.iter().map(|s| s.label.clone()).collect();
        assert!(labels.len() >= 2, "got sections {sections:?}");
        // First section starts at bar 1.
        assert_eq!(sections.first().unwrap().start_bar, 1);
        // Last section ends at bar 8.
        assert_eq!(sections.last().unwrap().end_bar, 8);
    }

    #[test]
    fn cluster_merges_short_runs_under_minimum() {
        // Two bars of A, one bar of B (short), then two bars of A again.
        // With section_min_bars = 2 the B run must be absorbed into a
        // neighbour — every emitted section is >= 2 bars and shares the
        // root label "A" (the absorbing run was longer of two A-runs).
        let bar_ticks = ts_4_4().ticks_per_bar();
        let mut notes = Vec::new();
        for b in 0..5u32 {
            let pitches: &[u8] = if b == 2 { &[66, 69] } else { &[60, 64] };
            for (i, midi) in pitches.iter().enumerate() {
                notes.push(note(
                    0,
                    b * bar_ticks + (i as u32) * (bar_ticks / 2),
                    *midi,
                    0.7,
                    bar_ticks / 2,
                ));
            }
        }
        let bars = build_bar_features(&notes, ts_4_4(), 5);
        let sections = cluster_sections(&bars, 0.85, 2);
        assert!(
            sections.iter().all(|s| s.length_bars() >= 2),
            "expected every section >= 2 bars: {sections:?}"
        );
        // Every emitted section's root label (prime-stripped) is "A" — the
        // B-run was absorbed, no foreign labels survived.
        for s in &sections {
            assert_eq!(strip_prime(&s.label), "A", "got {sections:?}");
        }
    }

    #[test]
    fn next_label_extends_past_z() {
        assert_eq!(next_label(0), "A");
        assert_eq!(next_label(25), "Z");
        assert_eq!(next_label(26), "AA");
        assert_eq!(next_label(27), "AB");
    }
}
