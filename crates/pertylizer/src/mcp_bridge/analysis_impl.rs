use super::*;
use synth_core::Milliseconds;

#[must_use]
#[derive(Debug, Clone, Copy)]
struct AnalysisDuration(u32);

impl AnalysisDuration {
    const fn from_millis(milliseconds: u32) -> Self {
        Self(milliseconds)
    }

    const fn as_millis(self) -> u32 {
        self.0
    }
}

#[must_use]
#[derive(Debug, Clone, Copy)]
pub struct NoteAnalysisQuery {
    instrument_id: InstrumentId,
    note: MidiNote,
    velocity: Velocity,
    duration: AnalysisDuration,
    tail: AnalysisDuration,
    expected_note: Option<MidiNote>,
    envelope_window: Option<Milliseconds>,
}

impl NoteAnalysisQuery {
    pub fn from_wire(
        instrument_id: InstrumentId,
        note: MidiNote,
        velocity: u8,
        duration_ms: u32,
        tail_ms: u32,
        expected_note: Option<u8>,
        envelope_window_ms: Option<f32>,
    ) -> Self {
        Self {
            instrument_id,
            note,
            velocity: Velocity::from_midi(velocity),
            duration: AnalysisDuration::from_millis(duration_ms),
            tail: AnalysisDuration::from_millis(tail_ms),
            expected_note: expected_note.map(MidiNote::new),
            envelope_window: envelope_window_ms.map(Milliseconds::new),
        }
    }
}

/// Renders one note and derives the requested audio metrics.
///
/// Keeping the analysis next to the offline renderer makes its assumptions
/// explicit and keeps the public MCP adapter focused on request translation.
pub(super) fn analyze_rendered_note(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    query: NoteAnalysisQuery,
) -> Result<synth_mcp::types::AnalyzeNoteResult, McpBridgeError> {
    let rendered = crate::audio::preview::render_note_to_buffer(
        session,
        sample_library,
        query.instrument_id,
        query.note,
        query.velocity,
        query.duration.as_millis(),
        query.tail.as_millis(),
    )?;

    Ok(analyze_rendered_buffer_with_window(
        &rendered,
        query.note,
        query.velocity.to_midi(),
        query.duration.as_millis(),
        query.expected_note.map(MidiNote::as_u8),
        resolve_envelope_window_ms(query.envelope_window.map(Milliseconds::as_f32)),
    ))
}

#[must_use]
#[derive(Debug, Clone, Copy)]
struct SweepNoteQuery {
    note: MidiNote,
    velocity: Velocity,
    duration: AnalysisDuration,
    tail: AnalysisDuration,
    expected_note: MidiNote,
}

/// Like [`analyze_rendered_note`] but renders on a reused
/// [`OfflineNoteSession`], so a sweep amortizes the engine + patch-load setup
/// across all its steps instead of building one engine per note/velocity.
fn analyze_rendered_note_in_session(
    sess: &mut crate::audio::preview::OfflineNoteSession,
    query: SweepNoteQuery,
) -> Result<synth_mcp::types::AnalyzeNoteResult, McpBridgeError> {
    let rendered = sess.render(
        query.note,
        query.velocity,
        query.duration.as_millis(),
        query.tail.as_millis(),
    )?;

    Ok(analyze_rendered_buffer(
        &rendered,
        query.note,
        query.velocity.to_midi(),
        query.duration.as_millis(),
        Some(query.expected_note.as_u8()),
    ))
}

/// Default note duration for sweep tools. Long enough for the envelope to
/// reach sustain on typical patches; short enough that 60-note sweeps don't
/// take minutes.
const SWEEP_DEFAULT_DURATION_MS: u32 = 400;
/// Default release tail for sweep tools.
const SWEEP_DEFAULT_TAIL_MS: u32 = 200;
const SWEEP_DEFAULT_STEP_SEMITONES: u8 = 12;
const SWEEP_DEFAULT_VELOCITY: u8 = 100;
const SWEEP_DEFAULT_VELOCITY_LOW: u8 = 1;
const SWEEP_DEFAULT_VELOCITY_HIGH: u8 = 127;
const SWEEP_DEFAULT_VELOCITY_STEP: u8 = 16;
const SWEEP_NYQUIST_HZ: f32 = crate::audio::preview::PREVIEW_SAMPLE_RATE as f32 / 2.0;

#[must_use]
#[derive(Debug, Clone, Copy)]
struct SweepStep(u8);

impl SweepStep {
    const fn new(value: u8) -> Self {
        Self(if value == 0 { 1 } else { value })
    }

    const fn as_u8(self) -> u8 {
        self.0
    }
}

#[must_use]
#[derive(Debug, Clone, Copy)]
pub struct InstrumentRangeQuery {
    instrument_id: InstrumentId,
    low_note: MidiNote,
    high_note: MidiNote,
    step: SweepStep,
    velocity: Velocity,
    duration: AnalysisDuration,
    tail: AnalysisDuration,
}

impl InstrumentRangeQuery {
    pub fn from_wire(
        instrument_id: InstrumentId,
        low_note: u8,
        high_note: u8,
        step_semitones: Option<u8>,
        velocity: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<Self, McpBridgeError> {
        if low_note > high_note {
            return Err(McpBridgeError::Other(format!(
                "low_note ({low_note}) must be <= high_note ({high_note})"
            )));
        }
        Ok(Self {
            instrument_id,
            low_note: MidiNote::new(low_note),
            high_note: MidiNote::new(high_note),
            step: SweepStep::new(step_semitones.unwrap_or(SWEEP_DEFAULT_STEP_SEMITONES)),
            velocity: Velocity::from_midi(velocity.unwrap_or(SWEEP_DEFAULT_VELOCITY)),
            duration: AnalysisDuration::from_millis(
                duration_ms.unwrap_or(SWEEP_DEFAULT_DURATION_MS),
            ),
            tail: AnalysisDuration::from_millis(tail_ms.unwrap_or(SWEEP_DEFAULT_TAIL_MS)),
        })
    }
}

#[must_use]
#[derive(Debug, Clone, Copy)]
pub struct VelocityResponseQuery {
    instrument_id: InstrumentId,
    note: MidiNote,
    low: Velocity,
    high: Velocity,
    step: SweepStep,
    duration: AnalysisDuration,
    tail: AnalysisDuration,
}

impl VelocityResponseQuery {
    pub fn from_wire(
        instrument_id: InstrumentId,
        note: MidiNote,
        velocity_low: Option<u8>,
        velocity_high: Option<u8>,
        velocity_step: Option<u8>,
        duration_ms: Option<u32>,
        tail_ms: Option<u32>,
    ) -> Result<Self, McpBridgeError> {
        let velocity_low = velocity_low.unwrap_or(SWEEP_DEFAULT_VELOCITY_LOW).max(1);
        let velocity_high = velocity_high
            .unwrap_or(SWEEP_DEFAULT_VELOCITY_HIGH)
            .min(127);
        if velocity_low > velocity_high {
            return Err(McpBridgeError::Other(format!(
                "velocity_low ({velocity_low}) must be <= velocity_high ({velocity_high})"
            )));
        }
        Ok(Self {
            instrument_id,
            note,
            low: Velocity::from_midi(velocity_low),
            high: Velocity::from_midi(velocity_high),
            step: SweepStep::new(velocity_step.unwrap_or(SWEEP_DEFAULT_VELOCITY_STEP)),
            duration: AnalysisDuration::from_millis(
                duration_ms.unwrap_or(SWEEP_DEFAULT_DURATION_MS),
            ),
            tail: AnalysisDuration::from_millis(tail_ms.unwrap_or(SWEEP_DEFAULT_TAIL_MS)),
        })
    }
}

/// Walk `lo..=hi` in `step` increments, always including `hi` as the final
/// value. Calls `on_step` per value; pushes a `"{label} {val}: render failed: {e}"`
/// warning when `on_step` errors and continues the sweep.
fn sweep_range<T>(
    lo: u8,
    hi: u8,
    step: u8,
    label: &str,
    warnings: &mut Vec<String>,
    mut on_step: impl FnMut(u8) -> Result<T, McpBridgeError>,
) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    let mut val = lo;
    loop {
        match on_step(val) {
            Ok(t) => out.push(t),
            Err(e) => warnings.push(format!("{label} {val}: render failed: {e}")),
        }
        if val == hi {
            break;
        }
        val = val.saturating_add(step).min(hi);
    }
    out
}

/// Sweep an instrument across a MIDI note range. One offline render per step,
/// reuses the `analyze_note` path; cross-step issues are derived in
/// `analysis::patch_sweep`.
#[doc(hidden)]
pub fn analyze_instrument_range_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    query: InstrumentRangeQuery,
) -> Result<synth_mcp::types::AnalyzeInstrumentRangeResult, McpBridgeError> {
    use crate::analysis::patch_sweep::{range_issues_from_steps, range_step_from_analysis};

    let low_note = query.low_note.as_u8();
    let high_note = query.high_note.as_u8();
    let step = query.step.as_u8();

    let mut warnings: Vec<String> = Vec::new();
    // Build the offline engine + load the patch once, then reuse it for every
    // step instead of spinning up a fresh engine per note.
    let (mut sess, setup_warnings) = crate::audio::preview::OfflineNoteSession::new(
        session,
        sample_library,
        query.instrument_id,
    )?;
    warnings.extend(setup_warnings);
    let steps_out = sweep_range(low_note, high_note, step, "note", &mut warnings, |note| {
        let note = MidiNote::new(note);
        let result = analyze_rendered_note_in_session(
            &mut sess,
            SweepNoteQuery {
                note,
                velocity: query.velocity,
                duration: query.duration,
                tail: query.tail,
                expected_note: note,
            },
        )?;
        Ok(range_step_from_analysis(
            note.as_u8(),
            &result,
            SWEEP_NYQUIST_HZ,
        ))
    });

    let issues = range_issues_from_steps(&steps_out);
    Ok(synth_mcp::types::AnalyzeInstrumentRangeResult {
        instrument_id: query.instrument_id,
        velocity: query.velocity.to_midi(),
        low_note,
        high_note,
        step_semitones: step,
        duration_ms: query.duration.as_millis(),
        tail_ms: query.tail.as_millis(),
        steps: steps_out,
        issues,
        warnings,
    })
}

/// Hold one note and sweep velocity. Same render path as
/// `analyze_instrument_range`, but the note is fixed and velocity walks the
/// range.
#[doc(hidden)]
pub fn analyze_velocity_response_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    query: VelocityResponseQuery,
) -> Result<synth_mcp::types::AnalyzeVelocityResponseResult, McpBridgeError> {
    use crate::analysis::patch_sweep::{velocity_issues_from_steps, velocity_step_from_analysis};

    let velocity_low = query.low.to_midi();
    let velocity_high = query.high.to_midi();
    let velocity_step = query.step.as_u8();

    let mut warnings: Vec<String> = Vec::new();
    // Build the offline engine + load the patch once, then reuse it for every
    // velocity step instead of spinning up a fresh engine per step.
    let (mut sess, setup_warnings) = crate::audio::preview::OfflineNoteSession::new(
        session,
        sample_library,
        query.instrument_id,
    )?;
    warnings.extend(setup_warnings);
    let steps_out = sweep_range(
        velocity_low,
        velocity_high,
        velocity_step,
        "velocity",
        &mut warnings,
        |velocity| {
            let result = analyze_rendered_note_in_session(
                &mut sess,
                SweepNoteQuery {
                    note: query.note,
                    velocity: Velocity::from_midi(velocity),
                    duration: query.duration,
                    tail: query.tail,
                    expected_note: query.note,
                },
            )?;
            Ok(velocity_step_from_analysis(velocity, &result))
        },
    );

    let issues = velocity_issues_from_steps(&steps_out);
    Ok(synth_mcp::types::AnalyzeVelocityResponseResult {
        instrument_id: query.instrument_id,
        note: query.note,
        velocity_low,
        velocity_high,
        velocity_step,
        duration_ms: query.duration.as_millis(),
        tail_ms: query.tail.as_millis(),
        steps: steps_out,
        issues,
        warnings,
    })
}

/// Default chord-detection window for pattern-scope analysis: one quarter
/// note at 960 PPQN. Patterns are short enough that fine resolution keeps
/// the output compact.
const DEFAULT_PATTERN_GROUPING_TICKS: u32 = 960;

/// Default chord-detection window for arrangement-scope analysis: one bar
/// (assumed 4/4). Arrangements span many bars and a per-quarter resolution
/// blows past the MCP response-size limit; per-bar resolution keeps the
/// chord-event list readable. Callers can override with a smaller value.
const DEFAULT_ARRANGEMENT_GROUPING_TICKS: u32 = 3840;

/// End tick for an open-ended note (no `duration`): one grouping window
/// past `start` so the note contributes weight to exactly one chord event.
fn synthetic_note_end(start: u32, grouping_ticks: u32) -> u32 {
    start.saturating_add(grouping_ticks)
}

/// Convert an absolute tick to 1-indexed (bar, beat) under the given time
/// signature. `Tick::to_bar_beat_tick` returns 0-indexed values; we shift
/// to 1-indexed for human readability ("Bar 1 beat 1" = song start).
fn tick_to_bar_beat_1based(tick: Tick, time_sig: synth_sequencer::TimeSignature) -> (u32, u32) {
    let (bar, beat, _) = tick.to_bar_beat_tick(time_sig);
    (bar + 1, beat + 1)
}

/// Merge consecutive `HarmonyChordEvent`s that share a chord symbol (or are
/// both unidentified) into single spans. Keeps the chord-event list compact
/// when a chord is held for several grouping windows.
fn merge_consecutive_chord_events(events: Vec<HarmonyChordEvent>) -> Vec<HarmonyChordEvent> {
    let mut out: Vec<HarmonyChordEvent> = Vec::with_capacity(events.len());
    for e in events {
        if let Some(last) = out.last_mut()
            && last.symbol == e.symbol
            && last.in_key == e.in_key
        {
            last.end_tick = e.end_tick;
            for m in e.midi_notes {
                if !last.midi_notes.contains(&m) {
                    last.midi_notes.push(m);
                }
            }
            last.midi_notes.sort_unstable();
            continue;
        }
        out.push(e);
    }
    out
}

/// Input scope for [`analyze_song_harmony`]: a single pattern, or an arrangement
/// range plus its track-exclusion options. The exclusion options live **only** on
/// the `Arrangement` variant, so passing them in pattern scope is a compile-time
/// impossibility for internal callers. The flat MCP params map in via
/// [`harmony_query_from_flat`], which is the one place that warns if a client
/// sends the nonsensical pattern-id-plus-exclusion combo.
#[doc(hidden)]
pub enum HarmonyQuery {
    Pattern {
        pattern_id: PatternId,
    },
    Arrangement {
        start_tick: Option<Tick>,
        end_tick: Option<Tick>,
        exclude_drums: bool,
        exclude_track_ids: Vec<TrackId>,
    },
}

/// Map the flat `analyze_harmony` MCP params into a [`HarmonyQuery`], returning
/// any "ignored in pattern scope" warnings — the only place the flat API can
/// express a pattern id alongside arrangement-only exclusion options. Internal
/// callers build a `HarmonyQuery` directly and so can't hit this case.
#[doc(hidden)]
#[must_use]
pub fn harmony_query_from_flat(
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> (HarmonyQuery, Vec<String>) {
    match pattern_id {
        Some(pattern_id) => {
            let mut warnings = Vec::new();
            if exclude_track_ids.as_ref().is_some_and(|v| !v.is_empty()) {
                warnings.push(
                    "exclude_track_ids is ignored in pattern scope — a pattern is not tied to a specific track".to_string(),
                );
            }
            if exclude_drums.is_some() {
                warnings.push(
                    "exclude_drums is ignored in pattern scope — a pattern has no track/instrument assignment to classify".to_string(),
                );
            }
            (HarmonyQuery::Pattern { pattern_id }, warnings)
        }
        None => (
            HarmonyQuery::Arrangement {
                start_tick: arrangement_start_tick,
                end_tick: arrangement_end_tick,
                exclude_drums: exclude_drums.unwrap_or(true),
                exclude_track_ids: exclude_track_ids.unwrap_or_default(),
            },
            Vec::new(),
        ),
    }
}

/// Implementation of the `analyze_harmony` bridge method.
#[doc(hidden)]
pub fn analyze_song_harmony(
    session: &SynthSession,
    shared: &McpSharedState,
    query: HarmonyQuery,
    grouping_ticks: Option<u32>,
) -> Result<AnalyzeHarmonyResult, McpBridgeError> {
    use synth_sequencer::{InstrumentId, PatternId, TrackId};

    let song = shared.song.read();
    let default_grouping = if matches!(query, HarmonyQuery::Pattern { .. }) {
        DEFAULT_PATTERN_GROUPING_TICKS
    } else {
        DEFAULT_ARRANGEMENT_GROUPING_TICKS
    };
    let grouping = grouping_ticks
        .filter(|g| *g > 0)
        .unwrap_or(default_grouping);
    let mut warnings: Vec<String> = Vec::new();

    // Lock in the time signature once per request so chord-event bar/beat
    // formatting is consistent across the whole scope. Mid-arrangement time
    // signature changes will report bar/beat under the time signature at the
    // scope start; that's accurate enough for the typical case where TS
    // changes are rare.
    let default_ts = song.default_time_signature;
    let scope_time_signature = match &query {
        HarmonyQuery::Arrangement {
            start_tick: Some(t),
            ..
        } => song.time_signature_at(*t),
        _ => default_ts,
    };

    // Analyze the played stream: expand each pattern through its bound note graph
    // (plan §7), not just the authored source.
    let bpm = song.tempo_at(synth_sequencer::Tick(0));

    let (scope, notes, range_start, range_end) = match query {
        HarmonyQuery::Pattern { pattern_id: pid } => {
            let pid_typed = pid;
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let expanded = song.expanded_pattern_notes(pid_typed, bpm);
            let mut notes = Vec::with_capacity(expanded.len());
            for n in &expanded {
                let start_pt = n.start.0;
                let end_pt = match n.duration {
                    Some(d) => start_pt.saturating_add(d.0),
                    None => synthetic_note_end(start_pt, grouping),
                };
                notes.push(crate::harmony::AnalysisNote {
                    pitch: n.pitch,
                    start_tick: u64::from(start_pt),
                    end_tick: u64::from(end_pt),
                });
            }
            if notes.is_empty() {
                warnings.push(format!("Pattern {pid} contains no notes"));
            }
            (
                HarmonyScope::Pattern { pattern_id: pid },
                notes,
                0u64,
                u64::from(length_ticks),
            )
        }
        HarmonyQuery::Arrangement {
            start_tick,
            end_tick,
            exclude_drums,
            exclude_track_ids,
        } => {
            let (start, end) = resolve_arrangement_range(&song, start_tick, end_tick)?;
            let explicit_excluded: std::collections::HashSet<TrackId> =
                exclude_track_ids.into_iter().collect();

            // Resolve which tracks to skip. A track is excluded when either:
            //   1. It appears in the explicit `exclude_track_ids` list, or
            //   2. `exclude_drums` is true and `infer_all_profiles` classifies
            //      its assigned instrument as Drums with confidence >= 0.6.
            //      Manual `set_instrument_category` still wins (it produces
            //      role Drums with confidence 1.0 via manual-override), but
            //      the inference also catches percussion that was never
            //      manually tagged — closing the §8.2 silent-no-op.
            // Tracks with no instrument assignment are never auto-excluded.
            let drum_profiles: std::collections::HashMap<
                InstrumentId,
                crate::analysis::InstrumentProfile,
            > = if exclude_drums {
                crate::analysis::infer_all_profiles(&song, session.state())
                    .into_iter()
                    .filter(|p| {
                        p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6
                    })
                    .map(|p| (p.instrument_id, p))
                    .collect()
            } else {
                std::collections::HashMap::new()
            };
            let auto_excluded_tracks: std::collections::HashSet<TrackId> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument;
                    drum_profiles.contains_key(&seq).then_some(t.id)
                })
                .collect();
            let excluded_tracks: std::collections::HashSet<TrackId> = auto_excluded_tracks
                .iter()
                .chain(explicit_excluded.iter())
                .copied()
                .collect();
            if !excluded_tracks.is_empty() {
                let descriptions: Vec<String> = song
                    .tracks()
                    .filter(|t| excluded_tracks.contains(&t.id))
                    .map(|t| {
                        let base = format!("{}({})", t.name, t.id.0);
                        // Signal trail only for drum-auto-excludes — explicit
                        // excludes have no inference behind them.
                        let seq = t.instrument;
                        if let Some(profile) = drum_profiles.get(&seq) {
                            let sigs = profile
                                .role
                                .signals
                                .iter()
                                .map(|s| format!("{}:{}", s.axis, s.detail))
                                .collect::<Vec<_>>()
                                .join(", ");
                            return format!(
                                "{base} [drums conf={:.2}; {sigs}]",
                                profile.role.confidence
                            );
                        }
                        base
                    })
                    .collect();
                warnings.push(format!(
                    "Excluded {} track(s) from harmony analysis: {}",
                    excluded_tracks.len(),
                    descriptions.join(", ")
                ));
            }

            let mut notes: Vec<crate::harmony::AnalysisNote> = Vec::new();
            // A pattern's expansion is identical across all its placements (the
            // bake keys on PatternId, not the placement), so cache it per pattern
            // rather than re-baking the graph for every placement.
            let mut expanded_cache: HashMap<PatternId, Vec<synth_sequencer::Note>> = HashMap::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                if excluded_tracks.contains(&placement.track_id) {
                    continue;
                }
                let placement_start = placement.start.0;
                // Expanded (graph-processed) notes are pattern-local, like the
                // source, so the placement offset + transpose apply unchanged.
                let pid = placement.pattern_id;
                let expanded = expanded_cache
                    .entry(pid)
                    .or_insert_with(|| song.expanded_pattern_notes(pid, bpm));
                for n in expanded.iter() {
                    let n_start = n.start.0;
                    let n_end_pt = match n.duration {
                        Some(d) => n_start.saturating_add(d.0),
                        None => synthetic_note_end(n_start, grouping),
                    };
                    let abs_start = placement_start.saturating_add(u64::from(n_start));
                    let abs_end = placement_start.saturating_add(u64::from(n_end_pt));
                    if abs_end <= start || abs_start >= end {
                        continue;
                    }
                    let transposed = n.pitch.transpose(placement.transpose);
                    let Some(pitch) = transposed else {
                        warnings.push(format!(
                            "Note at tick {abs_start} dropped: transpose out of MIDI range"
                        ));
                        continue;
                    };
                    notes.push(crate::harmony::AnalysisNote {
                        pitch,
                        start_tick: abs_start,
                        end_tick: abs_end,
                    });
                }
            }
            if notes.is_empty() {
                warnings.push("No notes found in arrangement range".to_string());
            }
            (
                HarmonyScope::Arrangement {
                    start_tick: Tick(start),
                    end_tick: Tick(end),
                },
                notes,
                start,
                end,
            )
        }
    };

    drop(song);

    let opts = crate::harmony::AnalysisOptions {
        grouping_ticks: u64::from(grouping),
        range_start_tick: range_start,
        range_end_tick: range_end,
    };
    let analysis = crate::harmony::analyze(&notes, &opts);

    // Convert the harmony module's output into the serializable MCP types.
    // Track identified / distinct counts on the raw per-window events so the
    // stats reflect the analyzer's resolution, not the post-merge view.
    let mut distinct = std::collections::HashSet::new();
    let mut identified = 0u32;
    let raw_chords: Vec<HarmonyChordEvent> = analysis
        .events
        .iter()
        .map(|e| {
            let (symbol, root, quality) = match &e.chord {
                Some(c) => {
                    distinct.insert(c.symbol.clone());
                    identified += 1;
                    (
                        Some(c.symbol.clone()),
                        Some(c.root),
                        Some(c.quality.to_string()),
                    )
                }
                None => (None, None, None),
            };
            let (start_bar, start_beat) =
                tick_to_bar_beat_1based(Tick(e.start_tick), scope_time_signature);
            HarmonyChordEvent {
                start_bar,
                start_beat,
                start_tick: Tick(e.start_tick),
                end_tick: Tick(e.end_tick),
                midi_notes: e.midi_notes.clone(),
                symbol,
                root,
                quality,
                in_key: e.in_key,
            }
        })
        .collect();
    let raw_event_count = raw_chords.len() as u32;
    let chords = merge_consecutive_chord_events(raw_chords);

    let to_key_estimate = |k: &crate::harmony::KeyEstimate| HarmonyKeyEstimate {
        tonic: k.tonic,
        tonic_name: synth_sequencer::NoteName::from_midi(k.tonic).to_string(),
        mode: k.mode.to_string(),
        label: k.label(),
        correlation: k.correlation,
    };

    let avg_polyphony = if chords.is_empty() {
        0.0
    } else {
        chords
            .iter()
            .map(|c| c.midi_notes.len() as f32)
            .sum::<f32>()
            / chords.len() as f32
    };
    let (lo, hi) = analysis.pitch_range.unwrap_or((0, 0));
    let stats = HarmonyStats {
        total_notes: analysis.total_notes,
        chord_event_count: raw_event_count,
        distinct_chord_count: distinct.len() as u32,
        identified_chord_count: identified,
        pitch_range_low: lo,
        pitch_range_high: hi,
        avg_polyphony,
        grouping_ticks: grouping,
    };

    Ok(AnalyzeHarmonyResult {
        scope,
        chords,
        inferred_key: analysis.inferred_key.as_ref().map(to_key_estimate),
        key_candidates: analysis
            .key_candidates
            .iter()
            .map(to_key_estimate)
            .collect(),
        pitch_class_histogram: analysis.histogram,
        in_key_ratio: analysis.in_key_ratio,
        out_of_scale_pitch_classes: analysis.out_of_scale_pcs,
        harmonic_stability_score: analysis.harmonic_stability_score,
        stats,
        warnings,
    })
}

