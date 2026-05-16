//! Music-theory primitives for symbolic harmonic analysis.
//!
//! Pure symbolic — operates on `synth_sequencer::Pitch` / `NoteName` sets.
//! No audio path. Used by the `analyze_harmony` MCP tool.
//!
//! Provides:
//! - Scale templates (major modes, minor variants, pentatonic, blues, chromatic).
//! - Chord identification from a pitch-class set to a chord symbol.
//! - Key inference using Krumhansl-Schmuckler key-profile correlation.
//! - Harmonic stability scoring (in-key ratio, chord-key fit).

use synth_sequencer::{NoteName, Pitch};

/// Pitch class (0..=11), semitones above C.
pub(crate) type PitchClass = u8;

// ---------------------------------------------------------------------------
// Scales
// ---------------------------------------------------------------------------

/// A scale template: ordered semitone offsets from the tonic.
#[must_use]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScaleTemplate {
    pub name: &'static str,
    pub intervals: &'static [u8],
}

/// All scale templates considered by `infer_key`. Order is significant only
/// for tie-breaking — keep major and natural minor first.
pub(crate) const SCALES: &[ScaleTemplate] = &[
    ScaleTemplate {
        name: "major",
        intervals: &[0, 2, 4, 5, 7, 9, 11],
    },
    ScaleTemplate {
        name: "minor",
        intervals: &[0, 2, 3, 5, 7, 8, 10],
    },
    ScaleTemplate {
        name: "harmonic_minor",
        intervals: &[0, 2, 3, 5, 7, 8, 11],
    },
    ScaleTemplate {
        name: "melodic_minor",
        intervals: &[0, 2, 3, 5, 7, 9, 11],
    },
    ScaleTemplate {
        name: "dorian",
        intervals: &[0, 2, 3, 5, 7, 9, 10],
    },
    ScaleTemplate {
        name: "phrygian",
        intervals: &[0, 1, 3, 5, 7, 8, 10],
    },
    ScaleTemplate {
        name: "lydian",
        intervals: &[0, 2, 4, 6, 7, 9, 11],
    },
    ScaleTemplate {
        name: "mixolydian",
        intervals: &[0, 2, 4, 5, 7, 9, 10],
    },
    ScaleTemplate {
        name: "locrian",
        intervals: &[0, 1, 3, 5, 6, 8, 10],
    },
    ScaleTemplate {
        name: "pentatonic_major",
        intervals: &[0, 2, 4, 7, 9],
    },
    ScaleTemplate {
        name: "pentatonic_minor",
        intervals: &[0, 3, 5, 7, 10],
    },
    ScaleTemplate {
        name: "blues",
        intervals: &[0, 3, 5, 6, 7, 10],
    },
    ScaleTemplate {
        name: "chromatic",
        intervals: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
    },
];

impl ScaleTemplate {
    /// Returns the pitch classes (0..12) of this scale in a given key.
    #[must_use]
    pub fn pitch_classes(&self, tonic: PitchClass) -> [bool; 12] {
        let mut set = [false; 12];
        for &interval in self.intervals {
            set[((u16::from(tonic) + u16::from(interval)) % 12) as usize] = true;
        }
        set
    }
}

// ---------------------------------------------------------------------------
// Chord identification
// ---------------------------------------------------------------------------

/// Identified chord. `None` returned by `identify_chord` when no template
/// matches the input pitch set.
#[must_use]
#[derive(Debug, Clone)]
pub struct Chord {
    /// Chord symbol e.g. `"Cm7"`, `"F#maj7"`, `"Bbsus4"`.
    pub symbol: String,
    /// Root pitch class (0..12), C = 0.
    pub root: PitchClass,
    /// Quality template name e.g. `"minor7"`.
    pub quality: &'static str,
}

/// Chord template: interval set above the root (always contains 0).
pub(crate) struct ChordTemplate {
    /// Quality name (used in the output symbol after the root).
    pub quality: &'static str,
    /// Short suffix appended to the chord symbol after the root name.
    /// e.g. for `quality = "minor7"`, `suffix = "m7"` produces "Cm7".
    pub suffix: &'static str,
    /// Interval set above the root (sorted, deduplicated).
    pub intervals: &'static [u8],
}

