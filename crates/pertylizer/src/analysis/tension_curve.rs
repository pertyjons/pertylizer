//! Bar-level tension-curve analysis (Tier-2 #14, Group D).
//!
//! Pure synthesis layer: consumes existing per-bar features, harmony events,
//! and (optionally) per-bar audio metrics, and produces a single composite
//! tension score per bar plus cross-bar shape warnings ("chorus doesn't
//! lift", "build peaks too early", "drop loses low end", "monotone
//! tension").
//!
//! This module introduces NO new measurements — it normalizes and blends
//! values that the bar_features, harmony, harmonic_function, and mix_analysis
//! pipelines already produce. The bridge is responsible for running those
//! pipelines and handing their outputs in.
//!
//! Symbolic axes are mandatory; audio axes (loudness / brightness / band
//! entropy / stereo width) are optional. When audio is absent the composite
//! tension is derived from harmony + density + dissonance + register + rhythm
//! only.

use crate::analysis::bar_features::{BarFeature, ClusteredSection, strip_prime};

/// Per-bar chord-tension entry consumed by the curve. The bridge maps the
/// harmonic_function output's `(start_tick, end_tick, tension)` triples into
/// scope-relative ticks before handing them in.
#[derive(Debug, Clone, Copy)]
pub struct ChordTensionSpan {
    /// Scope-relative tick where the chord window begins.
    pub start_tick: u32,
    /// Scope-relative tick where the chord window ends (exclusive).
    pub end_tick: u32,
    /// Per-chord tension in `0..=1` from `harmonic_function::analyze`.
    pub tension: f32,
    /// `chord.in_key` from the harmony analyzer. Used to compute per-bar
    /// dissonance — the fraction of chord-window-ticks that are out of key.
    pub in_key: bool,
}

/// Optional audio-driven metrics for one bar. Slot in `audio_per_bar` so
/// callers can mix-and-match (e.g. provide audio only when the caller
/// rendered the scope; omit for cheap pure-symbolic mode).
#[derive(Debug, Clone, Copy)]
pub struct BarAudio {
    /// LUFS-M (momentary, 400 ms K-weighted block). `-200.0` for silence.
    pub lufs_momentary: f32,
    /// RMS in dBFS. Used as a fallback loudness reading when LUFS-M is at
    /// the silent floor (very short bars can fall below the LUFS gate).
    pub rms_dbfs: f32,
    /// Sub-band RMS energy (0–100 Hz). Used by the drop-loses-low-end
    /// warning.
    pub band_sub: f32,
    /// Low-band RMS energy (100–500 Hz).
    pub band_low: f32,
    /// Mid-band RMS energy (500–2000 Hz).
    pub band_mid: f32,
    /// High-band RMS energy (2000+ Hz). Used as the brightness numerator.
    pub band_high: f32,
    /// Pearson L/R correlation in `[-1, 1]`.
    pub stereo_correlation: f32,
    /// `side_rms / mid_rms` from the bar's mix analysis.
    pub stereo_width: f32,
}