pub(super) fn analyze_pattern_impl(
    shared: &McpSharedState,
    pattern_id: PatternId,
) -> Result<AnalyzePatternResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzePatternResult, PatternDensity, PatternPitch, PatternRepetition, PatternRhythm,
        PatternVelocity,
    };

    let song = shared.song.read();
    let pid = pattern_id;
    let Some(pattern) = song.pattern(pid) else {
        return Err(McpBridgeError::Other(format!(
            "Pattern {pattern_id} not found"
        )));
    };
    let length_ticks = pattern.length.0;
    let pattern_name = pattern.name.clone();
    // Analyze the played stream: expand through a bound note graph (plan §7), not
    // just the authored source. No binding ⇒ the source notes, unchanged.
    let bpm = song.tempo_at(synth_sequencer::Tick(0));
    let notes: Vec<synth_sequencer::Note> = song.expanded_pattern_notes(pid, bpm);
    let time_sig = song.default_time_signature;
    drop(song);

    let analysis = crate::analysis::pattern_analysis::analyze(&notes, length_ticks, time_sig);

    Ok(AnalyzePatternResult {
        pattern_id,
        pattern_name,
        length_ticks: analysis.length_ticks,
        length_bars: analysis.length_bars,
        time_signature_numerator: time_sig.numerator,
        time_signature_denominator: time_sig.denominator,
        note_count: analysis.note_count,
        density: PatternDensity {
            notes_per_bar: analysis.density.notes_per_bar,
            notes_per_beat: analysis.density.notes_per_beat,
            active_ratio: analysis.density.active_ratio,
        },
        pitch: PatternPitch {
            low: analysis.pitch.low,
            high: analysis.pitch.high,
            range_semitones: analysis.pitch.range_semitones,
            mean: analysis.pitch.mean,
            distinct_count: analysis.pitch.distinct_count,
            class_histogram: analysis.pitch.class_histogram,
        },
        velocity: PatternVelocity {
            min: analysis.velocity.min,
            max: analysis.velocity.max,
            mean: analysis.velocity.mean,
            std_dev: analysis.velocity.std_dev,
            range: analysis.velocity.range,
        },
        rhythm: PatternRhythm {
            max_polyphony: analysis.rhythm.max_polyphony,
            mean_polyphony: analysis.rhythm.mean_polyphony,
            is_monophonic: analysis.rhythm.is_monophonic,
            distinct_onset_count: analysis.rhythm.distinct_onset_count,
            distinct_duration_count: analysis.rhythm.distinct_duration_count,
            mean_ioi_ticks: analysis.rhythm.mean_ioi_ticks,
            ioi_std_ticks: analysis.rhythm.ioi_std_ticks,
            regularity_score: analysis.rhythm.regularity_score,
        },
        repetition: PatternRepetition {
            distinct_bars: analysis.repetition.distinct_bars,
            total_bars: analysis.repetition.total_bars,
            bar_repetition_score: analysis.repetition.bar_repetition_score,
        },
        warnings: analysis.warnings,
    })
}

/// Resolve the arrangement range used by `analyze_drum_groove` /
/// `analyze_bass_drum_lock` / `analyze_harmonic_function` when the caller
/// passes `start`/`end` as `None`.
/// Build a `TrackId → DrumComponent` map from per-track drum infos by running
/// each track name through [`crate::analysis::DrumComponent::from_track_name`].
/// Tracks whose name is generic (`"Drums"`, `"Percussion"`, …) are omitted so
/// callers fall back to GM-MIDI for those.
fn drum_components_by_track_id(
    infos: &[synth_mcp::types::DrumTrackInfo],
) -> std::collections::HashMap<synth_sequencer::TrackId, crate::analysis::DrumComponent> {
    infos
        .iter()
        .filter_map(|d| {
            crate::analysis::DrumComponent::from_track_name(&d.track_name).map(|c| (d.track_id, c))
        })
        .collect()
}

fn resolve_arrangement_range(
    song: &synth_sequencer::Song,
    start: Option<Tick>,
    end: Option<Tick>,
) -> Result<(u64, u64), McpBridgeError> {
    let song_end = song.calculate_length().0;
    let start = start.map_or(0, |tick| tick.0);
    let end = end.map_or(song_end, |tick| tick.0);
    if end <= start {
        return Err(McpBridgeError::Other(format!(
            "Arrangement range invalid: end ({end}) must be greater than start ({start})"
        )));
    }
    Ok((start, end))
}

pub(super) fn analyze_drum_groove_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
) -> Result<synth_mcp::types::AnalyzeDrumGrooveResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzeDrumGrooveResult, DrumBackbeat, DrumComposition, DrumFills, DrumGhostNotes, DrumHat,
        DrumRepetition, DrumTrackInfo, HarmonyScope,
    };
    use synth_sequencer::InstrumentId;

    let song = shared.song.read();
    let mut warnings: Vec<String> = Vec::new();

    let (scope, time_sig, length_ticks, notes, drum_tracks, start_tick, end_tick) = match pattern_id
    {
        Some(pid) => {
            let pid_typed = pid;
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let notes: Vec<crate::analysis::DrumNote> = pattern
                .notes()
                .iter()
                .map(|n| crate::analysis::DrumNote::from_note(n, 0))
                .collect();
            let ts = song.default_time_signature;
            (
                HarmonyScope::Pattern { pattern_id: pid },
                ts,
                length_ticks,
                notes,
                Vec::<DrumTrackInfo>::new(),
                0u64,
                u64::from(length_ticks),
            )
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;

            // Find drum-track candidates the same way `analyze_harmony` does.
            let drum_profiles: std::collections::HashMap<
                InstrumentId,
                crate::analysis::InstrumentProfile,
            > = crate::analysis::infer_all_profiles(&song, session.state())
                .into_iter()
                .filter(|p| p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6)
                .map(|p| (p.instrument_id, p))
                .collect();

            if drum_profiles.is_empty() {
                warnings.push(
                    "No drum tracks identified by infer_all_profiles (confidence >= 0.6)"
                        .to_string(),
                );
            }

            let mut drum_track_infos: Vec<DrumTrackInfo> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument;
                    let profile = drum_profiles.get(&seq)?;
                    Some(DrumTrackInfo {
                        track_id: t.id,
                        track_name: t.name.clone(),
                        instrument_id: seq,
                        instrument_name: profile.instrument_name.clone(),
                        drum_confidence: profile.role.confidence,
                    })
                })
                .collect();
            drum_track_infos.sort_by_key(|d| d.track_id);

            let drum_track_ids: std::collections::HashSet<synth_sequencer::TrackId> =
                drum_track_infos.iter().map(|d| d.track_id).collect();
            let track_role_by_id = drum_components_by_track_id(&drum_track_infos);

            let mut notes: Vec<crate::analysis::DrumNote> = Vec::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                if !drum_track_ids.contains(&placement.track_id) {
                    continue;
                }
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };
                let component_hint = track_role_by_id.get(&placement.track_id).copied();
                let placement_start = placement.start.0;
                for n in pattern.notes() {
                    let abs_start = placement_start.saturating_add(u64::from(n.start.0));
                    if abs_start < start || abs_start >= end {
                        continue;
                    }
                    // Drum analysis works in range-relative tick space so
                    // `length_ticks = end - start` and per-bar math lines
                    // up with the analyzed window.
                    let rel = (abs_start - start) as u32;
                    notes.push(crate::analysis::DrumNote {
                        tick: rel,
                        midi: n.pitch.as_midi(),
                        velocity: n.velocity.as_f32(),
                        component_hint,
                    });
                }
            }

            let length_ticks: u32 = (end - start).try_into().unwrap_or(u32::MAX);
            let ts = song.time_signature_at(synth_sequencer::Tick(start));
            (
                HarmonyScope::Arrangement {
                    start_tick: Tick(start),
                    end_tick: Tick(end),
                },
                ts,
                length_ticks,
                notes,
                drum_track_infos,
                start,
                end,
            )
        }
    };

    drop(song);

    let mut analysis = crate::analysis::drum_groove::analyze(&notes, length_ticks, time_sig);
    warnings.append(&mut analysis.warnings);

    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(start_tick), time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(end_tick), time_sig);

    Ok(AnalyzeDrumGrooveResult {
        scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_ticks: analysis.length_ticks,
        length_bars: analysis.length_bars,
        time_signature_numerator: time_sig.numerator,
        time_signature_denominator: time_sig.denominator,
        total_drum_notes: analysis.total_drum_notes,
        drum_tracks,
        composition: DrumComposition {
            kick: analysis.composition.kick,
            snare: analysis.composition.snare,
            hat_closed: analysis.composition.hat_closed,
            hat_open: analysis.composition.hat_open,
            tom: analysis.composition.tom,
            cymbal: analysis.composition.cymbal,
            clap: analysis.composition.clap,
            other: analysis.composition.other,
        },
        backbeat: DrumBackbeat {
            strength: analysis.backbeat.strength,
            expected_backbeats: analysis.backbeat.expected_backbeats,
            matched_backbeats: analysis.backbeat.matched_backbeats,
            off_backbeat_snares: analysis.backbeat.off_backbeat_snares,
        },
        hat: DrumHat {
            subdivision: analysis.hat.subdivision,
            hat_density_per_beat: analysis.hat.hat_density_per_beat,
            hat_count: analysis.hat.hat_count,
        },
        ghost_notes: DrumGhostNotes {
            count: analysis.ghost_notes.count,
            velocity_threshold: analysis.ghost_notes.velocity_threshold,
        },
        fills: DrumFills {
            fill_bar_count: analysis.fills.fill_bar_count,
            density_threshold: analysis.fills.density_threshold,
            mean_density_per_bar: analysis.fills.mean_density_per_bar,
        },
        repetition: DrumRepetition {
            distinct_bars: analysis.repetition.distinct_bars,
            total_bars: analysis.repetition.total_bars,
            bar_repetition_score: analysis.repetition.bar_repetition_score,
        },
        warnings,
    })
}

