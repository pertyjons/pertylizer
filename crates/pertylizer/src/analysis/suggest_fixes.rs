//! Meta-analysis rule engine (Tier-2 #16, Group D).
//!
//! Consumes outputs from the existing analyzers and emits a ranked list of
//! concrete fix suggestions with evidence references. Each rule reads
//! exactly one analyzer's output (or compares two), and runs only when its
//! preconditions are met — e.g. mix-bus rules silently skip in symbolic-
//! only mode. No new measurements are produced here; the rules package
//! findings the caller can already retrieve from the individual analyzer
//! tools.
//!
//! All rules use stable string IDs (`"harmony.no_key_inferred"`, etc.) so a
//! caller can suppress a specific rule in a follow-up call without parsing
//! the human-readable title or detail.

use synth_mcp::types::{
    AnalyzeBassDrumLockResult, AnalyzeDrumGrooveResult, AnalyzeFormMapResult, AnalyzeHarmonyResult,
    AnalyzeHookStrengthResult, AnalyzeMaskingMatrixResult, AnalyzeMixBusResult,
    AnalyzeTensionCurveResult, FixSuggestion,
};

/// Snapshot of every analyzer output the rule engine knows how to consume.
/// All fields are optional — a rule firing only requires the analyzer it
/// needs. The bridge populates whichever ones make sense for the requested
/// scope and category filter.
#[derive(Default)]
pub struct SuggestionInputs<'a> {
    pub harmony: Option<&'a AnalyzeHarmonyResult>,
    pub mix_bus: Option<&'a AnalyzeMixBusResult>,
    pub masking: Option<&'a AnalyzeMaskingMatrixResult>,
    pub drum_groove: Option<&'a AnalyzeDrumGrooveResult>,
    pub bass_drum_lock: Option<&'a AnalyzeBassDrumLockResult>,
    pub form_map: Option<&'a AnalyzeFormMapResult>,
    pub hook: Option<&'a AnalyzeHookStrengthResult>,
    pub tension_curve: Option<&'a AnalyzeTensionCurveResult>,
}

/// Stable category strings used by both [`ALL_CATEGORIES`] and each rule's
/// `FixSuggestion.category`. Centralized so a typo can't drift one rule out
/// of the user-facing filter set.
pub const CAT_HARMONY: &str = "harmony";
pub const CAT_MIX: &str = "mix";
pub const CAT_GROOVE: &str = "groove";
pub const CAT_ARRANGEMENT: &str = "arrangement";
pub const CAT_COMPOSITION: &str = "composition";
pub const CAT_PATCH: &str = "patch";

/// Categories the bridge filter operates on. Stable strings rather than an
/// enum so the bridge can pass user-supplied filters through directly.
pub const ALL_CATEGORIES: &[&str] = &[
    CAT_HARMONY,
    CAT_MIX,
    CAT_GROOVE,
    CAT_ARRANGEMENT,
    CAT_COMPOSITION,
    CAT_PATCH,
];

/// Result of [`suggest`].
pub struct SuggestionOutput {
    pub suggestions: Vec<FixSuggestion>,
    pub rules_clean: Vec<String>,
}

