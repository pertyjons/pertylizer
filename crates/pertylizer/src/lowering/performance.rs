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
//!
//! # What a lowered automation lane is (`P07-S002b`)
//!
//! V1 runs a placed pattern's lanes on every tick the placement is active, reading each lane
//! through `AutomationLane::value_at` at the placement's pattern tick, and emits the value as
//! a parameter event when it has moved by more than `AUTOMATION_DEDUP_THRESHOLD` since the
//! last one it emitted for that target. An instrument lane on a module parameter — the
//! filter's cutoff and resonance, the envelope's four — then resolves to the **first** module
//! of that type in the instrument's graph, in `ModuleId` order, and is denormalized through
//! that module's own descriptor before it lands as a transient override. Every one of those
//! steps is V1's own function here rather than a copy: the tick is `pattern_tick_at`'s, the
//! curve is `value_at`'s, the threshold is the sequencer's constant, the range is the
//! descriptor's `denormalize`, and the module is the lowest identity of its type.
//!
//! Each emission lowers to one `SetParameter` override write at the tick's plan position, on
//! the slot the control `P07-S002a` declared, in the control's own unit. The write is
//! composed through the slot — an authored base, this override, whatever modulation is in
//! force — under `SOUND-INV-023`, and a quantum-rate control reads it at the boundary that
//! follows, which is the per-quantum granularity the phase states. The writes count toward
//! the plan's compiled event share exactly as note edges do, through the same peak.
//!
//! V1 clears every transient override when the transport stops, and the offline renderers
//! stop it at the song's end before rendering a tail. So the tail hears the authored values,
//! and one further write per touched target restores the prepared base at the song's end.
//!
//! Two lanes writing one target at one sample are refused by name — the master plan's
//! multiple-writer rule in its strict form, while ADR-0012 stays `Proposed` for Phase 10.
//! Read structurally, as two lanes on one target whose active tick ranges intersect: every
//! sample in the intersection has two absolute writers, whatever their values.

use synth_core::{BipolarValue, ModuleDescriptor, ModuleType, NormalizedValue};
use synth_engine::instrument::InstrumentId;
use synth_engine::sequencer_engine::AUTOMATION_DEDUP_THRESHOLD;
use synth_engine_v2::ir::{NodeId, ParameterId, parameters};
use synth_engine_v2::offline::OfflineEvent;
use synth_engine_v2::plan::{CompiledPlan, ParameterSlot};
use synth_engine_v2::quantities::{
    CutoffFrequency, EventCount, KeyIdentity, NormalizedLevel, NoteVelocity, ParameterValue,
    Resonance, SampleRate, Seconds,
};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::tempo::{Bpm as V2Bpm, MusicalTick, TempoChange as V2TempoChange, TempoMap};
use synth_engine_v2::time::{FrameCount, SampleTime};
use synth_sequencer::{
    AutoInstrumentParam, AutomationLane, AutomationTarget, GlobalParam, Pattern, PatternId,
    PatternPlacement, PlacementLoopMode, Song, Tick, TrackId, TrackParam,
};

use super::diagnostics::{Fidelity, LoweringDiagnostic, LoweringReason, ProjectSubject, Severity};
use super::identity::ResolvedIdentities;
use crate::patch::ModuleState;

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

/// One lane V1 would run on this instrument, with the ticks it is active over.
struct ActiveLane<'a> {
    lane: &'a AutomationLane,
    placement: &'a PatternPlacement,
    pattern: &'a Pattern,
    param: AutoInstrumentParam,
    /// The first absolute tick V1 reads the lane at.
    start: u64,
    /// One past the last: `pattern_tick_at` resolves no tick from here on.
    end: u64,
}

/// One value V1 emits for one target, placed in the plan but not yet on a slot.
#[derive(Debug, Clone, Copy)]
struct LaneWrite {
    /// The plan position of the tick V1 emits it at.
    position: u64,
    param: AutoInstrumentParam,
    /// The lane's own value, before V1's descriptor denormalizes it.
    value: NormalizedValue,
    /// The pattern whose lane emitted it, for a diagnostic.
    pattern: PatternId,
}