pub(super) fn analyze_bass_drum_lock_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    onset_tolerance_ticks: Option<u32>,
) -> Result<synth_mcp::types::AnalyzeBassDrumLockResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzeBassDrumLockResult, BassDrumAlignment, BassPitchStability, BassTrackInfo,
        DrumTrackInfo, HarmonyScope,
    };
    use synth_sequencer::InstrumentId;

    let song = shared.song.read();
    let mut warnings: Vec<String> = Vec::new();
    let tolerance = onset_tolerance_ticks
        .unwrap_or(crate::analysis::bass_drum_lock::DEFAULT_ONSET_TOLERANCE_TICKS);

    let (
        scope,
        time_sig,
        length_ticks,
        kicks,
        bass,
        drum_tracks,
        bass_tracks,
        start_tick,
        end_tick,
    ) = match pattern_id {
        Some(pid) => {
            let pid_typed = pid;
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let mut kicks: Vec<crate::analysis::KickOnset> = Vec::new();
            let mut bass: Vec<crate::analysis::BassOnset> = Vec::new();
            for n in pattern.notes() {
                let midi = n.pitch.as_midi();
                if matches!(
                    crate::analysis::DrumComponent::from_midi(midi),
                    crate::analysis::DrumComponent::Kick
                ) {
                    kicks.push(crate::analysis::KickOnset { tick: n.start.0 });
                } else {
                    bass.push(crate::analysis::BassOnset {
                        tick: n.start.0,
                        midi,
                    });
                }
            }
            (
                HarmonyScope::Pattern { pattern_id: pid },
                song.default_time_signature,
                length_ticks,
                kicks,
                bass,
                Vec::<DrumTrackInfo>::new(),
                Vec::<BassTrackInfo>::new(),
                0u64,
                u64::from(length_ticks),
            )
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;

            let profiles = crate::analysis::infer_all_profiles(&song, session.state());
            let drum_profiles: std::collections::HashMap<
                InstrumentId,
                crate::analysis::InstrumentProfile,
            > = profiles
                .iter()
                .filter(|p| p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6)
                .cloned()
                .map(|p| (p.instrument_id, p))
                .collect();
            let bass_profiles: std::collections::HashMap<
                InstrumentId,
                crate::analysis::InstrumentProfile,
            > = profiles
                .into_iter()
                .filter(|p| p.role.role == crate::analysis::Role::Bass && p.role.confidence >= 0.6)
                .map(|p| (p.instrument_id, p))
                .collect();

            if drum_profiles.is_empty() {
                warnings.push(
                    "No drum tracks identified by infer_all_profiles — kick onset count will be 0"
                        .to_string(),
                );
            }
            if bass_profiles.is_empty() {
                warnings.push(
                    "No bass tracks identified by infer_all_profiles — bass onset count will be 0"
                        .to_string(),
                );
            }

            let mut drum_track_infos: Vec<DrumTrackInfo> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument;
                    let profile = drum_profiles.get(&seq)?;
                    Some(DrumTrackInfo {
                        track_id: t.id,
                        track_name: t.name.clone(),
                        instrument_id: seq,
                        instrument_name: profile.instrument_name.clone(),
                        drum_confidence: profile.role.confidence,
                    })
                })
                .collect();
            drum_track_infos.sort_by_key(|d| d.track_id);
            let mut bass_track_infos: Vec<BassTrackInfo> = song
                .tracks()
                .filter_map(|t| {
                    let seq = t.instrument;
                    let profile = bass_profiles.get(&seq)?;
                    Some(BassTrackInfo {
                        track_id: t.id,
                        track_name: t.name.clone(),
                        instrument_id: seq,
                        instrument_name: profile.instrument_name.clone(),
                        bass_confidence: profile.role.confidence,
                    })
                })
                .collect();
            bass_track_infos.sort_by_key(|b| b.track_id);

            let drum_track_ids: std::collections::HashSet<synth_sequencer::TrackId> =
                drum_track_infos.iter().map(|d| d.track_id).collect();
            let bass_track_ids: std::collections::HashSet<synth_sequencer::TrackId> =
                bass_track_infos.iter().map(|b| b.track_id).collect();
            // Tracks whose name explicitly marks them as the kick — every hit
            // on these counts as a kick onset regardless of MIDI number. Lets
            // projects that map each drum to its own trigger note still get
            // accurate kick/bass-lock analysis without GM-MIDI assumptions.
            let track_role_by_id = drum_components_by_track_id(&drum_track_infos);

            let mut kicks: Vec<crate::analysis::KickOnset> = Vec::new();
            let mut bass: Vec<crate::analysis::BassOnset> = Vec::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                let is_drum = drum_track_ids.contains(&placement.track_id);
                let is_bass = bass_track_ids.contains(&placement.track_id);
                if !is_drum && !is_bass {
                    continue;
                }
                let track_is_kick_by_name = track_role_by_id.get(&placement.track_id)
                    == Some(&crate::analysis::DrumComponent::Kick);
                let Some(pattern) = song.pattern(placement.pattern_id) else {
                    continue;
                };
                let placement_start = placement.start.0;
                for n in pattern.notes() {
                    let abs_start = placement_start.saturating_add(u64::from(n.start.0));
                    if abs_start < start || abs_start >= end {
                        continue;
                    }
                    let rel = (abs_start - start) as u32;
                    let midi = n.pitch.as_midi();
                    if is_drum {
                        let is_kick_hit = track_is_kick_by_name
                            || matches!(
                                crate::analysis::DrumComponent::from_midi(midi),
                                crate::analysis::DrumComponent::Kick
                            );
                        if is_kick_hit {
                            kicks.push(crate::analysis::KickOnset { tick: rel });
                        }
                    } else {
                        let transposed = n.pitch.transpose(placement.transpose);
                        let Some(p) = transposed else {
                            continue;
                        };
                        bass.push(crate::analysis::BassOnset {
                            tick: rel,
                            midi: p.as_midi(),
                        });
                    }
                }
            }

            let length_ticks: u32 = (end - start).try_into().unwrap_or(u32::MAX);
            let ts = song.time_signature_at(synth_sequencer::Tick(start));
            (
                HarmonyScope::Arrangement {
                    start_tick: Tick(start),
                    end_tick: Tick(end),
                },
                ts,
                length_ticks,
                kicks,
                bass,
                drum_track_infos,
                bass_track_infos,
                start,
                end,
            )
        }
    };

    drop(song);

    let mut analysis =
        crate::analysis::bass_drum_lock::analyze(&kicks, &bass, length_ticks, time_sig, tolerance);
    warnings.append(&mut analysis.warnings);

    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(start_tick), time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(end_tick), time_sig);

    let on_kick_root_name = analysis
        .bass_pitch
        .on_kick_root_pc
        .map(|pc| synth_sequencer::NoteName::from_midi(pc).to_string());

    Ok(AnalyzeBassDrumLockResult {
        scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_ticks: analysis.length_ticks,
        length_bars: analysis.length_bars,
        time_signature_numerator: time_sig.numerator,
        time_signature_denominator: time_sig.denominator,
        drum_tracks,
        bass_tracks,
        kick_onset_count: analysis.kick_onset_count,
        bass_onset_count: analysis.bass_onset_count,
        onset_tolerance_ticks: analysis.onset_tolerance_ticks,
        alignment: BassDrumAlignment {
            matched_onsets: analysis.alignment.matched_onsets,
            kick_only: analysis.alignment.kick_only,
            bass_only: analysis.alignment.bass_only,
            lock_score: analysis.alignment.lock_score,
            coverage_score: analysis.alignment.coverage_score,
        },
        bass_pitch: BassPitchStability {
            on_kick_root_pc: analysis.bass_pitch.on_kick_root_pc,
            on_kick_root_name,
            on_kick_root_share: analysis.bass_pitch.on_kick_root_share,
            distinct_pcs_on_kick: analysis.bass_pitch.distinct_pcs_on_kick,
            distinct_pcs_total: analysis.bass_pitch.distinct_pcs_total,
            mean_bass_midi: analysis.bass_pitch.mean_bass_midi,
        },
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn analyze_harmonic_function_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    grouping_ticks: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::AnalyzeHarmonicFunctionResult, McpBridgeError> {
    use synth_mcp::types::{
        AnalyzeHarmonicFunctionResult, ChordFunctionEvent, FunctionDistribution,
        HarmonicCadenceEvent, HarmonicCadenceKind, TensionStats,
    };

    // Reuse the harmony analyzer end-to-end so the key inference + chord
    // identification + drum-exclusion behaviour stays in lock-step with
    // analyze_harmony.
    let (harmony_query, scope_warnings) = harmony_query_from_flat(
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums,
        exclude_track_ids,
    );
    let harmony = analyze_song_harmony(session, shared, harmony_query, grouping_ticks)?;

    let key_mode = harmony
        .inferred_key
        .as_ref()
        .map(|k| crate::analysis::KeyMode::from_label(&k.mode))
        .unwrap_or(crate::analysis::KeyMode::Major);
    let tonic = harmony.inferred_key.as_ref().map(|k| k.tonic);

    let chord_inputs: Vec<crate::analysis::ChordInput> = harmony
        .chords
        .iter()
        .map(|e| crate::analysis::ChordInput {
            symbol: e.symbol.clone(),
            root: e.root,
            quality: e.quality.clone(),
            in_key: e.in_key,
        })
        .collect();

    let analysis = crate::analysis::harmonic_function::analyze(&chord_inputs, tonic, key_mode);

    // Pattern-scope "ignored exclude_*" warnings first, then the analyzers'.
    let mut warnings = scope_warnings;
    warnings.extend(harmony.warnings.iter().cloned());
    warnings.extend(analysis.warnings.iter().cloned());

    let chord_events: Vec<ChordFunctionEvent> = harmony
        .chords
        .iter()
        .zip(analysis.chords.iter())
        .map(|(harmony_event, fn_event)| ChordFunctionEvent {
            symbol: fn_event.symbol.clone(),
            start_bar: harmony_event.start_bar,
            start_beat: harmony_event.start_beat,
            start_tick: harmony_event.start_tick,
            end_tick: harmony_event.end_tick,
            scale_degree: fn_event.scale_degree,
            roman_numeral: fn_event.roman_numeral.clone(),
            function: fn_event.function.as_str().to_string(),
            tension: fn_event.tension,
            in_key: fn_event.in_key,
            cadence: fn_event.cadence.map(|c| c.as_str().to_string()),
        })
        .collect();

    let cadences: Vec<HarmonicCadenceEvent> = analysis
        .cadences
        .iter()
        .map(|c| HarmonicCadenceEvent {
            chord_index: c.chord_index,
            kind: match c.kind {
                crate::analysis::CadenceKind::Authentic => HarmonicCadenceKind::Authentic,
                crate::analysis::CadenceKind::Plagal => HarmonicCadenceKind::Plagal,
                crate::analysis::CadenceKind::HalfCadence => HarmonicCadenceKind::HalfCadence,
                crate::analysis::CadenceKind::Deceptive => HarmonicCadenceKind::Deceptive,
            },
        })
        .collect();

    Ok(AnalyzeHarmonicFunctionResult {
        scope: harmony.scope,
        key: harmony.inferred_key.clone(),
        chords: chord_events,
        cadences,
        function_distribution: FunctionDistribution {
            tonic: analysis.function_distribution.tonic,
            subdominant: analysis.function_distribution.subdominant,
            dominant: analysis.function_distribution.dominant,
            other: analysis.function_distribution.other,
            chromatic: analysis.function_distribution.chromatic,
        },
        tension: TensionStats {
            mean: analysis.tension.mean,
            peak: analysis.tension.peak,
            trough: analysis.tension.trough,
            std_dev: analysis.tension.std_dev,
        },
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Group C — form & motif analyzer impls
// ---------------------------------------------------------------------------

/// Shared scope resolution for the four Group C tools. Resolves to either a
/// single pattern or an arrangement range, applies drum-track filtering
/// (auto-inferred + explicit) when requested in arrangement scope, and
/// returns the scope-relative [`MelodicNote`] stream plus enough context
/// (`HarmonyScope`, `TimeSignature`, `length_ticks`, `length_bars`,
/// `start/end_tick`) to populate the wire-format header fields.
#[allow(clippy::too_many_arguments)]
fn collect_form_scope(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    exclude_drums: bool,
    exclude_track_ids: &[TrackId],
) -> Result<FormScopeData, McpBridgeError> {
    use crate::analysis::bar_features::MelodicNote;
    use synth_mcp::types::HarmonyScope;
    use synth_sequencer::InstrumentId;

    let song = shared.song.read();
    let mut warnings: Vec<String> = Vec::new();
    // Analyze the played stream: expand each pattern through its bound note graph
    // (plan §7), not just the authored source.
    let bpm = song.tempo_at(synth_sequencer::Tick(0));

    match pattern_id {
        Some(pid) => {
            let pid_typed = pid;
            let Some(pattern) = song.pattern(pid_typed) else {
                return Err(McpBridgeError::Other(format!("Pattern {pid} not found")));
            };
            let length_ticks = pattern.length.0;
            let ts = song.default_time_signature;
            let ticks_per_bar = ts.ticks_per_bar().max(1);
            let total_bars = length_ticks.div_ceil(ticks_per_bar).max(1);
            let notes: Vec<MelodicNote> = song
                .expanded_pattern_notes(pid_typed, bpm)
                .iter()
                .map(|n| MelodicNote {
                    track_id: 0,
                    tick: n.start.0,
                    duration_ticks: n.duration.map(|d| d.0).unwrap_or(240).max(1),
                    pitch: n.pitch,
                    velocity: n.velocity.as_f32(),
                })
                .collect();
            Ok(FormScopeData {
                scope: HarmonyScope::Pattern { pattern_id: pid },
                time_sig: ts,
                length_ticks: u64::from(length_ticks),
                total_bars,
                start_tick: Tick::ZERO,
                end_tick: Tick(u64::from(length_ticks)),
                notes,
                warnings,
            })
        }
        None => {
            let (start, end) =
                resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;
            let ts = song.time_signature_at(synth_sequencer::Tick(start));
            let ticks_per_bar = ts.ticks_per_bar().max(1);
            let span_ticks: u32 = (end - start).try_into().unwrap_or(u32::MAX);
            let total_bars = span_ticks.div_ceil(ticks_per_bar).max(1);

            // Build the drum-filter set (auto-inferred drums + explicit
            // exclusions).
            let mut excluded: std::collections::HashSet<TrackId> =
                exclude_track_ids.iter().copied().collect();
            if exclude_drums {
                let drum_instrument_ids: std::collections::HashSet<InstrumentId> =
                    crate::analysis::infer_all_profiles(&song, session.state())
                        .into_iter()
                        .filter(|p| {
                            p.role.role == crate::analysis::Role::Drums && p.role.confidence >= 0.6
                        })
                        .map(|p| p.instrument_id)
                        .collect();
                for t in song.tracks() {
                    if drum_instrument_ids.contains(&t.instrument) {
                        excluded.insert(t.id);
                    }
                }
            }

            let mut notes: Vec<MelodicNote> = Vec::new();
            // Cache each pattern's expansion — it's identical across placements
            // (the bake keys on PatternId), so re-baking per placement is waste.
            let mut expanded_cache: HashMap<PatternId, Vec<synth_sequencer::Note>> = HashMap::new();
            for placement in
                song.placements_in_range(synth_sequencer::Tick(start), synth_sequencer::Tick(end))
            {
                if excluded.contains(&placement.track_id) {
                    continue;
                }
                let placement_start = placement.start.0;
                // Expanded (graph-processed) notes are pattern-local, so the
                // placement offset + transpose apply the same as the source.
                let pid = placement.pattern_id;
                let expanded = expanded_cache
                    .entry(pid)
                    .or_insert_with(|| song.expanded_pattern_notes(pid, bpm));
                for n in expanded.iter() {
                    let abs_start = placement_start.saturating_add(u64::from(n.start.0));
                    if abs_start < start || abs_start >= end {
                        continue;
                    }
                    let Some(pitch) = n.pitch.transpose(placement.transpose) else {
                        warnings.push(format!(
                            "Note at tick {abs_start} dropped: placement transpose out of MIDI range"
                        ));
                        continue;
                    };
                    let rel = (abs_start - start) as u32;
                    notes.push(MelodicNote {
                        track_id: placement.track_id.0,
                        tick: rel,
                        duration_ticks: n.duration.map(|d| d.0).unwrap_or(240).max(1),
                        pitch,
                        velocity: n.velocity.as_f32(),
                    });
                }
            }

            if notes.is_empty() {
                warnings.push(
                    "No melodic notes inside the analyzed scope (drums excluded by default)"
                        .to_string(),
                );
            }
            Ok(FormScopeData {
                scope: HarmonyScope::Arrangement {
                    start_tick: Tick(start),
                    end_tick: Tick(end),
                },
                time_sig: ts,
                length_ticks: end - start,
                total_bars,
                start_tick: Tick(start),
                end_tick: Tick(end),
                notes,
                warnings,
            })
        }
    }
}

struct FormScopeData {
    scope: synth_mcp::types::HarmonyScope,
    time_sig: synth_sequencer::TimeSignature,
    length_ticks: u64,
    total_bars: u32,
    start_tick: Tick,
    end_tick: Tick,
    notes: Vec<crate::analysis::bar_features::MelodicNote>,
    warnings: Vec<String>,
}

const FORM_SIMILARITY_DEFAULT: f32 = 0.85;
const FORM_SECTION_MIN_BARS_DEFAULT: u32 = 2;

fn section_summary_to_wire(
    s: &crate::analysis::form::SectionSummary,
) -> synth_mcp::types::SectionSpan {
    synth_mcp::types::SectionSpan {
        label: s.label.clone(),
        start_bar: s.start_bar,
        end_bar: s.end_bar,
        length_bars: s.length_bars,
        mean_notes_per_bar: s.mean_notes_per_bar,
        mean_distinct_pitch_classes: s.mean_distinct_pitch_classes,
        mean_velocity: s.mean_velocity,
        active_track_ids: s.active_track_ids.iter().copied().map(TrackId).collect(),
    }
}

fn clamp_similarity(t: Option<f32>) -> f32 {
    let v = t.unwrap_or(FORM_SIMILARITY_DEFAULT);
    if v.is_nan() {
        FORM_SIMILARITY_DEFAULT
    } else {
        v.clamp(0.5, 0.999)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn analyze_arrangement_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    similarity_threshold: Option<f32>,
    section_min_bars: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::AnalyzeArrangementResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeArrangementResult, BarFeatureSummary, SectionSpan};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let threshold = clamp_similarity(similarity_threshold);
    let min_bars = section_min_bars
        .unwrap_or(FORM_SECTION_MIN_BARS_DEFAULT)
        .max(1);

    let analysis = crate::analysis::form::analyze_form(
        &scope_data.notes,
        scope_data.time_sig,
        scope_data.total_bars,
        threshold,
        min_bars,
    );

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let bars: Vec<BarFeatureSummary> = analysis
        .bars
        .iter()
        .map(|b| BarFeatureSummary {
            bar: b.bar,
            note_count: b.note_count,
            distinct_pitch_classes: b.distinct_pitch_classes,
            dominant_pitch_class: b.dominant_pitch_class,
            mean_velocity: b.mean_velocity,
            active_track_ids: b.active_track_ids.iter().copied().map(TrackId).collect(),
        })
        .collect();

    let sections: Vec<SectionSpan> = analysis
        .sections
        .iter()
        .map(section_summary_to_wire)
        .collect();

    let mut warnings = scope_data.warnings;
    if scope_data.total_bars < 2 {
        warnings.push("Scope is shorter than 2 bars — section clustering skipped".to_string());
    }

    Ok(AnalyzeArrangementResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_ticks: scope_data.length_ticks,
        length_bars: scope_data.total_bars,
        time_signature_numerator: scope_data.time_sig.numerator,
        time_signature_denominator: scope_data.time_sig.denominator,
        similarity_threshold: threshold,
        bars,
        sections,
        distinct_section_count: analysis.distinct_section_count,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn analyze_form_map_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    similarity_threshold: Option<f32>,
    section_min_bars: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::AnalyzeFormMapResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeFormMapResult, SectionSpan};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;
    let threshold = clamp_similarity(similarity_threshold);
    let min_bars = section_min_bars
        .unwrap_or(FORM_SECTION_MIN_BARS_DEFAULT)
        .max(1);

    let analysis = crate::analysis::form::analyze_form(
        &scope_data.notes,
        scope_data.time_sig,
        scope_data.total_bars,
        threshold,
        min_bars,
    );

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let sections: Vec<SectionSpan> = analysis
        .sections
        .iter()
        .map(section_summary_to_wire)
        .collect();

    Ok(AnalyzeFormMapResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_bars: scope_data.total_bars,
        time_signature_numerator: scope_data.time_sig.numerator,
        time_signature_denominator: scope_data.time_sig.denominator,
        similarity_threshold: threshold,
        bar_labels: analysis.bar_labels,
        form_string: analysis.form_string,
        sections,
        distinct_section_count: analysis.distinct_section_count,
        warnings: scope_data.warnings,
    })
}

const MOTIF_MIN_LEN_DEFAULT: u8 = 3;
const MOTIF_MAX_LEN_DEFAULT: u8 = 6;
const MOTIF_MIN_COUNT_DEFAULT: u32 = 3;
const MOTIF_TOP_N_DEFAULT: u32 = 10;
const MOTIF_LEN_HARD_CAP: u8 = 12;
/// Default cap on the `occurrences` list per motif on the wire. The
/// authoritative count is always carried in `MotifEntry.count`; the list
/// is a sample of locations and would otherwise blow up the response on
/// repetitive hooks.
const MOTIF_OCCURRENCES_DEFAULT_CAP: u32 = 5;
const MOTIF_OCCURRENCES_HARD_CAP: u32 = 50;

/// Resolve the caller's `max_occurrences_per_motif` against the
/// default/hard-cap. Lives next to `clamp_motif_lengths` for symmetry.
fn clamp_motif_occurrences(n: Option<u32>) -> u32 {
    n.unwrap_or(MOTIF_OCCURRENCES_DEFAULT_CAP)
        .min(MOTIF_OCCURRENCES_HARD_CAP)
}

fn clamp_motif_lengths(min_len: Option<u8>, max_len: Option<u8>) -> (u8, u8) {
    let lo = min_len
        .unwrap_or(MOTIF_MIN_LEN_DEFAULT)
        .clamp(2, MOTIF_LEN_HARD_CAP);
    let mut hi = max_len
        .unwrap_or(MOTIF_MAX_LEN_DEFAULT)
        .min(MOTIF_LEN_HARD_CAP);
    if hi < lo {
        hi = lo;
    }
    (lo, hi)
}

fn motif_occurrences_to_wire(
    hits: &[crate::analysis::motifs::MotifHit],
    scope_start_tick: Tick,
    time_sig: synth_sequencer::TimeSignature,
    max_per_motif: u32,
) -> Vec<synth_mcp::types::MotifOccurrence> {
    let limit = (max_per_motif as usize).min(hits.len());
    hits.iter()
        .take(limit)
        .map(|h| {
            let abs_tick = scope_start_tick.0 + u64::from(h.start_tick);
            let (bar, beat) = tick_to_bar_beat_1based(Tick(abs_tick), time_sig);
            synth_mcp::types::MotifOccurrence {
                track_id: TrackId(h.track_id),
                start_tick: Tick(abs_tick),
                start_bar: bar,
                start_beat: beat,
                first_pitch: h.first_pitch,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn find_motifs_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    min_interval_length: Option<u8>,
    max_interval_length: Option<u8>,
    min_count: Option<u32>,
    top_n: Option<u32>,
    max_occurrences_per_motif: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::FindMotifsResult, McpBridgeError> {
    use synth_mcp::types::{FindMotifsResult, MotifEntry};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let (min_len, max_len) = clamp_motif_lengths(min_interval_length, max_interval_length);
    let min_count_v = min_count.unwrap_or(MOTIF_MIN_COUNT_DEFAULT).max(2);
    let top_n_v = top_n.unwrap_or(MOTIF_TOP_N_DEFAULT).max(1);
    let max_occ = clamp_motif_occurrences(max_occurrences_per_motif);

    let motifs = crate::analysis::motifs::find_motifs(
        &scope_data.notes,
        min_len,
        max_len,
        min_count_v,
        top_n_v,
    );

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let wire_motifs: Vec<MotifEntry> = motifs
        .iter()
        .map(|m| MotifEntry {
            length: m.length,
            intervals: m.intervals.clone(),
            count: m.count(),
            occurrences: motif_occurrences_to_wire(
                &m.occurrences,
                scope_data.start_tick,
                scope_data.time_sig,
                max_occ,
            ),
        })
        .collect();

    let mut warnings = scope_data.warnings;
    if scope_data.notes.len() < (max_len as usize) + 1 {
        warnings.push(format!(
            "Only {} melodic notes in scope — too few to form motifs of length {}",
            scope_data.notes.len(),
            max_len + 1
        ));
    }

    Ok(FindMotifsResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        min_interval_length: min_len,
        max_interval_length: max_len,
        min_count: min_count_v,
        total_notes: scope_data.notes.len() as u32,
        motifs: wire_motifs,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn analyze_hook_strength_impl(
    session: &SynthSession,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    min_interval_length: Option<u8>,
    min_count: Option<u32>,
    max_occurrences_per_motif: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::AnalyzeHookStrengthResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeHookStrengthResult, MotifEntry};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let min_len = min_interval_length
        .unwrap_or(MOTIF_MIN_LEN_DEFAULT)
        .clamp(2, MOTIF_LEN_HARD_CAP);
    let min_count_v = min_count.unwrap_or(MOTIF_MIN_COUNT_DEFAULT).max(2);
    let max_occ = clamp_motif_occurrences(max_occurrences_per_motif);
    // Hook-strength always sweeps up to the hard cap so a long but rare
    // motif can still win — the caller controls the *minimum*, not the
    // maximum length considered.
    let motifs = crate::analysis::motifs::find_motifs(
        &scope_data.notes,
        min_len,
        MOTIF_LEN_HARD_CAP,
        min_count_v,
        50,
    );
    let analysis =
        crate::analysis::motifs::hook_strength(&scope_data.notes, &motifs, min_len, min_count_v);

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    let strongest_wire = analysis.strongest.map(|m| MotifEntry {
        length: m.length,
        intervals: m.intervals.clone(),
        count: m.count(),
        occurrences: motif_occurrences_to_wire(
            &m.occurrences,
            scope_data.start_tick,
            scope_data.time_sig,
            max_occ,
        ),
    });

    Ok(AnalyzeHookStrengthResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        total_notes: scope_data.notes.len() as u32,
        hook_score: analysis.score,
        coverage_ratio: analysis.coverage,
        strongest_motif: strongest_wire,
        min_interval_length: min_len,
        min_count: min_count_v,
        warnings: scope_data.warnings,
    })
}

// ---------------------------------------------------------------------------
// Group D — meta-analysis tool impls (analyze_tension_curve,
// suggest_music_fixes).
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn analyze_tension_curve_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    include_audio: Option<bool>,
    similarity_threshold: Option<f32>,
    section_min_bars: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::AnalyzeTensionCurveResult, McpBridgeError> {
    use synth_mcp::types::{AnalyzeTensionCurveResult, TensionCurveBar, TensionCurveSummary};

    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;

    let threshold = clamp_similarity(similarity_threshold);
    let min_bars = section_min_bars
        .unwrap_or(FORM_SECTION_MIN_BARS_DEFAULT)
        .max(1);

    let ticks_per_bar = scope_data.time_sig.ticks_per_bar().max(1);

    // Run the form pipeline once — we want both the per-bar feature stream
    // and the section clustering for the cross-bar warnings.
    let form_analysis = crate::analysis::form::analyze_form(
        &scope_data.notes,
        scope_data.time_sig,
        scope_data.total_bars,
        threshold,
        min_bars,
    );

    // Per-bar mean MIDI pitch + distinct-16th-note-onset cells.
    let mut bar_mean_pitches: Vec<Option<f32>> = vec![None; scope_data.total_bars as usize];
    let mut bar_onsets: Vec<u32> = vec![0; scope_data.total_bars as usize];
    {
        let mut pitch_sum = vec![0.0_f32; scope_data.total_bars as usize];
        let mut pitch_count = vec![0_u32; scope_data.total_bars as usize];
        let mut grid_cells: Vec<std::collections::HashSet<u32>> = (0..scope_data.total_bars)
            .map(|_| std::collections::HashSet::new())
            .collect();
        for n in &scope_data.notes {
            let bar_idx = (n.tick / ticks_per_bar) as usize;
            if bar_idx >= bar_mean_pitches.len() {
                continue;
            }
            pitch_sum[bar_idx] += f32::from(n.pitch.as_midi());
            pitch_count[bar_idx] += 1;
            let cell = (n.tick % ticks_per_bar) * 16 / ticks_per_bar;
            grid_cells[bar_idx].insert(cell);
        }
        for i in 0..scope_data.total_bars as usize {
            if pitch_count[i] > 0 {
                bar_mean_pitches[i] = Some(pitch_sum[i] / pitch_count[i] as f32);
            }
            bar_onsets[i] = grid_cells[i].len() as u32;
        }
    }

    // Run harmony + harmonic_function to get per-chord tension. Pull
    // grouping_ticks from the scope's TS so it lines up with the chord-window
    // defaults used by `analyze_harmonic_function`.
    let grouping_ticks_opt = Some(synth_sequencer::Duration::QUARTER.0);
    let harmony = analyze_song_harmony(
        session,
        shared,
        harmony_query_from_flat(
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            Some(exclude_drums_v),
            Some(exclude_track_ids_v.clone()),
        )
        .0,
        grouping_ticks_opt,
    )?;
    let key_mode = harmony
        .inferred_key
        .as_ref()
        .map(|k| crate::analysis::KeyMode::from_label(&k.mode))
        .unwrap_or(crate::analysis::KeyMode::Major);
    let tonic = harmony.inferred_key.as_ref().map(|k| k.tonic);
    let chord_inputs: Vec<crate::analysis::ChordInput> = harmony
        .chords
        .iter()
        .map(|e| crate::analysis::ChordInput {
            symbol: e.symbol.clone(),
            root: e.root,
            quality: e.quality.clone(),
            in_key: e.in_key,
        })
        .collect();
    let harm_fn = crate::analysis::harmonic_function::analyze(&chord_inputs, tonic, key_mode);

    let chord_spans: Vec<crate::analysis::tension_curve::ChordTensionSpan> = harmony
        .chords
        .iter()
        .zip(harm_fn.chords.iter())
        .map(|(ev, fnev)| {
            // Harmony events carry absolute ticks in arrangement scope and
            // pattern-relative ticks in pattern scope. Subtract the scope
            // start so the inner module always sees scope-relative ticks.
            let rel_start = ev.start_tick.0.saturating_sub(scope_data.start_tick.0) as u32;
            let rel_end = ev.end_tick.0.saturating_sub(scope_data.start_tick.0) as u32;
            crate::analysis::tension_curve::ChordTensionSpan {
                start_tick: rel_start,
                end_tick: rel_end,
                tension: fnev.tension,
                in_key: fnev.in_key,
            }
        })
        .collect();

    // Optional audio path — one full-scope render sliced into per-bar
    // BarAudio entries. include_audio defaults to true for arrangement
    // scope, false for pattern scope (patterns are usually short and
    // include_audio brings in renderer warnings the caller probably
    // doesn't want).
    let want_audio = include_audio.unwrap_or(pattern_id.is_none());
    let mut warnings = scope_data.warnings.clone();
    let mut audio_per_bar_buf: Vec<crate::analysis::tension_curve::BarAudio> = Vec::new();
    let mut has_audio = false;
    if want_audio {
        if pattern_id.is_some() {
            warnings.push(
                "include_audio is true but pattern scope renders only the pattern's note window — \
                 results may differ from arrangement audio"
                    .to_string(),
            );
        }
        // Render the entire scope. For pattern scope we render the
        // arrangement starting at 0 for `length_ticks` worth of ticks; for
        // arrangement scope we render the [start, end) range directly.
        let render_start = scope_data.start_tick;
        let render_end = render_start.0 + scope_data.length_ticks;
        match crate::audio::arrangement_render::render_arrangement_to_buffer(
            session,
            sample_library,
            &shared.song,
            render_start.0,
            render_end,
        ) {
            Ok(rendered) => {
                for w in &rendered.warnings {
                    warnings.push(w.clone());
                }
                // Precompute the (total_bars + 1) bar-boundary frame
                // offsets once. `tick_to_seconds` walks the tempo-change
                // list per call; doing the lookup inside the per-bar loop
                // is O(bars × tempo_changes).
                let bar_boundary_frames: Vec<usize> = {
                    let song = shared.song.read();
                    let start_seconds = song.tick_to_seconds(render_start);
                    (0..=scope_data.total_bars)
                        .map(|i| {
                            let tick = (render_start.0 + u64::from(i) * u64::from(ticks_per_bar))
                                .min(render_end);
                            let s = song.tick_to_seconds(Tick(tick));
                            (((s - start_seconds) * f64::from(rendered.sample_rate)).max(0.0))
                                as usize
                        })
                        .collect()
                };
                let total_frames = rendered.samples.len() / 2;
                let bar_audio = (0..scope_data.total_bars as usize)
                    .map(|i| {
                        let start_frame = bar_boundary_frames[i].min(total_frames);
                        let end_frame = bar_boundary_frames[i + 1]
                            .min(total_frames)
                            .max(start_frame);
                        let slice = &rendered.samples[start_frame * 2..end_frame * 2];
                        let m = crate::audio::mix_analysis::analyze_mix_buffer(
                            slice,
                            rendered.sample_rate,
                        );
                        crate::analysis::tension_curve::BarAudio {
                            lufs_momentary: m.lufs_momentary_max,
                            rms_dbfs: m.rms_dbfs,
                            band_sub: m.energy_bands.sub,
                            band_low: m.energy_bands.low,
                            band_mid: m.energy_bands.mid,
                            band_high: m.energy_bands.high,
                            stereo_correlation: m.stereo_correlation,
                            stereo_width: m.stereo_width,
                        }
                    })
                    .collect::<Vec<_>>();
                audio_per_bar_buf = bar_audio;
                has_audio = true;
            }
            Err(e) => {
                warnings.push(format!(
                    "include_audio requested but render failed — falling back to symbolic mode: {e}"
                ));
            }
        }
    }

    let sections_from_form: Vec<crate::analysis::bar_features::ClusteredSection> = form_analysis
        .sections
        .iter()
        .map(|s| crate::analysis::bar_features::ClusteredSection {
            label: s.label.clone(),
            start_bar: s.start_bar,
            end_bar: s.end_bar,
        })
        .collect();

    let inputs = crate::analysis::tension_curve::TensionCurveInputs {
        bars: &form_analysis.bars,
        bar_mean_pitches: &bar_mean_pitches,
        bar_distinct_onsets: &bar_onsets,
        ticks_per_bar,
        chord_spans: &chord_spans,
        audio_per_bar: if has_audio {
            Some(&audio_per_bar_buf)
        } else {
            None
        },
        sections: &sections_from_form,
    };
    let analysis = crate::analysis::tension_curve::analyze_tension_curve(&inputs);

    warnings.extend(analysis.warnings.iter().cloned());

    let bars_wire: Vec<TensionCurveBar> = analysis
        .bars
        .iter()
        .map(|b| TensionCurveBar {
            bar: b.bar,
            harmonic_tension: b.harmonic_tension,
            dissonance: b.dissonance,
            density_score: b.density_score,
            register_score: b.register_score,
            rhythmic_activity: b.rhythmic_activity,
            mean_velocity: b.mean_velocity,
            active_track_count: b.active_track_count,
            loudness_score: b.loudness_score,
            brightness: b.brightness,
            band_entropy: b.band_entropy,
            stereo_width_score: b.stereo_width_score,
            composite_tension: b.composite_tension,
        })
        .collect();

    let sections_wire = form_analysis
        .sections
        .iter()
        .map(section_summary_to_wire)
        .collect();

    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    Ok(AnalyzeTensionCurveResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_bars: scope_data.total_bars,
        time_signature_numerator: scope_data.time_sig.numerator,
        time_signature_denominator: scope_data.time_sig.denominator,
        has_audio,
        bars: bars_wire,
        sections: sections_wire,
        summary: TensionCurveSummary {
            peak_bar: analysis.summary.peak_bar,
            peak_value: analysis.summary.peak_value,
            trough_bar: analysis.summary.trough_bar,
            trough_value: analysis.summary.trough_value,
            mean: analysis.summary.mean,
            std_dev: analysis.summary.std_dev,
        },
        warnings,
    })
}

/// The offline-render window `suggest_music_fixes`' audio-backed rules analyze.
struct AudioAnalysisWindow {
    start_tick: Tick,
    end_tick: Tick,
    duration_seconds: f32,
}

/// Return the start (in the same unit as the inputs) of the `window`-long span
/// that covers the most `onsets`, or `fallback` when there are no onsets.
///
/// `onsets` must be sorted ascending. Candidate starts are the onsets
/// themselves: a window opening exactly on an onset can only cover at least as
/// many onsets as one opening just before it, so the optimum is always onset-
/// aligned. Two-pointer sweep, O(n).
pub(super) fn densest_window_start(onsets: &[f32], window: f32, fallback: f32) -> f32 {
    let mut best_start = fallback;
    let mut best_count = 0usize;
    let mut hi = 0usize;
    for (lo, &lo_time) in onsets.iter().enumerate() {
        if hi < lo {
            hi = lo;
        }
        while hi < onsets.len() && onsets[hi] < lo_time + window {
            hi += 1;
        }
        let count = hi - lo;
        if count > best_count {
            best_count = count;
            best_start = lo_time;
        }
    }
    best_start
}

/// Pick the offline-render window for `suggest_music_fixes`' audio-backed rules.
///
/// A single offline render is capped at [`MAX_ANALYSIS_WINDOW_SECONDS`]. When
/// the analyzed scope fits under the cap it is analyzed whole (unchanged
/// behavior). When it is longer — a full song rather than a section — the
/// densest cap-length window is sampled instead, because clipping, masking, and
/// loudness problems concentrate where the arrangement is busiest, and a warning
/// records the sampled range so the caller knows the audio rules judged a
/// sub-window rather than the whole song. This lets the mix rules run on long
/// songs instead of being skipped, without the caller reproducing the
/// meta-analyzer by hand.
fn resolve_representative_audio_window(
    shared: &McpSharedState,
    scope: &FormScopeData,
    warnings: &mut Vec<String>,
) -> AudioAnalysisWindow {
    let song = shared.song.read();
    let scope_start = scope.start_tick;
    let scope_end = scope.end_tick;
    let start_seconds = song.tick_to_seconds(scope_start) as f32;
    let end_seconds = song.tick_to_seconds(scope_end) as f32;
    let full_dur = (end_seconds - start_seconds).max(0.0);

    // Short enough to render in one pass: analyze the whole scope.
    if full_dur <= MAX_ANALYSIS_WINDOW_SECONDS {
        return AudioAnalysisWindow {
            start_tick: scope_start,
            end_tick: scope_end,
            duration_seconds: full_dur,
        };
    }

    // Long scope: slide a cap-length window and keep the one covering the most
    // note onsets — that busiest span is where mix problems concentrate.
    let window = MAX_ANALYSIS_WINDOW_SECONDS;
    let mut onsets: Vec<f32> = scope
        .notes
        .iter()
        .map(|n| song.tick_to_seconds(Tick(scope_start.0 + u64::from(n.tick))) as f32)
        .collect();
    onsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let best_start = densest_window_start(&onsets, window, start_seconds);

    // Keep the window inside the scope.
    let max_start = (end_seconds - window).max(start_seconds);
    let win_start_seconds = best_start.clamp(start_seconds, max_start);
    let win_end_seconds = (win_start_seconds + window).min(end_seconds);
    let dur = (win_end_seconds - win_start_seconds).max(0.0);

    let start_tick = song.seconds_to_tick(f64::from(win_start_seconds));
    let end_tick = song.seconds_to_tick(f64::from(win_end_seconds));
    let ts = song.time_signature_at(start_tick);
    let (start_bar, _) = tick_to_bar_beat_1based(start_tick, ts);
    let (end_bar, _) = tick_to_bar_beat_1based(end_tick, ts);
    drop(song);

    warnings.push(format!(
        "Audio mix analysis sampled a {dur:.0}s window (bars {start_bar}–{end_bar}, the densest \
         region) because the full {full_dur:.0}s scope exceeds the {window:.0}s single-render limit"
    ));

    AudioAnalysisWindow {
        start_tick,
        end_tick,
        duration_seconds: dur,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn suggest_music_fixes_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    pattern_id: Option<PatternId>,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    categories: Option<Vec<String>>,
    include_audio: Option<bool>,
    max_suggestions: Option<u32>,
    exclude_drums: Option<bool>,
    exclude_track_ids: Option<Vec<TrackId>>,
) -> Result<synth_mcp::types::SuggestMusicFixesResult, McpBridgeError> {
    use synth_mcp::types::SuggestMusicFixesResult;

    let categories_v: Vec<String> = categories
        .unwrap_or_default()
        .into_iter()
        .filter(|c| {
            crate::analysis::suggest_fixes::ALL_CATEGORIES
                .iter()
                .any(|allowed| allowed == c)
        })
        .collect();
    let max = max_suggestions.unwrap_or(15).clamp(1, 50);
    let exclude_drums_v = exclude_drums.unwrap_or(true);
    let exclude_track_ids_v = exclude_track_ids.unwrap_or_default();
    let include_audio_v = include_audio.unwrap_or(true);

    let cat_enabled =
        |c: &str| -> bool { categories_v.is_empty() || categories_v.iter().any(|e| e == c) };

    let mut warnings: Vec<String> = Vec::new();

    // Resolve the scope header (start_bar/start_beat/length_bars). Reuse
    // collect_form_scope so the wire-format scope/end_tick/etc. match
    // analyze_tension_curve byte-for-byte.
    let scope_data = collect_form_scope(
        session,
        shared,
        pattern_id,
        arrangement_start_tick,
        arrangement_end_tick,
        exclude_drums_v,
        &exclude_track_ids_v,
    )?;
    for w in &scope_data.warnings {
        warnings.push(w.clone());
    }
    let (start_bar, start_beat) =
        tick_to_bar_beat_1based(scope_data.start_tick, scope_data.time_sig);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(scope_data.end_tick, scope_data.time_sig);

    // ─── Harmony ────────────────────────────────────────────────────────
    let harmony = if cat_enabled("harmony") {
        match analyze_song_harmony(
            session,
            shared,
            harmony_query_from_flat(
                pattern_id,
                arrangement_start_tick,
                arrangement_end_tick,
                Some(exclude_drums_v),
                Some(exclude_track_ids_v.clone()),
            )
            .0,
            None,
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("harmony analyzer skipped: {e}"));
                None
            }
        }
    } else {
        None
    };

    // ─── Form / arrangement ────────────────────────────────────────────
    let form_map = if cat_enabled("arrangement") {
        match analyze_form_map_impl(
            session,
            shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            None,
            None,
            Some(exclude_drums_v),
            Some(exclude_track_ids_v.clone()),
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("form-map analyzer skipped: {e}"));
                None
            }
        }
    } else {
        None
    };

    // ─── Hook strength ──────────────────────────────────────────────────
    let hook = if cat_enabled("composition") {
        match analyze_hook_strength_impl(
            session,
            shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            None,
            None,
            None,
            Some(exclude_drums_v),
            Some(exclude_track_ids_v.clone()),
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("hook analyzer skipped: {e}"));
                None
            }
        }
    } else {
        None
    };

    // ─── Tension curve ──────────────────────────────────────────────────
    let tension_curve = if cat_enabled("arrangement") {
        match analyze_tension_curve_impl(
            session,
            sample_library,
            shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            Some(include_audio_v && pattern_id.is_none()),
            None,
            None,
            Some(exclude_drums_v),
            Some(exclude_track_ids_v.clone()),
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("tension curve analyzer skipped: {e}"));
                None
            }
        }
    } else {
        None
    };

    // ─── Drum groove + bass-drum lock ──────────────────────────────────
    let drum_groove = if cat_enabled("groove") {
        match analyze_drum_groove_impl(
            session,
            shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("drum-groove analyzer skipped: {e}"));
                None
            }
        }
    } else {
        None
    };
    let bass_drum_lock = if cat_enabled("groove") {
        match analyze_bass_drum_lock_impl(
            session,
            shared,
            pattern_id,
            arrangement_start_tick,
            arrangement_end_tick,
            None,
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("bass-drum-lock analyzer skipped: {e}"));
                None
            }
        }
    } else {
        None
    };

    // ─── Audio-render-backed checks ─────────────────────────────────────
    // Both mix-bus and masking render the arrangement offline, and one render
    // window is capped at MAX_ANALYSIS_WINDOW_SECONDS. A full-song scope exceeds
    // that cap, so sample a bounded representative window (the densest region)
    // and let both audio analyzers share it — otherwise mix-bus errored out
    // ("exceeds the 300-second maximum") and masking rendered the whole song
    // per track.
    let audio_window = if cat_enabled("mix") && include_audio_v && pattern_id.is_none() {
        Some(resolve_representative_audio_window(
            shared,
            &scope_data,
            &mut warnings,
        ))
    } else {
        None
    };

    let mix_bus = match &audio_window {
        Some(win) if win.duration_seconds > 0.0 => match analyze_mix_bus_impl(
            session,
            sample_library,
            shared,
            win.duration_seconds,
            Some(win.start_tick),
            None,
            synth_core::AnalysisScope::default(),
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("mix-bus analyzer skipped: {e}"));
                None
            }
        },
        _ => None,
    };

    let masking = match &audio_window {
        Some(win) if win.duration_seconds > 0.0 => match analyze_masking_matrix_impl(
            session,
            sample_library,
            shared,
            Some(win.start_tick),
            Some(win.end_tick),
            None,
            synth_core::AnalysisScope::default(),
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                warnings.push(format!("masking-matrix analyzer skipped: {e}"));
                None
            }
        },
        _ => None,
    };

    let inputs = crate::analysis::suggest_fixes::SuggestionInputs {
        harmony: harmony.as_ref(),
        mix_bus: mix_bus.as_ref(),
        masking: masking.as_ref(),
        drum_groove: drum_groove.as_ref(),
        bass_drum_lock: bass_drum_lock.as_ref(),
        form_map: form_map.as_ref(),
        hook: hook.as_ref(),
        tension_curve: tension_curve.as_ref(),
    };
    let output = crate::analysis::suggest_fixes::suggest(&inputs, &categories_v, max);

    Ok(SuggestMusicFixesResult {
        scope: scope_data.scope,
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        length_bars: scope_data.total_bars,
        include_audio: include_audio_v,
        categories: categories_v,
        suggestions: output.suggestions,
        rules_clean: output.rules_clean,
        warnings,
    })
}

