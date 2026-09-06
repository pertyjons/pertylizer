//! Lowering a song's arrangement into the events a V2 plan renders.
//!
//! # What a lowered note is
//!
//! A gate edge and the note's own two magnitudes: its saved pitch, moved by its placement's
//! transpose and validated as a `KeyIdentity`, and its saved velocity, revalidated as a
//! `NoteVelocity`. The engine resolves the key through the plan's prepared tuning and expands
//! the note-on to the control writes its scope declares.
//!
//! # Why the outcome is still not a parity claim
//!
//! V1 applies one saved velocity **twice** — `1 − sensitivity × (1 − velocity)` at the
//! envelope and an independent `velocity_to_amp` at the voice output — and V2 applies it as
//! one scale on the envelope. `SOUND-INV-021` puts that composition on Phase 6 and the work
//! list says closing `P03-R003` "does not decide Phase 6's tuning or expression-composition
//! model". So one [`LoweringReason::OwnedByLaterPhase`] is raised per lowering that places a
//! note, the outcome is [`Fidelity::UnsupportedScope`], and the A/B path refuses to compare it
//! for parity.
//!
//! # Why the tick mapping is direct, and why there is a test for it
//!
//! Both engines count 960 ticks to a quarter note — `synth_sequencer`'s `TICKS_PER_QUARTER`
//! and `synth_engine_v2::tempo::TICKS_PER_QUARTER`. So a saved tick is a V2 musical tick with
//! no conversion at all. That is a coincidence of two independent constants rather than a
//! contract between them, so a test asserts they are equal: if either moves, every position
//! this module computes moves with it and nothing else would notice.
//!
//! # Why overlapping notes are refused
//!
//! A V2 plan reaches one scalar gate. Two notes overlapping on it would have the first
//! release lower the gate while the second occurrence is still held, so the second note ends
//! early and silently. Phase 6 owns voice allocation, which is what makes overlap meaningful;
//! until then the case is refused where the user authored it rather than rendered wrongly.

use synth_core::{BipolarValue, NormalizedValue};
use synth_engine::instrument::InstrumentId;
use synth_engine_v2::ir::NodeId;
use synth_engine_v2::offline::OfflineEvent;
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::{EventCount, KeyIdentity, NoteVelocity, SampleRate};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::tempo::{Bpm as V2Bpm, MusicalTick, TempoChange as V2TempoChange, TempoMap};
use synth_engine_v2::time::{FrameCount, SampleTime};
use synth_sequencer::{PatternId, Song, TrackId};

use super::diagnostics::{Fidelity, LoweringDiagnostic, LoweringReason, ProjectSubject, Severity};

/// A song's arrangement, lowered against one compiled plan.
#[derive(Debug)]
#[must_use]
pub struct LoweredPerformance {
    /// The events, ascending in time. Empty when the lowering was refused.
    pub events: Vec<OfflineEvent>,
    /// How many frames the arrangement occupies, including the last note's release.
    pub frames: FrameCount,
    /// What the lowering has to say.
    pub diagnostics: Vec<LoweringDiagnostic>,
}

impl LoweredPerformance {
    /// Whether a parity comparison may read this.
    pub fn fidelity(&self) -> Fidelity {
        Fidelity::of(&self.diagnostics)
    }

    /// Whether the lowering stopped.
    ///
    /// Derived from the diagnostics rather than stored beside them, so the two cannot
    /// disagree. It matters because a refusal and a genuinely note-free arrangement both
    /// produce an empty event list, and only one of them may go on to render: an earlier
    /// revision read the empty list alone and rendered a refused arrangement's tail.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity() == Severity::Refused)
    }
}

/// One note the arrangement places, in absolute song ticks, with the magnitudes it carries.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: u64,
    end: u64,
    pattern: PatternId,
    note: synth_sequencer::NoteId,
    /// The saved pitch, transposed by its placement, as a V2 key.
    key: KeyIdentity,
    /// The saved velocity, revalidated at this boundary.
    velocity: NoteVelocity,
}