/// The most ticks one lowering walks for its lanes, summed over every lane.
///
/// V1 evaluates a lane on every tick its placement is active, and this walk is V1's; but V1
/// does it in real time over the song's own length, and a lowering does it up front. The
/// smoke render's own bound is six hundred seconds of audio, which bounds nothing here: a
/// tempo is any finite positive number, so a placement of few frames can hold any number of
/// ticks. The walk is therefore bounded in ticks, and the bound is stated rather than
/// assumed: `2^25` ticks is six hundred seconds at three hundred beats per minute for the six
/// targets together, with a margin — three seconds or so of evaluation at the outside. An
/// arrangement past it is refused by name before a tick is walked. An independent read
/// found the walk unbounded.
const MAX_AUTOMATION_TICKS: u64 = 1 << 25;

/// Every lane V1 runs on this instrument's module parameters, over every placement.
///
/// Walked over **every** placement rather than this instrument's audible ones, because V1
/// runs a pattern's automation whether or not its track's notes are audible — a muted host
/// track still carries its fades — and a lane names its instrument itself, so a placement on
/// another instrument's track can still write this one's filter. What the walk classifies:
///
/// - an instrument lane on one of the six module parameters, for **this** instrument, is
///   lowered; one on `Volume` or `Pan` is instrument-level channel state Phase 8 owns and is
///   refused;
/// - an instrument or module lane for **another** instrument is not this render's, exactly as
///   that instrument's notes are not, and is skipped;
/// - a module-addressed lane for this instrument names a module positionally and denormalizes
///   through the descriptor `apply_module_param_override` reads; it is refused for a later
///   slice of this phase;
/// - a track lane over the fader, pan or mute is Phase 8's; a track pitch lane is a
///   channel-scoped pitch offset the controller layer owns; a global master-volume lane is
///   Phase 8's. All three are refused whatever track or instrument they touch, since the
///   lowerer sees one instrument and cannot tell whether that touch reaches it.
///
/// A lane with no points is not automation: `value_at` returns `None` for it and V1 emits
/// nothing. A placement that is never active — a zero-length pattern, or a zero-length
/// override — is skipped whatever it holds, because `pattern_tick_at` resolves no tick in it.
/// A lane on a parameter `declared` answers `false` for — no module of its type in the patch
/// — is V1's no-op, `apply_normalized_override` returning before it writes, and is dropped
/// here, **before** the conflict check below: two inert lanes conflict over nothing. An
/// independent read found them refused.
///
/// Returns `None` when a refusal was recorded, including two lanes writing one target over
/// one tick.
fn active_lanes<'a>(
    instrument: InstrumentId,
    song: &'a Song,
    declared: &dyn Fn(AutoInstrumentParam) -> bool,
    diagnostics: &mut Vec<LoweringDiagnostic>,
) -> Option<Vec<ActiveLane<'a>>> {
    let mut lanes = Vec::new();
    for placement in song.arrangement() {
        // A placement naming no pattern is refused by the note walk when it is this
        // instrument's, and `Song::calculate_length` skips it; there is nothing to read here.
        let Some(pattern) = song.pattern(placement.pattern_id) else {
            continue;
        };
        if pattern.length.0 == 0 || placement.effective_length(pattern.length).0 == 0 {
            continue;
        }
        let subject = || ProjectSubject::Pattern {
            pattern: pattern.id,
            name: pattern.name.clone(),
        };
        // The ticks `pattern_tick_at` resolves: a `Repeat` placement wraps to its end, a
        // `Clip` placement plays the source once and is silent past it.
        let start = placement.start.0;
        let end = placement.end(pattern.length).0;
        let end = match placement.loop_mode {
            PlacementLoopMode::Repeat => end,
            PlacementLoopMode::Clip => end.min(start.saturating_add(u64::from(pattern.length.0))),
        };

        for lane in &pattern.automation {
            if lane.is_empty() {
                continue;
            }
            let reason = match &lane.target {
                AutomationTarget::Instrument {
                    instrument: target,
                    param,
                } => {
                    if *target != instrument {
                        continue;
                    }
                    match param {
                        AutoInstrumentParam::Volume | AutoInstrumentParam::Pan => {
                            LoweringReason::OwnedByLaterPhase {
                                capability: "an instrument volume or pan automation lane, which \
                                             V1 applies to the instrument's channel",
                                owner: "Phase 8, with the mixer model",
                            }
                        }
                        lowered => {
                            if !declared(*lowered) {
                                continue;
                            }
                            lanes.push(ActiveLane {
                                lane,
                                placement,
                                pattern,
                                param: *lowered,
                                start,
                                end,
                            });
                            continue;
                        }
                    }
                }
                AutomationTarget::Module {
                    instrument: target, ..
                } => {
                    if *target != instrument {
                        continue;
                    }
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a module-addressed automation lane, which V1 resolves \
                                     positionally and denormalizes through the module's own \
                                     descriptor",
                        owner: "Phase 7, in a later slice",
                    }
                }
                AutomationTarget::Track { param, .. } => match param {
                    TrackParam::Volume | TrackParam::Pan | TrackParam::Mute => {
                        LoweringReason::OwnedByLaterPhase {
                            capability: "a track automation lane over the fader, pan or mute",
                            owner: "Phase 8, with the mixer and bus model",
                        }
                    }
                    TrackParam::Pitch => LoweringReason::OwnedByLaterPhase {
                        capability: "a track pitch lane, a channel-scoped pitch offset applied \
                                     to every voice the track plays",
                        owner: "Phase 7, with the controller layer",
                    },
                },
                AutomationTarget::Global(GlobalParam::MasterVolume) => {
                    LoweringReason::OwnedByLaterPhase {
                        capability: "a master volume automation lane",
                        owner: "Phase 8, with the mixer model",
                    }
                }
            };
            diagnostics.push(LoweringDiagnostic::refused(subject(), reason));
            return None;
        }
    }

    // Two writers on one target at one sample. Lanes on one target sorted by their first
    // tick: any lane starting before its predecessor ends shares every tick from there on
    // with it, and each such tick has two absolute writers whatever the two values are.
    lanes.sort_by_key(|lane| (lane.start, lane.end));
    for (index, second) in lanes.iter().enumerate() {
        let conflicting = lanes[..index]
            .iter()
            .find(|first| first.param == second.param && second.start < first.end);
        if let Some(first) = conflicting {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Pattern {
                    pattern: second.pattern.id,
                    name: second.pattern.name.clone(),
                },
                LoweringReason::ConflictingWriters {
                    target: second.param.display_name().to_owned(),
                    first: first.pattern.id,
                    second: second.pattern.id,
                },
            ));
            return None;
        }
    }
    Some(lanes)
}