/// Default mix-bus render duration when the caller leaves it unspecified.
const DEFAULT_MIX_BUS_SECONDS: f32 = 10.0;

/// Longest window (seconds) a single offline analysis render may span. Enforced
/// by [`resolve_duration_window`] and used by `suggest_music_fixes` to decide
/// when the analyzed scope must be sub-sampled into a representative window.
const MAX_ANALYSIS_WINDOW_SECONDS: f32 = 300.0;

/// Convert a `MixAnalysis` into the wire-format `MixBusMetrics`.
fn mix_metrics_from_analysis(
    analysis: &crate::audio::mix_analysis::MixAnalysis,
    sample_rate: u32,
    duration_seconds: f32,
) -> MixBusMetrics {
    MixBusMetrics {
        sample_rate,
        duration_seconds,
        peak: analysis.peak,
        peak_dbfs: analysis.peak_dbfs,
        peak_left: analysis.peak_left,
        peak_right: analysis.peak_right,
        true_peak: analysis.true_peak,
        true_peak_dbtp: analysis.true_peak_dbtp,
        rms: analysis.rms,
        rms_dbfs: analysis.rms_dbfs,
        crest_factor_db: analysis.crest_factor_db,
        lufs_integrated: analysis.lufs_integrated,
        lufs_momentary_max: analysis.lufs_momentary_max,
        lufs_short_term_max: analysis.lufs_short_term_max,
        energy_bands: analysis.energy_bands.into(),
        stereo_correlation: analysis.stereo_correlation,
        mid_rms: analysis.mid_rms,
        side_rms: analysis.side_rms,
        stereo_width: analysis.stereo_width,
        mono_compat: analysis.mono_compat,
        clipped_samples: analysis.clipped_samples,
    }
}