/// Per-bar tension breakdown. All scalar fields are pre-normalized to
/// `[0, 1]` unless explicitly noted, so the caller can plot them on a single
/// axis without extra work. Audio-driven fields are `None` in symbolic-only
/// mode.
#[derive(Debug, Clone, Copy)]
pub struct TensionBar {
    /// 1-indexed bar in scope.
    pub bar: u32,
    /// Mean per-chord tension across chord windows that overlap the bar.
    /// `0.0` when no chord window touches the bar.
    pub harmonic_tension: f32,
    /// Fraction of in-bar chord-window-ticks marked out of key (proxy for
    /// vertical dissonance). `0.0` when no chord window touches the bar.
    pub dissonance: f32,
    /// `note_count / 16`, capped at 1. Picks up arpeggios and dense
    /// runs without exploding past 1.0 for very busy bars.
    pub density_score: f32,
    /// `(mean_midi - 36) / 60`, clamped — places a low C2 (36) near 0 and
    /// a high C7 (96) near 1. `0.0` for empty bars.
    pub register_score: f32,
    /// `distinct_onsets_in_grid / 16`, capped at 1 — onset positions on a
    /// 16th-note grid (one grid cell ≈ `ticks_per_bar / 16`). Picks up
    /// syncopation without being fooled by chordal multi-note onsets.
    pub rhythmic_activity: f32,
    /// Mean note velocity in `[0, 1]`.
    pub mean_velocity: f32,
    /// Number of distinct tracks that contributed notes to the bar.
    pub active_track_count: u32,
    /// Optional audio axes (see [`BarAudio`]). All four are produced
    /// together — if any one is `Some`, all four are `Some`.
    pub loudness_score: Option<f32>,
    pub brightness: Option<f32>,
    pub band_entropy: Option<f32>,
    pub stereo_width_score: Option<f32>,
    /// Final composite tension in `[0, 1]`. Weighted blend of the
    /// normalized component scores — bars with optional audio fields use
    /// the audio-augmented blend; bars without fall back to the symbolic
    /// blend.
    pub composite_tension: f32,
}

/// Curve summary statistics — peak, trough, mean, std-dev over the
/// `composite_tension` series.
#[derive(Debug, Clone, Copy, Default)]
pub struct TensionSummary {
    /// 1-indexed bar with the highest composite tension. `None` for empty
    /// scopes.
    pub peak_bar: Option<u32>,
    pub peak_value: f32,
    /// 1-indexed bar with the lowest composite tension among bars that had
    /// any notes. `None` when every bar is empty.
    pub trough_bar: Option<u32>,
    pub trough_value: f32,
    pub mean: f32,
    pub std_dev: f32,
}

/// Full tension-curve analysis output.
#[derive(Debug, Clone)]
pub struct TensionCurveAnalysis {
    pub bars: Vec<TensionBar>,
    pub summary: TensionSummary,
    pub warnings: Vec<String>,
    /// True when the analysis received `BarAudio` for at least one bar.
    pub has_audio: bool,
}

/// Inputs to [`analyze_tension_curve`]. Pure data — caller assembles this
/// from bar features, chord events, optional per-bar audio, and form-
/// clustering output.
#[derive(Debug, Clone)]
pub struct TensionCurveInputs<'a> {
    pub bars: &'a [BarFeature],
    pub bar_mean_pitches: &'a [Option<f32>],
    pub bar_distinct_onsets: &'a [u32],
    pub ticks_per_bar: u32,
    pub chord_spans: &'a [ChordTensionSpan],
    pub audio_per_bar: Option<&'a [BarAudio]>,
    pub sections: &'a [ClusteredSection],
}

const LUFS_FLOOR: f32 = -50.0;
const LUFS_CEIL: f32 = -10.0;