/// The values V1 emits for these lanes, placed in the plan, in position order.
///
/// V1's own walk: every tick a placement is active, the lane's value at the placement's
/// pattern tick, emitted when it has moved by more than the sequencer's threshold since the
/// last emission **for that target** — across placements, because V1 keys its last value by
/// target rather than by lane, so a second placement's first tick emits only what differs
/// from where the first left off. The lanes on one target are disjoint in ticks here, which
/// [`active_lanes`] established, so walking them in start order is walking the song in tick
/// order for that target.
///
/// Disjoint in ticks is not disjoint in samples: at a low rate and a high tempo two ticks
/// round to one frame, so a lane's last emission and its successor's first can land on one
/// sample. That is two absolute writers at one sample as much as an overlap is, and it is
/// refused here, where the positions are known — the same refusal, naming both patterns. Two
/// emissions of **one** lane on one frame are one writer, and the later one is in force, as
/// it is in V1's block.
///
/// Linear in the ticks the placements span, bounded by [`MAX_AUTOMATION_TICKS`] and refused
/// by name past it, and run off the audio thread.
fn placed_writes(
    lanes: &[ActiveLane<'_>],
    tempo: &TempoMap,
) -> Result<Vec<LaneWrite>, Box<LoweringDiagnostic>> {
    let ticks = lanes
        .iter()
        .fold(0_u64, |sum, lane| sum.saturating_add(lane.end - lane.start));
    if ticks > MAX_AUTOMATION_TICKS {
        return Err(Box::new(LoweringDiagnostic::refused(
            ProjectSubject::Project,
            LoweringReason::OwnedByLaterPhase {
                capability: "an arrangement whose automation spans more ticks than the \
                             bounded smoke scope walks",
                owner: "ADR-0028, with the long-running job contract",
            },
        )));
    }

    let mut writes = Vec::new();
    // Per target: the last emitted value, and the last placed write's position and lane.
    let mut last: Vec<(AutoInstrumentParam, f32, u64, usize)> = Vec::new();
    for (writer, lane) in lanes.iter().enumerate() {
        for tick in lane.start..lane.end {
            let Some(pattern_tick) = lane
                .placement
                .pattern_tick_at(Tick(tick), lane.pattern.length)
            else {
                continue;
            };
            let Some(value) = lane.lane.value_at(pattern_tick) else {
                continue;
            };
            let previous = last.iter_mut().find(|(param, ..)| *param == lane.param);
            let changed = previous.as_ref().is_none_or(|(_, emitted, ..)| {
                (value.as_f32() - *emitted).abs() > AUTOMATION_DEDUP_THRESHOLD
            });
            if !changed {
                continue;
            }
            let position = tempo
                .position_of(MusicalTick::new(tick))
                .map_err(|error| {
                    Box::new(LoweringDiagnostic::refused(
                        ProjectSubject::Pattern {
                            pattern: lane.pattern.id,
                            name: lane.pattern.name.clone(),
                        },
                        LoweringReason::UnsupportedParameterValue {
                            value: error.to_string(),
                        },
                    ))
                })?
                .as_u64();
            match previous {
                Some((_, emitted, placed, by)) => {
                    if *placed == position && *by != writer {
                        let first = &lanes[*by];
                        return Err(Box::new(LoweringDiagnostic::refused(
                            ProjectSubject::Pattern {
                                pattern: lane.pattern.id,
                                name: lane.pattern.name.clone(),
                            },
                            LoweringReason::ConflictingWriters {
                                target: lane.param.display_name().to_owned(),
                                first: first.pattern.id,
                                second: lane.pattern.id,
                            },
                        )));
                    }
                    *emitted = value.as_f32();
                    *placed = position;
                    *by = writer;
                }
                None => last.push((lane.param, value.as_f32(), position, writer)),
            }
            writes.push(LaneWrite {
                position,
                param: lane.param,
                value,
                pattern: lane.pattern.id,
            });
        }
    }
    // Stable, so two targets emitting at one position keep the order V1 walks the lanes in.
    writes.sort_by_key(|write| write.position);
    Ok(writes)
}

/// Where V1's instrument automation lands in one lowered graph (`P07-S002b`).
///
/// V1's `apply_normalized_override` finds the **first** module of the parameter's type in the
/// instrument's graph — a `BTreeMap` keyed by `ModuleId`, so the lowest identity of that type
/// — and denormalizes the lane's value through that module's own descriptor. Both are
/// resolved once here, from the identities the graph was lowered through and from V1's own
/// module factory, and read per write. A missing module is V1's no-op: the lane is inert
/// there and lowers to nothing here.
#[derive(Debug)]
#[must_use]
pub struct AutomationTargets {
    filter: Option<(NodeId, ModuleDescriptor)>,
    envelope: Option<(NodeId, ModuleDescriptor)>,
}

impl AutomationTargets {
    /// Resolve the two module types V1's instrument automation addresses.
    pub fn resolve(identities: &ResolvedIdentities) -> Self {
        let first = |kind: ModuleType| {
            identities
                .pairs()
                .filter(|(id, _)| id.module_type == kind)
                .min_by_key(|(id, _)| *id)
                .and_then(|(_, node)| {
                    crate::module_factory::create_voice_module(kind)
                        .map(|(_, declarations)| (node, declarations))
                })
        };
        Self {
            filter: first(ModuleType::Filter),
            envelope: first(ModuleType::Envelope),
        }
    }

    /// Whether V1 would find a module for this parameter in the lowered graph.
    fn declares(&self, param: AutoInstrumentParam) -> bool {
        crate::mod_grid_build::instrument_param_module(param).is_some_and(|(kind, _, _)| match kind
        {
            ModuleType::Filter => self.filter.is_some(),
            ModuleType::Envelope => self.envelope.is_some(),
            _ => false,
        })
    }

    /// Whether V1 would find a module for this parameter, from the saved modules alone.
    ///
    /// The peak is counted before the graph is lowered, so it cannot ask the resolved
    /// targets; it asks the saved patch the same question. A module of the type that later
    /// fails to lower refuses the whole lowering, so the two answers cannot differ for a plan
    /// that renders.
    fn declared_in(param: AutoInstrumentParam, modules: &[ModuleState]) -> bool {
        crate::mod_grid_build::instrument_param_module(param)
            .is_some_and(|(kind, _, _)| modules.iter().any(|module| module.module_type == kind))
    }

    /// The write V1's emission becomes: the node, the control, and the value in its unit.
    ///
    /// `Ok(None)` is V1's no-op — no module of the type — and `Err` names a value the
    /// control's unit refuses, which the descriptor's clamped range makes unreachable and
    /// which is refused by name rather than assumed away.
    fn override_value(
        &self,
        param: AutoInstrumentParam,
        value: NormalizedValue,
    ) -> Result<Option<(NodeId, ParameterId, ParameterValue)>, String> {
        let Some((kind, _, key)) = crate::mod_grid_build::instrument_param_module(param) else {
            return Ok(None);
        };
        let resolved = match kind {
            ModuleType::Filter => self.filter.as_ref(),
            ModuleType::Envelope => self.envelope.as_ref(),
            _ => None,
        };
        let Some((node, declarations)) = resolved else {
            return Ok(None);
        };
        // V1's own range and curve: the descriptor's `denormalize`, which clamps the lane's
        // value into `[0, 1]` and maps it as the widget does — logarithmic for the corner,
        // exponential for a time, linear for the rest.
        let Some(declared) = declarations.find_parameter(key) else {
            return Err(format!(
                "{} is not a parameter V1's {kind:?} declares",
                param.display_name()
            ));
        };
        let denormalized = declared.denormalize(value.as_f32());
        let (parameter, value) = match param {
            AutoInstrumentParam::FilterCutoff => (
                parameters::FILTER_CUTOFF,
                CutoffFrequency::new(denormalized)
                    .map(ParameterValue::from_cutoff)
                    .map_err(|error| error.to_string())?,
            ),
            AutoInstrumentParam::FilterResonance => (
                parameters::FILTER_RESONANCE,
                Resonance::new(super::graph::v1_quality(denormalized))
                    .map(ParameterValue::from_resonance)
                    .map_err(|error| error.to_string())?,
            ),
            AutoInstrumentParam::Attack
            | AutoInstrumentParam::Decay
            | AutoInstrumentParam::Release => (
                match param {
                    AutoInstrumentParam::Attack => parameters::ENVELOPE_ATTACK,
                    AutoInstrumentParam::Decay => parameters::ENVELOPE_DECAY,
                    _ => parameters::ENVELOPE_RELEASE,
                },
                Seconds::new(denormalized)
                    .map(ParameterValue::from_seconds)
                    .map_err(|error| error.to_string())?,
            ),
            AutoInstrumentParam::Sustain => (
                parameters::ENVELOPE_SUSTAIN,
                NormalizedLevel::new(denormalized)
                    .map(ParameterValue::from_level)
                    .map_err(|error| error.to_string())?,
            ),
            AutoInstrumentParam::Volume | AutoInstrumentParam::Pan => return Ok(None),
        };
        Ok(Some((*node, parameter, value)))
    }
}

/// The most events this arrangement puts in any one render quantum: note edges and the
/// override writes its automation lanes emit, with the restoring writes at the song's end.
///
/// Admission needs this **before** the plan is compiled, and the plan is needed before an
/// event can name a note slot — so the count is taken from the timeline rather than from the
/// events. The notes come from [`note_spans`] and the writes from [`lane_writes`], the same
/// two functions [`lower_performance`] reads, so the number admission is told is a count of
/// the same events the renderer is later given. `modules` is the saved patch, which decides
/// whether a lane has a module to land on at all.
///
/// Returns `None` when the arrangement could not be read; the caller lowers anyway and the
/// refusal surfaces there with its subject intact.
pub fn peak_events_per_quantum(
    instrument: InstrumentId,
    modules: &[ModuleState],
    song: &Song,
    sample_rate: SampleRate,
) -> Option<EventCount> {
    let mut ignored = Vec::new();
    let spans = note_spans(instrument, song, &mut ignored)?;
    let declared = |param| AutomationTargets::declared_in(param, modules);
    let lanes = active_lanes(instrument, song, &declared, &mut ignored)?;
    let tempo = lower_tempo(song, sample_rate, &mut ignored).ok()?;

    let mut frames = Vec::with_capacity(spans.len() * 2);
    for span in &spans {
        for tick in [span.start, span.end] {
            frames.push(tempo.position_of(MusicalTick::new(tick)).ok()?.as_u64());
        }
    }
    let mut written: Vec<AutoInstrumentParam> = Vec::new();
    for write in placed_writes(&lanes, &tempo).ok()? {
        frames.push(write.position);
        if !written.contains(&write.param) {
            written.push(write.param);
        }
    }
    // One restoring write per touched target where the transport stops.
    let end = tempo
        .position_of(MusicalTick::new(song_end(song, &mut ignored)?))
        .ok()?
        .as_u64();
    frames.extend(std::iter::repeat_n(end, written.len()));
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
    targets: &AutomationTargets,
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

    // The arrangement occupies the song as V1 bounds it, not only up to its last release: a
    // trailing rest, or a section drawn past the last placement, is silence V1 renders and
    // this render would otherwise omit. The same `calculate_length` that clips a release
    // above is what extends the frame count here — and it is where V1's transport stops,
    // which the automation below needs.
    let Some(song_end) = song_end(song, &mut diagnostics) else {
        return refused(diagnostics);
    };
    let end_frame = match tempo.position_of(MusicalTick::new(song_end)) {
        Ok(position) => position.as_u64(),
        Err(error) => {
            diagnostics.push(LoweringDiagnostic::refused(
                ProjectSubject::Project,
                LoweringReason::UnsupportedParameterValue {
                    value: error.to_string(),
                },
            ));
            return refused(diagnostics);
        }
    };
    last_frame = last_frame.max(end_frame);

    // `P07-S002b`: the automation lanes, as override writes. Every emission V1 makes lands
    // as one `SetParameter` on the declared control's slot, at the emission's own position;
    // the renderer composes it through the slot and reads it at the boundary that follows.
    let declared = |param| targets.declares(param);
    let Some(lanes) = active_lanes(instrument, song, &declared, &mut diagnostics) else {
        return refused(diagnostics);
    };
    let writes = match placed_writes(&lanes, &tempo) {
        Ok(writes) => writes,
        Err(diagnostic) => {
            diagnostics.push(*diagnostic);
            return refused(diagnostics);
        }
    };
    // Each touched slot with its prepared base, for the restoring write below.
    let mut touched: Vec<(ParameterSlot, ParameterValue)> = Vec::new();
    for write in writes {
        let subject = || ProjectSubject::Pattern {
            pattern: write.pattern,
            name: song
                .pattern(write.pattern)
                .map(|pattern| pattern.name.clone())
                .unwrap_or_default(),
        };
        let (node, parameter, value) = match targets.override_value(write.param, write.value) {
            // V1's no-op: no module of the type, so the lane is inert there and here.
            Ok(None) => continue,
            Ok(Some(resolved)) => resolved,
            Err(value) => {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue { value },
                ));
                return refused(diagnostics);
            }
        };
        // Every lowered kind declares the six as controls since `P07-S002a`, so a plan that
        // compiled addresses each; a refusal here rather than a skip, so a declaration that
        // narrows is found rather than silenced.
        let Some(slot) = plan.resolve_parameter(node, parameter) else {
            diagnostics.push(LoweringDiagnostic::refused(
                subject(),
                LoweringReason::UnsupportedParameterValue {
                    value: format!(
                        "{} addresses a control the plan does not declare",
                        write.param.display_name()
                    ),
                },
            ));
            return refused(diagnostics);
        };
        events.push(OfflineEvent::new(
            SampleTime::new(write.position),
            CompiledPayload::SetParameter { slot, value },
        ));
        if !touched.iter().any(|(known, _)| *known == slot) {
            // The compiled base is the prepared value, which is the authored one.
            let Some(base) = plan
                .parameter_targets()
                .get(slot.index())
                .map(|target| target.base)
            else {
                diagnostics.push(LoweringDiagnostic::refused(
                    subject(),
                    LoweringReason::UnsupportedParameterValue {
                        value: format!(
                            "{} resolves to a slot the plan's target table does not hold",
                            write.param.display_name()
                        ),
                    },
                ));
                return refused(diagnostics);
            };
            touched.push((slot, base));
        }
    }
    // V1 clears every transient override when its transport stops, and the offline
    // renderers stop it at the song's end before the tail. The tail hears the authored
    // values there, so it hears them here: one write of the prepared base per touched slot.
    // Pushed after every lane write, so at an equal position the stable sort below keeps the
    // restore last.
    for (slot, base) in touched {
        events.push(OfflineEvent::new(
            SampleTime::new(end_frame),
            CompiledPayload::SetParameter { slot, value: base },
        ));
    }

    // Ascending, as the offline renderer requires. Sorting spans by start tick does not
    // establish it: a release is emitted beside its own note-on rather than in time order,
    // and the lane writes follow every note. Stable, so events at one position keep the
    // order they were emitted in: a note's edges before the lane writes at its tick, and the
    // restoring writes after everything.
    events.sort_by_key(OfflineEvent::time);

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
