//! Symbolic drum-feel analysis.
//!
//! Drum-specific diagnostics that are more actionable than a generic groove
//! score: backbeat strength, hat subdivision, ghost notes, fills, and
//! repeated-bar sameness. Pure symbolic — runs on the notes attached to drum
//! tracks (identified via [`crate::analysis::infer_all_profiles`]) or on a
//! single pattern's notes directly. No audio rendering, no engine snapshot.

use synth_sequencer::{Note, TICKS_PER_QUARTER, TimeSignature};

use crate::analysis::repetition::{BarSignatureNote, REPETITION_GRID_TICKS, bar_repetition_score};

/// Drum-component classification for a single hit. Derived from the MIDI note
/// number using the General MIDI drum map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DrumComponent {
    Kick,
    Snare,
    HatClosed,
    HatOpen,
    Tom,
    Cymbal,
    Clap,
    OtherPercussion,
}

impl DrumComponent {
    /// Map a MIDI note number to a drum component using the GM drum map.
    /// Unknown notes fall through to `OtherPercussion` so users with custom
    /// drum maps still get a usable analysis instead of dropped notes.
    #[must_use]
    pub fn from_midi(midi: u8) -> Self {
        match midi {
            // Acoustic Bass Drum, Bass Drum 1.
            35 | 36 => Self::Kick,
            // Side Stick, Acoustic Snare, Electric Snare.
            37 | 38 | 40 => Self::Snare,
            // Hand Clap.
            39 => Self::Clap,
            // Toms: low/floor/mid/high/low-mid/hi.
            41 | 43 | 45 | 47 | 48 | 50 => Self::Tom,
            // Closed Hi-Hat, Pedal Hi-Hat.
            42 | 44 => Self::HatClosed,
            // Open Hi-Hat.
            46 => Self::HatOpen,
            // Crashes / rides / splashes / chinas / cowbells (all "cymbal-ish").
            49 | 51 | 52 | 53 | 55 | 57 | 59 => Self::Cymbal,
            _ => Self::OtherPercussion,
        }
    }

    /// Map a track name to a kit-specific drum component when the name calls
    /// out a single piece (e.g. `"Kick"`, `"Open Hat"`, `"Snare 1"`). Returns
    /// `None` for generic names like `"Drums"` or `"Percussion"` so the caller
    /// falls back to the GM-MIDI mapping. Word-match (not substring) so a track
    /// called `"Kickback"` doesn't trigger `Kick`. Required for projects that
    /// don't follow GM kit MIDI — common in electronic music where each drum
    /// gets its own trigger note.
    #[must_use]
    pub fn from_track_name(track_name: &str) -> Option<Self> {
        let tokens = crate::analysis::instrument_profile::tokenize_name(track_name);
        let has = |word: &str| tokens.iter().any(|t| t == word);

        // Hi-hats first so "Open Hat" wins over the plain hat fallthrough.
        if has("hat") || has("hh") || has("hihat") {
            return Some(if has("open") {
                Self::HatOpen
            } else {
                Self::HatClosed
            });
        }
        if has("kick") || has("bd") || has("bassdrum") {
            return Some(Self::Kick);
        }
        if has("snare") || has("sd") || has("rim") || has("rimshot") {
            return Some(Self::Snare);
        }
        if has("clap") || has("cp") {
            return Some(Self::Clap);
        }
        if has("tom") || has("floor") || has("rack") {
            return Some(Self::Tom);
        }
        if has("crash") || has("ride") || has("cymbal") || has("china") || has("splash") {
            return Some(Self::Cymbal);
        }
        None
    }

    /// Friendly name for warnings / serialization.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kick => "kick",
            Self::Snare => "snare",
            Self::HatClosed => "hat_closed",
            Self::HatOpen => "hat_open",
            Self::Tom => "tom",
            Self::Cymbal => "cymbal",
            Self::Clap => "clap",
            Self::OtherPercussion => "other_percussion",
        }
    }
}

/// One drum note positioned in absolute or pattern-relative ticks. Input to
/// [`analyze`]. `component_hint` lets the caller pre-classify the hit using
/// track-role context (e.g. notes from a track named `"Kick"` are kicks
/// regardless of MIDI number) — when set, the hint takes precedence over the
/// GM-MIDI mapping inside [`analyze`].
#[derive(Debug, Clone, Copy)]
pub struct DrumNote {
    pub tick: u32,
    pub midi: u8,
    pub velocity: f32,
    pub component_hint: Option<DrumComponent>,
}