/// Catalog of chord templates. Order matters — templates with more notes are
/// preferred over their subsets (e.g. min7 over min) so identification picks
/// the most specific match. Also drives `composition::chord_gen`: the longest
/// matching suffix wins when parsing a chord symbol like `"Cm7"`.
pub(crate) const CHORD_TEMPLATES: &[ChordTemplate] = &[
    // 4-note tetrads
    ChordTemplate {
        quality: "major7",
        suffix: "maj7",
        intervals: &[0, 4, 7, 11],
    },
    ChordTemplate {
        quality: "minor7",
        suffix: "m7",
        intervals: &[0, 3, 7, 10],
    },
    ChordTemplate {
        quality: "dominant7",
        suffix: "7",
        intervals: &[0, 4, 7, 10],
    },
    ChordTemplate {
        quality: "minor7_flat5",
        suffix: "m7b5",
        intervals: &[0, 3, 6, 10],
    },
    ChordTemplate {
        quality: "diminished7",
        suffix: "dim7",
        intervals: &[0, 3, 6, 9],
    },
    ChordTemplate {
        quality: "major6",
        suffix: "6",
        intervals: &[0, 4, 7, 9],
    },
    ChordTemplate {
        quality: "minor6",
        suffix: "m6",
        intervals: &[0, 3, 7, 9],
    },
    ChordTemplate {
        quality: "minor_major7",
        suffix: "mMaj7",
        intervals: &[0, 3, 7, 11],
    },
    ChordTemplate {
        quality: "augmented7",
        suffix: "aug7",
        intervals: &[0, 4, 8, 10],
    },
    ChordTemplate {
        quality: "dominant7_sus4",
        suffix: "7sus4",
        intervals: &[0, 5, 7, 10],
    },
    // Triads
    ChordTemplate {
        quality: "major",
        suffix: "",
        intervals: &[0, 4, 7],
    },
    ChordTemplate {
        quality: "minor",
        suffix: "m",
        intervals: &[0, 3, 7],
    },
    ChordTemplate {
        quality: "diminished",
        suffix: "dim",
        intervals: &[0, 3, 6],
    },
    ChordTemplate {
        quality: "augmented",
        suffix: "aug",
        intervals: &[0, 4, 8],
    },
    // sus4 listed before sus2 so the sus4/sus2 ambiguity (identical pitch sets,
    // different roots — Csus4 = C/F/G = Fsus2) resolves to sus4 by convention
    // when no bass hint is supplied.
    ChordTemplate {
        quality: "sus4",
        suffix: "sus4",
        intervals: &[0, 5, 7],
    },
    ChordTemplate {
        quality: "sus2",
        suffix: "sus2",
        intervals: &[0, 2, 7],
    },
    // 2-note dyads — fall through to last so triads/tetrads are preferred.
    ChordTemplate {
        quality: "power",
        suffix: "5",
        intervals: &[0, 7],
    },
];

/// Identify a chord from a slice of MIDI pitch classes (any range).
///
/// `bass_hint`, when supplied, is the pitch class of the lowest sounding note
/// and is used to disambiguate chord-symbol equivalents (e.g. Csus4 vs Fsus2,
/// which share the same pitch-class set). Templates rooted on the bass hint
/// receive a tie-breaking bonus.
///
/// Returns `None` if the pitch set is empty or no template matches. Picks the
/// template with the largest interval overlap; ties are broken by (1) root
/// matching the bass hint, then (2) template order (more-specific templates
/// listed first).
#[must_use]
pub fn identify_chord(
    pitch_classes: &[PitchClass],
    bass_hint: Option<PitchClass>,
) -> Option<Chord> {
    let mut set = [false; 12];
    let mut count = 0u32;
    for &pc in pitch_classes {
        if pc < 12 && !set[pc as usize] {
            set[pc as usize] = true;
            count += 1;
        }
    }
    // A single pitch class is a note, not a chord — refuse to fit it to any
    // template (the smallest matchable template is the power chord at two
    // pitch classes).
    if count < 2 {
        return None;
    }

    // Tuple ranked lexicographically by `(score, bass_match, -template_idx)`.
    let mut best: Option<(u32, bool, usize, PitchClass)> = None;

    for root in 0u8..12 {
        let mut intervals = [false; 12];
        for (pc, present) in set.iter().enumerate() {
            if *present {
                let interval = ((pc as i16 - root as i16).rem_euclid(12)) as usize;
                intervals[interval] = true;
            }
        }

        for (idx, template) in CHORD_TEMPLATES.iter().enumerate() {
            let all_present = template.intervals.iter().all(|&i| intervals[i as usize]);
            if !all_present {
                continue;
            }
            let matched = template.intervals.len() as u32;
            let unexplained = count.saturating_sub(matched);
            let score = matched * 100 - unexplained.min(matched);
            let bass_match = bass_hint == Some(root);

            let beats_current = match best {
                None => true,
                Some((b_score, b_bass, b_idx, _)) => {
                    if score != b_score {
                        score > b_score
                    } else if bass_match != b_bass {
                        bass_match
                    } else {
                        idx < b_idx
                    }
                }
            };
            if beats_current {
                best = Some((score, bass_match, idx, root));
            }
        }
    }

    best.map(|(_, _, idx, root)| {
        let template = &CHORD_TEMPLATES[idx];
        let root_name = NoteName::from_midi(root);
        Chord {
            symbol: format!("{root_name}{}", template.suffix),
            root,
            quality: template.quality,
        }
    })
}