/// Every note this instrument's tracks place, in absolute song ticks.
///
/// Shared by the event lowering and the event-peak calculation, so the two cannot disagree
/// about which notes the plan contains — the peak is what admission is told, and telling it
/// about a different set than the renderer receives is how a plan is admitted for a load it
/// does not carry.
///
/// Returns `None` when a refusal was recorded.
fn note_spans(
    instrument: InstrumentId,
    song: &Song,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<Vec<Span>> {
    // V1's rule: when any track is soloed, only soloed tracks sound.
    let any_soloed = song.tracks().any(|track| track.solo);

    // The song's end as V1 computes it — the later of the last placement's end and the last
    // section's — is where V1's sequencer auto-stops and releases every note it holds. A note
    // whose own duration runs past it is therefore released **there** in V1, so its release is
    // clipped to it here rather than sounded to its authored end. A squash review found the
    // authored end used.
    let song_end = song_end(song, diagnostics)?;

    let mut spans = Vec::new();
    // A track's fader and pan are read once per track rather than once per placement: V1
    // applies them to whatever that track plays, so they are a property of the track.
    let mut reported_tracks: Vec<TrackId> = Vec::new();
    for placement in song.arrangement() {
        let Some(track) = song.track(placement.track_id) else {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Track {
                    track: placement.track_id,
                    name: String::new(),
                },
                LoweringReason::UnresolvedEndpoint {
                    spelling: format!("{:?}", placement.track_id),
                },
            ));
            return None;
        };

        // A placement on another instrument's track is not this plan's to render. Skipping it
        // is the whole reason the track is resolved at all: an earlier revision added every
        // placement in the song to this instrument, which both sounded the wrong notes and
        // raised the single-gate overlap refusal against notes that never collide.
        if track.instrument != instrument {
            continue;
        }
        if track.mute || (any_soloed && !track.solo) {
            continue;
        }

        let subject = || ProjectSubject::Track {
            track: placement.track_id,
            name: track.name.clone(),
        };

        // Every saved track field, dispositioned once per track rather than once per placement.
        if !reported_tracks.contains(&placement.track_id) {
            reported_tracks.push(placement.track_id);
            if !track_dispositions(track, diagnostics) {
                return None;
            }
        }

        let Some(pattern) = song.pattern(placement.pattern_id) else {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Pattern {
                    pattern: placement.pattern_id,
                    name: String::new(),
                },
                LoweringReason::UnresolvedEndpoint {
                    spelling: format!("{:?}", placement.pattern_id),
                },
            ));
            return None;
        };

        // A placement that is never active is skipped whatever it holds, because
        // `PatternPlacement::pattern_tick_at` resolves no tick inside it and V1 neither expands
        // nor automates it. Two ways to be never active, and V1 has both: a zero-length
        // **pattern** yields no pattern tick however long its placement is, and a
        // `length_override` of zero ends the placement where it starts. The second was refused
        // below as an override until the squash review read `pattern_tick_at`.
        if pattern.length.0 == 0 || placement.effective_length(pattern.length).0 == 0 {
            continue;
        }

        // **Validated before it is used in arithmetic.** `Semitones` is a transparent `f32`
        // with a derived `Deserialize`, so a persisted `1e40` arrives as `f32::INFINITY`;
        // `Pitch::transpose` rounds that, saturates the cast to `i16::MAX`, and adds it to a
        // pitch — which overflows and panics in a checked build. An independent review found
        // it. The bound is the keyboard's own width: nothing outside it can move a pitch to
        // another pitch, so a value beyond it is refused rather than saturated into one.
        let transpose = placement.transpose;
        if !transpose.as_f32().is_finite() || transpose.as_f32().abs() > 127.0 {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnsupportedParameterValue {
                    value: format!(
                        "a placement transpose of {} semitones is not a keyboard offset",
                        transpose.as_f32()
                    ),
                },
            ));
            return None;
        }
        if placement.gain != synth_core::Gain::UNITY {
            diagnostics.push(LoweringDiagnostic::unrepresented(
                subject(),
                LoweringReason::OwnedByLaterPhase {
                    capability: "a per-placement gain",
                    owner: "Phase 8",
                },
            ));
        }
        // A length override changes the **note set**, not the level: shorter than its pattern
        // clips the onsets past it, and longer under `Repeat` emits further passes. Lowering
        // the source notes exactly once would sound a stream V1 never plays, so it is refused
        // rather than reported. An independent review found it reported, which contradicted the
        // rule that a note-set change is refused.
        if placement.length_override.is_some() {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::OwnedByLaterPhase {
                    capability: "a placement length override, which clips its pattern's later \
                                 onsets or repeats it for further passes",
                    owner: "Phase 9, with the loop-wrap law ADR-0052 owes",
                },
            ));
            return None;
        }

        // What V1 runs a playing pattern's notes through before it plays them, checked **here**
        // — on a placement that passed the instrument, mute and solo filters — because this is
        // where V1 runs it: `SequencerEngine::collect_events_at_tick` expands a pattern only
        // under `if audible`, and only for a placement it is walking. A rack on a pattern the
        // arrangement never places, or placed only on a muted track, expands nothing V1 plays;
        // an earlier revision refused every pattern in the song for it, and an independent
        // review found the false refusal. A pooled Note Grid graph that no playing pattern
        // binds is inert for the same reason, so the pool itself is not inspected.
        //
        // Precedence is V1's own: a bound graph that resolves in the pool takes precedence
        // over the rack, and a **dangling** binding falls back to the rack — so the graph is
        // resolved through the pool exactly as `pattern.note_graph().and_then(song.note_graph)`
        // does there, rather than read as a bare `Option`. A graph with **no nodes** has
        // nothing to act with — its expansion is the seeded source, untouched — so it is the
        // pass-through V1 makes of it. That is a structural fact rather than derived state:
        // the spine and processing order are recomputed after load and a freshly deserialized
        // graph does not carry them, so reading them here would call a real graph empty.
        //
        // **Where this stops.** A stage is refused when V1 installs it on a placement it walks
        // and it has something to act with: a rack with a processor, a graph with a node. What
        // the stage then computes — a processor over a pattern with no note, a node off the
        // spine — is not evaluated, exactly as a master effect at neutral settings is refused
        // rather than measured. The contract states that rule; it is not a gap to close.
        let pattern_subject = || ProjectSubject::Pattern {
            pattern: pattern.id,
            name: pattern.name.clone(),
        };
        match pattern
            .note_graph()
            .and_then(|graph| song.note_graph(graph))
        {
            Some(graph) if graph.node_count() > 0 => {
                diagnostics.push(LoweringDiagnostic::refused(
                    pattern_subject(),
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a pattern bound to a Note Grid graph, which transforms \
                                     the notes it plays",
                        owner: "Phase 6, with the note-processing model",
                    },
                ));
                return None;
            }
            // A resolved graph is what V1 runs, whether or not it has nodes — the rack is the
            // `None` arm of its `match` and never runs beside a bound graph. So a node-less
            // graph is pass-through **and** shadows the rack; an independent review found the
            // rack refused underneath it.
            Some(_) => {}
            // A note-processor rack expands a pattern's notes exactly as a per-note ornament
            // does — strums, held pitches, ornaments — so a lowering that emitted the authored
            // notes would sound a stream V1 never plays. The per-note refusal below does not
            // reach it, because the rack lives on the pattern rather than on any note it
            // expands. Found by the persisted-field pin, which is the mechanism working rather
            // than a lucky read.
            None => {
                if !pattern.processors().is_empty() {
                    diagnostics.push(LoweringDiagnostic::refused(
                        pattern_subject(),
                        LoweringReason::OwnedByLaterPhase {
                            capability: "a pattern note-processor rack, which V1 expands into \
                                         the notes it plays",
                            owner: "Phase 6, with the note-processing model",
                        },
                    ));
                    return None;
                }
            }
        }

        for note in pattern.notes() {
            let note_subject = ProjectSubject::Note {
                pattern: pattern.id,
                note: note.id,
            };

            // A note-scope graph articulates this one note through a pooled Note Grid graph
            // before the pattern's own stage sees it. Resolved through the pool as V1's
            // `seed_source_at_tick` resolves it: a dangling id is pass-through there, and so is
            // a graph with no nodes. Checked **before** the hidden-note skip below, because V1
            // seeds every note's graph on every active tick regardless of the note's own
            // start: a source-independent generator bound to a note past the pattern's end
            // still emits, so skipping that note first would drop what V1 plays. An independent
            // review found the check on the wrong side of the skip.
            if note
                .note_graph
                .and_then(|graph| song.note_graph(graph))
                .is_some_and(|graph| graph.node_count() > 0)
            {
                diagnostics.push(LoweringDiagnostic::refused(
                    note_subject,
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a note bound to a note-scope Note Grid graph, which \
                                     articulates it before the pattern plays",
                        owner: "Phase 6, with the note-processing model",
                    },
                ));
                return None;
            }

            // An ornament is the second thing V1 evaluates on every active tick regardless of
            // the note's own start: a lead-in figure's grace hits land *before* `note.start`,
            // so a note at the pattern's end with a lead-in ornament sounds inside the pattern
            // although its own onset never does. Refused before the hidden-note skip for the
            // same reason as the note-scope graph; an independent review found it after.
            if note.ornament.is_some() {
                diagnostics.push(LoweringDiagnostic::refused(
                    note_subject,
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a note ornament, which V1 expands before playing and \
                                     whose lead-in hits land before the note's own onset",
                        owner: "Phase 6, with the expression model",
                    },
                ));
                return None;
            }

            // A note at or past its pattern's length is hidden: the sequencer never plays it,
            // because the pattern ends first. Emitting it would sound a note V1 does not,
            // lengthen the render, and can raise a false overlap refusal.
            if note.start.0 >= pattern.length.0 {
                continue;
            }
            // The saved pitch, moved by its placement's transpose. **Applied**, not
            // reported: the payload carries a key now, so a placement transposed by a fifth
            // whose notes lowered untransposed would render the wrong music silently.
            //
            // A result off the keyboard falls back to the **authored** pitch, because that is
            // what V1 does — `sequencer_engine::make_pending_note` writes
            // `.transpose(transpose).unwrap_or(expanded.pitch)`. An earlier revision refused
            // the whole performance here, on the belief that V1 dropped such a note; an
            // independent review read the V1 site and showed it does not. Refusing would also
            // have suppressed every unrelated note in the arrangement.
            let transposed = note.pitch.transpose(transpose).unwrap_or(note.pitch);
            let key = match KeyIdentity::new(transposed.as_midi()) {
                Ok(key) => key,
                Err(error) => {
                    diagnostics.push(LoweringDiagnostic::refused(
                        note_subject,
                        LoweringReason::UnsupportedParameterValue {
                            value: error.to_string(),
                        },
                    ));
                    return None;
                }
            };
            // **What this can and cannot catch, stated rather than assumed.** A persisted
            // velocity outside `[0, 1]` never reaches here as itself: `synth_core::Velocity`
            // deserializes through `From<f32>`, whose constructor clamps, so a saved `2.0`
            // arrives as `1.0` and the substitution happens in the project format's own type
            // before this module sees it. An independent review found this comment claiming
            // otherwise. What **does** survive that clamp is `NaN` — `f32::clamp` returns it
            // unchanged — and that is what this refuses, where a diagnostic is still possible
            // and before it multiplies every sample an envelope emits.
            let velocity = match NoteVelocity::new(note.velocity.as_f32()) {
                Ok(velocity) => velocity,
                Err(error) => {
                    diagnostics.push(LoweringDiagnostic::refused(
                        note_subject,
                        LoweringReason::UnsupportedParameterValue {
                            value: error.to_string(),
                        },
                    ));
                    return None;
                }
            };
            if note.legato || note.glide.is_some() {
                diagnostics.push(LoweringDiagnostic::unrepresented(
                    note_subject.clone(),
                    LoweringReason::OwnedByLaterPhase {
                        capability: "per-note legato or glide",
                        owner: "Phase 6, with the expression model",
                    },
                ));
            }
            // An expression is not decoration: V1 shapes the note with it before playing, and
            // it can suppress the note outright. Emitting the authored span as if it were
            // absent would sound notes V1 does not, and silence none it does. Unlike the
            // ornament above it acts only when the note itself plays, so a hidden note's
            // expression is never read.
            if note.expression.is_some() {
                diagnostics.push(LoweringDiagnostic::refused(
                    note_subject.clone(),
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a note expression, which V1 applies before playing and \
                                     which can suppress the note entirely",
                        owner: "Phase 6, with the expression model",
                    },
                ));
                return None;
            }

            let Some(duration) = note.duration else {
                // A note with no duration never ends, so it has no release to place. V1
                // sustains it to the pattern's end; deciding that here would be inventing a
                // length the project does not state.
                diagnostics.push(LoweringDiagnostic::refused(
                    note_subject,
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a note with no declared duration",
                        owner: "Phase 6",
                    },
                ));
                return None;
            };

            // Checked, because a placement start is persisted and a `u64` sum can wrap. A
            // wrapped release lands before its own onset, which is a note that never ends.
            let Some(start) = placement.start.0.checked_add(u64::from(note.start.0)) else {
                diagnostics.push(LoweringDiagnostic::refused(
                    note_subject,
                    LoweringReason::UnsupportedParameterValue {
                        value: "the note's absolute position does not fit".to_owned(),
                    },
                ));
                return None;
            };
            let Some(end) = start.checked_add(u64::from(duration.0)) else {
                diagnostics.push(LoweringDiagnostic::refused(
                    note_subject,
                    LoweringReason::UnsupportedParameterValue {
                        value: "the note's release does not fit".to_owned(),
                    },
                ));
                return None;
            };
            // Released where V1 releases it. The onset is inside the song by construction —
            // a placement ends no later than the song does — so the clip cannot invert a span.
            let end = end.min(song_end);
            spans.push(Span {
                start,
                end,
                pattern: pattern.id,
                note: note.id,
                key,
                velocity,
            });
        }
    }

    // One gate, so one note at a time. Sorted by start, any span beginning before its
    // predecessor ends is an overlap.
    spans.sort_by_key(|span| (span.start, span.end));
    for window in spans.windows(2) {
        if window[1].start < window[0].end {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Note {
                    pattern: window[1].pattern,
                    note: window[1].note,
                },
                LoweringReason::OwnedByLaterPhase {
                    capability: "two notes sounding at once through one gate",
                    owner: "Phase 6, with voice allocation",
                },
            ));
            return None;
        }
    }
    Some(spans)
}