impl DrumNote {
    #[must_use]
    pub fn from_note(n: &Note, base_tick: u32) -> Self {
        Self {
            tick: base_tick.saturating_add(n.start.0),
            midi: n.pitch.as_midi(),
            velocity: n.velocity.as_f32(),
            component_hint: None,
        }
    }
}

/// Per-component counts.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrumComposition {
    pub kick: u32,
    pub snare: u32,
    pub hat_closed: u32,
    pub hat_open: u32,
    pub tom: u32,
    pub cymbal: u32,
    pub clap: u32,
    pub other: u32,
}

/// Backbeat = snare hits landing on beats 2 and 4 of a 4-beat bar (or beat 2
/// in 3/4). `strength` is the fraction of expected backbeat positions that
/// actually carry a snare hit; values near 1.0 are tight, near 0.0 are flat.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrumBackbeat {
    pub strength: f32,
    pub expected_backbeats: u32,
    pub matched_backbeats: u32,
    pub off_backbeat_snares: u32,
}

/// Hat subdivision classification.
#[derive(Debug, Clone, Default)]
pub struct DrumHat {
    /// `"quarter" | "8th" | "16th" | "triplet_8th" | "triplet_16th" |
    /// "irregular" | "none"`.
    pub subdivision: String,
    pub hat_density_per_beat: f32,
    pub hat_count: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DrumGhostNotes {
    pub count: u32,
    /// Threshold used (velocity in 0..=1) below which a snare hit is counted
    /// as a ghost note. Reported so the AI can sanity-check the heuristic.
    pub velocity_threshold: f32,
}

/// Fill detection on a per-bar basis. A bar is flagged as a "fill candidate"
/// when its drum-note density exceeds `FILL_DENSITY_MULTIPLIER` times the
/// mean drum-note density across the rest of the range.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrumFills {
    pub fill_bar_count: u32,
    pub density_threshold: f32,
    pub mean_density_per_bar: f32,
}

/// Bar-level self-similarity over drum notes.
#[derive(Debug, Clone, Copy, Default)]
pub struct DrumRepetition {
    pub distinct_bars: u32,
    pub total_bars: u32,
    /// `0.0..=1.0`. `1.0` = every bar carries the same drum hits; `0.0` =
    /// every bar unique. `0.0` for ≤ 1 bar of drum activity (no repetition
    /// to measure).
    pub bar_repetition_score: f32,
}

/// Aggregate result of [`analyze`].
#[derive(Debug, Clone, Default)]
pub struct DrumGrooveAnalysis {
    pub length_ticks: u32,
    pub length_bars: f32,
    pub total_drum_notes: u32,
    pub composition: DrumComposition,
    pub backbeat: DrumBackbeat,
    pub hat: DrumHat,
    pub ghost_notes: DrumGhostNotes,
    pub fills: DrumFills,
    pub repetition: DrumRepetition,
    pub warnings: Vec<String>,
}

/// A snare hit qualifies as a ghost note when its velocity is below this
/// fraction of the strongest snare hit in the range. Fixed (not data-driven)
/// so the heuristic stays interpretable.
const GHOST_NOTE_VELOCITY_FRACTION: f32 = 0.5;

/// Bars are flagged as fills when their drum density is at least this many
/// times the mean drum density of the surrounding range.
const FILL_DENSITY_MULTIPLIER: f32 = 2.0;

/// Maximum onset offset (in ticks) that still counts as "on the beat" for
/// backbeat detection. Quarter of a quarter-note = 16th-note tolerance.
const BACKBEAT_TOLERANCE_TICKS: u32 = TICKS_PER_QUARTER / 4;