/// Run every rule whose category is enabled and whose input is present.
/// `enabled_categories` is the user filter; an empty list means "all".
#[must_use]
pub fn suggest(
    inputs: &SuggestionInputs<'_>,
    enabled_categories: &[String],
    max_suggestions: u32,
) -> SuggestionOutput {
    let cat_enabled = |c: &str| -> bool {
        enabled_categories.is_empty() || enabled_categories.iter().any(|e| e == c)
    };
    let max = max_suggestions.clamp(1, 50) as usize;

    let mut fired: Vec<FixSuggestion> = Vec::new();
    let mut clean: Vec<String> = Vec::new();

    // ─── Harmony rules ───────────────────────────────────────────────────
    if cat_enabled(CAT_HARMONY)
        && let Some(harmony) = inputs.harmony
    {
        run_rule(
            "harmony.no_key_inferred",
            &mut fired,
            &mut clean,
            harmony_no_key(harmony),
        );
        run_rule(
            "harmony.low_in_key_ratio",
            &mut fired,
            &mut clean,
            harmony_low_in_key(harmony),
        );
        run_rule(
            "harmony.low_stability",
            &mut fired,
            &mut clean,
            harmony_low_stability(harmony),
        );
        run_rule(
            "harmony.no_identified_chords",
            &mut fired,
            &mut clean,
            harmony_no_chords(harmony),
        );
    }

    // ─── Mix rules ───────────────────────────────────────────────────────
    if cat_enabled(CAT_MIX)
        && let Some(mix) = inputs.mix_bus
    {
        run_rule("mix.clipping", &mut fired, &mut clean, mix_clipping(mix));
        run_rule(
            "mix.true_peak_overshoot",
            &mut fired,
            &mut clean,
            mix_true_peak_overshoot(mix),
        );
        run_rule("mix.low_lufs", &mut fired, &mut clean, mix_low_lufs(mix));
        run_rule(
            "mix.poor_mono_compat",
            &mut fired,
            &mut clean,
            mix_poor_mono(mix),
        );
        run_rule(
            "mix.anti_phase",
            &mut fired,
            &mut clean,
            mix_anti_phase(mix),
        );
        run_rule(
            "mix.low_crest_factor",
            &mut fired,
            &mut clean,
            mix_low_crest(mix),
        );
    }

    if cat_enabled(CAT_MIX)
        && let Some(mask) = inputs.masking
    {
        run_rule(
            "mix.heavy_masking_pair",
            &mut fired,
            &mut clean,
            mix_heavy_masking(mask),
        );
    }

    // ─── Groove rules ────────────────────────────────────────────────────
    if cat_enabled(CAT_GROOVE)
        && let Some(groove) = inputs.drum_groove
    {
        run_rule(
            "groove.weak_backbeat",
            &mut fired,
            &mut clean,
            groove_weak_backbeat(groove),
        );
        run_rule(
            "groove.no_fills",
            &mut fired,
            &mut clean,
            groove_no_fills(groove),
        );
        run_rule(
            "groove.identical_bars",
            &mut fired,
            &mut clean,
            groove_identical_bars(groove),
        );
    }
    if cat_enabled(CAT_GROOVE)
        && let Some(lock) = inputs.bass_drum_lock
    {
        run_rule(
            "groove.loose_bass_kick_lock",
            &mut fired,
            &mut clean,
            groove_loose_lock(lock),
        );
    }

    // ─── Arrangement rules ───────────────────────────────────────────────
    if cat_enabled(CAT_ARRANGEMENT)
        && let Some(form) = inputs.form_map
    {
        run_rule(
            "arrangement.single_section",
            &mut fired,
            &mut clean,
            arrangement_single_section(form),
        );
        run_rule(
            "arrangement.too_few_distinct_sections",
            &mut fired,
            &mut clean,
            arrangement_low_distinct(form),
        );
    }
    if cat_enabled(CAT_ARRANGEMENT)
        && let Some(curve) = inputs.tension_curve
    {
        run_rule(
            "arrangement.monotone_tension",
            &mut fired,
            &mut clean,
            arrangement_monotone(curve),
        );
        run_rule(
            "arrangement.peak_too_early",
            &mut fired,
            &mut clean,
            arrangement_peak_too_early(curve),
        );
        run_rule(
            "arrangement.chorus_doesnt_lift",
            &mut fired,
            &mut clean,
            arrangement_chorus_doesnt_lift(curve),
        );
    }

    // ─── Composition rules ───────────────────────────────────────────────
    if cat_enabled(CAT_COMPOSITION)
        && let Some(hook) = inputs.hook
    {
        run_rule(
            "composition.weak_hook",
            &mut fired,
            &mut clean,
            composition_weak_hook(hook),
        );
    }

    // Sort by severity descending, stable for tie-breaking.
    fired.sort_by(|a, b| {
        b.severity
            .partial_cmp(&a.severity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fired.truncate(max);

    SuggestionOutput {
        suggestions: fired,
        rules_clean: clean,
    }
}

fn run_rule(
    id: &str,
    fired: &mut Vec<FixSuggestion>,
    clean: &mut Vec<String>,
    result: Option<FixSuggestion>,
) {
    match result {
        Some(s) => fired.push(s),
        None => clean.push(id.to_string()),
    }
}

// ─── Harmony rules ───────────────────────────────────────────────────────

fn harmony_no_key(h: &AnalyzeHarmonyResult) -> Option<FixSuggestion> {
    if h.inferred_key.is_some() || h.stats.total_notes == 0 {
        return None;
    }
    Some(FixSuggestion {
        id: "harmony.no_key_inferred".to_string(),
        category: CAT_HARMONY.to_string(),
        severity: 0.8,
        title: "No key could be inferred from the melodic content".to_string(),
        detail: "Krumhansl-Schmuckler key inference failed — the duration-weighted pitch-class \
                 histogram does not match any major or minor key strongly enough. Likely cause: \
                 too few notes, atonal material, or chromatic noodling without a tonal centre. \
                 Add a sustained tonic / fifth, or extend the scope, to give the analyzer a \
                 stable target."
            .to_string(),
        evidence: vec![format!(
            "total_notes={} chord_event_count={}",
            h.stats.total_notes, h.stats.chord_event_count
        )],
    })
}

fn harmony_low_in_key(h: &AnalyzeHarmonyResult) -> Option<FixSuggestion> {
    let key = h.inferred_key.as_ref()?;
    if h.in_key_ratio >= 0.7 || h.stats.total_notes < 4 {
        return None;
    }
    let out_pcs: Vec<String> = h
        .out_of_scale_pitch_classes
        .iter()
        .map(|pc| synth_sequencer::NoteName::from_midi(*pc).to_string())
        .collect();
    Some(FixSuggestion {
        id: "harmony.low_in_key_ratio".to_string(),
        category: CAT_HARMONY.to_string(),
        severity: (1.0 - h.in_key_ratio).clamp(0.3, 0.9),
        title: format!(
            "Only {:.0}% of melodic weight stays in {}",
            h.in_key_ratio * 100.0,
            key.label
        ),
        detail: format!(
            "The duration-weighted in-key ratio is below the 0.7 threshold. \
             Out-of-scale pitch classes ({}) drive the dissonance. Either \
             commit to the inferred key by snapping stragglers in or pick a \
             different scale that covers them.",
            if out_pcs.is_empty() {
                "—".to_string()
            } else {
                out_pcs.join(", ")
            }
        ),
        evidence: vec![
            format!("in_key_ratio={:.3}", h.in_key_ratio),
            format!("out_of_scale_pcs={}", h.out_of_scale_pitch_classes.len()),
            format!("inferred_key={}", key.label),
        ],
    })
}

fn harmony_low_stability(h: &AnalyzeHarmonyResult) -> Option<FixSuggestion> {
    if h.harmonic_stability_score >= 0.5 || h.stats.total_notes < 4 {
        return None;
    }
    Some(FixSuggestion {
        id: "harmony.low_stability".to_string(),
        category: CAT_HARMONY.to_string(),
        severity: (0.5 - h.harmonic_stability_score) + 0.3,
        title: format!(
            "Harmonic stability is low ({:.2})",
            h.harmonic_stability_score
        ),
        detail: "harmonic_stability_score blends in-key ratio, chord-identification ratio, and \
                 key-correlation strength. A score below 0.5 means the harmony is hard to pin \
                 down — either the chord shapes are ambiguous (try cleaner triads / 7ths) or \
                 the key correlation is weak."
            .to_string(),
        evidence: vec![
            format!("stability={:.3}", h.harmonic_stability_score),
            format!(
                "identified_chord_count={}/{}",
                h.stats.identified_chord_count, h.stats.chord_event_count
            ),
        ],
    })
}

fn harmony_no_chords(h: &AnalyzeHarmonyResult) -> Option<FixSuggestion> {
    if h.stats.identified_chord_count > 0 || h.stats.chord_event_count == 0 {
        return None;
    }
    Some(FixSuggestion {
        id: "harmony.no_identified_chords".to_string(),
        category: CAT_HARMONY.to_string(),
        severity: 0.6,
        title: format!(
            "{} chord windows analyzed but zero matched a chord template",
            h.stats.chord_event_count
        ),
        detail: "Every window's pitch set was treated as 'no chord'. Likely cause: monophonic \
                 lines, drone-only material, or unconventional voicings. If chord progressions \
                 are intended, ensure at least three distinct pitches sound together within \
                 each grouping window."
            .to_string(),
        evidence: vec![format!(
            "chord_event_count={} identified=0",
            h.stats.chord_event_count
        )],
    })
}

// ─── Mix rules ───────────────────────────────────────────────────────────

fn mix_clipping(m: &AnalyzeMixBusResult) -> Option<FixSuggestion> {
    if m.metrics.clipped_samples == 0 {
        return None;
    }
    Some(FixSuggestion {
        id: "mix.clipping".to_string(),
        category: CAT_MIX.to_string(),
        severity: 0.95,
        title: format!(
            "Mix bus clips {} samples (peak {:.2} dBFS)",
            m.metrics.clipped_samples, m.metrics.peak_dbfs
        ),
        detail: "The master bus reached the ±0.999 ceiling — any downstream int conversion \
                 will wrap. Drop master gain by at least the peak's margin or attenuate the \
                 dominant track."
            .to_string(),
        evidence: vec![
            format!("clipped_samples={}", m.metrics.clipped_samples),
            format!("peak_dbfs={:.2}", m.metrics.peak_dbfs),
        ],
    })
}

fn mix_true_peak_overshoot(m: &AnalyzeMixBusResult) -> Option<FixSuggestion> {
    if m.metrics.clipped_samples > 0 {
        return None;
    }
    if m.metrics.true_peak_dbtp <= -0.5 {
        return None;
    }
    Some(FixSuggestion {
        id: "mix.true_peak_overshoot".to_string(),
        category: CAT_MIX.to_string(),
        severity: 0.7,
        title: format!(
            "True peak is {:.2} dBTP (sample peak {:.2} dBFS)",
            m.metrics.true_peak_dbtp, m.metrics.peak_dbfs
        ),
        detail: "Inter-sample peaks above -0.5 dBTP cause overshoot after DA conversion or \
                 lossy encoding even when the sample-grid peak looks safe. Either add a true-peak \
                 limiter or pull master gain back so true_peak_dbtp is under -1 dBTP."
            .to_string(),
        evidence: vec![
            format!("true_peak_dbtp={:.2}", m.metrics.true_peak_dbtp),
            format!("peak_dbfs={:.2}", m.metrics.peak_dbfs),
        ],
    })
}

fn mix_low_lufs(m: &AnalyzeMixBusResult) -> Option<FixSuggestion> {
    if m.metrics.lufs_integrated > -23.0 || m.metrics.lufs_integrated <= -150.0 {
        return None;
    }
    Some(FixSuggestion {
        id: "mix.low_lufs".to_string(),
        category: CAT_MIX.to_string(),
        severity: ((-23.0 - m.metrics.lufs_integrated) / 20.0).clamp(0.3, 0.8),
        title: format!("Loudness is low at {:.1} LUFS-I", m.metrics.lufs_integrated),
        detail: "Integrated loudness sits below -23 LUFS — common streaming targets are \
                 -14 (Spotify) to -16 (Apple) LUFS. Boost master / instrument gain or add bus \
                 compression to lift average loudness; the headroom in `peak_dbfs` will tell \
                 you how much room you have before clipping."
            .to_string(),
        evidence: vec![
            format!("lufs_integrated={:.2}", m.metrics.lufs_integrated),
            format!("peak_dbfs={:.2}", m.metrics.peak_dbfs),
        ],
    })
}

fn mix_poor_mono(m: &AnalyzeMixBusResult) -> Option<FixSuggestion> {
    if m.metrics.mono_compat >= 0.7 {
        return None;
    }
    Some(FixSuggestion {
        id: "mix.poor_mono_compat".to_string(),
        category: CAT_MIX.to_string(),
        severity: (0.7 - m.metrics.mono_compat) + 0.3,
        title: format!("Mono compatibility is poor ({:.2})", m.metrics.mono_compat),
        detail: "Significant energy will cancel when the mix sums to mono — a hard fail for \
                 club/PA playback. Pull back wide stereo effects (chorus, ping-pong delay, \
                 Haas widening) on bass-range elements and check for inverted-phase samples."
            .to_string(),
        evidence: vec![
            format!("mono_compat={:.3}", m.metrics.mono_compat),
            format!("stereo_correlation={:.3}", m.metrics.stereo_correlation),
        ],
    })
}

fn mix_anti_phase(m: &AnalyzeMixBusResult) -> Option<FixSuggestion> {
    if m.metrics.stereo_correlation >= -0.1 {
        return None;
    }
    Some(FixSuggestion {
        id: "mix.anti_phase".to_string(),
        category: CAT_MIX.to_string(),
        severity: 0.85,
        title: format!(
            "L/R correlation is {:.2} — the master bus is largely anti-phase",
            m.metrics.stereo_correlation
        ),
        detail: "Negative L/R correlation is the hallmark of inverted-phase content. The mix \
                 will hollow out the moment it hits a mono fold-down. Check for accidental \
                 invert plugins, mis-paired stereo channels, or extreme Haas widening."
            .to_string(),
        evidence: vec![format!(
            "stereo_correlation={:.3}",
            m.metrics.stereo_correlation
        )],
    })
}

fn mix_low_crest(m: &AnalyzeMixBusResult) -> Option<FixSuggestion> {
    if m.metrics.crest_factor_db >= 6.0 {
        return None;
    }
    Some(FixSuggestion {
        id: "mix.low_crest_factor".to_string(),
        category: CAT_MIX.to_string(),
        severity: 0.55,
        title: format!(
            "Crest factor is only {:.1} dB — mix is over-compressed",
            m.metrics.crest_factor_db
        ),
        detail: "A peak-to-RMS distance under 6 dB usually signals brick-wall limiting or \
                 heavy bus compression. The mix will sound flat and lose transient impact. \
                 Loosen the compressor or pull the limiter ceiling back."
            .to_string(),
        evidence: vec![
            format!("crest_factor_db={:.2}", m.metrics.crest_factor_db),
            format!("peak_dbfs={:.2}", m.metrics.peak_dbfs),
            format!("rms_dbfs={:.2}", m.metrics.rms_dbfs),
        ],
    })
}

fn mix_heavy_masking(mask: &AnalyzeMaskingMatrixResult) -> Option<FixSuggestion> {
    let pair = mask.pairs.iter().max_by(|a, b| {
        a.conflict_score
            .partial_cmp(&b.conflict_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    if pair.conflict_score < 0.4 {
        return None;
    }
    let hint = pair.hint.clone().unwrap_or_else(|| {
        format!(
            "{}({}) and {}({}) share significant energy",
            pair.track_a_name, pair.track_a_id, pair.track_b_name, pair.track_b_id
        )
    });
    Some(FixSuggestion {
        id: "mix.heavy_masking_pair".to_string(),
        category: CAT_MIX.to_string(),
        severity: pair.conflict_score.min(0.9),
        title: format!(
            "Heavy masking between {} and {} (conflict {:.2})",
            pair.track_a_name, pair.track_b_name, pair.conflict_score,
        ),
        detail: format!(
            "{}. Carve a complementary EQ notch on whichever track should sit further back, \
             or change one track's octave / register so they don't fight the same band.",
            hint
        ),
        evidence: vec![
            format!("conflict_score={:.3}", pair.conflict_score),
            format!(
                "tracks={}({}),{}({})",
                pair.track_a_name, pair.track_a_id, pair.track_b_name, pair.track_b_id
            ),
        ],
    })
}

// ─── Groove rules ────────────────────────────────────────────────────────

fn groove_weak_backbeat(g: &AnalyzeDrumGrooveResult) -> Option<FixSuggestion> {
    if g.backbeat.expected_backbeats == 0 || g.backbeat.strength >= 0.6 {
        return None;
    }
    Some(FixSuggestion {
        id: "groove.weak_backbeat".to_string(),
        category: CAT_GROOVE.to_string(),
        severity: ((0.6 - g.backbeat.strength) + 0.4).clamp(0.3, 0.85),
        title: format!(
            "Backbeat fills only {:.0}% of beat-2/4 positions",
            g.backbeat.strength * 100.0,
        ),
        detail: "Snare hits on beats 2 and 4 are the load-bearing pulse for most contemporary \
                 grooves. A backbeat strength below 0.6 means the snare is either missing or \
                 frequently lands off the grid. Either re-quantize the snare or move displaced \
                 hits back onto the backbeat slots."
            .to_string(),
        evidence: vec![
            format!("backbeat_strength={:.2}", g.backbeat.strength),
            format!(
                "matched/expected={}/{}",
                g.backbeat.matched_backbeats, g.backbeat.expected_backbeats
            ),
        ],
    })
}

fn groove_no_fills(g: &AnalyzeDrumGrooveResult) -> Option<FixSuggestion> {
    if g.fills.fill_bar_count > 0 || g.length_bars < 8.0 || g.repetition.bar_repetition_score < 0.5
    {
        return None;
    }
    Some(FixSuggestion {
        id: "groove.no_fills".to_string(),
        category: CAT_GROOVE.to_string(),
        severity: 0.45,
        title: format!(
            "{} bars of drums with no fill candidates",
            g.repetition.total_bars
        ),
        detail: "Drum density never exceeds the fill threshold (1.6× the mean). For sections \
                 longer than 8 bars, a fill every 4 or 8 bars is what marks phrase boundaries. \
                 Add a busy bar (16th-note tom run, snare roll, or hat opening) at the section \
                 transition."
            .to_string(),
        evidence: vec![
            format!("fill_bar_count={}", g.fills.fill_bar_count),
            format!("total_bars={}", g.repetition.total_bars),
            format!(
                "bar_repetition_score={:.2}",
                g.repetition.bar_repetition_score
            ),
        ],
    })
}

fn groove_identical_bars(g: &AnalyzeDrumGrooveResult) -> Option<FixSuggestion> {
    if g.repetition.total_bars < 4 || g.repetition.distinct_bars > 1 {
        return None;
    }
    Some(FixSuggestion {
        id: "groove.identical_bars".to_string(),
        category: CAT_GROOVE.to_string(),
        severity: 0.4,
        title: format!("All {} drum bars are identical", g.repetition.total_bars),
        detail: "Every drum bar carries exactly the same hits — the groove never breathes. \
                 Vary at least one element (hat opening on bar 4, snare flam, ghost notes) so \
                 the listener tracks musical time rather than treating the drums as a loop."
            .to_string(),
        evidence: vec![format!("distinct_bars=1/{}", g.repetition.total_bars)],
    })
}

fn groove_loose_lock(lock: &AnalyzeBassDrumLockResult) -> Option<FixSuggestion> {
    if lock.alignment.lock_score >= 0.5
        || lock.alignment.matched_onsets + lock.alignment.kick_only == 0
    {
        return None;
    }
    Some(FixSuggestion {
        id: "groove.loose_bass_kick_lock".to_string(),
        category: CAT_GROOVE.to_string(),
        severity: ((0.5 - lock.alignment.lock_score) + 0.3).clamp(0.3, 0.75),
        title: format!(
            "Bass aligns with only {:.0}% of kick onsets",
            lock.alignment.lock_score * 100.0,
        ),
        detail: "Lock score below 0.5 means the bass and kick are out of register-level \
                 sync — the low-end groove will feel detached. Either move bass onsets to the \
                 nearest kick, or remove kicks that lack bass support so the surviving kicks \
                 carry the groove on their own."
            .to_string(),
        evidence: vec![
            format!("lock_score={:.2}", lock.alignment.lock_score),
            format!(
                "matched={} kick_only={} bass_only={}",
                lock.alignment.matched_onsets, lock.alignment.kick_only, lock.alignment.bass_only,
            ),
        ],
    })
}

// ─── Arrangement rules ───────────────────────────────────────────────────

fn arrangement_single_section(f: &AnalyzeFormMapResult) -> Option<FixSuggestion> {
    if f.length_bars < 8 || f.distinct_section_count > 1 {
        return None;
    }
    Some(FixSuggestion {
        id: "arrangement.single_section".to_string(),
        category: CAT_ARRANGEMENT.to_string(),
        severity: 0.55,
        title: format!(
            "{} bars collapsed into a single section ({})",
            f.length_bars, f.form_string
        ),
        detail: "Every bar in the analyzed scope clusters under one label — there is no \
                 contrast in pitch-class or density. Introduce at least one B-section: change \
                 the chord centre, swap an instrument, or shift the active register so \
                 listeners get a release."
            .to_string(),
        evidence: vec![
            format!("form_string={}", f.form_string),
            format!("length_bars={}", f.length_bars),
        ],
    })
}

fn arrangement_low_distinct(f: &AnalyzeFormMapResult) -> Option<FixSuggestion> {
    if f.distinct_section_count > 1 || f.length_bars < 16 {
        return None;
    }
    // Same trigger as single_section but at longer length — different
    // severity & wording. Defer to single_section when both fire.
    Some(FixSuggestion {
        id: "arrangement.too_few_distinct_sections".to_string(),
        category: CAT_ARRANGEMENT.to_string(),
        severity: 0.5,
        title: format!("{} bars but only one distinct section label", f.length_bars),
        detail: "A song this long usually has 2-4 distinct sections. Pull the strongest \
                 contrasting bar out into its own pattern, or split the arrangement at a \
                 natural breath point."
            .to_string(),
        evidence: vec![format!(
            "distinct_section_count=1 length_bars={}",
            f.length_bars
        )],
    })
}

fn arrangement_monotone(c: &AnalyzeTensionCurveResult) -> Option<FixSuggestion> {
    if c.summary.std_dev >= 0.05 {
        return None;
    }
    Some(FixSuggestion {
        id: "arrangement.monotone_tension".to_string(),
        category: CAT_ARRANGEMENT.to_string(),
        severity: 0.65,
        title: format!(
            "Composite tension is flat across the scope (std-dev {:.3})",
            c.summary.std_dev
        ),
        detail: "Tension never moves more than ±0.05 from the mean — listeners get no \
                 emotional arc. Drop the density / register / loudness in one section and \
                 raise it in another so the curve has a visible peak."
            .to_string(),
        evidence: vec![format!(
            "std_dev={:.3} mean={:.3}",
            c.summary.std_dev, c.summary.mean
        )],
    })
}

fn arrangement_peak_too_early(c: &AnalyzeTensionCurveResult) -> Option<FixSuggestion> {
    let peak_bar = c.summary.peak_bar?;
    let len = c.length_bars;
    if len < 8 || peak_bar > len / 3 {
        return None;
    }
    Some(FixSuggestion {
        id: "arrangement.peak_too_early".to_string(),
        category: CAT_ARRANGEMENT.to_string(),
        severity: 0.55,
        title: format!(
            "Tension peaks at bar {} ({:.2}) — first third of the scope",
            peak_bar, c.summary.peak_value
        ),
        detail: "The most intense bar arrives before the listener has had time to settle in. \
                 Move the climax later (typically two thirds in) or extend the intro so the \
                 peak doesn't blow its load early."
            .to_string(),
        evidence: vec![
            format!("peak_bar={} of {}", peak_bar, len),
            format!("peak_value={:.2}", c.summary.peak_value),
        ],
    })
}

fn arrangement_chorus_doesnt_lift(c: &AnalyzeTensionCurveResult) -> Option<FixSuggestion> {
    // Mirror the tension_curve module's warning into a suggestion when it
    // fired. Severity gets a moderate floor so it stays visible.
    let hit = c
        .warnings
        .iter()
        .find(|w| w.contains("returns at lower energy"))?
        .clone();
    Some(FixSuggestion {
        id: "arrangement.chorus_doesnt_lift".to_string(),
        category: CAT_ARRANGEMENT.to_string(),
        severity: 0.6,
        title: "A reprise of an earlier section comes back at lower energy".to_string(),
        detail: format!(
            "{} — when a section returns, it should at least match the previous appearance's \
             energy. Either thicken the arrangement on the reprise (octave double, layer pad, \
             raise drum density) or accept the drop and label it as a breakdown rather than a \
             chorus.",
            hit
        ),
        evidence: vec![hit],
    })
}

// ─── Composition rules ───────────────────────────────────────────────────

fn composition_weak_hook(h: &AnalyzeHookStrengthResult) -> Option<FixSuggestion> {
    if h.hook_score >= 0.4 || h.total_notes < 8 {
        return None;
    }
    Some(FixSuggestion {
        id: "composition.weak_hook".to_string(),
        category: CAT_COMPOSITION.to_string(),
        severity: ((0.4 - h.hook_score) + 0.3).clamp(0.3, 0.7),
        title: format!(
            "Hook strength is only {:.2} — no memorable phrase emerges",
            h.hook_score
        ),
        detail: "find_motifs found no pitch-interval phrase that recurs enough to dominate \
                 the melody. Pick one 4-8 note motif and restate it at least three times \
                 (transposed copies count) — the listener needs a shape to hang onto."
            .to_string(),
        evidence: vec![
            format!("hook_score={:.2}", h.hook_score),
            format!("coverage_ratio={:.2}", h.coverage_ratio),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_mcp::types::{
        AnalyzeEnergyBands, HarmonyKeyEstimate, HarmonyScope, HarmonyStats, MixBusMetrics,
    };

    fn empty_mix() -> AnalyzeMixBusResult {
        AnalyzeMixBusResult {
            start_bar: 1,
            start_beat: 1,
            end_bar: 1,
            end_beat: 1,
            start_tick: 0,
            end_tick: 0,
            metrics: MixBusMetrics {
                sample_rate: 44100,
                duration_seconds: 1.0,
                peak: 0.5,
                peak_dbfs: -6.0,
                peak_left: 0.5,
                peak_right: 0.5,
                true_peak: 0.5,
                true_peak_dbtp: -6.0,
                rms: 0.1,
                rms_dbfs: -20.0,
                crest_factor_db: 14.0,
                lufs_integrated: -16.0,
                lufs_momentary_max: -16.0,
                lufs_short_term_max: -16.0,
                energy_bands: AnalyzeEnergyBands {
                    sub: 0.1,
                    low: 0.1,
                    mid: 0.1,
                    high: 0.1,
                },
                stereo_correlation: 0.9,
                mid_rms: 0.1,
                side_rms: 0.02,
                stereo_width: 0.2,
                mono_compat: 0.95,
                clipped_samples: 0,
            },
            per_track: Vec::new(),
            signal_chain: String::new(),
            warnings: Vec::new(),
        }
    }

    fn empty_harmony() -> AnalyzeHarmonyResult {
        AnalyzeHarmonyResult {
            scope: HarmonyScope::Pattern { pattern_id: 0 },
            chords: Vec::new(),
            inferred_key: Some(HarmonyKeyEstimate {
                tonic: 0,
                tonic_name: "C".to_string(),
                mode: "major".to_string(),
                label: "C major".to_string(),
                correlation: 0.9,
            }),
            key_candidates: Vec::new(),
            pitch_class_histogram: [0.0; 12],
            in_key_ratio: 0.9,
            out_of_scale_pitch_classes: Vec::new(),
            harmonic_stability_score: 0.8,
            stats: HarmonyStats {
                total_notes: 12,
                chord_event_count: 4,
                distinct_chord_count: 3,
                identified_chord_count: 4,
                pitch_range_low: 60,
                pitch_range_high: 72,
                avg_polyphony: 3.0,
                grouping_ticks: 960,
            },
            warnings: Vec::new(),
        }
    }

    #[test]
    fn no_inputs_no_suggestions() {
        let out = suggest(&SuggestionInputs::default(), &[], 15);
        assert!(out.suggestions.is_empty());
        assert!(out.rules_clean.is_empty());
    }

    #[test]
    fn clipping_rule_fires_high() {
        let mut mix = empty_mix();
        mix.metrics.clipped_samples = 42;
        mix.metrics.peak_dbfs = -0.05;
        let inputs = SuggestionInputs {
            mix_bus: Some(&mix),
            ..Default::default()
        };
        let out = suggest(&inputs, &[], 15);
        assert!(out.suggestions.iter().any(|s| s.id == "mix.clipping"));
        let clip = out
            .suggestions
            .iter()
            .find(|s| s.id == "mix.clipping")
            .unwrap();
        assert!(clip.severity > 0.9);
    }

    #[test]
    fn category_filter_drops_unrelated_rules() {
        let mut mix = empty_mix();
        mix.metrics.clipped_samples = 1;
        let mut harmony = empty_harmony();
        harmony.in_key_ratio = 0.3;
        harmony.out_of_scale_pitch_classes = vec![1, 3, 6];
        let inputs = SuggestionInputs {
            mix_bus: Some(&mix),
            harmony: Some(&harmony),
            ..Default::default()
        };
        let out = suggest(&inputs, &["mix".to_string()], 15);
        for s in &out.suggestions {
            assert_eq!(s.category, "mix", "category filter leaked: {s:?}");
        }
    }

    #[test]
    fn clean_rules_listed_when_threshold_unmet() {
        let mix = empty_mix(); // clean values
        let inputs = SuggestionInputs {
            mix_bus: Some(&mix),
            ..Default::default()
        };
        let out = suggest(&inputs, &[], 15);
        assert!(out.suggestions.is_empty());
        assert!(out.rules_clean.iter().any(|r| r == "mix.clipping"));
    }

    #[test]
    fn severity_sort_descending() {
        let mut mix = empty_mix();
        mix.metrics.clipped_samples = 5;
        mix.metrics.mono_compat = 0.4;
        let inputs = SuggestionInputs {
            mix_bus: Some(&mix),
            ..Default::default()
        };
        let out = suggest(&inputs, &[], 15);
        assert!(out.suggestions.len() >= 2);
        for w in out.suggestions.windows(2) {
            assert!(w[0].severity >= w[1].severity);
        }
    }

    #[test]
    fn truncates_to_max() {
        // Engineer many failing rules so we exceed max.
        let mut mix = empty_mix();
        mix.metrics.clipped_samples = 5;
        mix.metrics.mono_compat = 0.4;
        mix.metrics.stereo_correlation = -0.5;
        mix.metrics.crest_factor_db = 3.0;
        mix.metrics.lufs_integrated = -30.0;
        let mut harmony = empty_harmony();
        harmony.in_key_ratio = 0.3;
        harmony.harmonic_stability_score = 0.2;
        harmony.out_of_scale_pitch_classes = vec![1, 3];
        let inputs = SuggestionInputs {
            mix_bus: Some(&mix),
            harmony: Some(&harmony),
            ..Default::default()
        };
        let out = suggest(&inputs, &[], 2);
        assert_eq!(out.suggestions.len(), 2);
    }
}