/// `analyze_mix_bus` bridge implementation. Renders `duration_seconds` of the
/// master bus offline starting at `start_tick` (default 0).
/// Human-readable description of the signal chain an offline analysis render
/// measured: the master fader plus which optional stages were included. Makes
/// `analyze_mix_bus` / `analyze_section` unambiguous about pre- vs post-master.
fn describe_signal_chain(scope: synth_core::AnalysisScope, master_volume: f32) -> String {
    let stage = |on: bool| if on { "included" } else { "excluded" };
    format!(
        "instruments + track faders + returns, through master fader {master_volume:.3}x; \
         master effects: {}; return effects: {}; rendered @ {} Hz",
        stage(scope.master_effects),
        stage(scope.return_effects),
        scope.render_sample_rate.as_u32(),
    )
}

/// Resolve a duration-window analysis request to an absolute `[start, end)`
/// tick range. Applies the shared NaN/non-positive default and 300-second cap,
/// then hands the tempo arithmetic to [`crate::render::tick_window_from_seconds`].
/// Shared by the duration-window analyzers (`analyze_mix_bus`,
/// `analyze_master_chain`) so the validation stays in one place. The analyzer
/// policy — the default duration and the cap — lives here rather than in the
/// render core, which the render command drives under different limits.
fn resolve_duration_window(
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
) -> Result<(u64, u64), McpBridgeError> {
    let dur = if duration_seconds.is_nan() || duration_seconds <= 0.0 {
        DEFAULT_MIX_BUS_SECONDS
    } else {
        duration_seconds
    };
    if dur > MAX_ANALYSIS_WINDOW_SECONDS {
        return Err(McpBridgeError::Other(format!(
            "duration_seconds {dur} exceeds the {MAX_ANALYSIS_WINDOW_SECONDS}-second maximum"
        )));
    }
    let window = crate::render::tick_window_from_seconds(
        &shared.song,
        start_tick,
        synth_core::Seconds::new(dur),
    )?;
    Ok((window.start().0, window.end().0))
}

/// Render one `[start, end)` range against `song` on a prepared offline session
/// and reduce the result to mix-bus metrics, folding any non-duplicate render
/// warnings into `warnings`. Returns the metrics plus the actual rendered tick
/// range. Shared by the incremental analyzers (`analyze_master_chain`,
/// `analyze_return_busses`) so the render→analyze→reduce→dedup sequence stays in
/// one place.
fn render_range_to_metrics(
    engine_session: &mut crate::audio::arrangement_render::OfflineEngineSession,
    song: &Arc<synth_sequencer::SharedSong>,
    start: u64,
    end: u64,
    warnings: &mut Vec<String>,
) -> Result<(MixBusMetrics, u64, u64), McpBridgeError> {
    let rendered = engine_session.render_range(song, start, end)?;
    let analysis =
        crate::audio::mix_analysis::analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    let metrics =
        mix_metrics_from_analysis(&analysis, rendered.sample_rate, rendered.duration_seconds);
    for w in rendered.warnings {
        if !warnings.contains(&w) {
            warnings.push(w);
        }
    }
    Ok((metrics, rendered.start_tick, rendered.end_tick))
}

/// `render_to_wav` bridge implementation. Renders the requested window exactly
/// like `analyze_mix_bus_impl` (same offline render + scope), optionally soloing
/// one instrument against a cloned song so the live project is untouched, then
/// writes the interleaved buffer to a 32-bit float WAV.
///
/// The render and the write themselves live in [`crate::render`], which the
/// `pertylizer render` command drives too; this is the MCP-facing adapter that
/// resolves the window, applies instrument isolation, and reshapes the outcome
/// into the tool's result type.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn render_to_wav_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    path: String,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    instrument_id: Option<InstrumentId>,
    scope: synth_core::AnalysisScope,
) -> Result<RenderToWavResult, McpBridgeError> {
    render_to_wav_with_tail_impl(
        session,
        sample_library,
        shared,
        path,
        duration_seconds,
        start_tick,
        instrument_id,
        scope,
        synth_core::Seconds::ZERO,
    )
}

/// Tail-capturing variant used by the MCP tool. The transport stops exactly at
/// the requested end before the additional audio is rendered.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn render_to_wav_with_tail_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    path: String,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    instrument_id: Option<InstrumentId>,
    scope: synth_core::AnalysisScope,
    tail: synth_core::Seconds,
) -> Result<RenderToWavResult, McpBridgeError> {
    let (start, end) = resolve_duration_window(shared, duration_seconds, start_tick)?;
    let window = crate::render::TickWindow::new(Tick(start), Tick(end))?;
    let mut warnings = Vec::new();
    let song = if let Some(inst_id) = instrument_id {
        let mut isolated = shared.song.read().clone();
        let audible_tracks = isolated.isolate_instrument(inst_id);
        if audible_tracks == 0 {
            warnings.push(format!(
                "instrument_id {} drives no track — the render will be silent",
                inst_id.as_u64()
            ));
        }
        warnings.extend(instrument_solo_conflicts(session, inst_id.as_u64()));
        synth_engine::shared_song(isolated)
    } else {
        Arc::clone(&shared.song)
    };

    let path_buf = std::path::PathBuf::from(&path);
    let mut outcome = crate::render::render_window_to_wav(
        session,
        sample_library,
        &crate::render::WavRenderRequest {
            song: &song,
            window,
            tail,
            scope,
            // Deliberately not configurable: this render feeds the analyzers and
            // fingerprinting, so it must stay lossless against the engine's own
            // `f32` output. Quantizing here would put encoding noise into a
            // measurement.
            format: crate::audio::wav_format::WavFormat::Float32,
            output_path: &path_buf,
        },
    )?;
    warnings.append(&mut outcome.warnings);

    Ok(RenderToWavResult {
        path: outcome.path.to_string_lossy().into_owned(),
        sample_rate: outcome.sample_rate,
        channels: outcome.channels,
        duration_seconds: outcome.duration_seconds,
        frames: outcome.frames,
        peak: outcome.peak,
        soloed_instrument_id: instrument_id,
        warnings,
    })
}

/// Warnings describing live *instrument*-level mute/solo state that the offline
/// render replays from the engine snapshot and that an isolated render (
/// `render_to_wav` / `analyze_spectrum`) cannot override via the cloned song's
/// track flags. Returns the (possibly empty) set of human-readable warnings for
/// soloing `target_seq_id`.
fn instrument_solo_conflicts(session: &SynthSession, target_seq_id: u64) -> Vec<String> {
    let snapshots = session.state().instrument_snapshots.read();
    let mut warnings = Vec::new();

    if let Some(target) = snapshots.iter().find(|s| s.id.as_u64() == target_seq_id)
        && (target.muted || !target.enabled)
    {
        warnings.push(format!(
            "instrument {target_seq_id} is muted/disabled in the live project — \
             the render will be silent; unmute it to fingerprint its sound"
        ));
    }

    if snapshots
        .iter()
        .any(|s| s.solo && s.id.as_u64() != target_seq_id)
    {
        warnings.push(format!(
            "another instrument is soloed in the live project — instrument-level solo \
             silences instrument {target_seq_id} in the offline render; clear the solo to \
             fingerprint it"
        ));
    }

    warnings
}

/// Render `[start, end)` for offline analysis, optionally soloing one
/// instrument on a *clone* of the song (the live project's mute/solo state is
/// never mutated). Returns the rendered buffer plus the render warnings,
/// including any instrument-level solo/mute conflict notes. Shared by
/// `render_to_wav_impl` and `analyze_spectrum_impl`.
fn render_analysis_window(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start: u64,
    end: u64,
    instrument_id: Option<InstrumentId>,
    scope: synth_core::AnalysisScope,
) -> Result<
    (
        crate::audio::arrangement_render::RenderedArrangement,
        Vec<String>,
    ),
    McpBridgeError,
> {
    let mut warnings = Vec::new();
    let song_handle = if let Some(inst_id) = instrument_id {
        let mut isolated = shared.song.read().clone();
        let audible_tracks = isolated.isolate_instrument(inst_id);
        if audible_tracks == 0 {
            warnings.push(format!(
                "instrument_id {} drives no track — the render will be silent",
                inst_id.as_u64()
            ));
        }
        // Track-level isolation cannot override *instrument*-level mute/solo,
        // which the offline render replays verbatim from the live engine
        // snapshot. Warn rather than produce a misleadingly empty render.
        warnings.extend(instrument_solo_conflicts(session, inst_id.as_u64()));
        synth_engine::shared_song(isolated)
    } else {
        Arc::clone(&shared.song)
    };

    let mut rendered = crate::audio::arrangement_render::render_arrangement_to_buffer_with_scope(
        session,
        sample_library,
        &song_handle,
        start,
        end,
        scope,
    )?;
    warnings.append(&mut rendered.warnings);
    Ok((rendered, warnings))
}

/// Build `SpectrumOpts` from the shared MCP knobs (`f0_hint`, `max_partials`,
/// `log_bins`), leaving everything else at its default. Shared by
/// `analyze_spectrum_impl` and `analyze_sample_spectrum_impl`.
fn spectrum_opts(
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
) -> crate::audio::analysis::spectrum::SpectrumOpts {
    let mut opts = crate::audio::analysis::spectrum::SpectrumOpts {
        f0_hint: f0_hint.map(synth_core::Hertz::new),
        ..Default::default()
    };
    if let Some(n) = max_partials {
        opts.max_partials = n;
    }
    if let Some(n) = log_bins {
        opts.log_bins = n;
    }
    opts
}

/// Convert an analysis-layer `SpectrumResult` into the MCP `SpectrumDescriptor`
/// (the field block shared by `analyze_spectrum` and `analyze_sample_spectrum`).
fn spectrum_descriptor(
    result: &crate::audio::analysis::spectrum::SpectrumResult,
) -> synth_mcp::types::SpectrumDescriptor {
    synth_mcp::types::SpectrumDescriptor {
        f0_hz: result.f0.map(synth_core::Hertz::as_f32),
        voiced: result.voiced,
        partials: result
            .partials
            .iter()
            .map(|p| synth_mcp::types::AnalyzePartial {
                frequency_hz: p.frequency.as_f32(),
                amplitude_db: p.amplitude.as_f32(),
                harmonic_number: p.harmonic_number,
                inharmonicity_cents: p.inharmonicity.as_f32(),
            })
            .collect(),
        centroid_hz: result.centroid.as_f32(),
        flatness: result.flatness.as_f32(),
        rolloff_hz: result.rolloff.as_f32(),
        inharmonicity: result.inharmonicity.as_f32(),
        odd_even_ratio: result.odd_even_ratio,
        energy_bands: result.bands.into(),
        log_bins_db: result
            .log_bins
            .iter()
            .copied()
            .map(synth_core::Decibels::as_f32)
            .collect(),
    }
}

/// Render a window for offline analysis and mono-sum it, returning the mono
/// buffer, the rendered metadata (sample rate, tick range), and any warnings.
/// Shared by `analyze_spectrum_impl` and `analyze_spectrogram_impl`.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn render_window_mono(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    instrument_id: Option<InstrumentId>,
    scope: synth_core::AnalysisScope,
) -> Result<
    (
        Vec<f32>,
        crate::audio::arrangement_render::RenderedArrangement,
        Vec<String>,
    ),
    McpBridgeError,
> {
    let (start, end) = resolve_duration_window(shared, duration_seconds, start_tick)?;
    let (rendered, warnings) = render_analysis_window(
        session,
        sample_library,
        shared,
        start,
        end,
        instrument_id,
        scope,
    )?;
    let mono = downmix_interleaved(&rendered.samples, rendered.channels);
    Ok((mono, rendered, warnings))
}

/// `analyze_spectrum` bridge implementation. Renders the requested window (same
/// path as `render_to_wav`), mono-sums it, and runs the detailed spectral
/// analysis.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_spectrum_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    instrument_id: Option<InstrumentId>,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
    scope: synth_core::AnalysisScope,
) -> Result<synth_mcp::types::AnalyzeSpectrumResult, McpBridgeError> {
    let (mono, rendered, warnings) = render_window_mono(
        session,
        sample_library,
        shared,
        duration_seconds,
        start_tick,
        instrument_id,
        scope,
    )?;

    let opts = spectrum_opts(f0_hint, max_partials, log_bins);
    let result =
        crate::audio::analysis::spectrum::analyze_spectrum(&mono, rendered.sample_rate, opts);

    Ok(synth_mcp::types::AnalyzeSpectrumResult {
        start_tick: Tick(rendered.start_tick),
        end_tick: Tick(rendered.end_tick),
        spectrum: spectrum_descriptor(&result),
        soloed_instrument_id: instrument_id,
        warnings,
    })
}

/// Default spectrogram hop / window in milliseconds. ~20 ms ≈ one PAL video
/// frame, the rate a SID voice switches waveform.
const DEFAULT_SPECTROGRAM_HOP_MS: f32 = 20.0;
const DEFAULT_SPECTROGRAM_WINDOW_MS: f32 = 40.0;

/// `analyze_spectrogram` bridge implementation. Renders the window once (same
/// path as `analyze_spectrum`), mono-sums it, and slides the FFT in Rust to
/// produce one spectrum per hop — O(1) renders for the whole spectrogram.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_spectrogram_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    instrument_id: Option<InstrumentId>,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
    hop_ms: Option<f32>,
    window_len_ms: Option<f32>,
    scope: synth_core::AnalysisScope,
) -> Result<synth_mcp::types::AnalyzeSpectrogramResult, McpBridgeError> {
    let (mono, rendered, mut warnings) = render_window_mono(
        session,
        sample_library,
        shared,
        duration_seconds,
        start_tick,
        instrument_id,
        scope,
    )?;
    let sr = rendered.sample_rate;
    let (hop_samples, window_len_samples) = spectrogram_frame_samples(sr, hop_ms, window_len_ms);
    let opts = spectrum_opts(f0_hint, max_partials, log_bins);
    let frames = spectrogram_frames(
        &mono,
        sr,
        hop_samples,
        window_len_samples,
        opts,
        &mut warnings,
    );

    Ok(synth_mcp::types::AnalyzeSpectrogramResult {
        start_tick: Tick(rendered.start_tick),
        end_tick: Tick(rendered.end_tick),
        sample_rate: sr,
        hop_seconds: hop_samples as f32 / sr as f32,
        window_seconds: window_len_samples as f32 / sr as f32,
        frames,
        soloed_instrument_id: instrument_id,
        warnings,
    })
}

/// Resolve hop/window ms (each with its default and NaN/non-positive guard) to
/// sample counts at `sr` (≥ 1 each). Shared by the render and sample spectrograms.
fn spectrogram_frame_samples(
    sr: u32,
    hop_ms: Option<f32>,
    window_len_ms: Option<f32>,
) -> (usize, usize) {
    let ms_to_samples = |ms: f32, default: f32| -> usize {
        let ms = if ms.is_finite() && ms > 0.0 {
            ms
        } else {
            default
        };
        ((ms * 0.001 * sr as f32) as usize).max(1)
    };
    (
        ms_to_samples(
            hop_ms.unwrap_or(DEFAULT_SPECTROGRAM_HOP_MS),
            DEFAULT_SPECTROGRAM_HOP_MS,
        ),
        ms_to_samples(
            window_len_ms.unwrap_or(DEFAULT_SPECTROGRAM_WINDOW_MS),
            DEFAULT_SPECTROGRAM_WINDOW_MS,
        ),
    )
}

/// Slide the spectrogram FFT over `mono` and map the frames to the MCP type,
/// pushing a truncation warning if the DSP frame cap was hit. Shared by the
/// render and sample spectrograms.
fn spectrogram_frames(
    mono: &[f32],
    sr: u32,
    hop_samples: usize,
    window_len_samples: usize,
    opts: crate::audio::analysis::spectrum::SpectrumOpts,
    warnings: &mut Vec<String>,
) -> Vec<synth_mcp::types::SpectrogramFrame> {
    let dsp_frames = crate::audio::analysis::spectrum::analyze_spectrogram(
        mono,
        sr,
        hop_samples,
        window_len_samples,
        opts,
    );
    // The DSP layer caps the frame count to bound memory; surface that so the
    // caller doesn't mistake a truncated spectrogram for full coverage.
    if dsp_frames.len() >= crate::audio::analysis::spectrum::MAX_SPECTROGRAM_FRAMES {
        warnings.push(format!(
            "spectrogram truncated at the {}-frame cap (~{:.1} s covered) — \
             increase hop_ms or shorten the source for full coverage",
            crate::audio::analysis::spectrum::MAX_SPECTROGRAM_FRAMES,
            dsp_frames.last().map(|f| f.time.as_f32()).unwrap_or(0.0)
        ));
    }
    dsp_frames
        .iter()
        .map(|f| synth_mcp::types::SpectrogramFrame {
            time_seconds: f.time.as_f32(),
            spectrum: spectrum_descriptor(&f.spectrum),
        })
        .collect()
}

/// `analyze_sample_spectrogram` bridge implementation. The sample counterpart of
/// `analyze_spectrogram_impl`: resolves `sample_id_or_path` to audio, downmixes
/// to mono, and slides the same FFT — but at the file's NATIVE sample rate, so a
/// 32 kHz dump reports its true frequencies rather than the engine's grid.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_sample_spectrogram_impl(
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    sample_id_or_path: String,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
    hop_ms: Option<f32>,
    window_len_ms: Option<f32>,
) -> Result<synth_mcp::types::AnalyzeSampleSpectrogramResult, McpBridgeError> {
    let src = resolve_sample_source(sample_library, &sample_id_or_path)?;
    let mono = downmix_interleaved(&src.data, src.channels);
    let sr = src.sample_rate;

    let (hop_samples, window_len_samples) = spectrogram_frame_samples(sr, hop_ms, window_len_ms);
    let opts = spectrum_opts(f0_hint, max_partials, log_bins);
    let mut warnings = Vec::new();
    let frames = spectrogram_frames(
        &mono,
        sr,
        hop_samples,
        window_len_samples,
        opts,
        &mut warnings,
    );

    Ok(synth_mcp::types::AnalyzeSampleSpectrogramResult {
        sample_name: src.name,
        sample_rate: sr,
        channels: src.channels,
        hop_seconds: hop_samples as f32 / sr as f32,
        window_seconds: window_len_samples as f32 / sr as f32,
        frames,
        warnings,
    })
}

/// `analyze_sample_spectrum` bridge implementation. Resolves `sample_id_or_path`
/// to audio — an already-imported sample id (numeric, looked up in the library)
/// or a WAV file on disk (decoded via the sampler's `load_wav`) — downmixes to
/// mono, and runs the same spectral analysis as `analyze_spectrum`.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_sample_spectrum_impl(
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    sample_id_or_path: String,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
    start_ms: Option<f32>,
    window_len_ms: Option<f32>,
) -> Result<synth_mcp::types::AnalyzeSampleSpectrumResult, McpBridgeError> {
    // Resolve to (name, sample_rate, channels, interleaved data). Prefer an
    // imported sample id when the string is a number that exists in the library;
    // otherwise treat it as a filesystem path.
    let src = resolve_sample_source(sample_library, &sample_id_or_path)?;

    let mono = downmix_interleaved(&src.data, src.channels);
    let frame = window_mono(mono, src.sample_rate, start_ms, window_len_ms)?;
    let opts = spectrum_opts(f0_hint, max_partials, log_bins);
    let result = crate::audio::analysis::spectrum::analyze_spectrum(&frame, src.sample_rate, opts);

    Ok(synth_mcp::types::AnalyzeSampleSpectrumResult {
        sample_name: src.name,
        sample_rate: src.sample_rate,
        frame_count: frame.len() as u64,
        channels: src.channels,
        spectrum: spectrum_descriptor(&result),
        warnings: Vec::new(),
    })
}