/// Identify a chord from a slice of `Pitch` values. Uses the lowest-pitched
/// note's pitch class as the bass hint.
#[must_use]
pub fn identify_chord_from_pitches(pitches: &[Pitch]) -> Option<Chord> {
    if pitches.is_empty() {
        return None;
    }
    let bass = pitches.iter().map(Pitch::as_midi).min().map(|m| m % 12);
    let pcs: Vec<PitchClass> = pitches.iter().map(|p| p.as_midi() % 12).collect();
    identify_chord(&pcs, bass)
}

// ---------------------------------------------------------------------------
// Key inference (Krumhansl-Schmuckler)
// ---------------------------------------------------------------------------

/// Krumhansl-Kessler major key profile (probe-tone tonal hierarchy).
const KK_MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

/// Krumhansl-Kessler minor key profile.
const KK_MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// Inferred key.
#[must_use]
#[derive(Debug, Clone)]
pub struct KeyEstimate {
    pub tonic: PitchClass,
    /// `"major"` or `"minor"` — only these two are inferred by KS.
    pub mode: &'static str,
    /// Pearson correlation in `-1.0..=1.0`. Higher = better fit.
    pub correlation: f32,
}

impl KeyEstimate {
    /// Format as e.g. `"C major"`, `"F# minor"`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} {}", NoteName::from_midi(self.tonic), self.mode)
    }
}

/// Build a weighted pitch-class histogram from notes with durations.
///
/// `weights[i]` is the total weight (typically duration in ticks) of pitch class `i`.
#[must_use]
pub(crate) fn weighted_pitch_class_histogram(notes: &[(Pitch, f32)]) -> [f32; 12] {
    let mut hist = [0.0f32; 12];
    for &(pitch, weight) in notes {
        hist[(pitch.as_midi() % 12) as usize] += weight;
    }
    hist
}

/// Infer the most likely key using Krumhansl-Schmuckler correlation against
/// all 24 major/minor keys. Returns the top match and up to two runner-ups.
#[must_use]
pub(crate) fn infer_key(histogram: &[f32; 12]) -> Vec<KeyEstimate> {
    let total: f32 = histogram.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }

    let mut estimates = Vec::with_capacity(24);

    for tonic in 0u8..12 {
        for (mode, profile) in [("major", &KK_MAJOR_PROFILE), ("minor", &KK_MINOR_PROFILE)] {
            // Rotate the profile so it starts at this tonic.
            let mut rotated = [0.0f32; 12];
            for i in 0..12 {
                rotated[(i + tonic as usize) % 12] = profile[i];
            }
            let r = pearson_correlation(histogram, &rotated);
            estimates.push(KeyEstimate {
                tonic,
                mode,
                correlation: r,
            });
        }
    }

    estimates.sort_by(|a, b| {
        b.correlation
            .partial_cmp(&a.correlation)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    estimates.truncate(3);
    estimates
}

fn pearson_correlation(x: &[f32; 12], y: &[f32; 12]) -> f32 {
    let mean_x = x.iter().sum::<f32>() / 12.0;
    let mean_y = y.iter().sum::<f32>() / 12.0;
    let mut num = 0.0f32;
    let mut sx = 0.0f32;
    let mut sy = 0.0f32;
    for i in 0..12 {
        let dx = x[i] - mean_x;
        let dy = y[i] - mean_y;
        num += dx * dy;
        sx += dx * dx;
        sy += dy * dy;
    }
    let denom = (sx * sy).sqrt();
    if denom > 0.0 { num / denom } else { 0.0 }
}

// ---------------------------------------------------------------------------
// Harmonic stability
// ---------------------------------------------------------------------------

/// Fraction of histogram weight that falls inside the given key (0.0..=1.0).
#[must_use]
pub(crate) fn in_key_ratio(histogram: &[f32; 12], tonic: PitchClass, scale: &ScaleTemplate) -> f32 {
    let total: f32 = histogram.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }
    let in_scale = scale.pitch_classes(tonic);
    let in_total: f32 = histogram
        .iter()
        .zip(in_scale.iter())
        .filter_map(|(w, in_set)| if *in_set { Some(*w) } else { None })
        .sum();
    (in_total / total).clamp(0.0, 1.0)
}