/// Run the drum-groove analysis.
///
/// `notes` are drum-track notes positioned in the same tick space as
/// `length_ticks` (i.e. pattern-relative ticks when analyzing one pattern,
/// arrangement-absolute ticks shifted so the range starts at 0 when analyzing
/// a section). `time_sig` controls bar/beat normalization.
#[must_use]
pub fn analyze(
    notes: &[DrumNote],
    length_ticks: u32,
    time_sig: TimeSignature,
) -> DrumGrooveAnalysis {
    let mut warnings: Vec<String> = Vec::new();
    let ticks_per_bar = time_sig.ticks_per_bar().max(1);
    let ticks_per_beat = time_sig.ticks_per_beat().max(1);
    let total_bars = length_ticks.div_ceil(ticks_per_bar).max(1);
    let length_bars = if length_ticks == 0 {
        0.0
    } else {
        length_ticks as f32 / ticks_per_bar as f32
    };

    if length_ticks == 0 {
        warnings.push("Drum range length is 0 ticks — analysis skipped".to_string());
        return DrumGrooveAnalysis {
            warnings,
            ..Default::default()
        };
    }

    if notes.is_empty() {
        warnings.push("No drum notes in scope".to_string());
        return DrumGrooveAnalysis {
            length_ticks,
            length_bars,
            warnings,
            ..Default::default()
        };
    }

    let mut composition = DrumComposition::default();
    let mut classified: Vec<(DrumComponent, DrumNote)> = Vec::with_capacity(notes.len());
    for n in notes {
        if n.tick >= length_ticks {
            continue;
        }
        let component = n
            .component_hint
            .unwrap_or_else(|| DrumComponent::from_midi(n.midi));
        match component {
            DrumComponent::Kick => composition.kick += 1,
            DrumComponent::Snare => composition.snare += 1,
            DrumComponent::HatClosed => composition.hat_closed += 1,
            DrumComponent::HatOpen => composition.hat_open += 1,
            DrumComponent::Tom => composition.tom += 1,
            DrumComponent::Cymbal => composition.cymbal += 1,
            DrumComponent::Clap => composition.clap += 1,
            DrumComponent::OtherPercussion => composition.other += 1,
        }
        classified.push((component, *n));
    }
    let total_drum_notes = classified.len() as u32;

    let backbeat = analyze_backbeat(&classified, length_ticks, time_sig);
    let hat = analyze_hat(&classified, ticks_per_beat, length_ticks);
    let ghost_notes = analyze_ghost_notes(&classified);
    let fills = analyze_fills(&classified, ticks_per_bar, total_bars);
    let repetition = analyze_repetition(&classified, ticks_per_bar, total_bars, &mut warnings);

    DrumGrooveAnalysis {
        length_ticks,
        length_bars,
        total_drum_notes,
        composition,
        backbeat,
        hat,
        ghost_notes,
        fills,
        repetition,
        warnings,
    }
}

fn analyze_backbeat(
    classified: &[(DrumComponent, DrumNote)],
    length_ticks: u32,
    time_sig: TimeSignature,
) -> DrumBackbeat {
    let ticks_per_beat = time_sig.ticks_per_beat().max(1);
    let ticks_per_bar = time_sig.ticks_per_bar().max(1);
    let total_bars = length_ticks.div_ceil(ticks_per_bar).max(1);

    // Backbeat positions: in 4/4 → beats 2, 4 (1-indexed) → ticks 1×beat, 3×beat.
    // In 3/4 → beat 2. In other meters: every other beat starting from beat 2.
    let beats_per_bar = time_sig.numerator.max(1) as u32;
    let mut backbeat_offsets: Vec<u32> = Vec::new();
    let mut beat = 1u32;
    while beat < beats_per_bar {
        backbeat_offsets.push(beat * ticks_per_beat);
        beat += 2;
    }
    if backbeat_offsets.is_empty() {
        // 1/4 or 2/4: there's only one beat / no off-beat, fall back to beat 2 if it exists.
        backbeat_offsets.push(ticks_per_beat);
    }

    let mut expected = 0u32;
    let mut matched = 0u32;
    let mut off_backbeat = 0u32;

    let snare_ticks: Vec<u32> = classified
        .iter()
        .filter(|(c, _)| *c == DrumComponent::Snare)
        .map(|(_, n)| n.tick)
        .collect();

    for bar in 0..total_bars {
        let bar_start = bar * ticks_per_bar;
        for &offset in &backbeat_offsets {
            let target = bar_start.saturating_add(offset);
            if target >= length_ticks {
                continue;
            }
            expected += 1;
            let has_snare = snare_ticks.iter().any(|&t| {
                let lo = target.saturating_sub(BACKBEAT_TOLERANCE_TICKS);
                let hi = target.saturating_add(BACKBEAT_TOLERANCE_TICKS);
                t >= lo && t <= hi
            });
            if has_snare {
                matched += 1;
            }
        }
    }

    for &tick in &snare_ticks {
        let bar_pos = tick % ticks_per_bar;
        let is_on_backbeat = backbeat_offsets.iter().any(|&o| {
            let lo = o.saturating_sub(BACKBEAT_TOLERANCE_TICKS);
            let hi = o.saturating_add(BACKBEAT_TOLERANCE_TICKS);
            bar_pos >= lo && bar_pos <= hi
        });
        if !is_on_backbeat {
            off_backbeat += 1;
        }
    }

    let strength = if expected > 0 {
        matched as f32 / expected as f32
    } else {
        0.0
    };

    DrumBackbeat {
        strength: strength.clamp(0.0, 1.0),
        expected_backbeats: expected,
        matched_backbeats: matched,
        off_backbeat_snares: off_backbeat,
    }
}

