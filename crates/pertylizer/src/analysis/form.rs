//! Form / section diagnostic — packages `bar_features` clustering output
//! into per-section summaries with aggregate stats and the compact form
//! string used by `analyze_form_map`.
//!
//! Pure symbolic, no audio rendering. Both `analyze_arrangement` and
//! `analyze_form_map` go through the same [`analyze_form`] entry point so
//! their clustering decisions can't drift.

use std::collections::HashSet;

use crate::analysis::bar_features::{
    BarFeature, ClusteredSection, MelodicNote, build_bar_features, cluster_sections, strip_prime,
};
use synth_sequencer::TimeSignature;

/// Per-section roll-up with the same stats `BarFeatureSummary` carries
/// per-bar — caller can serialize it to wire format directly.
#[derive(Debug, Clone)]
pub struct SectionSummary {
    pub label: String,
    pub start_bar: u32,
    pub end_bar: u32,
    pub length_bars: u32,
    pub mean_notes_per_bar: f32,
    pub mean_distinct_pitch_classes: f32,
    pub mean_velocity: f32,
    pub active_track_ids: Vec<u16>,
}

/// Result of [`analyze_form`]. Carries everything both wire-format tools
/// need; the bridge picks the fields it wants for each tool.
#[derive(Debug, Clone)]
pub struct FormAnalysis {
    pub bars: Vec<BarFeature>,
    pub sections: Vec<SectionSummary>,
    pub bar_labels: Vec<String>,
    pub form_string: String,
    pub distinct_section_count: u32,
}

const EMPTY_BAR_LABEL: &str = "·";

/// Run the full form-analysis pipeline. `total_bars` is the number of bars
/// in the analyzed scope. `time_sig` is the scope's time signature.
#[must_use]
pub fn analyze_form(
    notes: &[MelodicNote],
    time_sig: TimeSignature,
    total_bars: u32,
    similarity_threshold: f32,
    section_min_bars: u32,
) -> FormAnalysis {
    let bars = build_bar_features(notes, time_sig, total_bars);
    // Collapse adjacent same-label clusters upstream so `section_summary` runs
    // once per merged span — keeps `sections.len()` and `form_string` in lock-
    // step even when prime-decision and run-length-emit disagree on a drift.
    let clustered = merge_adjacent_clusters(cluster_sections(
        &bars,
        similarity_threshold,
        section_min_bars,
    ));

    let sections: Vec<SectionSummary> = clustered
        .iter()
        .map(|sec| section_summary(sec, &bars))
        .collect();

    let mut bar_labels = vec![EMPTY_BAR_LABEL.to_string(); bars.len()];
    for sec in &clustered {
        for b in (sec.start_bar - 1)..=(sec.end_bar - 1) {
            if let Some(slot) = bar_labels.get_mut(b as usize) {
                // Empty bars stay marked '·' so the caller can see them in
                // the per-bar map; the form string skips them.
                if !bars.get(b as usize).is_some_and(|f| f.is_empty()) {
                    *slot = sec.label.clone();
                }
            }
        }
    }

    let form_string = compress_run_length(&bar_labels);

    let mut distinct: HashSet<String> = HashSet::new();
    for sec in &clustered {
        distinct.insert(strip_prime(&sec.label).to_string());
    }

    FormAnalysis {
        bars,
        sections,
        bar_labels,
        form_string,
        distinct_section_count: distinct.len() as u32,
    }
}

fn section_summary(sec: &ClusteredSection, bars: &[BarFeature]) -> SectionSummary {
    let mut note_total: u32 = 0;
    let mut distinct_total: u32 = 0;
    let mut vel_sum: f32 = 0.0;
    let mut bar_count: u32 = 0;
    let mut tracks: HashSet<u16> = HashSet::new();
    for b in bars {
        if b.bar < sec.start_bar || b.bar > sec.end_bar {
            continue;
        }
        note_total += b.note_count;
        distinct_total += u32::from(b.distinct_pitch_classes);
        vel_sum += b.mean_velocity;
        bar_count += 1;
        for t in &b.active_track_ids {
            tracks.insert(*t);
        }
    }
    let denom = bar_count.max(1) as f32;
    let mut active: Vec<u16> = tracks.into_iter().collect();
    active.sort_unstable();
    SectionSummary {
        label: sec.label.clone(),
        start_bar: sec.start_bar,
        end_bar: sec.end_bar,
        length_bars: sec.length_bars(),
        mean_notes_per_bar: note_total as f32 / denom,
        mean_distinct_pitch_classes: distinct_total as f32 / denom,
        mean_velocity: vel_sum / denom,
        active_track_ids: active,
    }
}

/// Collapse runs of same-label `ClusteredSection`s into single spans so
/// downstream `section_summary` computes one aggregate per logical section.
fn merge_adjacent_clusters(clustered: Vec<ClusteredSection>) -> Vec<ClusteredSection> {
    let mut out: Vec<ClusteredSection> = Vec::with_capacity(clustered.len());
    for sec in clustered {
        match out.last_mut() {
            Some(prev) if prev.label == sec.label => prev.end_bar = sec.end_bar,
            _ => out.push(sec),
        }
    }
    out
}