/// Pitch classes (0..12) with non-zero weight that do not belong to the scale.
#[must_use]
pub(crate) fn out_of_scale_pitch_classes(
    histogram: &[f32; 12],
    tonic: PitchClass,
    scale: &ScaleTemplate,
) -> Vec<PitchClass> {
    let in_scale = scale.pitch_classes(tonic);
    (0u8..12)
        .filter(|&pc| histogram[pc as usize] > 0.0 && !in_scale[pc as usize])
        .collect()
}

/// Look up a scale template by name (case-sensitive). Returns major scale if
/// the name is unknown, to keep the analyzer robust.
pub(crate) fn scale_by_name(name: &str) -> &'static ScaleTemplate {
    SCALES.iter().find(|s| s.name == name).unwrap_or(&SCALES[0])
}

// ---------------------------------------------------------------------------
// Time-domain analysis driver
// ---------------------------------------------------------------------------

/// One note positioned in absolute or pattern-relative ticks. Input to
/// `analyze`. Open-ended notes (no duration in the sequencer) should be
/// passed with `end_tick = start_tick + grouping_ticks` so they still
/// contribute weight to a single chord event.
#[derive(Debug, Clone)]
pub struct AnalysisNote {
    pub pitch: Pitch,
    pub start_tick: u64,
    pub end_tick: u64,
}

/// Tunables for `analyze`.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisOptions {
    /// Chord-detection window size in ticks.
    pub grouping_ticks: u64,
    /// Tick range to analyze (`[range_start_tick, range_end_tick)`). Notes are
    /// trimmed to this range; notes outside are ignored.
    pub range_start_tick: u64,
    pub range_end_tick: u64,
}

/// One chord-detection window emitted by `analyze`.
#[derive(Debug, Clone)]
pub struct AnalysisEvent {
    pub start_tick: u64,
    pub end_tick: u64,
    pub midi_notes: Vec<u8>,
    pub chord: Option<Chord>,
    pub in_key: bool,
}

/// Aggregate analysis output. Bridge code maps this to the serializable MCP types.
#[derive(Debug, Clone)]
pub struct AnalysisOutput {
    pub events: Vec<AnalysisEvent>,
    pub inferred_key: Option<KeyEstimate>,
    pub key_candidates: Vec<KeyEstimate>,
    pub histogram: [f32; 12],
    pub in_key_ratio: f32,
    pub out_of_scale_pcs: Vec<PitchClass>,
    pub harmonic_stability_score: f32,
    pub total_notes: u32,
    pub pitch_range: Option<(u8, u8)>,
}