/// Slice a mono buffer to the `[start_ms, start_ms + window_len_ms)` window at
/// `sample_rate`. The start is clamped into range and any tail past the end of
/// the buffer is zero-padded so the analysis frame is always exactly
/// `window_len_ms` long. A start at or beyond the end of the audio is a clean
/// [`McpBridgeError::WindowOutOfBounds`] rather than an empty frame. When both
/// window arguments are `None` the whole buffer passes through unchanged.
fn window_mono(
    mono: Vec<f32>,
    sample_rate: u32,
    start_ms: Option<f32>,
    window_len_ms: Option<f32>,
) -> Result<Vec<f32>, McpBridgeError> {
    /// Hard cap on a zero-padded analysis frame, in seconds — bounds allocation
    /// against an absurd `window_len_ms` (frames of interest are milliseconds).
    const MAX_WINDOW_SECONDS: f32 = 60.0;

    if start_ms.is_none() && window_len_ms.is_none() {
        return Ok(mono);
    }
    let sr = sample_rate.max(1) as f32;
    let total = mono.len();
    let available_ms = total as f32 / sr * 1000.0;
    let start = start_ms.unwrap_or(0.0).max(0.0);
    let start_sample = ((start / 1000.0) * sr).round() as usize;
    if start_sample >= total {
        return Err(McpBridgeError::WindowOutOfBounds {
            start_ms: start,
            available_ms,
        });
    }
    // A positive window_len_ms fixes the frame length (zero-padded past the end);
    // otherwise analyse from the start to the end of the buffer.
    let len_samples = match window_len_ms {
        Some(l) if l > 0.0 => {
            // Cap the frame length: padding beyond the audio is only zero-fill
            // (which the FFT adds internally anyway), so a generous bound loses
            // nothing useful while guarding a pathological/inf window_len_ms
            // from triggering a multi-gigabyte allocation. `f32::min` also
            // tames +inf (min(inf, cap) = cap).
            let n = ((l / 1000.0) * sr).round();
            n.min(MAX_WINDOW_SECONDS * sr) as usize
        }
        _ => total - start_sample,
    };
    let avail_end = start_sample.saturating_add(len_samples).min(total);
    let mut frame = mono[start_sample..avail_end].to_vec();
    frame.resize(len_samples.max(1), 0.0);
    Ok(frame)
}

/// Decoded audio resolved from a sample id or WAV path, ready for analysis.
struct ResolvedSampleAudio {
    name: String,
    sample_rate: u32,
    channels: u16,
    /// Interleaved samples.
    data: std::sync::Arc<[f32]>,
}

/// Resolve `sample_id_or_path` to decoded audio. A bare integer that names a
/// sample in the library is used directly; anything else is decoded from disk
/// as a WAV.
fn resolve_sample_source(
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    sample_id_or_path: &str,
) -> Result<ResolvedSampleAudio, McpBridgeError> {
    if let Ok(id) = sample_id_or_path.parse::<u64>() {
        let lib = sample_library
            .read()
            .map_err(|_| McpBridgeError::Other("Sample library lock poisoned".to_string()))?;
        if let Some(sample) = lib.get(synth_sampler::SampleId::new(id)) {
            return Ok(ResolvedSampleAudio {
                name: sample.meta.name.clone(),
                sample_rate: sample.meta.sample_rate.as_u32(),
                channels: sample.meta.channels.count(),
                data: std::sync::Arc::clone(&sample.data),
            });
        }
    }

    // Not an imported id → decode the path. Keep the source rate (target 0 = no
    // resample) so the spectrum reflects the file's real frequencies.
    let path = std::path::Path::new(sample_id_or_path);
    let sample = synth_sampler::load_wav(path, synth_core::audio::DeviceSampleRate::new(0))
        .map_err(|e| {
            McpBridgeError::Other(format!("could not load sample '{sample_id_or_path}': {e}"))
        })?;
    Ok(ResolvedSampleAudio {
        name: sample.meta.name.clone(),
        sample_rate: sample.meta.sample_rate.as_u32(),
        channels: sample.meta.channels.count(),
        data: sample.data,
    })
}

/// Downmix an interleaved `channels`-channel buffer to mono by averaging each
/// frame's channels. Mono passes through unchanged.
fn downmix_interleaved(data: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        return data.to_vec();
    }
    data.chunks_exact(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect()
}

/// Default number of log-spaced bins `compare_spectra` analyses each source
/// with, so the broadband `log_spectral_distance` is always available.
const DEFAULT_COMPARE_LOG_BINS: u32 = 128;
/// Default number of log-mel filterbank bands `compare_spectra` analyses each
/// source with, so `mel_l2_distance` is always available (40 = the common
/// MFCC-style filterbank size).
const DEFAULT_COMPARE_MEL_BANDS: u32 = 40;

/// One analysed `compare_spectra` source: its aggregate spectrum, the exact mono
/// buffer that spectrum was computed over (the windowed frame for a sample, the
/// rendered mono for a render) at `sample_rate`, and any render/decode warnings.
/// The mono buffer + rate feed the `time_resolved` framed path.
struct AnalyzedSource {
    result: crate::audio::analysis::spectrum::SpectrumResult,
    mono: Vec<f32>,
    sample_rate: u32,
    warnings: Vec<String>,
}

/// Analyse one `compare_spectra` source — a render (optionally soloed) or an
/// imported sample / WAV — into an [`AnalyzedSource`] (aggregate spectrum plus
/// the mono buffer it was measured over).
#[allow(clippy::too_many_arguments)]
fn analyze_spectrum_source(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    source: &synth_mcp::SpectrumSource,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
    mel_bands: Option<u32>,
    scope: synth_core::AnalysisScope,
) -> Result<AnalyzedSource, McpBridgeError> {
    let mut opts = spectrum_opts(f0_hint, max_partials, log_bins);
    if let Some(n) = mel_bands {
        opts.mel_bands = n;
    }
    if let Some(id_or_path) = &source.sample_id_or_path {
        let src = resolve_sample_source(sample_library, id_or_path)?;
        let mono = downmix_interleaved(&src.data, src.channels);
        let frame = window_mono(mono, src.sample_rate, source.start_ms, source.window_len_ms)?;
        let result =
            crate::audio::analysis::spectrum::analyze_spectrum(&frame, src.sample_rate, opts);
        Ok(AnalyzedSource {
            result,
            mono: frame,
            sample_rate: src.sample_rate,
            warnings: Vec::new(),
        })
    } else {
        // A render addresses its window in ticks; window_len_ms (if given)
        // overrides the render duration so both sides can frame identically.
        let dur = source
            .window_len_ms
            .map(|ms| ms / 1000.0)
            .or(source.duration_seconds)
            .unwrap_or(10.0);
        let (start, end) = resolve_duration_window(shared, dur, source.start_tick)?;
        let (rendered, warnings) = render_analysis_window(
            session,
            sample_library,
            shared,
            start,
            end,
            source.instrument_id,
            scope,
        )?;
        let mono = downmix_interleaved(&rendered.samples, rendered.channels);
        let result =
            crate::audio::analysis::spectrum::analyze_spectrum(&mono, rendered.sample_rate, opts);
        Ok(AnalyzedSource {
            result,
            mono,
            sample_rate: rendered.sample_rate,
            warnings,
        })
    }
}

/// Active-time fraction (per [`spectrum::active_time_fraction`]) below which the
/// aggregate distances are averaging over mostly-silence and the caller should
/// switch to the time-resolved path — the §2.3 honesty guard's threshold.
const AGGREGATE_ACTIVE_TIME_MIN: f32 = 0.6;

/// Shift the two mono buffers so their onsets line up, given an envelope lag in
/// [`spectrum::ENV_ALIGN_WINDOW_MS`] windows (positive = candidate lags target).
/// Drops samples from the front of whichever side started later, converting the
/// window lag to samples at that buffer's own rate.
fn apply_alignment(mono_t: &mut Vec<f32>, sr_t: u32, mono_c: &mut Vec<f32>, sr_c: u32, lag: i64) {
    use crate::audio::analysis::spectrum::ENV_ALIGN_WINDOW_MS;
    // Round (not truncate) to match `rms_envelope`'s window sizing, so the drain
    // and the envelope grid the lag was measured on agree at non-integer rates.
    let window_samples =
        |sr: u32| ((ENV_ALIGN_WINDOW_MS * 0.001 * sr as f32).round() as usize).max(1);
    if lag > 0 {
        // Candidate lags the target → drop the candidate's leading silence.
        let drop = (lag as usize)
            .saturating_mul(window_samples(sr_c))
            .min(mono_c.len());
        mono_c.drain(0..drop);
    } else if lag < 0 {
        let drop = ((-lag) as usize)
            .saturating_mul(window_samples(sr_t))
            .min(mono_t.len());
        mono_t.drain(0..drop);
    }
}

/// `compare_spectra` bridge implementation. Analyses both sources with the same
/// options (log bins forced on so the broadband distance is meaningful) and runs
/// `analysis::spectrum::compare`. When `time_resolved.enabled`, it additionally
/// aligns and frames both mono buffers and fills the per-frame masked distance
/// fields; otherwise it emits the §2.3 honesty warning on time-sparse material.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn compare_spectra_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    target: synth_mcp::SpectrumSource,
    candidate: synth_mcp::SpectrumSource,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    log_bins: Option<u32>,
    mel_bands: Option<u32>,
    scope: synth_core::AnalysisScope,
    time_resolved: synth_mcp::TimeResolvedOptions,
) -> Result<synth_mcp::types::CompareSpectraResult, McpBridgeError> {
    use crate::audio::analysis::spectrum;

    let bins = Some(log_bins.unwrap_or(DEFAULT_COMPARE_LOG_BINS).max(1));
    let mel = Some(mel_bands.unwrap_or(DEFAULT_COMPARE_MEL_BANDS).max(1));
    let mut a = analyze_spectrum_source(
        session,
        sample_library,
        shared,
        &target,
        f0_hint,
        max_partials,
        bins,
        mel,
        scope,
    )?;
    let mut b = analyze_spectrum_source(
        session,
        sample_library,
        shared,
        &candidate,
        f0_hint,
        max_partials,
        bins,
        mel,
        scope,
    )?;
    let mut warnings = std::mem::take(&mut a.warnings);
    warnings.append(&mut b.warnings);

    let dist = spectrum::compare(&a.result, &b.result);
    let to_diff = |pd: &spectrum::PartialDiff| synth_mcp::types::PartialDiff {
        frequency_hz: pd.frequency.as_f32(),
        amplitude_db: pd.amplitude.as_f32(),
    };

    let mut result = synth_mcp::types::CompareSpectraResult {
        log_spectral_distance: dist.log_spectral_distance,
        voicing_penalty_db: dist.voicing_penalty_db,
        mel_l2_distance: dist.mel_l2_distance,
        centroid_delta_hz: dist.centroid_delta.as_f32(),
        rolloff_delta_hz: dist.rolloff_delta.as_f32(),
        flatness_delta: dist.flatness_delta,
        inharmonicity_delta: dist.inharmonicity_delta,
        odd_even_ratio_delta_db: dist.odd_even_ratio_delta_db,
        voicing_mismatch: dist.voicing_mismatch,
        floor_coverage: dist.floor_coverage,
        floor_limited: dist.floor_limited,
        target_voiced: a.result.voiced,
        candidate_voiced: b.result.voiced,
        missing_partials: dist.missing_partials.iter().map(to_diff).collect(),
        extra_partials: dist.extra_partials.iter().map(to_diff).collect(),
        time_resolved_lsd: None,
        time_resolved_mel_l2: None,
        frames_compared: None,
        frames_masked: None,
        alignment_offset_ms: None,
        worst_frames: None,
        warnings,
    };

    if time_resolved.enabled {
        fill_time_resolved(
            &mut result,
            &a,
            &b,
            &time_resolved,
            f0_hint,
            max_partials,
            bins,
            mel,
        );
    } else {
        // §2.3 honesty guard: the aggregate scalars average over the whole
        // window, so on time-sparse material they average over silence. Warn and
        // point the caller at the framed path. A window too short to yield enough
        // envelope hops to assess is skipped (`None`) rather than mislabelled as
        // 0% active.
        let assess = |mono: &[f32], sr: u32| -> Option<f32> {
            // Need at least ~100 ms (≥10 hops) to call something time-sparse.
            if (mono.len() as f32) < 0.1 * sr as f32 {
                None
            } else {
                Some(spectrum::active_time_fraction(mono, sr))
            }
        };
        let frac_t = assess(&a.mono, a.sample_rate);
        let frac_c = assess(&b.mono, b.sample_rate);
        let sparse = |f: Option<f32>| f.is_some_and(|v| v < AGGREGATE_ACTIVE_TIME_MIN);
        if sparse(frac_t) || sparse(frac_c) {
            let pct = |f: Option<f32>| {
                f.map_or_else(|| "n/a".to_string(), |v| format!("{:.0}%", v * 100.0))
            };
            result.warnings.push(format!(
                "time-sparse content (target active {}, candidate active {} of window) — \
                 aggregate distances average over silence; use time_resolved: true",
                pct(frac_t),
                pct(frac_c)
            ));
        }
    }

    Ok(result)
}

/// Fill the `time_resolved_*` fields of `result` by aligning and framing the two
/// analysed sources and running `spectrum::compare_time_resolved`. Each source is
/// framed at its own sample rate with the same hop/frame length (in ms), so the
/// frames pair by index across differing rates.
#[allow(clippy::too_many_arguments)]
fn fill_time_resolved(
    result: &mut synth_mcp::types::CompareSpectraResult,
    a: &AnalyzedSource,
    b: &AnalyzedSource,
    opts: &synth_mcp::TimeResolvedOptions,
    f0_hint: Option<f32>,
    max_partials: Option<u32>,
    bins: Option<u32>,
    mel: Option<u32>,
) {
    use crate::audio::analysis::spectrum;

    let mut mono_t = a.mono.clone();
    let mut mono_c = b.mono.clone();

    // 1. Align (envelope cross-correlation) before framing so onsets pair up.
    let alignment_offset_ms = if opts.align_envelope {
        let max_lag_ms = opts.align_max_ms.unwrap_or(250.0);
        let lag =
            spectrum::envelope_align(&mono_t, a.sample_rate, &mono_c, b.sample_rate, max_lag_ms);
        apply_alignment(&mut mono_t, a.sample_rate, &mut mono_c, b.sample_rate, lag);
        lag as f32 * spectrum::ENV_ALIGN_WINDOW_MS
    } else {
        0.0
    };

    // 2. Frame both sources at their own rate with the same hop/frame length.
    let mut frame_opts = spectrum_opts(f0_hint, max_partials, bins);
    if let Some(m) = mel {
        frame_opts.mel_bands = m;
    }
    let (hop_t, win_t) = spectrogram_frame_samples(a.sample_rate, opts.hop_ms, opts.frame_len_ms);
    let (hop_c, win_c) = spectrogram_frame_samples(b.sample_rate, opts.hop_ms, opts.frame_len_ms);
    let frames_t = spectrum::analyze_spectrogram(&mono_t, a.sample_rate, hop_t, win_t, frame_opts);
    let frames_c = spectrum::analyze_spectrogram(&mono_c, b.sample_rate, hop_c, win_c, frame_opts);

    for (label, len) in [("target", frames_t.len()), ("candidate", frames_c.len())] {
        if len >= spectrum::MAX_SPECTROGRAM_FRAMES {
            result.warnings.push(format!(
                "time_resolved {label} spectrogram truncated at the {}-frame cap — \
                 increase hop_ms or shorten the window for full coverage",
                spectrum::MAX_SPECTROGRAM_FRAMES
            ));
        }
    }

    // 3. Per-frame masked distance.
    let mask = if opts.mask_target_energy {
        spectrum::FrameMask::TargetEnergy
    } else {
        spectrum::FrameMask::None
    };
    let tr = spectrum::compare_time_resolved(&frames_t, &frames_c, mask);

    result.time_resolved_lsd = Some(tr.lsd);
    result.time_resolved_mel_l2 = Some(tr.mel_l2);
    result.frames_compared = Some(tr.frames_compared);
    result.frames_masked = Some(tr.frames_masked);
    result.alignment_offset_ms = Some(alignment_offset_ms);
    result.worst_frames = Some(
        tr.worst_frames
            .iter()
            .map(|w| synth_mcp::types::WorstFrame {
                time_seconds: w.time.as_f32(),
                lsd: w.lsd,
            })
            .collect(),
    );
}

/// Default RMS-envelope window (ms) for `compare_envelopes` — fine enough to
/// resolve a fast attack, coarse enough to keep the DTW matrix small.
const DEFAULT_ENVELOPE_WINDOW_MS: f32 = 5.0;
/// Default attack-transient span (ms) for `compare_envelopes`.
const DEFAULT_TRANSIENT_WINDOW_MS: f32 = 20.0;
/// Cap on the RMS-envelope length fed to DTW (its cost is `O(N·M)`). Longer
/// contours are strided down to this before warping; the ADSR / transient
/// estimates still use the full-resolution envelope.
const MAX_DTW_WINDOWS: usize = 2048;

/// Resolve a `compare_envelopes` source to a mono buffer + its sample rate,
/// spanning the whole note/region — unlike [`analyze_spectrum_source`], which
/// frames a single short window for the FFT. A render uses `duration_seconds`
/// (default 2 s, long enough to cover a note's attack→release); a sample uses
/// `start_ms`/`window_len_ms` (whole sample when both omitted).
fn resolve_source_mono(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    source: &synth_mcp::SpectrumSource,
    scope: synth_core::AnalysisScope,
) -> Result<(Vec<f32>, u32, Vec<String>), McpBridgeError> {
    if let Some(id_or_path) = &source.sample_id_or_path {
        let src = resolve_sample_source(sample_library, id_or_path)?;
        let mono = downmix_interleaved(&src.data, src.channels);
        let frame = window_mono(mono, src.sample_rate, source.start_ms, source.window_len_ms)?;
        Ok((frame, src.sample_rate, Vec::new()))
    } else {
        let dur = source.duration_seconds.unwrap_or(2.0);
        let (start, end) = resolve_duration_window(shared, dur, source.start_tick)?;
        let (rendered, warnings) = render_analysis_window(
            session,
            sample_library,
            shared,
            start,
            end,
            source.instrument_id,
            scope,
        )?;
        let mono = downmix_interleaved(&rendered.samples, rendered.channels);
        Ok((mono, rendered.sample_rate, warnings))
    }
}

/// Stride a contour down to at most `max` evenly-spaced samples (clones when
/// already short enough). Bounds the DTW cost matrix for long envelopes.
fn stride_to(v: &[f32], max: usize) -> Vec<f32> {
    if max == 0 || v.len() <= max {
        return v.to_vec();
    }
    (0..max).map(|i| v[i * v.len() / max]).collect()
}