/// The most note edges this arrangement puts in any one render quantum.
///
/// Admission needs this **before** the plan is compiled, and the plan is needed before an
/// event can name a note slot — so the count is taken from the timeline rather than from the
/// events. Both come from [`note_spans`], so the number admission is told is a count of the
/// same notes the renderer is later given.
///
/// Returns `None` when the arrangement could not be read; the caller lowers anyway and the
/// refusal surfaces there with its subject intact.
pub fn peak_events_per_quantum(
    instrument: InstrumentId,
    song: &Song,
    sample_rate: SampleRate,
) -> Option<EventCount> {
    let mut ignored = Vec::new();
    let spans = note_spans(instrument, song, &mut ignored)?;
    let tempo = lower_tempo(song, sample_rate, &mut ignored).ok()?;

    let mut frames = Vec::with_capacity(spans.len() * 2);
    for span in &spans {
        for tick in [span.start, span.end] {
            frames.push(tempo.position_of(MusicalTick::new(tick)).ok()?.as_u64());
        }
    }
    frames.sort_unstable();

    // The worst case over every anchor phase, counted the way admission counts it: a `Q`-frame
    // window `[first, first + Q)` slid over the sorted edges, rather than a bucket per absolute
    // quantum. Which quantum a frame lands in depends on where the stream is anchored, so a
    // bucketed count answers the wrong question — two edges 25 frames apart across an absolute
    // boundary are one quantum's load after an ordinary seek — and a declaration taken from it
    // would be admitted while its own stream is refused. The squash review found the buckets.
    let quantum = u64::from(synth_engine_v2::time::QUANTUM_FRAMES);
    let mut peak = 0_usize;
    let mut end = 0_usize;
    for (start, first) in frames.iter().copied().enumerate() {
        if end < start {
            end = start;
        }
        while end < frames.len() && frames[end].saturating_sub(first) < quantum {
            end += 1;
        }
        peak = peak.max(end - start);
    }
    Some(EventCount::measured(
        u32::try_from(peak).unwrap_or(u32::MAX),
    ))
}