/// Symbolic harmonic analysis pipeline.
///
/// 1. Trims notes to the analysis range.
/// 2. Builds a duration-weighted pitch-class histogram.
/// 3. Infers the top key by Krumhansl-Schmuckler correlation.
/// 4. Walks `[range_start, range_end)` in `grouping_ticks` windows, collects
///    notes sounding in each window, identifies a chord per window.
/// 5. Computes `in_key_ratio`, out-of-scale pitch classes, and a composite
///    stability score.
#[must_use]
pub fn analyze(notes: &[AnalysisNote], opts: &AnalysisOptions) -> AnalysisOutput {
    let grouping = opts.grouping_ticks.max(1);

    let trimmed: Vec<AnalysisNote> = notes
        .iter()
        .filter_map(|n| {
            let s = n.start_tick.max(opts.range_start_tick);
            let e = n.end_tick.min(opts.range_end_tick);
            (s < e).then_some(AnalysisNote {
                pitch: n.pitch,
                start_tick: s,
                end_tick: e,
            })
        })
        .collect();

    let hist_pairs: Vec<(Pitch, f32)> = trimmed
        .iter()
        .map(|n| (n.pitch, (n.end_tick - n.start_tick) as f32))
        .collect();
    let histogram = weighted_pitch_class_histogram(&hist_pairs);

    let mut all_keys = infer_key(&histogram);
    let inferred_key = if all_keys.is_empty() {
        None
    } else {
        Some(all_keys.remove(0))
    };
    let key_candidates = all_keys;

    let scale_for_key: Option<&'static ScaleTemplate> = inferred_key.as_ref().map(|k| {
        if k.mode == "major" {
            scale_by_name("major")
        } else {
            scale_by_name("minor")
        }
    });

    let (in_key_ratio_val, out_of_scale_pcs) = match (inferred_key.as_ref(), scale_for_key) {
        (Some(k), Some(scale)) => (
            in_key_ratio(&histogram, k.tonic, scale),
            out_of_scale_pitch_classes(&histogram, k.tonic, scale),
        ),
        _ => (0.0, Vec::new()),
    };

    let mut events = Vec::new();
    let mut cursor = opts.range_start_tick;
    while cursor < opts.range_end_tick {
        let window_end = (cursor + grouping).min(opts.range_end_tick);

        let mut window_pitches: Vec<Pitch> = Vec::new();
        let mut seen = [false; 128];
        for n in &trimmed {
            if n.start_tick < window_end && n.end_tick > cursor {
                let midi = n.pitch.as_midi();
                if !seen[midi as usize] {
                    seen[midi as usize] = true;
                    window_pitches.push(n.pitch);
                }
            }
        }

        if !window_pitches.is_empty() {
            window_pitches.sort_by_key(Pitch::as_midi);
            let chord = identify_chord_from_pitches(&window_pitches);
            let in_key = match (scale_for_key, inferred_key.as_ref()) {
                (Some(scale), Some(k)) => {
                    let in_set = scale.pitch_classes(k.tonic);
                    window_pitches
                        .iter()
                        .all(|p| in_set[(p.as_midi() % 12) as usize])
                }
                _ => false,
            };
            events.push(AnalysisEvent {
                start_tick: cursor,
                end_tick: window_end,
                midi_notes: window_pitches.iter().map(Pitch::as_midi).collect(),
                chord,
                in_key,
            });
        }

        cursor = window_end;
    }

    let identified = events.iter().filter(|e| e.chord.is_some()).count();
    let id_ratio = if events.is_empty() {
        0.0
    } else {
        identified as f32 / events.len() as f32
    };
    let key_strength = inferred_key
        .as_ref()
        .map(|k| k.correlation.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let harmonic_stability_score =
        (0.5 * in_key_ratio_val + 0.25 * id_ratio + 0.25 * key_strength).clamp(0.0, 1.0);

    let pitch_range = trimmed.iter().map(|n| n.pitch.as_midi()).fold(
        None,
        |acc: Option<(u8, u8)>, m| match acc {
            None => Some((m, m)),
            Some((lo, hi)) => Some((lo.min(m), hi.max(m))),
        },
    );

    AnalysisOutput {
        events,
        inferred_key,
        key_candidates,
        histogram,
        in_key_ratio: in_key_ratio_val,
        out_of_scale_pcs,
        harmonic_stability_score,
        total_notes: trimmed.len() as u32,
        pitch_range,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn pc_set(pcs: &[u8]) -> Vec<PitchClass> {
        pcs.to_vec()
    }

    #[test]
    fn identify_basic_triads() {
        let chord = identify_chord(&pc_set(&[0, 4, 7]), Some(0)).unwrap();
        assert_eq!(chord.symbol, "C");
        assert_eq!(chord.quality, "major");
        assert_eq!(chord.root, 0);

        let chord = identify_chord(&pc_set(&[9, 0, 4]), Some(9)).unwrap();
        assert_eq!(chord.symbol, "Am");
        assert_eq!(chord.quality, "minor");

        let chord = identify_chord(&pc_set(&[2, 5, 8]), Some(2)).unwrap();
        assert_eq!(chord.symbol, "Ddim");
    }

    #[test]
    fn identify_seventh_chords() {
        let chord = identify_chord(&pc_set(&[0, 4, 7, 11]), Some(0)).unwrap();
        assert_eq!(chord.symbol, "Cmaj7");

        let chord = identify_chord(&pc_set(&[2, 5, 9, 0]), Some(2)).unwrap();
        assert_eq!(chord.symbol, "Dm7");

        let chord = identify_chord(&pc_set(&[7, 11, 2, 5]), Some(7)).unwrap();
        assert_eq!(chord.symbol, "G7");
    }

    #[test]
    fn identify_sus_chords() {
        // Csus4 = C/F/G with C in the bass.
        let chord = identify_chord(&pc_set(&[0, 5, 7]), Some(0)).unwrap();
        assert_eq!(chord.symbol, "Csus4");

        // Same pitch set, F in the bass → Fsus2.
        let chord = identify_chord(&pc_set(&[0, 5, 7]), Some(5)).unwrap();
        assert_eq!(chord.symbol, "Fsus2");

        // Dsus2 = D/E/A with D in the bass.
        let chord = identify_chord(&pc_set(&[2, 4, 9]), Some(2)).unwrap();
        assert_eq!(chord.symbol, "Dsus2");
    }

    #[test]
    fn identify_sus_no_bass_prefers_sus4() {
        // Without a bass hint, sus4 wins by template order (jazz convention).
        let chord = identify_chord(&pc_set(&[0, 5, 7]), None).unwrap();
        assert_eq!(chord.symbol, "Csus4");
    }

    #[test]
    fn identify_from_pitches_uses_bass() {
        // C3, F3, G3 → Csus4 (lowest = C).
        let pitches = [
            Pitch::new(48).unwrap(),
            Pitch::new(53).unwrap(),
            Pitch::new(55).unwrap(),
        ];
        let chord = identify_chord_from_pitches(&pitches).unwrap();
        assert_eq!(chord.symbol, "Csus4");

        // F3, G3, C4 → Fsus2 (lowest = F).
        let pitches = [
            Pitch::new(53).unwrap(),
            Pitch::new(55).unwrap(),
            Pitch::new(60).unwrap(),
        ];
        let chord = identify_chord_from_pitches(&pitches).unwrap();
        assert_eq!(chord.symbol, "Fsus2");
    }

    #[test]
    fn identify_power_chord() {
        let chord = identify_chord(&pc_set(&[0, 7]), Some(0)).unwrap();
        assert_eq!(chord.symbol, "C5");
    }

    #[test]
    fn identify_empty_returns_none() {
        assert!(identify_chord(&[], None).is_none());
        assert!(identify_chord_from_pitches(&[]).is_none());
    }

    #[test]
    fn key_inference_c_major() {
        // Strong C major bias: C, E, G, F, D, A, B with C heaviest.
        let mut hist = [0.0; 12];
        hist[0] = 10.0; // C
        hist[2] = 4.0; // D
        hist[4] = 6.0; // E
        hist[5] = 4.0; // F
        hist[7] = 7.0; // G
        hist[9] = 4.0; // A
        hist[11] = 3.0; // B

        let keys = infer_key(&hist);
        let top = &keys[0];
        assert_eq!(top.tonic, 0);
        assert_eq!(top.mode, "major");
    }

    #[test]
    fn key_inference_a_minor() {
        // Natural A minor pitches with A and E emphasized.
        let mut hist = [0.0; 12];
        hist[9] = 10.0; // A
        hist[11] = 3.0; // B
        hist[0] = 6.0; // C
        hist[2] = 4.0; // D
        hist[4] = 7.0; // E
        hist[5] = 4.0; // F
        hist[7] = 4.0; // G

        let keys = infer_key(&hist);
        let top = &keys[0];
        assert_eq!(top.tonic, 9);
        assert_eq!(top.mode, "minor");
    }

    #[test]
    fn in_key_ratio_perfect() {
        let mut hist = [0.0; 12];
        // C major pitches only.
        for pc in [0, 2, 4, 5, 7, 9, 11] {
            hist[pc] = 1.0;
        }
        let scale = scale_by_name("major");
        let ratio = in_key_ratio(&hist, 0, scale);
        assert!((ratio - 1.0).abs() < 1e-5);
    }

    #[test]
    fn in_key_ratio_partial() {
        let mut hist = [0.0; 12];
        hist[0] = 4.0; // C (in key)
        hist[1] = 1.0; // C# (out)
        let scale = scale_by_name("major");
        let ratio = in_key_ratio(&hist, 0, scale);
        assert!((ratio - 0.8).abs() < 1e-5);
    }

    #[test]
    fn out_of_scale_lists_non_diatonic() {
        let mut hist = [0.0; 12];
        hist[0] = 1.0; // C
        hist[1] = 1.0; // C# (out)
        hist[6] = 1.0; // F# (out)
        let scale = scale_by_name("major");
        let out = out_of_scale_pitch_classes(&hist, 0, scale);
        assert_eq!(out, vec![1, 6]);
    }

    fn note(midi: u8, start: u64, end: u64) -> AnalysisNote {
        AnalysisNote {
            pitch: Pitch::new(midi).unwrap(),
            start_tick: start,
            end_tick: end,
        }
    }

    #[test]
    fn analyze_empty_returns_no_events_or_key() {
        let out = analyze(
            &[],
            &AnalysisOptions {
                grouping_ticks: 960,
                range_start_tick: 0,
                range_end_tick: 3840,
            },
        );
        assert!(out.events.is_empty());
        assert!(out.inferred_key.is_none());
        assert_eq!(out.total_notes, 0);
        assert!(out.harmonic_stability_score < 1e-5);
        assert!(out.pitch_range.is_none());
    }

    #[test]
    fn analyze_c_major_progression_in_key() {
        // I-IV-V-I in C major, one quarter each (960 ticks).
        let notes = [
            // C major (C E G)
            note(60, 0, 960),
            note(64, 0, 960),
            note(67, 0, 960),
            // F major (F A C)
            note(65, 960, 1920),
            note(69, 960, 1920),
            note(72, 960, 1920),
            // G major (G B D)
            note(67, 1920, 2880),
            note(71, 1920, 2880),
            note(74, 1920, 2880),
            // C major
            note(60, 2880, 3840),
            note(64, 2880, 3840),
            note(67, 2880, 3840),
        ];
        let out = analyze(
            &notes,
            &AnalysisOptions {
                grouping_ticks: 960,
                range_start_tick: 0,
                range_end_tick: 3840,
            },
        );
        assert_eq!(out.events.len(), 4);
        assert_eq!(out.events[0].chord.as_ref().unwrap().symbol, "C");
        assert_eq!(out.events[1].chord.as_ref().unwrap().symbol, "F");
        assert_eq!(out.events[2].chord.as_ref().unwrap().symbol, "G");
        assert_eq!(out.events[3].chord.as_ref().unwrap().symbol, "C");

        let key = out.inferred_key.as_ref().unwrap();
        assert_eq!(key.tonic, 0);
        assert_eq!(key.mode, "major");

        assert!(out.events.iter().all(|e| e.in_key));
        assert!((out.in_key_ratio - 1.0).abs() < 1e-5);
        assert!(out.out_of_scale_pcs.is_empty());
        assert!(out.harmonic_stability_score > 0.8);
        assert_eq!(out.pitch_range, Some((60, 74)));
    }

    #[test]
    fn analyze_flags_out_of_scale_notes() {
        // C major chord plus an out-of-scale C# in the next window.
        let notes = [
            note(60, 0, 960),
            note(64, 0, 960),
            note(67, 0, 960),
            note(61, 960, 1920), // C# — not in C major
        ];
        let out = analyze(
            &notes,
            &AnalysisOptions {
                grouping_ticks: 960,
                range_start_tick: 0,
                range_end_tick: 1920,
            },
        );
        assert!(out.out_of_scale_pcs.contains(&1));
        assert!(out.in_key_ratio < 1.0);
        assert!(!out.events[1].in_key);
    }

    #[test]
    fn analyze_trims_notes_to_range() {
        // Note spans beyond range; should be trimmed.
        let notes = [note(60, 0, 4000)];
        let out = analyze(
            &notes,
            &AnalysisOptions {
                grouping_ticks: 960,
                range_start_tick: 960,
                range_end_tick: 1920,
            },
        );
        // Single event covering the trimmed window.
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].start_tick, 960);
        assert_eq!(out.events[0].end_tick, 1920);
        assert_eq!(out.total_notes, 1);
    }

    #[test]
    fn scale_pitch_classes_c_major() {
        let scale = scale_by_name("major");
        let pcs = scale.pitch_classes(0);
        // C major: C D E F G A B
        let expected = [
            true, false, true, false, true, true, false, true, false, true, false, true,
        ];
        assert_eq!(pcs, expected);
    }
}