/// `compare_envelopes` bridge implementation. Extracts an RMS envelope from each
/// source, estimates ADSR + attack transient per side, and aligns the contours
/// with DTW for a shape distance.
#[doc(hidden)]
#[allow(
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
pub fn compare_envelopes_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    target: synth_mcp::SpectrumSource,
    candidate: synth_mcp::SpectrumSource,
    envelope_window_ms: Option<f32>,
    note_duration_ms: Option<u32>,
    transient_window_ms: Option<f32>,
    scope: synth_core::AnalysisScope,
) -> Result<synth_mcp::types::CompareEnvelopesResult, McpBridgeError> {
    use crate::audio::analysis;

    let window_ms = envelope_window_ms
        .unwrap_or(DEFAULT_ENVELOPE_WINDOW_MS)
        .max(0.1);
    let transient_ms = transient_window_ms
        .unwrap_or(DEFAULT_TRANSIENT_WINDOW_MS)
        .max(0.1);

    let (mono_t, sr_t, mut warnings) =
        resolve_source_mono(session, sample_library, shared, &target, scope)?;
    let (mono_c, sr_c, c_warnings) =
        resolve_source_mono(session, sample_library, shared, &candidate, scope)?;
    warnings.extend(c_warnings);

    // Build one side's ADSR + transient breakdown and hand back its RMS contour
    // (for the DTW below). note-off defaults to the whole buffer (release = 0).
    let side = |mono: &[f32], sr: u32| -> (synth_mcp::types::EnvelopeSide, Vec<f32>) {
        let env = analysis::rms_envelope(mono, sr, window_ms);
        let dur_ms = if sr > 0 {
            (mono.len() as f32 / sr as f32) * 1000.0
        } else {
            0.0
        };
        let note_dur = note_duration_ms.unwrap_or_else(|| dur_ms.max(0.0) as u32);
        let est = analysis::envelope_estimate(&env, window_ms, note_dur);
        let tr = analysis::transient_metrics(mono, sr, transient_ms);
        let peak_rms = env.iter().copied().fold(0.0_f32, f32::max);
        let s = synth_mcp::types::EnvelopeSide {
            attack_ms: est.attack_ms,
            decay_ms: est.decay_ms,
            sustain_level: est.sustain_level,
            release_ms: est.release_ms,
            crest_factor_db: tr.crest_factor_db,
            energy_rise_db: tr.energy_rise_db,
            peak_rms,
            duration_ms: dur_ms,
            num_windows: env.len(),
        };
        (s, env)
    };

    let (target_side, env_t) = side(&mono_t, sr_t);
    let (candidate_side, env_c) = side(&mono_c, sr_c);

    if env_t.len() > MAX_DTW_WINDOWS || env_c.len() > MAX_DTW_WINDOWS {
        warnings.push(format!(
            "Envelope longer than {MAX_DTW_WINDOWS} windows was strided down for DTW; \
             raise envelope_window_ms for full-resolution warping."
        ));
    }
    let dtw_distance = analysis::dtw_distance(
        &stride_to(&env_t, MAX_DTW_WINDOWS),
        &stride_to(&env_c, MAX_DTW_WINDOWS),
    );

    Ok(synth_mcp::types::CompareEnvelopesResult {
        dtw_distance,
        attack_delta_ms: candidate_side.attack_ms - target_side.attack_ms,
        decay_delta_ms: candidate_side.decay_ms - target_side.decay_ms,
        sustain_delta: candidate_side.sustain_level - target_side.sustain_level,
        release_delta_ms: candidate_side.release_ms - target_side.release_ms,
        crest_factor_delta_db: candidate_side.crest_factor_db - target_side.crest_factor_db,
        energy_rise_delta_db: candidate_side.energy_rise_db - target_side.energy_rise_db,
        target: target_side,
        candidate: candidate_side,
        warnings,
    })
}

#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_mix_bus_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    include_per_track: Option<bool>,
    scope: synth_core::AnalysisScope,
) -> Result<AnalyzeMixBusResult, McpBridgeError> {
    let (start, end) = resolve_duration_window(shared, duration_seconds, start_tick)?;

    let rendered = crate::audio::arrangement_render::render_arrangement_to_buffer_with_scope(
        session,
        sample_library,
        &shared.song,
        start,
        end,
        scope,
    )?;
    let analysis =
        crate::audio::mix_analysis::analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    let metrics =
        mix_metrics_from_analysis(&analysis, rendered.sample_rate, rendered.duration_seconds);
    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(rendered.start_tick));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(rendered.start_tick), ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(rendered.end_tick), ts);

    let mut warnings = rendered.warnings;
    let per_track = if include_per_track.unwrap_or(false) {
        render_per_track_contributions(
            session,
            sample_library,
            shared,
            Tick(rendered.start_tick),
            Tick(rendered.end_tick),
            scope,
            &mut warnings,
        )?
    } else {
        Vec::new()
    };

    Ok(AnalyzeMixBusResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: Tick(rendered.start_tick),
        end_tick: Tick(rendered.end_tick),
        metrics,
        per_track,
        signal_chain: describe_signal_chain(scope, session.state().master_volume.load()),
        warnings,
    })
}

/// `analyze_master_chain` bridge implementation. Reconstructs the live master
/// effect chain in one reused offline session, then renders the master output
/// repeatedly with the chain truncated to successive prefixes: prefix 0 is the
/// chain input (post-return mix, no master effects), prefix `k` is the output
/// after the first `k` effects. Each effect's stage metrics are the prefix-`k`
/// render; its deltas are versus the prefix-`(k-1)` render. The master chain is
/// always loaded regardless of the incoming `scope`; `scope` only governs the
/// return-bus wet signal and render sample rate.
#[doc(hidden)]
pub fn analyze_master_chain_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    scope: synth_core::AnalysisScope,
) -> Result<AnalyzeMasterChainResult, McpBridgeError> {
    let (start, end) = resolve_duration_window(shared, duration_seconds, start_tick)?;

    // Label data for each stage. Read the live master chain in its declared
    // order; this is the same order `load_master_effects_into_offline` replays.
    let master_effects: Vec<synth_engine::shared_state::ReturnEffectSnapshot> =
        session.state().master_effects.read().clone();

    // The master chain must always be reconstructed — it is the subject of the
    // analysis. `scope` only selects the surrounding stages (return wet signal,
    // sample rate).
    let chain_scope = synth_core::AnalysisScope {
        master_effects: true,
        return_effects: scope.return_effects,
        render_sample_rate: scope.render_sample_rate,
    };

    let (mut engine_session, setup_warnings) =
        crate::audio::arrangement_render::OfflineEngineSession::new_with_scope(
            session,
            sample_library,
            chain_scope,
        )?;
    let mut warnings = setup_warnings;

    // Prefix 0 = the chain input (post-return mix, before any master effect).
    engine_session.set_master_effect_prefix(Some(0));
    let (input_metrics, rendered_start, rendered_end) =
        render_range_to_metrics(&mut engine_session, &shared.song, start, end, &mut warnings)?;

    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(rendered_start));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(rendered_start), ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(rendered_end), ts);

    let mut stages = Vec::with_capacity(master_effects.len());
    let mut prev = input_metrics;
    for (idx, eff) in master_effects.iter().enumerate() {
        engine_session.set_master_effect_prefix(Some(idx + 1));
        let (metrics, _, _) =
            render_range_to_metrics(&mut engine_session, &shared.song, start, end, &mut warnings)?;
        stages.push(synth_mcp::types::MasterEffectStage {
            module_id: eff.module_id.to_string(),
            effect_type: eff.module_type.prefix().to_string(),
            bypassed: eff.bypassed,
            lufs_delta: metrics.lufs_integrated - prev.lufs_integrated,
            peak_delta_db: metrics.peak_dbfs - prev.peak_dbfs,
            true_peak_delta_db: metrics.true_peak_dbtp - prev.true_peak_dbtp,
            rms_delta_db: metrics.rms_dbfs - prev.rms_dbfs,
            stereo_width_delta: metrics.stereo_width - prev.stereo_width,
            crest_delta_db: metrics.crest_factor_db - prev.crest_factor_db,
            gain_reduction_db: prev.rms_dbfs - metrics.rms_dbfs,
            metrics,
        });
        prev = metrics;
    }

    if stages.is_empty() {
        warnings.push("master chain has no effects — output equals input".to_string());
    }

    // `prev` holds the last stage's metrics, or the input metrics when the
    // chain is empty — either way the true master output.
    let output_metrics = prev;

    Ok(AnalyzeMasterChainResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: Tick(rendered_start),
        end_tick: Tick(rendered_end),
        input_metrics,
        output_metrics,
        stages,
        signal_chain: describe_signal_chain(chain_scope, session.state().master_volume.load()),
        warnings,
    })
}

/// `analyze_return_busses` bridge implementation. Renders the full master mix
/// once, then re-renders with each return bus muted in turn (against a cloned
/// song so the live project is untouched) and reports the full−muted deltas as
/// each return's marginal contribution. Return-bus effect chains are always
/// reconstructed; `scope` only governs the master chain and sample rate.
#[doc(hidden)]
pub fn analyze_return_busses_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    scope: synth_core::AnalysisScope,
) -> Result<AnalyzeReturnBussesResult, McpBridgeError> {
    let (start, end) = resolve_duration_window(shared, duration_seconds, start_tick)?;

    // Enumerate return busses (id + name) in declared order.
    let (return_busses, has_bus_to_bus): (Vec<(u16, String)>, bool) = {
        let song = shared.song.read();
        let busses: Vec<(u16, String)> = song
            .return_busses()
            .iter()
            .map(|b| (b.id.0, b.name.clone()))
            .collect();
        // Bus-to-bus sends break the parallel-contribution model: muting a
        // return that feeds another also removes its downstream-routed signal.
        let has_bus_to_bus = song
            .return_busses()
            .iter()
            .any(|b| b.sends.iter().any(|s| s.enabled));
        (busses, has_bus_to_bus)
    };

    // Return-bus effect chains are the subject of the analysis, so always load
    // them. `scope` only selects the surrounding stages.
    let chain_scope = synth_core::AnalysisScope {
        master_effects: scope.master_effects,
        return_effects: true,
        render_sample_rate: scope.render_sample_rate,
    };

    let (mut engine_session, setup_warnings) =
        crate::audio::arrangement_render::OfflineEngineSession::new_with_scope(
            session,
            sample_library,
            chain_scope,
        )?;
    let mut warnings = setup_warnings;
    if has_bus_to_bus {
        warnings.push(
            "bus-to-bus sends present: a return's delta includes the signal it routes \
             into other returns, so the per-return contributions are not independent and \
             do not sum to the full mix."
                .to_string(),
        );
    }

    // Full mix (every return active) — rendered against the live song directly.
    let (full_metrics, rendered_start, rendered_end) =
        render_range_to_metrics(&mut engine_session, &shared.song, start, end, &mut warnings)?;

    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(rendered_start));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(rendered_start), ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(rendered_end), ts);

    let mut returns = Vec::with_capacity(return_busses.len());
    if return_busses.is_empty() {
        warnings.push("song has no return busses — nothing to analyze".to_string());
    } else {
        // Clone the song once; each variant re-clones from this base and mutes
        // exactly one return so the user's project is never mutated.
        let base_song = shared.song.read().clone();
        for (rid, rname) in &return_busses {
            let mut variant = base_song.clone();
            if let Some(bus) = variant.return_bus_mut(synth_sequencer::ReturnBusId(*rid)) {
                bus.mute = true;
            }
            let muted_song = synth_engine::shared_song(variant);
            let (muted, _, _) = render_range_to_metrics(
                &mut engine_session,
                &muted_song,
                start,
                end,
                &mut warnings,
            )?;
            returns.push(synth_mcp::types::ReturnBusContribution {
                return_id: ReturnBusId(*rid),
                return_name: rname.clone(),
                lufs_delta: full_metrics.lufs_integrated - muted.lufs_integrated,
                peak_delta_db: full_metrics.peak_dbfs - muted.peak_dbfs,
                true_peak_delta_db: full_metrics.true_peak_dbtp - muted.true_peak_dbtp,
                rms_delta_db: full_metrics.rms_dbfs - muted.rms_dbfs,
                stereo_width_delta: full_metrics.stereo_width - muted.stereo_width,
            });
        }
    }

    Ok(AnalyzeReturnBussesResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: Tick(rendered_start),
        end_tick: Tick(rendered_end),
        full_metrics,
        returns,
        signal_chain: describe_signal_chain(chain_scope, session.state().master_volume.load()),
        warnings,
    })
}

/// `compare_mix_before_after` bridge implementation. `capture` renders the mix
/// and stores its metrics + render settings in the session's `mix_baseline`;
/// `compare` re-renders with the stored settings (so the A/B is apples-to-apples)
/// and reports `current − baseline` deltas. The baseline is transient session
/// state, never persisted to the project.
#[allow(clippy::too_many_arguments)]
#[doc(hidden)]
pub fn compare_mix_before_after_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    action: &str,
    duration_seconds: f32,
    start_tick: Option<Tick>,
    label: Option<String>,
    scope: synth_core::AnalysisScope,
) -> Result<CompareMixResult, McpBridgeError> {
    match action {
        "capture" => {
            let rendered = analyze_mix_bus_impl(
                session,
                sample_library,
                shared,
                duration_seconds,
                start_tick,
                None,
                scope,
            )?;
            let label = label.unwrap_or_else(|| "baseline".to_string());
            *shared
                .mix_baseline
                .lock()
                .map_err(|_| McpBridgeError::Other("mix-baseline lock poisoned".to_string()))? =
                Some(crate::mcp_shared::MixBaseline {
                    label: label.clone(),
                    metrics: rendered.metrics,
                    duration_seconds,
                    start_tick: start_tick.map(|tick| tick.0),
                    scope,
                });
            Ok(CompareMixResult {
                action: "capture".to_string(),
                label,
                message: "Mix baseline captured. Make your change, then call \
                          compare_mix_before_after with action=compare."
                    .to_string(),
                baseline_metrics: rendered.metrics,
                current_metrics: None,
                deltas: None,
                signal_chain: rendered.signal_chain,
                warnings: rendered.warnings,
            })
        }
        "compare" => {
            // Snapshot the baseline, then release the lock before rendering.
            let baseline = shared
                .mix_baseline
                .lock()
                .map_err(|_| McpBridgeError::Other("mix-baseline lock poisoned".to_string()))?
                .clone();
            let Some(baseline) = baseline else {
                return Err(McpBridgeError::Other(
                    "No mix baseline captured — call compare_mix_before_after with \
                     action=capture first."
                        .to_string(),
                ));
            };

            // Re-render with the baseline's settings so the comparison is exact.
            let current = analyze_mix_bus_impl(
                session,
                sample_library,
                shared,
                baseline.duration_seconds,
                baseline.start_tick.map(Tick),
                None,
                baseline.scope,
            )?;
            let b = &baseline.metrics;
            let c = &current.metrics;
            let deltas = MixDelta {
                lufs_delta: c.lufs_integrated - b.lufs_integrated,
                peak_delta_db: c.peak_dbfs - b.peak_dbfs,
                true_peak_delta_db: c.true_peak_dbtp - b.true_peak_dbtp,
                rms_delta_db: c.rms_dbfs - b.rms_dbfs,
                crest_delta_db: c.crest_factor_db - b.crest_factor_db,
                stereo_width_delta: c.stereo_width - b.stereo_width,
                mono_compat_delta: c.mono_compat - b.mono_compat,
            };
            // The dBFS fields floor at -200 for silence; a delta against a
            // silent side is a floor artifact, not a real loudness change.
            const SILENCE_FLOOR_DBFS: f32 = -190.0;
            let mut warnings = current.warnings;
            if b.lufs_integrated <= SILENCE_FLOOR_DBFS || c.lufs_integrated <= SILENCE_FLOOR_DBFS {
                warnings.push(
                    "baseline or current mix is effectively silent — the dB deltas reflect the \
                     -200 dBFS floor, not a real loudness change."
                        .to_string(),
                );
            }
            Ok(CompareMixResult {
                action: "compare".to_string(),
                label: baseline.label.clone(),
                message: format!(
                    "Compared current mix against baseline '{}': {:+.1} LUFS, {:+.1} dB true peak.",
                    baseline.label, deltas.lufs_delta, deltas.true_peak_delta_db
                ),
                baseline_metrics: baseline.metrics,
                current_metrics: Some(current.metrics),
                deltas: Some(deltas),
                signal_chain: current.signal_chain,
                warnings,
            })
        }
        other => Err(McpBridgeError::Other(format!(
            "Unknown action '{other}' — expected 'capture' or 'compare'."
        ))),
    }
}

/// `auto_gain_stage` bridge implementation. Renders the full master chain once,
/// measures integrated LUFS + true peak, and sets the master fader to reach
/// `target_lufs` without breaching `true_peak_ceiling_dbtp`. Because the master
/// fader sits after all effects, loudness and peak scale linearly with it, so a
/// single render is enough — the post-adjustment figures are predicted, not
/// re-rendered.
#[allow(clippy::too_many_arguments)]
pub(super) fn auto_gain_stage_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    target_lufs: f32,
    true_peak_ceiling_dbtp: f32,
    duration_seconds: f32,
    start_tick: Option<Tick>,
) -> Result<AutoGainStageResult, McpBridgeError> {
    // Measure the real output: include the master + return effect chains.
    let scope = synth_core::AnalysisScope {
        master_effects: true,
        return_effects: true,
        ..synth_core::AnalysisScope::default()
    };
    let measured = analyze_mix_bus_impl(
        session,
        sample_library,
        shared,
        duration_seconds,
        start_tick,
        None,
        scope,
    )?;

    let measured_lufs = measured.metrics.lufs_integrated;
    let measured_tp = measured.metrics.true_peak_dbtp;
    if !measured_lufs.is_finite() || measured_lufs < -100.0 {
        return Err(McpBridgeError::Other(
            "mix is silent (no measurable loudness) — nothing to gain-stage".to_string(),
        ));
    }

    let current_master = session.state().master_volume.load();
    if current_master <= 0.0 {
        return Err(McpBridgeError::Other(
            "master volume is 0 — raise it before auto gain-staging".to_string(),
        ));
    }

    // Gain the target loudness asks for, vs. the headroom the ceiling allows.
    let gain_for_lufs = target_lufs - measured_lufs;
    let gain_headroom = true_peak_ceiling_dbtp - measured_tp;
    let (mut applied_gain_db, mut limited_by) = if gain_for_lufs <= gain_headroom {
        (gain_for_lufs, "target_lufs")
    } else {
        (gain_headroom, "true_peak_ceiling")
    };

    // Apply to the master fader, clamped to the engine's 0..2 range.
    let raw_new_master = current_master * 10f32.powf(applied_gain_db / 20.0);
    let new_master = raw_new_master.clamp(0.0, 2.0);
    if (new_master - raw_new_master).abs() > f32::EPSILON {
        limited_by = "master_volume_range";
        applied_gain_db = synth_core::Decibels::from_linear(new_master / current_master).as_f32();
    }

    if session
        .command_sender()
        .send(EngineCommand::SetMasterVolume(synth_core::Gain::new(
            new_master,
        )))
    {
        Ok(AutoGainStageResult {
            measured_lufs,
            measured_true_peak_dbtp: measured_tp,
            target_lufs,
            true_peak_ceiling_dbtp,
            applied_gain_db,
            previous_master_volume: current_master,
            new_master_volume: new_master,
            predicted_lufs: measured_lufs + applied_gain_db,
            predicted_true_peak_dbtp: measured_tp + applied_gain_db,
            limited_by: limited_by.to_string(),
            warnings: measured.warnings,
        })
    } else {
        Err(McpBridgeError::CommandSendFailed {
            command: "SetMasterVolume",
        })
    }
}

/// `analyze_section` bridge implementation. Renders an explicit
/// `[start_tick, end_tick)` arrangement range offline and returns
/// mix-bus metrics. When `include_per_track` is true, also re-renders each
/// audible track soloed in turn and returns per-track contribution metrics.
#[doc(hidden)]
pub fn analyze_section_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: Tick,
    end_tick: Tick,
    include_per_track: Option<bool>,
    scope: synth_core::AnalysisScope,
) -> Result<AnalyzeSectionResult, McpBridgeError> {
    let rendered = crate::audio::arrangement_render::render_arrangement_to_buffer_with_scope(
        session,
        sample_library,
        &shared.song,
        start_tick.0,
        end_tick.0,
        scope,
    )?;
    let analysis =
        crate::audio::mix_analysis::analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    let metrics =
        mix_metrics_from_analysis(&analysis, rendered.sample_rate, rendered.duration_seconds);
    let ts = shared
        .song
        .read()
        .time_signature_at(synth_sequencer::Tick(rendered.start_tick));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(rendered.start_tick), ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(rendered.end_tick), ts);

    let mut warnings = rendered.warnings;
    let per_track = if include_per_track.unwrap_or(false) {
        render_per_track_contributions(
            session,
            sample_library,
            shared,
            start_tick,
            end_tick,
            scope,
            &mut warnings,
        )?
    } else {
        Vec::new()
    };

    Ok(AnalyzeSectionResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: Tick(rendered.start_tick),
        end_tick: Tick(rendered.end_tick),
        metrics,
        per_track,
        signal_chain: describe_signal_chain(scope, session.state().master_volume.load()),
        warnings,
    })
}

/// Resolve which tracks have placements overlapping the section and
/// re-render each one soloed against a cloned song. Warnings from each
/// soloed render are accumulated into `warnings`.
/// One soloed-track contribution paired with the warnings its render produced.
type ContributionWithWarnings = (synth_mcp::types::TrackContribution, Vec<String>);