/// Run the analysis. `bars` is the per-bar feature stream from
/// [`crate::analysis::bar_features::build_bar_features`].
/// `bar_mean_pitches[i]` and `bar_distinct_onsets[i]` are the per-bar pitch
/// mean (MIDI 0..127) and distinct 16th-note onset count for bar i — the
/// bridge precomputes these from the same note stream that fed
/// `build_bar_features`. Audio is optional; pass `None` to skip the
/// loudness/brightness/band-entropy/stereo-width axes.
#[must_use]
pub fn analyze_tension_curve(inputs: &TensionCurveInputs) -> TensionCurveAnalysis {
    let mut warnings: Vec<String> = Vec::new();

    if inputs.bars.is_empty() {
        warnings.push("Scope contains zero bars — tension curve is empty".to_string());
        return TensionCurveAnalysis {
            bars: Vec::new(),
            summary: TensionSummary::default(),
            warnings,
            has_audio: false,
        };
    }

    if let Some(audio) = inputs.audio_per_bar
        && audio.len() != inputs.bars.len()
    {
        warnings.push(format!(
            "audio_per_bar length ({}) does not match bar count ({}) — audio axes dropped",
            audio.len(),
            inputs.bars.len()
        ));
    }
    let audio_aligned = inputs
        .audio_per_bar
        .filter(|a| a.len() == inputs.bars.len());
    let has_audio = audio_aligned.is_some();

    let ticks_per_bar = inputs.ticks_per_bar.max(1);

    // Per-bar harmonic stats: mean tension over the chord-window-ticks that
    // touch the bar, and fraction of those ticks marked out of key.
    let mut harmonic_tension_per_bar = vec![0.0f32; inputs.bars.len()];
    let mut dissonance_per_bar = vec![0.0f32; inputs.bars.len()];
    for (idx, bar) in inputs.bars.iter().enumerate() {
        let bar_start = (bar.bar - 1) * ticks_per_bar;
        let bar_end = bar_start + ticks_per_bar;
        let mut weighted_tension = 0.0_f64;
        let mut weighted_dissonance = 0.0_f64;
        let mut weight_total = 0.0_f64;
        for span in inputs.chord_spans {
            let overlap_start = span.start_tick.max(bar_start);
            let overlap_end = span.end_tick.min(bar_end);
            if overlap_end <= overlap_start {
                continue;
            }
            let w = f64::from(overlap_end - overlap_start);
            weighted_tension += f64::from(span.tension) * w;
            if !span.in_key {
                weighted_dissonance += w;
            }
            weight_total += w;
        }
        if weight_total > 0.0 {
            harmonic_tension_per_bar[idx] = (weighted_tension / weight_total) as f32;
            dissonance_per_bar[idx] = (weighted_dissonance / weight_total) as f32;
        }
    }

    // Build the per-bar entries.
    let bars: Vec<TensionBar> = inputs
        .bars
        .iter()
        .enumerate()
        .map(|(i, bar)| {
            let density_score = ((bar.note_count as f32) / 16.0).min(1.0);
            let register_score = match inputs.bar_mean_pitches.get(i).copied().flatten() {
                Some(mean_midi) => ((mean_midi - 36.0) / 60.0).clamp(0.0, 1.0),
                None => 0.0,
            };
            let onsets = inputs.bar_distinct_onsets.get(i).copied().unwrap_or(0);
            let rhythmic_activity = ((onsets as f32) / 16.0).min(1.0);
            let active_track_count = bar.active_track_ids.len() as u32;

            let (loudness_score, brightness, band_entropy, stereo_width_score) =
                if let Some(audio) = audio_aligned {
                    let a = audio[i];
                    let loud_raw = if a.lufs_momentary <= -150.0 {
                        a.rms_dbfs
                    } else {
                        a.lufs_momentary
                    };
                    let loud = ((loud_raw - LUFS_FLOOR) / (LUFS_CEIL - LUFS_FLOOR)).clamp(0.0, 1.0);
                    let band_total = a.band_sub + a.band_low + a.band_mid + a.band_high;
                    let bright = if band_total > 1e-9 {
                        ((a.band_mid + a.band_high) / band_total).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let entropy = crate::audio::analysis::EnergyBands {
                        sub: a.band_sub,
                        low: a.band_low,
                        mid: a.band_mid,
                        high: a.band_high,
                    }
                    .shannon_entropy_normalized();
                    let width = a.stereo_width.clamp(0.0, 1.0);
                    (Some(loud), Some(bright), Some(entropy), Some(width))
                } else {
                    (None, None, None, None)
                };

            let composite_tension = composite_score(
                harmonic_tension_per_bar[i],
                dissonance_per_bar[i],
                density_score,
                register_score,
                rhythmic_activity,
                loudness_score,
                brightness,
                band_entropy,
            );

            TensionBar {
                bar: bar.bar,
                harmonic_tension: harmonic_tension_per_bar[i],
                dissonance: dissonance_per_bar[i],
                density_score,
                register_score,
                rhythmic_activity,
                mean_velocity: bar.mean_velocity,
                active_track_count,
                loudness_score,
                brightness,
                band_entropy,
                stereo_width_score,
                composite_tension,
            }
        })
        .collect();

    let summary = build_summary(&bars);

    // Warnings.
    detect_monotone(&bars, &mut warnings);
    detect_chorus_doesnt_lift(&bars, inputs.sections, &mut warnings);
    detect_build_peaks_early(&bars, inputs.sections, &mut warnings);
    if let Some(audio_aligned) = audio_aligned {
        detect_drop_loses_low_end(&bars, audio_aligned, inputs.sections, &mut warnings);
    }

    TensionCurveAnalysis {
        bars,
        summary,
        warnings,
        has_audio,
    }
}

#[allow(clippy::too_many_arguments)]
fn composite_score(
    harmonic: f32,
    dissonance: f32,
    density: f32,
    register: f32,
    rhythm: f32,
    loudness: Option<f32>,
    brightness: Option<f32>,
    entropy: Option<f32>,
) -> f32 {
    // Symbolic weights — sum to 1.0 in pure mode.
    const W_HARM: f32 = 0.35;
    const W_DISS: f32 = 0.15;
    const W_DENS: f32 = 0.20;
    const W_REG: f32 = 0.10;
    const W_RHY: f32 = 0.20;
    let symbolic = W_HARM * harmonic
        + W_DISS * dissonance
        + W_DENS * density
        + W_REG * register
        + W_RHY * rhythm;
    match (loudness, brightness, entropy) {
        (Some(l), Some(b), Some(e)) => {
            // 60 % symbolic, 40 % audio when audio is present.
            const W_LOUD: f32 = 0.20;
            const W_BRIGHT: f32 = 0.12;
            const W_ENTROPY: f32 = 0.08;
            let audio = W_LOUD * l + W_BRIGHT * b + W_ENTROPY * e;
            (0.6 * symbolic + audio).clamp(0.0, 1.0)
        }
        _ => symbolic.clamp(0.0, 1.0),
    }
}

fn build_summary(bars: &[TensionBar]) -> TensionSummary {
    if bars.is_empty() {
        return TensionSummary::default();
    }
    let mut peak: Option<(u32, f32)> = None;
    let mut trough: Option<(u32, f32)> = None;
    let mut sum = 0.0_f64;
    let mut sum_sq = 0.0_f64;
    let mut count = 0_u32;
    for b in bars {
        let v = b.composite_tension;
        sum += f64::from(v);
        sum_sq += f64::from(v) * f64::from(v);
        count += 1;
        peak = match peak {
            Some((_, pv)) if pv >= v => peak,
            _ => Some((b.bar, v)),
        };
        // Skip empty bars in the trough comparison — an arrangement intro
        // of rests would otherwise always win.
        let bar_has_content = b.density_score > 0.0 || b.harmonic_tension > 0.0;
        if bar_has_content {
            trough = match trough {
                Some((_, tv)) if tv <= v => trough,
                _ => Some((b.bar, v)),
            };
        }
    }
    let denom = f64::from(count.max(1));
    let mean = (sum / denom) as f32;
    let variance = ((sum_sq / denom) - f64::from(mean) * f64::from(mean)).max(0.0);
    let std_dev = (variance.sqrt()) as f32;
    TensionSummary {
        peak_bar: peak.map(|(b, _)| b),
        peak_value: peak.map(|(_, v)| v).unwrap_or(0.0),
        trough_bar: trough.map(|(b, _)| b),
        trough_value: trough.map(|(_, v)| v).unwrap_or(0.0),
        mean,
        std_dev,
    }
}

fn detect_monotone(bars: &[TensionBar], warnings: &mut Vec<String>) {
    if bars.len() < 4 {
        return;
    }
    let mean: f32 = bars.iter().map(|b| b.composite_tension).sum::<f32>() / bars.len() as f32;
    let var: f32 = bars
        .iter()
        .map(|b| {
            let d = b.composite_tension - mean;
            d * d
        })
        .sum::<f32>()
        / bars.len() as f32;
    if var.sqrt() < 0.05 {
        warnings.push(format!(
            "Composite tension is nearly flat across {} bars (std-dev {:.3}) — section contrast is weak",
            bars.len(),
            var.sqrt()
        ));
    }
}

/// "Chorus doesn't lift": for each pair of sections sharing a root label
/// (e.g. `A` and `A'`), if the later one's mean composite tension is more
/// than 0.10 lower than the earlier one's, flag it.
fn detect_chorus_doesnt_lift(
    bars: &[TensionBar],
    sections: &[ClusteredSection],
    warnings: &mut Vec<String>,
) {
    if sections.len() < 2 {
        return;
    }
    // Build label-root → first-seen (mean, label) so reprises are compared
    // to the first appearance.
    let mut first_seen: std::collections::HashMap<String, (f32, String, u32, u32)> =
        std::collections::HashMap::new();
    for sec in sections {
        let root = strip_prime(&sec.label).to_string();
        let mean = section_mean_tension(bars, sec);
        if let Some(entry) = first_seen.get(&root).cloned() {
            // Reprise — compare.
            let (orig_mean, orig_label, orig_start, orig_end) = entry;
            if orig_mean - mean > 0.10 {
                warnings.push(format!(
                    "Section {} (bars {}-{}) returns at lower energy than the earlier {} (bars {}-{}): \
                     mean tension dropped {:.2} → {:.2}",
                    sec.label,
                    sec.start_bar,
                    sec.end_bar,
                    orig_label,
                    orig_start,
                    orig_end,
                    orig_mean,
                    mean,
                ));
            }
        } else {
            first_seen.insert(root, (mean, sec.label.clone(), sec.start_bar, sec.end_bar));
        }
    }
}

/// "Build peaks too early": within any section of length ≥ 4 bars, the bar
/// with the highest composite tension is earlier than the section midpoint
/// AND tension at the section's last bar is at least 0.10 below the peak.
fn detect_build_peaks_early(
    bars: &[TensionBar],
    sections: &[ClusteredSection],
    warnings: &mut Vec<String>,
) {
    for sec in sections {
        if sec.length_bars() < 4 {
            continue;
        }
        let mut local_peak: Option<(u32, f32)> = None;
        let mut last_value: f32 = 0.0;
        for b in bars {
            if b.bar < sec.start_bar || b.bar > sec.end_bar {
                continue;
            }
            if local_peak.is_none_or(|(_, v)| b.composite_tension > v) {
                local_peak = Some((b.bar, b.composite_tension));
            }
            if b.bar == sec.end_bar {
                last_value = b.composite_tension;
            }
        }
        let Some((peak_bar, peak_value)) = local_peak else {
            continue;
        };
        let midpoint = sec.start_bar + sec.length_bars() / 2;
        if peak_bar < midpoint && peak_value - last_value > 0.10 {
            warnings.push(format!(
                "Section {} (bars {}-{}) peaks early at bar {} ({:.2}) and tails off to {:.2} \
                 — the build runs out of energy before the section ends",
                sec.label, sec.start_bar, sec.end_bar, peak_bar, peak_value, last_value,
            ));
        }
    }
}

/// "Drop loses low-end": a bar that follows a clear loudness rise but whose
/// sub-band energy is more than 50 % lower than the previous bar's sub-band
/// energy. Requires audio.
fn detect_drop_loses_low_end(
    bars: &[TensionBar],
    audio: &[BarAudio],
    sections: &[ClusteredSection],
    warnings: &mut Vec<String>,
) {
    if bars.len() < 2 || audio.len() != bars.len() {
        return;
    }
    // Only flag inside the first bar of a new section (where the "drop"
    // would land).
    let section_starts: std::collections::HashSet<u32> =
        sections.iter().map(|s| s.start_bar).collect();
    for i in 1..bars.len() {
        let bar = &bars[i];
        if !section_starts.contains(&bar.bar) {
            continue;
        }
        let prev = &audio[i - 1];
        let cur = &audio[i];
        if prev.band_sub <= 1e-6 {
            continue;
        }
        let drop = (prev.band_sub - cur.band_sub) / prev.band_sub;
        if drop > 0.5 && bar.composite_tension >= 0.5 {
            warnings.push(format!(
                "Section starting at bar {} loses {:.0}% of sub-band energy vs. the prior bar \
                 even though overall tension is high — the drop has no low end",
                bar.bar,
                drop * 100.0,
            ));
        }
    }
}

fn section_mean_tension(bars: &[TensionBar], sec: &ClusteredSection) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_u32;
    for b in bars {
        if b.bar >= sec.start_bar && b.bar <= sec.end_bar {
            sum += b.composite_tension;
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_sequencer::{Pitch, TimeSignature};

    use crate::analysis::bar_features::{MelodicNote, build_bar_features, cluster_sections};

    fn note(track: u16, tick: u32, pitch: u8, dur: u32) -> MelodicNote {
        MelodicNote {
            track_id: track,
            tick,
            duration_ticks: dur,
            pitch: Pitch::new(pitch).unwrap(),
            velocity: 0.7,
        }
    }

    fn synthetic_inputs(
        notes: &[MelodicNote],
        time_sig: TimeSignature,
        total_bars: u32,
    ) -> (
        Vec<BarFeature>,
        Vec<Option<f32>>,
        Vec<u32>,
        Vec<ClusteredSection>,
    ) {
        let bars = build_bar_features(notes, time_sig, total_bars);
        let sections = cluster_sections(&bars, 0.85, 1);
        let ticks_per_bar = time_sig.ticks_per_bar().max(1);
        let mut mean_pitches = vec![None; bars.len()];
        let mut onsets = vec![0u32; bars.len()];
        for (i, _) in bars.iter().enumerate() {
            let bar_start = (i as u32) * ticks_per_bar;
            let bar_end = bar_start + ticks_per_bar;
            let mut sum = 0.0_f32;
            let mut count = 0_u32;
            let mut grid_cells = std::collections::HashSet::<u32>::new();
            for n in notes {
                if n.tick >= bar_start && n.tick < bar_end {
                    sum += n.pitch.as_midi() as f32;
                    count += 1;
                    let cell = (n.tick - bar_start) * 16 / ticks_per_bar;
                    grid_cells.insert(cell);
                }
            }
            if count > 0 {
                mean_pitches[i] = Some(sum / count as f32);
            }
            onsets[i] = grid_cells.len() as u32;
        }
        (bars, mean_pitches, onsets, sections)
    }

    fn ts_4_4() -> TimeSignature {
        TimeSignature::new(4, 4)
    }

    #[test]
    fn empty_scope_emits_warning() {
        let inputs = TensionCurveInputs {
            bars: &[],
            bar_mean_pitches: &[],
            bar_distinct_onsets: &[],
            ticks_per_bar: 3840,
            chord_spans: &[],
            audio_per_bar: None,
            sections: &[],
        };
        let out = analyze_tension_curve(&inputs);
        assert!(out.bars.is_empty());
        assert!(!out.warnings.is_empty());
    }

    #[test]
    fn density_score_scales_with_note_count() {
        let bar_ticks = ts_4_4().ticks_per_bar();
        // Bar 0: 4 notes; Bar 1: 16 notes (saturating).
        let mut notes = Vec::new();
        for i in 0..4 {
            notes.push(note(0, i * (bar_ticks / 4), 60, bar_ticks / 4));
        }
        for i in 0..16 {
            notes.push(note(
                0,
                bar_ticks + i * (bar_ticks / 16),
                60,
                bar_ticks / 16,
            ));
        }
        let (bars, mean_p, onsets, sections) = synthetic_inputs(&notes, ts_4_4(), 2);
        let inputs = TensionCurveInputs {
            bars: &bars,
            bar_mean_pitches: &mean_p,
            bar_distinct_onsets: &onsets,
            ticks_per_bar: bar_ticks,
            chord_spans: &[],
            audio_per_bar: None,
            sections: &sections,
        };
        let out = analyze_tension_curve(&inputs);
        assert!(out.bars[0].density_score < out.bars[1].density_score);
        assert!((out.bars[1].density_score - 1.0).abs() < 1e-4);
        assert!(out.bars[1].rhythmic_activity > out.bars[0].rhythmic_activity);
    }

    #[test]
    fn harmonic_tension_aggregates_overlapping_spans() {
        let bar_ticks = ts_4_4().ticks_per_bar();
        let notes = [note(0, 0, 60, bar_ticks), note(0, bar_ticks, 60, bar_ticks)];
        let (bars, mean_p, onsets, sections) = synthetic_inputs(&notes, ts_4_4(), 2);
        // Two chord windows: bar 1 has tension 0.2, bar 2 has tension 0.9.
        let chord_spans = vec![
            ChordTensionSpan {
                start_tick: 0,
                end_tick: bar_ticks,
                tension: 0.2,
                in_key: true,
            },
            ChordTensionSpan {
                start_tick: bar_ticks,
                end_tick: bar_ticks * 2,
                tension: 0.9,
                in_key: false,
            },
        ];
        let inputs = TensionCurveInputs {
            bars: &bars,
            bar_mean_pitches: &mean_p,
            bar_distinct_onsets: &onsets,
            ticks_per_bar: bar_ticks,
            chord_spans: &chord_spans,
            audio_per_bar: None,
            sections: &sections,
        };
        let out = analyze_tension_curve(&inputs);
        assert!((out.bars[0].harmonic_tension - 0.2).abs() < 1e-4);
        assert!((out.bars[1].harmonic_tension - 0.9).abs() < 1e-4);
        assert!(out.bars[0].dissonance < 0.1);
        assert!(out.bars[1].dissonance > 0.9);
        assert!(out.bars[1].composite_tension > out.bars[0].composite_tension);
    }

    #[test]
    fn monotone_warning_fires_on_flat_curve() {
        // Build 4 identical bars; no audio; constant chord tension. Curve
        // should be flat, monotone warning should fire.
        let bar_ticks = ts_4_4().ticks_per_bar();
        let mut notes = Vec::new();
        for b in 0..4 {
            notes.push(note(0, b * bar_ticks, 60, bar_ticks / 2));
            notes.push(note(0, b * bar_ticks + bar_ticks / 2, 64, bar_ticks / 2));
        }
        let (bars, mean_p, onsets, sections) = synthetic_inputs(&notes, ts_4_4(), 4);
        let inputs = TensionCurveInputs {
            bars: &bars,
            bar_mean_pitches: &mean_p,
            bar_distinct_onsets: &onsets,
            ticks_per_bar: bar_ticks,
            chord_spans: &[],
            audio_per_bar: None,
            sections: &sections,
        };
        let out = analyze_tension_curve(&inputs);
        assert!(
            out.warnings.iter().any(|w| w.contains("nearly flat")),
            "expected monotone warning, got {:?}",
            out.warnings
        );
    }

    #[test]
    fn audio_axes_blend_into_composite() {
        let bar_ticks = ts_4_4().ticks_per_bar();
        let notes = [note(0, 0, 60, bar_ticks), note(0, bar_ticks, 60, bar_ticks)];
        let (bars, mean_p, onsets, sections) = synthetic_inputs(&notes, ts_4_4(), 2);
        let quiet = BarAudio {
            lufs_momentary: -45.0,
            rms_dbfs: -45.0,
            band_sub: 0.1,
            band_low: 0.05,
            band_mid: 0.02,
            band_high: 0.01,
            stereo_correlation: 1.0,
            stereo_width: 0.0,
        };
        let loud = BarAudio {
            lufs_momentary: -12.0,
            rms_dbfs: -12.0,
            band_sub: 0.5,
            band_low: 0.5,
            band_mid: 0.5,
            band_high: 0.5,
            stereo_correlation: 0.4,
            stereo_width: 0.7,
        };
        let audio = vec![quiet, loud];
        let inputs = TensionCurveInputs {
            bars: &bars,
            bar_mean_pitches: &mean_p,
            bar_distinct_onsets: &onsets,
            ticks_per_bar: bar_ticks,
            chord_spans: &[],
            audio_per_bar: Some(&audio),
            sections: &sections,
        };
        let out = analyze_tension_curve(&inputs);
        assert!(out.has_audio);
        assert!(out.bars[0].loudness_score.unwrap() < out.bars[1].loudness_score.unwrap());
        assert!(out.bars[0].composite_tension < out.bars[1].composite_tension);
    }

    #[test]
    fn chorus_doesnt_lift_flags_quieter_reprise() {
        // Build two distinct bars first (A), then duplicate them with much
        // lower velocity so the cluster maps them to A' but the composite
        // tension drops.
        let bar_ticks = ts_4_4().ticks_per_bar();
        let mut notes = Vec::new();
        // Section A: bars 1-2 with eight notes each, full velocity.
        for b in 0..2u32 {
            for i in 0..8u32 {
                notes.push(MelodicNote {
                    track_id: 0,
                    tick: b * bar_ticks + i * (bar_ticks / 8),
                    duration_ticks: bar_ticks / 8,
                    pitch: Pitch::new(60).unwrap(),
                    velocity: 0.9,
                });
            }
        }
        // Section A' reprise: bars 3-4, fewer notes + lower velocity.
        for b in 2..4u32 {
            for i in 0..2u32 {
                notes.push(MelodicNote {
                    track_id: 0,
                    tick: b * bar_ticks + i * (bar_ticks / 2),
                    duration_ticks: bar_ticks / 2,
                    pitch: Pitch::new(60).unwrap(),
                    velocity: 0.2,
                });
            }
        }
        let (bars, mean_p, onsets, _) = synthetic_inputs(&notes, ts_4_4(), 4);
        // Force the section view: the synthetic notes cluster as one
        // run, but the test exercises the warning that fires between A
        // and its reprise A'.
        let manual_sections = vec![
            ClusteredSection {
                label: "A".to_string(),
                start_bar: 1,
                end_bar: 2,
            },
            ClusteredSection {
                label: "A'".to_string(),
                start_bar: 3,
                end_bar: 4,
            },
        ];
        let inputs = TensionCurveInputs {
            bars: &bars,
            bar_mean_pitches: &mean_p,
            bar_distinct_onsets: &onsets,
            ticks_per_bar: bar_ticks,
            chord_spans: &[],
            audio_per_bar: None,
            sections: &manual_sections,
        };
        let out = analyze_tension_curve(&inputs);
        assert!(
            out.warnings
                .iter()
                .any(|w| w.contains("returns at lower energy")),
            "expected chorus-doesn't-lift warning, got {:?}",
            out.warnings
        );
    }

    #[test]
    fn build_peaks_early_flagged() {
        // 4-bar section where bar 1 is the loudest and the section tails
        // off.
        let bar_ticks = ts_4_4().ticks_per_bar();
        let mut notes = Vec::new();
        // Bar 1: 16 notes (full density).
        for i in 0..16u32 {
            notes.push(MelodicNote {
                track_id: 0,
                tick: i * (bar_ticks / 16),
                duration_ticks: bar_ticks / 16,
                pitch: Pitch::new(60).unwrap(),
                velocity: 0.9,
            });
        }
        // Bars 2-4: one quiet note each.
        for b in 1..4u32 {
            notes.push(MelodicNote {
                track_id: 0,
                tick: b * bar_ticks,
                duration_ticks: bar_ticks,
                pitch: Pitch::new(60).unwrap(),
                velocity: 0.2,
            });
        }
        let (bars, mean_p, onsets, _) = synthetic_inputs(&notes, ts_4_4(), 4);
        let manual_sections = vec![ClusteredSection {
            label: "A".to_string(),
            start_bar: 1,
            end_bar: 4,
        }];
        let inputs = TensionCurveInputs {
            bars: &bars,
            bar_mean_pitches: &mean_p,
            bar_distinct_onsets: &onsets,
            ticks_per_bar: bar_ticks,
            chord_spans: &[],
            audio_per_bar: None,
            sections: &manual_sections,
        };
        let out = analyze_tension_curve(&inputs);
        assert!(
            out.warnings.iter().any(|w| w.contains("peaks early")),
            "expected build-peaks-early warning, got {:?}",
            out.warnings
        );
    }
}