fn compress_run_length(labels: &[String]) -> String {
    let mut out = String::new();
    let mut last: Option<&str> = None;
    for l in labels {
        if l == EMPTY_BAR_LABEL {
            continue;
        }
        if last != Some(l.as_str()) {
            out.push_str(l);
            last = Some(l.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_sequencer::Pitch;

    fn note(track: u16, tick: u32, pitch: u8, dur: u32) -> MelodicNote {
        MelodicNote {
            track_id: track,
            tick,
            duration_ticks: dur,
            pitch: Pitch::new(pitch).unwrap(),
            velocity: 0.7,
        }
    }

    #[test]
    fn empty_scope_returns_zero_sections() {
        let analysis = analyze_form(&[], TimeSignature::new(4, 4), 0, 0.85, 2);
        assert!(analysis.sections.is_empty());
        assert!(analysis.bar_labels.is_empty());
        assert!(analysis.form_string.is_empty());
        assert_eq!(analysis.distinct_section_count, 0);
    }

    #[test]
    fn single_uniform_section_collapses_to_a() {
        let ts = TimeSignature::new(4, 4);
        let bar_ticks = ts.ticks_per_bar();
        let mut notes = Vec::new();
        for b in 0..4u32 {
            notes.push(note(0, b * bar_ticks, 60, bar_ticks / 2));
            notes.push(note(0, b * bar_ticks + bar_ticks / 2, 64, bar_ticks / 2));
        }
        let analysis = analyze_form(&notes, ts, 4, 0.85, 1);
        assert_eq!(analysis.sections.len(), 1);
        assert_eq!(analysis.sections[0].label, "A");
        assert_eq!(analysis.form_string, "A");
        assert_eq!(analysis.distinct_section_count, 1);
    }

    #[test]
    fn empty_bars_render_as_dot_and_skip_form_string() {
        let ts = TimeSignature::new(4, 4);
        let bar_ticks = ts.ticks_per_bar();
        // Only bars 0 and 3 have notes; bars 1, 2 are empty.
        let notes = [
            note(0, 0, 60, bar_ticks / 2),
            note(0, bar_ticks / 2, 64, bar_ticks / 2),
            note(0, 3 * bar_ticks, 60, bar_ticks / 2),
            note(0, 3 * bar_ticks + bar_ticks / 2, 64, bar_ticks / 2),
        ];
        let analysis = analyze_form(&notes, ts, 4, 0.85, 1);
        // bar_labels has 4 entries; non-empty bars carry a letter, empties
        // remain '·'.
        assert_eq!(analysis.bar_labels.len(), 4);
        assert!(analysis.bar_labels.iter().any(|l| l == "·"));
        // Form string only contains letters, no '·'.
        assert!(!analysis.form_string.contains('·'));
    }

    #[test]
    fn aabbaa_merges_to_aba() {
        // Synthetic AABBAA cluster run — adjacent same-label spans collapse
        // into single ClusteredSections covering the combined bar range.
        let clusters = vec![
            ClusteredSection {
                label: "A".to_string(),
                start_bar: 1,
                end_bar: 6,
            },
            ClusteredSection {
                label: "A".to_string(),
                start_bar: 7,
                end_bar: 8,
            },
            ClusteredSection {
                label: "B".to_string(),
                start_bar: 9,
                end_bar: 12,
            },
            ClusteredSection {
                label: "B".to_string(),
                start_bar: 13,
                end_bar: 17,
            },
            ClusteredSection {
                label: "A".to_string(),
                start_bar: 18,
                end_bar: 22,
            },
            ClusteredSection {
                label: "A".to_string(),
                start_bar: 23,
                end_bar: 24,
            },
        ];
        let merged = merge_adjacent_clusters(clusters);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].label, "A");
        assert_eq!((merged[0].start_bar, merged[0].end_bar), (1, 8));
        assert_eq!(merged[1].label, "B");
        assert_eq!((merged[1].start_bar, merged[1].end_bar), (9, 17));
        assert_eq!(merged[2].label, "A");
        assert_eq!((merged[2].start_bar, merged[2].end_bar), (18, 24));
    }

    #[test]
    fn distinct_section_count_strips_primes() {
        // Build A A' style: nudge bar 3 slightly (same dominant PC) so
        // similarity falls into the prime-band rather than equality.
        let ts = TimeSignature::new(4, 4);
        let bar_ticks = ts.ticks_per_bar();
        let mut notes = Vec::new();
        for b in 0..4u32 {
            notes.push(note(0, b * bar_ticks, 60, bar_ticks / 2));
            // Last bar uses a slightly different second pitch.
            let second = if b == 3 { 65 } else { 64 };
            notes.push(note(
                0,
                b * bar_ticks + bar_ticks / 2,
                second,
                bar_ticks / 2,
            ));
        }
        let analysis = analyze_form(&notes, ts, 4, 0.85, 1);
        // Regardless of how clustering split this, distinct_section_count
        // strips primes so it never exceeds 2.
        let letters: HashSet<char> = analysis
            .sections
            .iter()
            .map(|s| s.label.chars().next().unwrap())
            .collect();
        assert!(letters.contains(&'A'));
        assert!(analysis.distinct_section_count as usize <= letters.len());
    }
}