fn analyze_hat(
    classified: &[(DrumComponent, DrumNote)],
    ticks_per_beat: u32,
    length_ticks: u32,
) -> DrumHat {
    let mut hat_ticks: Vec<u32> = classified
        .iter()
        .filter(|(c, _)| matches!(c, DrumComponent::HatClosed | DrumComponent::HatOpen))
        .map(|(_, n)| n.tick)
        .collect();
    hat_ticks.sort_unstable();
    hat_ticks.dedup();

    let hat_count = hat_ticks.len() as u32;
    let length_beats = length_ticks as f32 / ticks_per_beat.max(1) as f32;
    let density = if length_beats > 0.0 {
        hat_count as f32 / length_beats
    } else {
        0.0
    };

    if hat_count == 0 {
        return DrumHat {
            subdivision: "none".to_string(),
            hat_density_per_beat: 0.0,
            hat_count: 0,
        };
    }
    if hat_count < 4 {
        return DrumHat {
            subdivision: "irregular".to_string(),
            hat_density_per_beat: density,
            hat_count,
        };
    }

    // Classify by the dominant IOI between consecutive hat onsets.
    // Reference IOIs:
    //   quarter  = ticks_per_beat
    //   8th      = ticks_per_beat / 2
    //   16th     = ticks_per_beat / 4
    //   triplet 8th  = ticks_per_beat * 2 / 3
    //   triplet 16th = ticks_per_beat / 3
    let candidates: [(&'static str, f32); 5] = [
        ("quarter", ticks_per_beat as f32),
        ("8th", ticks_per_beat as f32 / 2.0),
        ("16th", ticks_per_beat as f32 / 4.0),
        ("triplet_8th", ticks_per_beat as f32 * 2.0 / 3.0),
        ("triplet_16th", ticks_per_beat as f32 / 3.0),
    ];

    let iois: Vec<f32> = hat_ticks.windows(2).map(|w| (w[1] - w[0]) as f32).collect();
    if iois.is_empty() {
        return DrumHat {
            subdivision: "irregular".to_string(),
            hat_density_per_beat: density,
            hat_count,
        };
    }

    // For each candidate, count how many IOIs are within ±15% of it.
    let mut best_name = "irregular";
    let mut best_score = 0usize;
    for (name, target) in &candidates {
        if *target <= 0.0 {
            continue;
        }
        let tolerance = *target * 0.15;
        let hits = iois
            .iter()
            .filter(|&&i| (i - *target).abs() <= tolerance)
            .count();
        if hits > best_score {
            best_score = hits;
            best_name = name;
        }
    }

    // Require at least 60% of IOIs to match a single subdivision; otherwise
    // call it irregular.
    let subdivision = if best_score * 100 >= 60 * iois.len() {
        best_name.to_string()
    } else {
        "irregular".to_string()
    };

    DrumHat {
        subdivision,
        hat_density_per_beat: density,
        hat_count,
    }
}

fn analyze_ghost_notes(classified: &[(DrumComponent, DrumNote)]) -> DrumGhostNotes {
    let snare_velocities: Vec<f32> = classified
        .iter()
        .filter(|(c, _)| *c == DrumComponent::Snare)
        .map(|(_, n)| n.velocity)
        .collect();

    if snare_velocities.len() < 2 {
        return DrumGhostNotes {
            count: 0,
            velocity_threshold: 0.0,
        };
    }

    let max_vel = snare_velocities
        .iter()
        .copied()
        .fold(0.0f32, |acc, v| acc.max(v));
    let threshold = max_vel * GHOST_NOTE_VELOCITY_FRACTION;
    let snare_count = snare_velocities.len();
    let below_threshold = snare_velocities.iter().filter(|&&v| v < threshold).count() as u32;

    // Skip uniform-velocity patterns: without a strong/quiet split, every
    // snare would falsely register as a ghost.
    let above_threshold = snare_count - below_threshold as usize;
    let count = if above_threshold > 0 && below_threshold > 0 {
        below_threshold
    } else {
        0
    };

    DrumGhostNotes {
        count,
        velocity_threshold: threshold,
    }
}

fn analyze_fills(
    classified: &[(DrumComponent, DrumNote)],
    ticks_per_bar: u32,
    total_bars: u32,
) -> DrumFills {
    if total_bars == 0 || classified.is_empty() {
        return DrumFills::default();
    }

    // Hits per bar.
    let mut per_bar = vec![0u32; total_bars as usize];
    for (_, n) in classified {
        let bar = (n.tick / ticks_per_bar) as usize;
        if let Some(slot) = per_bar.get_mut(bar) {
            *slot += 1;
        }
    }

    let mean_density: f32 = per_bar.iter().map(|&c| c as f32).sum::<f32>() / total_bars as f32;
    let threshold = (mean_density * FILL_DENSITY_MULTIPLIER).max(mean_density + 1.0);
    let fill_count = per_bar
        .iter()
        .filter(|&&c| c as f32 >= threshold && c > 0)
        .count() as u32;

    DrumFills {
        fill_bar_count: fill_count,
        density_threshold: threshold,
        mean_density_per_bar: mean_density,
    }
}

fn analyze_repetition(
    classified: &[(DrumComponent, DrumNote)],
    ticks_per_bar: u32,
    total_bars: u32,
    warnings: &mut Vec<String>,
) -> DrumRepetition {
    if total_bars < 2 {
        warnings.push(
            "Drum scope is shorter than 2 bars — bar_repetition_score is not meaningful"
                .to_string(),
        );
        return DrumRepetition {
            distinct_bars: 1,
            total_bars,
            bar_repetition_score: 0.0,
        };
    }

    let grid = REPETITION_GRID_TICKS.max(1);
    let signature_notes: Vec<BarSignatureNote<(u8, DrumComponent)>> = classified
        .iter()
        .filter_map(|(c, n)| {
            let bar = (n.tick / ticks_per_bar) as usize;
            (bar < total_bars as usize).then(|| BarSignatureNote {
                bar,
                tick_in_bar: (n.tick % ticks_per_bar) / grid * grid,
                discriminant: (n.midi, *c),
            })
        })
        .collect();
    let (distinct, score) = bar_repetition_score(&signature_notes, total_bars);

    DrumRepetition {
        distinct_bars: distinct,
        total_bars,
        bar_repetition_score: score,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn dn(tick: u32, midi: u8, vel: f32) -> DrumNote {
        DrumNote {
            tick,
            midi,
            velocity: vel,
            component_hint: None,
        }
    }

    fn dn_hint(tick: u32, midi: u8, vel: f32, hint: DrumComponent) -> DrumNote {
        DrumNote {
            tick,
            midi,
            velocity: vel,
            component_hint: Some(hint),
        }
    }

    #[test]
    fn empty_input_warns() {
        let out = analyze(&[], 3840, TimeSignature::COMMON);
        assert_eq!(out.total_drum_notes, 0);
        assert!(out.warnings.iter().any(|w| w.contains("No drum notes")));
    }

    #[test]
    fn zero_length_warns() {
        let out = analyze(&[dn(0, 36, 1.0)], 0, TimeSignature::COMMON);
        assert!(out.warnings.iter().any(|w| w.contains("length is 0")));
    }

    #[test]
    fn classic_44_backbeat_scores_one() {
        // 4/4 bar, ticks_per_beat = 960. Backbeats at 960 and 2880.
        let notes = vec![
            dn(0, 36, 1.0),    // Kick on 1
            dn(960, 38, 1.0),  // Snare on 2
            dn(1920, 36, 1.0), // Kick on 3
            dn(2880, 38, 1.0), // Snare on 4
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.backbeat.expected_backbeats, 2);
        assert_eq!(out.backbeat.matched_backbeats, 2);
        assert!((out.backbeat.strength - 1.0).abs() < 1e-5);
        assert_eq!(out.backbeat.off_backbeat_snares, 0);
        assert_eq!(out.composition.kick, 2);
        assert_eq!(out.composition.snare, 2);
    }

    #[test]
    fn snare_only_on_one_and_three_scores_zero_backbeat() {
        let notes = vec![
            dn(0, 38, 1.0),    // Snare on 1
            dn(1920, 38, 1.0), // Snare on 3
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert!(out.backbeat.strength < 1e-5);
        assert_eq!(out.backbeat.off_backbeat_snares, 2);
    }

    #[test]
    fn steady_8ths_hat_detected() {
        // Closed hat every 480 ticks (8th note in 4/4 with 960 tpb), 8 hits over the bar.
        let mut notes = Vec::new();
        for i in 0..8 {
            notes.push(dn(i * 480, 42, 0.8));
        }
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.hat.subdivision, "8th");
        assert_eq!(out.hat.hat_count, 8);
        assert!((out.hat.hat_density_per_beat - 2.0).abs() < 1e-3);
    }

    #[test]
    fn steady_16ths_hat_detected() {
        // 16th every 240 ticks, 16 hits over the bar.
        let mut notes = Vec::new();
        for i in 0..16 {
            notes.push(dn(i * 240, 42, 0.8));
        }
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.hat.subdivision, "16th");
        assert_eq!(out.hat.hat_count, 16);
    }

    #[test]
    fn ghost_notes_detected_when_velocity_split() {
        // 2 strong snares + 2 quiet snares (well below half of max).
        let notes = vec![
            dn(0, 38, 1.0),
            dn(240, 38, 0.2),
            dn(960, 38, 1.0),
            dn(1200, 38, 0.2),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert!(out.ghost_notes.count >= 2);
    }

    #[test]
    fn uniform_velocity_snare_yields_zero_ghosts() {
        let notes = vec![dn(0, 38, 0.9), dn(960, 38, 0.9), dn(1920, 38, 0.9)];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.ghost_notes.count, 0);
    }

    #[test]
    fn fill_bar_detected_at_high_density() {
        // 3 bars of 1 hit each, 1 bar of 16 hits. Mean density = (3+16)/4 = 4.75.
        // Threshold = max(mean * 2, mean + 1) = max(9.5, 5.75) = 9.5. The 16-hit
        // bar should exceed.
        let mut notes = Vec::new();
        for bar in 0..3 {
            notes.push(dn(bar * 3840, 36, 1.0));
        }
        for i in 0..16 {
            notes.push(dn(3 * 3840 + i * 240, 36, 1.0));
        }
        let out = analyze(&notes, 4 * 3840, TimeSignature::COMMON);
        assert_eq!(out.fills.fill_bar_count, 1);
    }

    #[test]
    fn identical_bars_score_full_repetition() {
        // Same kick on beat 1 across 4 bars.
        let notes = (0..4)
            .map(|bar| dn(bar * 3840, 36, 1.0))
            .collect::<Vec<_>>();
        let out = analyze(&notes, 4 * 3840, TimeSignature::COMMON);
        assert_eq!(out.repetition.total_bars, 4);
        assert_eq!(out.repetition.distinct_bars, 1);
        assert!((out.repetition.bar_repetition_score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn different_bars_score_low_repetition() {
        // Kick at different ticks each bar.
        let notes = vec![
            dn(0, 36, 1.0),
            dn(3840 + 240, 36, 1.0),
            dn(2 * 3840 + 480, 36, 1.0),
            dn(3 * 3840 + 720, 36, 1.0),
        ];
        let out = analyze(&notes, 4 * 3840, TimeSignature::COMMON);
        assert_eq!(out.repetition.total_bars, 4);
        assert_eq!(out.repetition.distinct_bars, 4);
        assert!(out.repetition.bar_repetition_score.abs() < 1e-5);
    }

    #[test]
    fn drum_component_gm_mapping() {
        assert_eq!(DrumComponent::from_midi(36), DrumComponent::Kick);
        assert_eq!(DrumComponent::from_midi(38), DrumComponent::Snare);
        assert_eq!(DrumComponent::from_midi(42), DrumComponent::HatClosed);
        assert_eq!(DrumComponent::from_midi(46), DrumComponent::HatOpen);
        assert_eq!(DrumComponent::from_midi(49), DrumComponent::Cymbal);
        assert_eq!(DrumComponent::from_midi(41), DrumComponent::Tom);
        assert_eq!(DrumComponent::from_midi(39), DrumComponent::Clap);
        assert_eq!(DrumComponent::from_midi(60), DrumComponent::OtherPercussion);
    }

    #[test]
    fn out_of_bounds_notes_dropped() {
        // Note past length is silently dropped.
        let notes = vec![dn(0, 36, 1.0), dn(5000, 36, 1.0)];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.total_drum_notes, 1);
    }

    #[test]
    fn track_name_classifier_recognises_kit_pieces() {
        assert_eq!(
            DrumComponent::from_track_name("Kick"),
            Some(DrumComponent::Kick)
        );
        assert_eq!(
            DrumComponent::from_track_name("kick L"),
            Some(DrumComponent::Kick)
        );
        assert_eq!(
            DrumComponent::from_track_name("BD"),
            Some(DrumComponent::Kick)
        );
        assert_eq!(
            DrumComponent::from_track_name("Snare 1"),
            Some(DrumComponent::Snare)
        );
        assert_eq!(
            DrumComponent::from_track_name("SD-top"),
            Some(DrumComponent::Snare)
        );
        assert_eq!(
            DrumComponent::from_track_name("Rim"),
            Some(DrumComponent::Snare)
        );
        assert_eq!(
            DrumComponent::from_track_name("Closed Hat"),
            Some(DrumComponent::HatClosed)
        );
        assert_eq!(
            DrumComponent::from_track_name("HH"),
            Some(DrumComponent::HatClosed)
        );
        assert_eq!(
            DrumComponent::from_track_name("HiHat"),
            Some(DrumComponent::HatClosed)
        );
        assert_eq!(
            DrumComponent::from_track_name("Open Hat"),
            Some(DrumComponent::HatOpen)
        );
        assert_eq!(
            DrumComponent::from_track_name("Hi-Hat Open"),
            Some(DrumComponent::HatOpen)
        );
        assert_eq!(
            DrumComponent::from_track_name("Clap"),
            Some(DrumComponent::Clap)
        );
        assert_eq!(
            DrumComponent::from_track_name("Floor Tom"),
            Some(DrumComponent::Tom)
        );
        assert_eq!(
            DrumComponent::from_track_name("Crash 1"),
            Some(DrumComponent::Cymbal)
        );
        assert_eq!(
            DrumComponent::from_track_name("Ride"),
            Some(DrumComponent::Cymbal)
        );
    }

    #[test]
    fn track_name_classifier_returns_none_for_generic_names() {
        // Generic names should fall through to MIDI mapping.
        assert_eq!(DrumComponent::from_track_name("Drums"), None);
        assert_eq!(DrumComponent::from_track_name("Percussion"), None);
        assert_eq!(DrumComponent::from_track_name("Beat"), None);
        assert_eq!(DrumComponent::from_track_name(""), None);
    }

    #[test]
    fn track_name_classifier_is_word_match_not_substring() {
        // "Kickback" would substring-match "kick" but is not a kick track.
        assert_eq!(DrumComponent::from_track_name("Kickback"), None);
        // "Hatchet" would substring-match "hat" but is not a hat track.
        assert_eq!(DrumComponent::from_track_name("Hatchet"), None);
    }

    #[test]
    fn component_hint_overrides_midi_classification() {
        // MIDI 30 falls outside GM kick range, but a track-name hint says kick.
        let notes = vec![
            dn_hint(0, 30, 1.0, DrumComponent::Kick),
            dn_hint(960, 38, 1.0, DrumComponent::Snare),
            dn_hint(1920, 30, 1.0, DrumComponent::Kick),
            dn_hint(2880, 38, 1.0, DrumComponent::Snare),
        ];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.composition.kick, 2);
        assert_eq!(out.composition.snare, 2);
        assert_eq!(out.composition.other, 0);
        // Backbeat detection sees the snare on beats 2 + 4.
        assert!((out.backbeat.strength - 1.0).abs() < 1e-5);
    }

    #[test]
    fn component_hint_none_falls_back_to_midi() {
        // No hint → existing GM-MIDI behaviour preserved.
        let notes = vec![dn(0, 36, 1.0), dn(960, 38, 1.0)];
        let out = analyze(&notes, 3840, TimeSignature::COMMON);
        assert_eq!(out.composition.kick, 1);
        assert_eq!(out.composition.snare, 1);
    }
}