fn render_per_track_contributions(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    start_tick: Tick,
    end_tick: Tick,
    scope: synth_core::AnalysisScope,
    warnings: &mut Vec<String>,
) -> Result<Vec<synth_mcp::types::TrackContribution>, McpBridgeError> {
    use synth_sequencer::TrackId;

    struct TargetMeta {
        track_id: TrackId,
        name: String,
        instrument_id: Option<InstrumentId>,
    }

    let (targets, base_song) = {
        let song = shared.song.read();
        let any_solo = song.any_solo();
        let mut covered: std::collections::HashSet<TrackId> = std::collections::HashSet::new();
        for placement in song.placements_in_range(start_tick, end_tick) {
            covered.insert(placement.track_id);
        }
        let mut targets: Vec<TargetMeta> = covered
            .into_iter()
            .filter_map(|tid| {
                let t = song.track(tid)?;
                t.is_audible(any_solo).then_some(TargetMeta {
                    track_id: tid,
                    name: t.name.clone(),
                    instrument_id: Some(t.instrument),
                })
            })
            .collect();
        targets.sort_by_key(|t| t.track_id.0);
        (targets, song.clone())
    };

    if targets.is_empty() {
        warnings.push(
            "include_per_track requested but no audible tracks overlap the section".to_string(),
        );
        return Ok(Vec::new());
    }

    // Snapshot each instrument's pan + volume so we can analytically reverse
    // their attenuation when computing `pre_master_peak`. The realtime engine
    // applies pan-law and per-instrument volume at the mix-down stage; the
    // soloed render already contains the resulting attenuated signal, so a
    // single division by (volume × max_pan_gain) on the loud-channel peak
    // recovers the patch's internal-signal peak. Saves a second render per
    // track vs. re-rendering with pan/volume overridden.
    let instrument_gains: std::collections::HashMap<InstrumentId, (Gain, BipolarValue)> = {
        let snapshots = session.state().instrument_snapshots.read();
        snapshots
            .iter()
            .map(|s| (s.id, (s.volume, s.pan)))
            .collect()
    };

    // Amortize the expensive engine build (which loads every instrument + its
    // sample data) by reusing ONE offline session across each chunk's tracks
    // instead of rebuilding per track — ≈`num_threads` builds rather than one
    // per track. `render_range` fully resets between calls (Stop + voice drain
    // + voice drain + re-attach song + return-bus rebuild), so reusing a
    // session across solo variants is bit-exact to a fresh one — the
    // `arrangement_render_determinism` test covers consecutive-render equality.
    // Chunks still render in parallel across threads; determinism also rests on
    // the §8.1 Round-2 BTreeMap ordering in `synth_engine::graph`.
    use rayon::prelude::*;

    let num_threads = rayon::current_num_threads().max(1);
    let chunk_size = targets.len().div_ceil(num_threads).max(1);

    let chunk_results: Result<Vec<Vec<ContributionWithWarnings>>, McpBridgeError> = targets
        .par_chunks(chunk_size)
        .enumerate()
        .map(|(chunk_idx, chunk)| -> Result<Vec<_>, McpBridgeError> {
            let (mut engine_session, setup_warnings) =
                crate::audio::arrangement_render::OfflineEngineSession::new_with_scope(
                    session,
                    sample_library,
                    scope,
                )?;

            // Engine-level setup warnings are identical for every chunk (same
            // live session), so emit them exactly once — attached to chunk 0's
            // first track via `mem::take` (empty for all later iterations).
            let mut pending_setup_warnings = if chunk_idx == 0 {
                setup_warnings
            } else {
                Vec::new()
            };

            // Clone the (potentially large) song once per chunk, not per track:
            // the chunk's renders run sequentially on this thread, so we reuse
            // one `Song` and just flip the solo flags between renders. The
            // engine read-locks the song only during `render_range`, and we hold
            // the brief write lock between renders, so there is no contention.
            let chunk_song = synth_engine::shared_song(base_song.clone());

            let mut chunk_out = Vec::with_capacity(chunk.len());
            for target in chunk {
                chunk_song.write().set_solo_only(target.track_id);

                let rendered =
                    engine_session.render_range(&chunk_song, start_tick.0, end_tick.0)?;

                let mut per_target_warnings = std::mem::take(&mut pending_setup_warnings);
                for w in &rendered.warnings {
                    per_target_warnings
                        .push(format!("{}({}): {w}", target.name, target.track_id.0));
                }

                let analysis = crate::audio::mix_analysis::analyze_mix_buffer(
                    &rendered.samples,
                    rendered.sample_rate,
                );
                let metrics = mix_metrics_from_analysis(
                    &analysis,
                    rendered.sample_rate,
                    rendered.duration_seconds,
                );
                let (pre_master_peak, pre_master_peak_dbfs) = pre_master_peak_for(
                    target
                        .instrument_id
                        .and_then(|id| instrument_gains.get(&id)),
                    analysis.peak_left,
                    analysis.peak_right,
                );

                chunk_out.push((
                    synth_mcp::types::TrackContribution {
                        track_id: target.track_id,
                        track_name: target.name.clone(),
                        instrument_id: target.instrument_id,
                        metrics,
                        pre_master_peak,
                        pre_master_peak_dbfs,
                        rms_share: 0.0,
                    },
                    per_target_warnings,
                ));
            }
            Ok(chunk_out)
        })
        .collect();

    let mut contributions: Vec<synth_mcp::types::TrackContribution> =
        Vec::with_capacity(targets.len());
    for chunk_out in chunk_results? {
        for (c, ws) in chunk_out {
            contributions.push(c);
            warnings.extend(ws);
        }
    }

    let total_rms: f32 = contributions.iter().map(|c| c.metrics.rms).sum();
    if total_rms > 0.0 {
        for c in contributions.iter_mut() {
            c.rms_share = (c.metrics.rms / total_rms).clamp(0.0, 1.0);
        }
    }

    Ok(contributions)
}

/// Reverse the engine's `volume × pan_gain` attenuation to recover the
/// instrument's pre-mix peak from the soloed render's per-channel peaks.
///
/// Constant-power pan-law gives `(gL, gR) = Gain::from_pan(pan)`; each channel
/// in the soloed render is `internal × volume × g_channel`. Dividing the
/// per-channel peak by its own gain reverses both effects in one step; we
/// take the larger of the two to handle hard-panned signals where one channel
/// is silent.
///
/// Returns `(linear, dBFS)`. When the instrument's gains are unknown (no
/// matching snapshot) or fully zero, falls back to the larger of the two raw
/// channel peaks so the caller still sees a usable lower bound.
pub(super) fn pre_master_peak_for(
    gains: Option<&(Gain, BipolarValue)>,
    peak_left: f32,
    peak_right: f32,
) -> (f32, f32) {
    let raw_peak = peak_left.max(peak_right);
    let restored = match gains {
        Some((volume, pan)) => {
            let v = volume.as_f32();
            let (gl, gr) = Gain::from_pan(*pan);
            let combined_l = v * gl.as_f32();
            let combined_r = v * gr.as_f32();
            let left = if combined_l > 1e-6 {
                peak_left / combined_l
            } else {
                0.0
            };
            let right = if combined_r > 1e-6 {
                peak_right / combined_r
            } else {
                0.0
            };
            let max = left.max(right);
            if max > 0.0 { max } else { raw_peak }
        }
        None => raw_peak,
    };
    (restored, crate::audio::mix_analysis::lin_to_db(restored))
}

/// Frequency band definitions for `analyze_masking_matrix`, matching the
/// `AnalyzeEnergyBands` split used by `analyze_mix_buffer`.
const MASKING_BAND_DEFS: &[(&str, f32, f32)] = &[
    ("sub", 0.0, 100.0),
    ("low", 100.0, 500.0),
    ("mid", 500.0, 2000.0),
    ("high", 2000.0, 20_000.0),
];

/// Minimum overlap energy (linear RMS) at which a band is loud enough to
/// warrant a textual hint. Below this the bands are reported but no hint is
/// generated — every multi-track section would otherwise produce noise.
const MASKING_HINT_MIN_ENERGY: f32 = 0.01;

/// Dominance margin in dB above which we call one track a "masker" of the
/// other on the worst-overlap band. Below this the pair is reported as an
/// even competition.
const MASKING_DOMINANCE_DB_THRESHOLD: f32 = 6.0;

/// Soloed-render RMS (dBFS) below which a track is treated as effectively
/// silent in the window and excluded from the pair matrix. Without this gate two
/// tracks that both sit at the renderer noise floor (~-85 dBFS) produce
/// near-identical band energies whose ratio normalizes to a spurious
/// `conflict_score` of 1.0, burying the genuine audible conflicts. -60 dBFS is
/// far below any part that actually contributes to the mix, so real (if quiet)
/// material is kept while noise-floor silence is dropped.
pub(super) const MASKING_SILENCE_FLOOR_DBFS: f32 = -60.0;

fn masking_band_overlap(name: &str, lo: f32, hi: f32, a: f32, b: f32) -> BandOverlap {
    let lower = a.min(b).max(0.0);
    let upper = a.max(b).max(0.0);
    let dominance_db = if upper < 1e-9 {
        0.0
    } else if lower < 1e-9 {
        200.0
    } else {
        crate::audio::mix_analysis::lin_to_db(upper / lower).min(200.0)
    };
    BandOverlap {
        band: name.to_string(),
        freq_low_hz: lo,
        freq_high_hz: hi,
        track_a_energy: a,
        track_b_energy: b,
        overlap_energy: lower,
        dominance_db,
    }
}

pub(super) fn masking_pair_bands(
    a: &synth_mcp::types::AnalyzeEnergyBands,
    b: &synth_mcp::types::AnalyzeEnergyBands,
) -> Vec<BandOverlap> {
    let aa = [a.sub, a.low, a.mid, a.high];
    let bb = [b.sub, b.low, b.mid, b.high];
    MASKING_BAND_DEFS
        .iter()
        .enumerate()
        .map(|(i, (n, lo, hi))| masking_band_overlap(n, *lo, *hi, aa[i], bb[i]))
        .collect()
}

pub(super) fn masking_conflict_score(bands: &[BandOverlap]) -> f32 {
    let overlap_total: f32 = bands.iter().map(|b| b.overlap_energy).sum();
    let max_total: f32 = bands
        .iter()
        .map(|b| b.track_a_energy.max(b.track_b_energy))
        .sum();
    if max_total > 1e-9 {
        (overlap_total / max_total).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Returns `(hint, dominant_track_id)` for the worst-overlap band, or
/// `(None, None)` when no band crosses `MASKING_HINT_MIN_ENERGY`. The
/// `dominant_track_id` is set only when the margin exceeds
/// `MASKING_DOMINANCE_DB_THRESHOLD`.
fn masking_hint_for_pair(
    bands: &[BandOverlap],
    a_name: &str,
    a_id: TrackId,
    b_name: &str,
    b_id: TrackId,
) -> (Option<String>, Option<TrackId>) {
    let Some(worst) = bands.iter().max_by(|x, y| {
        x.overlap_energy
            .partial_cmp(&y.overlap_energy)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return (None, None);
    };
    if worst.overlap_energy < MASKING_HINT_MIN_ENERGY {
        return (None, None);
    }
    let band_label = format!(
        "{} ({:.0}-{:.0} Hz)",
        worst.band, worst.freq_low_hz, worst.freq_high_hz
    );
    if worst.dominance_db >= MASKING_DOMINANCE_DB_THRESHOLD {
        let (masker_name, masker_id, masked_name, masked_id) =
            if worst.track_a_energy >= worst.track_b_energy {
                (a_name, a_id, b_name, b_id)
            } else {
                (b_name, b_id, a_name, a_id)
            };
        (
            Some(format!(
                "{masker_name}({masker_id}) masks {masked_name}({masked_id}) in {band_label}"
            )),
            Some(masker_id),
        )
    } else {
        (
            Some(format!(
                "{a_name}({a_id}) and {b_name}({b_id}) compete in {band_label}"
            )),
            None,
        )
    }
}

fn build_masking_pairs(contributions: &[synth_mcp::types::TrackContribution]) -> Vec<MaskingPair> {
    let n = contributions.len();
    let mut pairs: Vec<MaskingPair> = Vec::with_capacity(n.saturating_mul(n.saturating_sub(1)) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let (lo, hi) = if contributions[i].track_id <= contributions[j].track_id {
                (&contributions[i], &contributions[j])
            } else {
                (&contributions[j], &contributions[i])
            };
            let bands = masking_pair_bands(&lo.metrics.energy_bands, &hi.metrics.energy_bands);
            let conflict_score = masking_conflict_score(&bands);
            let (hint, dominant_track_id) = masking_hint_for_pair(
                &bands,
                &lo.track_name,
                lo.track_id,
                &hi.track_name,
                hi.track_id,
            );
            pairs.push(MaskingPair {
                track_a_id: lo.track_id,
                track_a_name: lo.track_name.clone(),
                track_b_id: hi.track_id,
                track_b_name: hi.track_name.clone(),
                bands,
                conflict_score,
                dominant_track_id,
                hint,
            });
        }
    }
    pairs.sort_by(|x, y| {
        y.conflict_score
            .partial_cmp(&x.conflict_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs
}

/// Default cap on returned pairs. The pair count is O(N²) in audible
/// tracks; without truncation a typical 18-track section emits 153 pair
/// objects with full per-band data, which exceeds the MCP response size
/// limit. `total_pair_count` in the result preserves the unclamped count.
const MASKING_TOP_PAIRS_DEFAULT: u32 = 20;
const MASKING_TOP_PAIRS_HARD_CAP: u32 = 200;

/// `analyze_masking_matrix` bridge implementation. Renders every audible
/// track soloed, then computes pairwise per-band overlap and an optional
/// textual hint. Returns pairs sorted by descending `conflict_score`,
/// truncated to `top_pairs` (default 20).
#[doc(hidden)]
pub fn analyze_masking_matrix_impl(
    session: &SynthSession,
    sample_library: &crate::audio::preview::SharedSampleLibrary,
    shared: &McpSharedState,
    arrangement_start_tick: Option<Tick>,
    arrangement_end_tick: Option<Tick>,
    top_pairs: Option<u32>,
    scope: synth_core::AnalysisScope,
) -> Result<AnalyzeMaskingMatrixResult, McpBridgeError> {
    let mut warnings: Vec<String> = Vec::new();
    let (start_tick, requested_end_tick, end_tick) = {
        let song = shared.song.read();
        let (start, requested_end) =
            resolve_arrangement_range(&song, arrangement_start_tick, arrangement_end_tick)?;
        // A single offline render is capped at MAX_ANALYSIS_WINDOW_SECONDS, so on
        // a longer requested range every per-track render would silently trim its
        // tail while the reported scope still claimed the full span. Clamp the
        // analyzed window up-front and report the real span (main fields) plus the
        // full request (`requested_end_tick`) so partial coverage is explicit.
        let start_s = song.tick_to_seconds(Tick(start));
        let cap_end = song
            .seconds_to_tick(start_s + f64::from(MAX_ANALYSIS_WINDOW_SECONDS))
            .0;
        (start, requested_end, requested_end.min(cap_end))
    };
    if end_tick < requested_end_tick {
        warnings.push(format!(
            "requested range exceeds the {MAX_ANALYSIS_WINDOW_SECONDS:.0}s render limit; analyzed \
             only the first {MAX_ANALYSIS_WINDOW_SECONDS:.0}s (see start_tick/end_tick; \
             requested_end_tick is the full request)"
        ));
    }

    let contributions = render_per_track_contributions(
        session,
        sample_library,
        shared,
        Tick(start_tick),
        Tick(end_tick),
        scope,
        &mut warnings,
    )?;

    // Drop tracks that are effectively silent in this window before scoring. Two
    // tracks sitting at the renderer noise floor have near-identical band
    // energies, so `masking_conflict_score` would rank them at a spurious 1.0 —
    // burying the genuine audible conflicts. Report them separately so the caller
    // can tell silence apart from missing data.
    let (audible, silent): (Vec<_>, Vec<_>) = contributions
        .into_iter()
        .partition(|c| c.metrics.rms_dbfs >= MASKING_SILENCE_FLOOR_DBFS);
    let mut tracks_below_floor: Vec<synth_mcp::types::TrackBelowFloor> = silent
        .into_iter()
        .map(|c| synth_mcp::types::TrackBelowFloor {
            track_id: c.track_id,
            track_name: c.track_name,
            rms_dbfs: c.metrics.rms_dbfs,
        })
        .collect();
    tracks_below_floor.sort_by_key(|t| t.track_id);
    if !tracks_below_floor.is_empty() {
        warnings.push(format!(
            "{} track(s) below the {:.0} dBFS audibility floor excluded from the masking \
             matrix (see tracks_below_floor)",
            tracks_below_floor.len(),
            MASKING_SILENCE_FLOOR_DBFS
        ));
    }

    if audible.len() < 2 {
        warnings.push(format!(
            "analyze_masking_matrix needs at least 2 audible tracks in the section; got {}",
            audible.len()
        ));
    }

    let mut pairs = build_masking_pairs(&audible);
    let total_pair_count = pairs.len() as u32;
    let cap = top_pairs
        .unwrap_or(MASKING_TOP_PAIRS_DEFAULT)
        .clamp(1, MASKING_TOP_PAIRS_HARD_CAP) as usize;
    if pairs.len() > cap {
        pairs.truncate(cap);
    }

    let ts = shared.song.read().time_signature_at(Tick(start_tick));
    let (start_bar, start_beat) = tick_to_bar_beat_1based(Tick(start_tick), ts);
    let (end_bar, end_beat) = tick_to_bar_beat_1based(Tick(end_tick), ts);
    let (requested_end_bar, _) = tick_to_bar_beat_1based(Tick(requested_end_tick), ts);

    Ok(AnalyzeMaskingMatrixResult {
        start_bar,
        start_beat,
        end_bar,
        end_beat,
        start_tick: Tick(start_tick),
        end_tick: Tick(end_tick),
        requested_end_tick: Tick(requested_end_tick),
        requested_end_bar,
        track_count: audible.len() as u32,
        total_pair_count,
        pairs,
        tracks_below_floor,
        warnings,
    })
}

/// Convert the internal `InstrumentProfile` to the MCP-wire form.
/// The two structs have identical serde shapes (snake_case enum variants),
/// so we go through `serde_json` for the enum→string conversion rather than
/// hand-maintain a parallel `as_str` impl on every enum.
pub(super) fn profile_to_result(
    profile: crate::analysis::InstrumentProfile,
) -> synth_mcp::types::InstrumentProfileResult {
    use synth_mcp::types::{InstrumentProfileResult, ProfileSignalResult, RoleInferenceResult};

    fn enum_to_str<T: serde::Serialize>(v: &T) -> String {
        match serde_json::to_value(v) {
            Ok(serde_json::Value::String(s)) => s,
            _ => String::new(),
        }
    }

    InstrumentProfileResult {
        instrument_id: profile.instrument_id,
        instrument_name: profile.instrument_name,
        role: RoleInferenceResult {
            role: enum_to_str(&profile.role.role),
            confidence: profile.role.confidence,
            signals: profile
                .role
                .signals
                .into_iter()
                .map(|s| ProfileSignalResult {
                    axis: s.axis.as_str().to_string(),
                    detail: s.detail.to_string(),
                })
                .collect(),
        },
        envelope_shape: enum_to_str(&profile.envelope_shape),
        pitch_role: enum_to_str(&profile.pitch_role),
        register: enum_to_str(&profile.register),
        texture: enum_to_str(&profile.texture),
    }
}

mod rendered_buffer;

use rendered_buffer::resolve_envelope_window_ms;
pub use rendered_buffer::{
    DEFAULT_NOTE_ENVELOPE_WINDOW_MS, analyze_rendered_buffer, analyze_rendered_buffer_with_window,
};

/// Format a parameter value for human-readable display.
///
/// When `unit` is provided, defers to `ParameterUnit::format` so the declared
/// unit on the parameter descriptor is honored. When the unit is `None` or
/// `ParameterUnit::None`, falls back to a name-based heuristic that picks a
/// reasonable suffix from the parameter's name. The heuristic should only be
/// reached when the descriptor is unavailable; prefer to pass the unit.
pub(super) fn format_param_display(param: &Param, unit: Option<ParameterUnit>) -> String {
    let value = param.as_f32();

    if let Some(u) = unit
        && u != ParameterUnit::None
    {
        return u.format(value);
    }

    let name_lower = param.name().to_ascii_lowercase();
    if name_lower.contains("frequency") || name_lower.contains("cutoff") || name_lower == "rate" {
        if value >= 1000.0 {
            format!("{:.1} kHz", value / 1000.0)
        } else {
            format!("{value:.1} Hz")
        }
    } else if name_lower.contains("time")
        || name_lower.contains("attack")
        || name_lower.contains("release")
        || name_lower.contains("decay")
    {
        if value >= 1.0 {
            format!("{value:.2} s")
        } else {
            format!("{:.1} ms", value * 1000.0)
        }
    } else if name_lower.contains("volume")
        || name_lower.contains("gain")
        || name_lower.contains("level")
        || name_lower.contains("master")
    {
        format!("{value:.2}")
    } else {
        format!("{value:.3}")
    }
}