/// Lower a song's arrangement into events that play `gate`.///
/// `gate` is the node a note plays, which `SOUND-INV-016` makes the node's own choice rather
/// than the caller's: only a kind declaring a note control resolves, and
/// [`CompiledPlan::resolve_note`] is what refuses one that does not.
pub fn lower_performance(
    instrument: InstrumentId,
    // The **instrument's** own name, which is what `ProjectSubject::Instrument` documents its
    // `name` to be. An earlier revision passed the song's, so a diagnostic about an instrument
    // named the project instead; an independent review found it.
    instrument_name: &str,
    song: &Song,
    plan: &CompiledPlan,
    gate: NodeId,
    sample_rate: SampleRate,
) -> LoweredPerformance {
    let mut diagnostics = Vec::new();

    let Some(slot) = plan.resolve_note(gate) else {
        diagnostics.push(LoweringDiagnostic::refused(
            ProjectSubject::Instrument {
                instrument,
                name: instrument_name.to_owned(),
            },
            LoweringReason::OwnedByLaterPhase {
                capability: "a voice patch whose graph declares no node a note can play",
                owner: "Phase 6, with the voice-instantiation model",
            },
        ));
        return refused(diagnostics);
    };

    let tempo = match lower_tempo(song, sample_rate, &mut diagnostics) {
        Ok(tempo) => tempo,
        Err(reason) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue { value: reason },
            ));
            return refused(diagnostics);
        }
    };

    let Some(spans) = note_spans(instrument, song, &mut diagnostics) else {
        return refused(diagnostics);
    };

    // Velocity is V1's since ADR-0059: the envelope lowers with its own sensitivity and the
    // instrument's amp sensitivity lowers to a velocity scaler, so a note renders at V1's
    // product of the two and the marker this site raised — "V1's two velocity sensitivities
    // and how they compose" — is discharged. `P04-R001` closes with it.

    // A second difference, and it is **not** the overlap this lowerer refuses. Overlapping
    // gates are refused above; what remains is that V1 gives each note its own voice, so a
    // release can still ring under the next one, while V2 has a single gate the next note
    // retriggers. The diagnostic names that **shape** rather than asserting a ringing release in
    // any particular arrangement: whether one actually rings depends on the envelope's release
    // against the gap, which this lowerer does not compute — an independent review caught the
    // stronger wording. Raised once per lowering, because it is a property of the single gate.
    if spans.len() > 1 {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            ProjectSubject::Instrument {
                instrument,
                name: instrument_name.to_owned(),
            },
            LoweringReason::OwnedByLaterPhase {
                capability: "two or more notes through one gate, where V1 allocates a voice \
                             per note and lets a release ring under the next while V2 \
                             retriggers its one gate and cuts it",
                owner: "Phase 6, with the voice allocator",
            },
        ));
    }

    let mut events: Vec<OfflineEvent> = Vec::with_capacity(spans.len() * 2);
    let mut last_frame = 0_u64;
    for span in spans {
        for (tick, payload) in [
            (
                span.start,
                CompiledPayload::NoteOn {
                    slot,
                    key: span.key,
                    velocity: span.velocity,
                },
            ),
            (
                span.end,
                CompiledPayload::NoteOff {
                    slot,
                    key: span.key,
                },
            ),
        ] {
            match tempo.position_of(MusicalTick::new(tick)) {
                Ok(position) => {
                    let frame = position.as_u64();
                    last_frame = last_frame.max(frame);
                    events.push(OfflineEvent::new(SampleTime::new(frame), payload));
                }
                Err(error) => {
                    diagnostics.push(LoweringDiagnostic::refused(
                        ProjectSubject::Note {
                            pattern: span.pattern,
                            note: span.note,
                        },
                        LoweringReason::UnsupportedParameterValue {
                            value: error.to_string(),
                        },
                    ));
                    return refused(diagnostics);
                }
            }
        }
    }

    // Ascending, as the offline renderer requires. Sorting spans by start tick does not
    // establish it: a release is emitted beside its own note-on rather than in time order.
    events.sort_by_key(OfflineEvent::time);

    // The arrangement occupies the song as V1 bounds it, not only up to its last release: a
    // trailing rest, or a section drawn past the last placement, is silence V1 renders and
    // this render would otherwise omit. The same `calculate_length` that clips a release
    // above is what extends the frame count here.
    let Some(song_end) = song_end(song, &mut diagnostics) else {
        return refused(diagnostics);
    };
    match tempo.position_of(MusicalTick::new(song_end)) {
        Ok(position) => last_frame = last_frame.max(position.as_u64()),
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return refused(diagnostics);
        }
    }

    LoweredPerformance {
        events,
        frames: FrameCount::new(last_frame),
        diagnostics,
    }
}

/// Where the song ends, in ticks, as V1 decides it.
///
/// Read through `Song::calculate_length` rather than recomputed, for the usual reason: a copy
/// would keep agreeing with an old V1. One check precedes the call, because that function adds
/// a placement's start to its length unchecked and a persisted start near `u64::MAX` would
/// overflow inside it — and a lowerer has no business panicking on a value it can refuse by
/// name first. A section's end saturates in V1 and needs no guard.
fn song_end(song: &Song, diagnostics: &mut Vec<LoweringDiagnostic>) -> Option<u64> {
    for placement in song.arrangement() {
        let Some(pattern) = song.pattern(placement.pattern_id) else {
            // Refused with its subject by the arrangement walk; `calculate_length` skips it.
            continue;
        };
        let length = placement.effective_length(pattern.length);
        if placement.start.0.checked_add(u64::from(length.0)).is_none() {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Pattern {
                    pattern: placement.pattern_id,
                    name: pattern.name.clone(),
                },
                LoweringReason::UnsupportedParameterValue {
                    value: "the placement's end does not fit".to_owned(),
                },
            ));
            return None;
        }
    }
    Some(song.calculate_length().0)
}

/// The song's tempo map, in V2's terms.
///
/// The two `TempoChange` types carry the same three fields — tick, bpm, and whether the
/// change ramps toward the next — so the *fields* translate one to one. The ramp's **law** does
/// not: V1 ramps the tempo number linearly in tick space and integrates its reciprocal, V2
/// ramps the beat's period (`SOUND-INV-019`, ADR-0049), so every event after a ramp that has
/// a next change to ramp toward lands at a different frame. ADR-0049 accepts that as an
/// intentional semantic change that must map to a comparison category rather than pass as
/// error, so a lowering carrying such a ramp is marked unrepresented here; the squash review
/// found the flag forwarded with no diagnostic. A ramp with nothing after it ramps toward
/// nothing in both engines, and is a step.
fn lower_tempo(
    song: &Song,
    sample_rate: SampleRate,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Result<TempoMap, String> {
    let initial = V2Bpm::new(f64::from(song.default_tempo.as_f32())).map_err(|e| e.to_string())?;
    let saved = song.tempo_changes();
    let mut changes = Vec::with_capacity(saved.len());
    let mut ramps_toward_a_change = false;
    for (index, change) in saved.iter().enumerate() {
        let bpm = V2Bpm::new(f64::from(change.bpm.as_f32())).map_err(|e| e.to_string())?;
        let tick = MusicalTick::new(change.tick.0);
        changes.push(if change.ramp {
            ramps_toward_a_change |= index + 1 < saved.len();
            V2TempoChange::ramp(tick, bpm)
        } else {
            V2TempoChange::step(tick, bpm)
        });
    }
    if ramps_toward_a_change {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "a tempo ramp, which V1 integrates linearly in tempo and V2 \
                             linearly in beat period, so every event after it moves — \
                             ADR-0049's comparison category",
                owner: "the first A/B consumer, which ADR-0049 has create that category",
            },
        ));
    }
    TempoMap::new(initial, &changes, sample_rate).map_err(|e| e.to_string())
}

/// The outcome of a refusal: no events, no length, and the diagnostics that say why.
fn refused(diagnostics: Vec<LoweringDiagnostic>) -> LoweredPerformance {
    LoweredPerformance {
        events: Vec::new(),
        frames: FrameCount::new(0),
        diagnostics,
    }
}

/// Every saved track field, with its disposition stated exactly once.
///
/// Destructured **without** `..`, so a new field on `SequencerTrack` is a compile error here
/// rather than a silent difference in the render — the same mechanism, and for the same reason,
/// as `render::instrument_state_dispositions`.
///
/// Returns whether lowering may continue.
fn track_dispositions(
    track: &synth_sequencer::SequencerTrack,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> bool {
    let synth_sequencer::SequencerTrack {
        // Represented: the identity a diagnostic subject and the placement filter are built on.
        id,
        name,
        // Metadata. Never reaches audio in either engine.
        description: _,
        color: _,
        // Represented: this is what decides whose notes a lowering carries.
        instrument: _,
        // V1 mixes the track through these — `auto.volume.unwrap_or(track.volume)` and the same
        // for pan — and V2 has no mixer stage to carry them. Reported rather than refused: the
        // notes V1 plays are still the notes lowered here, only their level and position differ.
        volume,
        pan,
        // Represented: both decide what is lowered at all, above.
        mute: _,
        solo: _,
        // One variant today, `Polyphonic`, and it is `#[default]`. Matched exhaustively rather
        // than compared, so a second variant — a mono-voice mode would change note behaviour —
        // becomes a compile error here instead of a silent difference.
        mode,
        // Handled where the return buses they target are, in `render::project_diagnostics`: an
        // enabled send at a non-zero level is refused there, naming this track.
        sends: _,
    } = track;

    match mode {
        synth_sequencer::TrackMode::Polyphonic => {}
    }

    let subject = || ProjectSubject::Track {
        track: *id,
        name: name.clone(),
    };
    if *volume != NormalizedValue::MAX {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a track volume other than unity",
                owner: "Phase 8",
            },
        ));
    }
    if *pan != BipolarValue::CENTER {
        diagnostics.push(LoweringDiagnostic::unrepresented(
            subject(),
            LoweringReason::OwnedByLaterPhase {
                capability: "a track pan",
                owner: "Phase 8",
            },
        ));
    }
    true
}
