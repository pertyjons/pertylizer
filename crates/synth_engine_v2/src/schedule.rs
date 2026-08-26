//! Compiled-event scheduling at the renderer's engine-time boundary.
//!
//! Phase 3 starts here, deliberately on the engine side of ADR-0022. A compiled event
//! already carries the current epoch's [`SampleTime`] once it is placed; this module does
//! not observe a host clock, compensate latency, or assign `Hardware`/`Arrival` provenance.
//!
//! # Admission happens before preparation, in plan time
//!
//! A compiled stream is admitted as [`AdmittedCompiledStream`], from [`PlanEvent`]s that
//! carry no anchor at all, and preparation consumes that admitted value rather than
//! deciding for itself whether the stream fits. The reason is ADR-0046 clause 4: which
//! quantum a frame belongs to depends on where the stream was anchored, so a per-absolute-
//! quantum count answers the wrong question and admits a plan that faults at publication
//! after an ordinary seek. Admission slides a `Q`-frame window instead, which is the worst
//! case over all `Q` anchor phases, and it does so in plan time — where a position exists
//! before any anchor does.
//!
//! Preparation then places every admitted position at the renderer's anchor, pairs and
//! stamps, and allocates the stamped list. It re-checks no capacity: the admitted value is
//! the proof, and a second proof at a second site is how the two come to disagree. The
//! [`CompiledEventScheduler::render`] path advances a cursor over that storage and
//! **publishes** the due span through ADR-0046's arbiter — it does not hand the renderer a
//! borrowed slice, which would make clause 2's "the only normal path that constructs
//! renderer input" false for this producer. Its source lives in `schedule/hot.rs` so the
//! real-time purity test scans it alongside the renderer, the arbiter and node kernels.

use thiserror::Error;

use crate::admit::{AdmissionError, admit_linear};
use crate::plan::{CompiledPlan, PlanId};
use crate::quantities::EventCount;
use crate::render::{EventEnvelope, EventPayload, NoteEdge, PreparedRenderer, TimedEvent};
use crate::time::{Located, PlanPosition, SampleTime, StreamAnchor, StreamEpoch, TimeSource};

/// One exact event produced by compiled plan time.
///
/// It has no epoch yet because preparation issues that identity. The prepared scheduler
/// stamps every event with the renderer's epoch and `Compiled` provenance before the
/// audio thread starts.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct CompiledEvent {
    time: SampleTime,
    payload: CompiledPayload,
}

/// What a compiled event does, before preparation stamps it.
///
/// Distinct from [`EventPayload`] for the same reason [`CompiledEvent`] has no epoch:
/// preparation supplies what only it can. A note edge's **occurrence** is minted here, so a
/// compiled list names the node on both edges — that is how preparation pairs them — while
/// the stamped event names the node on the on edge alone, per `SOUND-INV-017`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub enum CompiledPayload {
    /// Set one compiled parameter slot.
    SetParameter {
        /// Which compiled parameter.
        slot: crate::plan::ParameterSlot,
        /// The new value.
        value: crate::quantities::ParameterValue,
    },
    /// Play one compiled node.
    NoteOn {
        /// Which node is played.
        slot: crate::plan::NoteSlot,
    },
    /// Let go of the most recent unreleased note on one compiled node.
    ///
    /// The node is named so preparation can pair this with its note-on. It does **not**
    /// reach the stamped event: the occurrence carries the node from there on.
    NoteOff {
        /// Which node is released.
        slot: crate::plan::NoteSlot,
    },
}

/// One compiled event in **plan** time.
///
/// It carries no anchor, and that is the point. ADR-0046 clause 4 admits a compiled stream
/// against every `Q` anchor phase, so the artifact admission judges has to exist before an
/// anchor is chosen — a list already placed at one anchor can only be judged at that one.
/// [`CompiledEvent`] is the same event after an anchor has placed it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct PlanEvent {
    position: PlanPosition,
    payload: CompiledPayload,
}

impl PlanEvent {
    /// An event at a position in the plan.
    pub const fn new(position: PlanPosition, payload: CompiledPayload) -> Self {
        Self { position, payload }
    }

    /// Where in the plan the event happens.
    pub const fn position(self) -> PlanPosition {
        self.position
    }

    /// What the event does.
    pub const fn payload(self) -> CompiledPayload {
        self.payload
    }
}

/// Why a compiled stream was not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CompiledStreamError {
    /// The stream is not monotone in plan time.
    ///
    /// Refused rather than sorted, for the reason preparation used to give: ADR-0023 has
    /// not selected a same-sample ordering policy, and admission must not invent one as a
    /// side effect. The window scan also *assumes* ascending input, so an unsorted list
    /// would be measured against windows that do not exist.
    #[error("compiled event {event_index} at {position} precedes the previous event at {previous}")]
    EventsOutOfOrder {
        /// Position in the supplied event list.
        event_index: usize,
        /// The preceding event's position.
        previous: PlanPosition,
        /// The rejected event's position.
        position: PlanPosition,
    },

    /// An event carries a slot resolved against a different plan.
    #[error(
        "compiled event {event_index} carries a slot from {actual}, but the stream is {expected}"
    )]
    ForeignPlan {
        /// Position in the supplied event list.
        event_index: usize,
        /// The plan being admitted against.
        expected: PlanId,
        /// The plan that issued the event's slot.
        actual: PlanId,
    },

    /// Some `Q`-frame window holds more events than the compiled share admits.
    ///
    /// Only [`AdmissionError::WindowOverShare`] is reachable here: a linear stream has no
    /// loop interval, so the two loop-shaped variants describe a check this path does not
    /// run. They are carried rather than flattened so that the loop half, when it arrives,
    /// reports through the same error.
    #[error("the compiled stream is over its share: {0}")]
    Window(#[from] AdmissionError),
}

/// A compiled stream proven to fit its producer's share at **every** anchor phase.
///
/// ADR-0046 clause 4 is the whole of this type. Admission slides a `Q`-frame window over
/// the plan positions and refuses a stream where any window holds more events than
/// `compiled_event_share`, which "is exactly the worst case over all `Q` integer anchor
/// phases". Once that passes, no anchor can produce a quantum over the share, so
/// preparation and publication have nothing left to decide about capacity.
///
/// **It is a value, not a check.** The distinction is what closes the hole a per-call check
/// leaves open: a caller could hand preparation a different event set at every anchor and
/// have each one judged on its own, which proves nothing about the stream. Preparation
/// accepts only this type, so the set it places is the set that was admitted.
///
/// Constructing one allocates and its cost scales with the stream, so it belongs off the
/// audio thread — the same rule ADR-0046 clause 4 states for the window scan itself.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct AdmittedCompiledStream {
    plan: PlanId,
    events: Vec<PlanEvent>,
}

impl AdmittedCompiledStream {
    /// Admit `events` as `plan`'s compiled stream.
    ///
    /// Three properties, each checked where it can be answered: ascending plan order, every
    /// slot belonging to `plan`, and the sliding-window bound above. The share is the
    /// **compiled producer's**, not `max_events_per_quantum`: ADR-0046 clause 1 partitions
    /// the cap across six producers, and clause 3 makes a compiled runtime miss a producer
    /// defect rather than a load condition, so a stream that would overrun its share has to
    /// be refused here — where a caller can still be told.
    pub fn admit(plan: &CompiledPlan, events: &[PlanEvent]) -> Result<Self, CompiledStreamError> {
        let expected = plan.id();
        let mut previous: Option<PlanPosition> = None;
        let mut positions = Vec::with_capacity(events.len());

        for (event_index, event) in events.iter().copied().enumerate() {
            if let Some(previous) = previous
                && event.position() < previous
            {
                return Err(CompiledStreamError::EventsOutOfOrder {
                    event_index,
                    previous,
                    position: event.position(),
                });
            }
            previous = Some(event.position());

            let actual = payload_plan(event.payload());
            if actual != expected {
                return Err(CompiledStreamError::ForeignPlan {
                    event_index,
                    expected,
                    actual,
                });
            }
            positions.push(event.position());
        }

        admit_linear(&positions, plan.compiled_event_share())?;

        Ok(Self {
            plan: expected,
            events: events.to_vec(),
        })
    }

    /// The plan this stream was admitted against.
    pub const fn plan(&self) -> PlanId {
        self.plan
    }

    /// The admitted events, in ascending plan order.
    pub fn events(&self) -> &[PlanEvent] {
        &self.events
    }
}

impl CompiledEvent {
    /// An event at an exact engine time.
    pub const fn new(time: SampleTime, payload: CompiledPayload) -> Self {
        Self { time, payload }
    }

    /// The event's exact engine time.
    pub const fn time(self) -> SampleTime {
        self.time
    }

    /// What the event does.
    pub const fn payload(self) -> CompiledPayload {
        self.payload
    }
}

/// Why a compiled schedule could not be prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SchedulePrepareError {
    /// A release names a node with no unreleased note-on before it.
    ///
    /// A compiled list is authored, so this is a malformed plan rather than a runtime
    /// condition — and it is refused here, where a caller can still be told, rather than
    /// becoming an orphan the renderer counts. Preparation is the last place the pairing is
    /// knowable: after it, the release carries only an occurrence.
    #[error("compiled event {event_index} releases a node with no note sounding")]
    UnmatchedRelease {
        /// Position in the supplied event list.
        event_index: usize,
    },

    /// The plan declares no compiled note-on producer, but the list has note edges.
    ///
    /// Identities come from an admitted producer's range, and a plan that declares none has
    /// no range to mint from. Refusing names the gap; minting from a range nobody admitted
    /// would put occurrences outside the partition every other check relies on.
    #[error("the plan declares no compiled note-on producer, but event {event_index} is a note")]
    NoCompiledNoteProducer {
        /// Position in the supplied event list.
        event_index: usize,
    },

    /// An identity could not be minted for a compiled note-on.
    #[error("compiled event {event_index} could not be given an occurrence: {source}")]
    Identity {
        /// Position in the supplied event list.
        event_index: usize,
        /// Why minting refused.
        source: crate::identity::IdentityError,
    },

    /// An event carries a slot resolved against a different plan.
    #[error(
        "compiled event {event_index} carries a slot from {actual}, but the renderer uses \
         {expected}"
    )]
    ForeignPlan {
        /// Position in the supplied event list.
        event_index: usize,
        /// The renderer's plan.
        expected: PlanId,
        /// The plan that issued the event's slot.
        actual: PlanId,
    },
    /// The admitted stream belongs to a different plan than the renderer.
    ///
    /// A stream-level refusal rather than a per-event one: an [`AdmittedCompiledStream`]
    /// already proved every slot belongs to *its* plan, so one comparison settles the whole
    /// list and naming an event index here would suggest the others were checked separately.
    #[error("the admitted stream belongs to {actual}, but the renderer uses {expected}")]
    ForeignStream {
        /// The renderer's plan.
        expected: PlanId,
        /// The plan the stream was admitted against.
        actual: PlanId,
    },

    /// The placement anchor is not the one the renderer holds.
    ///
    /// The renderer's anchor is not decoration: its hot path derives every quantum's plan
    /// position from it, so placing a stream at a different pairing would put the events on
    /// one timeline and the position-aware kernels on another — an `Impulse` would keep
    /// sounding where the old anchor says while the notes moved.
    ///
    /// Re-anchoring a *prepared* renderer is what a seek and a loop wrap need and does not
    /// exist yet, so the disagreement is refused rather than silently resolved in favour of
    /// one side. Passing the anchor explicitly is what makes the refusal possible: a caller
    /// states the pairing it means, and preparation says no when the renderer is elsewhere.
    #[error(
        "the renderer is anchored at {} = {}, but the stream was placed at {} = {}",
        .renderer.position(), .renderer.time(), .supplied.position(), .supplied.time()
    )]
    AnchorMismatch {
        /// The pairing the renderer was prepared with.
        renderer: StreamAnchor,
        /// The pairing the caller asked to place at.
        supplied: StreamAnchor,
    },

    /// An admitted position lies before the renderer's anchor.
    ///
    /// The stream begins at the anchor, so this position is one the stream does not render.
    /// It is refused rather than skipped: ADR-0001 clause 16 forbids dropping an event
    /// silently, and a caller that meant to start here has admitted the wrong stream —
    /// a suffix is admissible whenever the whole is, so re-admitting the suffix is the
    /// answer rather than having preparation guess.
    #[error("compiled event {event_index} at {position} precedes the anchor at {anchor}")]
    BeforeAnchor {
        /// Position in the admitted event list.
        event_index: usize,
        /// The position that has no engine time in this stream.
        position: PlanPosition,
        /// The plan position the stream is anchored at.
        anchor: PlanPosition,
    },

    /// An admitted position has no representable engine time in this stream.
    ///
    /// Distinct from [`Self::BeforeAnchor`], which says the stream does not reach the
    /// position; this says the clock does. Reporting one as the other would send someone
    /// looking for a seek that never happened.
    #[error("compiled event {event_index} at {position} has no representable engine time")]
    TimeUnrepresentable {
        /// Position in the admitted event list.
        event_index: usize,
        /// The position that could not be placed.
        position: PlanPosition,
    },
}

/// Why a prepared schedule could not serve one renderer call.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ScheduledRenderError {
    /// The renderer was re-prepared after this schedule was stamped.
    #[error("compiled schedule belongs to {schedule}, but the renderer belongs to {renderer}")]
    EpochMismatch {
        /// Epoch captured at schedule preparation.
        schedule: StreamEpoch,
        /// Epoch of the renderer passed to the call.
        renderer: StreamEpoch,
    },
    /// The caller advanced the renderer without releasing the schedule's next event.
    #[error("compiled event at {event} is behind the renderer clock at {clock}")]
    MissedEvent {
        /// The first event the scheduler has not presented.
        event: SampleTime,
        /// First frame of the next unrendered quantum.
        clock: SampleTime,
    },
    /// The call's quantum-index window cannot be represented.
    #[error("the scheduled render window beginning at {clock} is not representable")]
    CallSpanUnrepresentable {
        /// First frame of the attempted window.
        clock: SampleTime,
    },
    /// The prepared list violated its summed per-quantum bound.
    ///
    /// Preparation makes this unreachable for a valid scheduler. The audio-thread scan
    /// retains the check so its work remains bounded even if that invariant regresses.
    #[error(
        "the compiled window at {clock} presents at least {presented_at_least} events, above \
         the {admissible} this call can admit"
    )]
    EventWindowTooDense {
        /// First frame of the attempted window.
        clock: SampleTime,
        /// Lower bound on how many events occupy the window.
        presented_at_least: usize,
        /// Sum of the per-quantum bounds over the quanta the call renders.
        admissible: usize,
    },
    /// The renderer rejected the selected span or output block.
    #[error("scheduled rendering failed: {0}")]
    Render(#[from] crate::diagnostics::RenderError),

    /// A second arbiter was offered to a schedule already publishing through one.
    ///
    /// ADR-0046 clause 2 admits exactly one arbiter per stream. A fresh store each callback
    /// would satisfy every capacity bound while silently restarting the stream's high-water
    /// history, which is the measurement Phase 3 exists to take, so the substitution is
    /// refused rather than tolerated.
    #[error("this schedule publishes through {latched}, but {offered} was supplied")]
    ForeignArbiter {
        /// The arbiter this schedule adopted.
        latched: crate::publish::ArbiterId,
        /// The one the call supplied.
        offered: crate::publish::ArbiterId,
    },

    /// Publication could not seal this call's input.
    ///
    /// **The stream is over when this is returned.** ADR-0046 clause 7 makes a share
    /// overrun or an over-full batch a contract violation rather than a load condition, so
    /// the renderer has already taken its terminal response — silence over this callback
    /// and every later one in the epoch, both carries invalidated, `needs_reprepare`
    /// published — before the error reaches the caller. Nothing here is retryable.
    #[error("publication failed and the stream was faulted: {0}")]
    Publication(#[from] crate::publish::PublicationFault),
}

/// A prepared, epoch-stamped compiled list and its release cursor.
#[derive(Debug)]
#[must_use]
pub struct CompiledEventScheduler {
    epoch: StreamEpoch,
    arbiter: Option<crate::publish::ArbiterId>,
    max_events_per_quantum: EventCount,
    events: Vec<TimedEvent>,
    next: usize,
}

impl CompiledEventScheduler {
    /// Place an admitted stream at `anchor` and stamp it.
    ///
    /// **Capacity is not decided here.** [`AdmittedCompiledStream`] already proved the
    /// stream fits its share at every anchor phase, so preparation only has to place it:
    /// re-deciding would mean two proofs of one property, and the one that is wrong is
    /// whichever the caller did not read.
    ///
    /// Placement is the anchor's forward mapping and nothing else, which keeps ADR-0032
    /// clause 27's "anchoring is the only place the two vocabularies meet" true of this
    /// path. Adding a constant preserves order, so the placed list is ascending because the
    /// admitted one was — there is no second ordering check and no second ordering policy.
    ///
    /// **`anchor` is the transport's current pairing** — [`SessionScheduler::anchor`] —
    /// stated by the caller rather than taken from the renderer. Clause 27 names seek and
    /// loop wrap as re-anchoring moments, and neither re-prepares the renderer, so the
    /// renderer's own anchor goes stale the first time the transport moves; reading it here
    /// would place a post-seek stream at the pre-seek pairing.
    ///
    /// It must nevertheless **agree** with the renderer's, and preparation refuses when it
    /// does not. The renderer's hot path derives every quantum's plan position from its own
    /// anchor, so a stream placed elsewhere would run the events on one timeline and the
    /// position-aware kernels on another. Re-anchoring a prepared renderer — moving both at
    /// once, which is what a seek actually is — is owed work, and until it exists the
    /// disagreement is a refusal rather than a silent choice of side.
    ///
    /// [`SessionScheduler::anchor`]: crate::session::SessionScheduler::anchor
    pub fn prepare(
        renderer: &mut PreparedRenderer,
        anchor: StreamAnchor,
        stream: &AdmittedCompiledStream,
    ) -> Result<Self, SchedulePrepareError> {
        let expected = renderer.plan().id();
        if stream.plan() != expected {
            return Err(SchedulePrepareError::ForeignStream {
                expected,
                actual: stream.plan(),
            });
        }

        if anchor != renderer.anchor() {
            return Err(SchedulePrepareError::AnchorMismatch {
                renderer: renderer.anchor(),
                supplied: anchor,
            });
        }

        let mut placed = Vec::with_capacity(stream.events().len());
        for (event_index, event) in stream.events().iter().copied().enumerate() {
            let time = match anchor.locate(event.position()) {
                Located::At(time) => time,
                Located::BeforeAnchor => {
                    return Err(SchedulePrepareError::BeforeAnchor {
                        event_index,
                        position: event.position(),
                        anchor: anchor.position(),
                    });
                }
                Located::Unrepresentable => {
                    return Err(SchedulePrepareError::TimeUnrepresentable {
                        event_index,
                        position: event.position(),
                    });
                }
            };
            placed.push(CompiledEvent::new(time, event.payload()));
        }

        let epoch = renderer.epoch();
        // The **compiled share**, not the per-quantum cap: ADR-0046 clause 1 partitions the
        // cap across six producers, and this producer spends its own. Carried so the audio
        // thread can bound one call's window without reaching for the profile.
        let max_events_per_quantum = renderer.plan().compiled_event_share();

        let stamped = stamp_compiled(renderer, &placed)?;

        Ok(Self {
            epoch,
            arbiter: None,
            max_events_per_quantum,
            events: stamped,
            next: 0,
        })
    }

    /// Whether every compiled event has been presented successfully.
    pub const fn is_complete(&self) -> bool {
        self.next == self.events.len()
    }
}

const fn payload_plan(payload: CompiledPayload) -> PlanId {
    match payload {
        CompiledPayload::SetParameter { slot, .. } => slot.plan(),
        CompiledPayload::NoteOn { slot } | CompiledPayload::NoteOff { slot } => slot.plan(),
    }
}

#[path = "schedule/hot.rs"]
mod hot;

/// Stamp a compiled list with this stream's epoch, minting an occurrence for every note-on.
///
/// **The sanctioned way to obtain note identities.** A producer cannot mint them itself: the
/// ranges are the plan's, partitioned at admission, and reaching around this would put
/// occurrences outside the partition every other check relies on.
///
/// Shared by the compiled scheduler and the offline renderer rather than written twice: two
/// implementations of "which note-on does this release end" is how they come to disagree.
///
/// The epoch is the renderer's own. Taking one from the caller would let a list be stamped
/// with another stream's epoch: it would succeed, reserve occurrences here, and then be
/// discarded event by event as stale — reserving a producer's whole range against a render
/// that never happens.
///
/// **All or nothing.** A refused list leaves the minter exactly as it found it. Pairing,
/// provenance and producer presence are all decided before the first mint — but minting can
/// fail on its own, because a list can pair correctly and still hold more notes at once than
/// the producer's range admits, and a check for that before the first mint would have to
/// reimplement allocation. So the minting pass works on a copy and assigns it back only on
/// success. Anything less leaves occurrences reserved for a list that was never returned,
/// and the next attempt fails as false over-emission.
pub fn stamp_compiled(
    renderer: &mut PreparedRenderer,
    events: &[CompiledEvent],
) -> Result<Vec<TimedEvent>, SchedulePrepareError> {
    let epoch = renderer.epoch();
    let expected = renderer.plan().id();
    let compiled_producer = renderer.plan().compiled_note_producer();

    // Pass one: everything that can refuse. **Pairing happens here or nowhere** — after
    // stamping a release carries only an occurrence, so this is the last point at which
    // "which note-on does this release end" is answerable from the list itself. One stack
    // per node, and a release takes the most recent unreleased note-on on that node, which
    // is what a keyboard does and what a compiled plan means by a matching pair.
    let mut sounding: Vec<crate::plan::NoteSlot> = Vec::new();
    for (event_index, event) in events.iter().copied().enumerate() {
        // **A note edge's provenance, checked here rather than only in the scheduler.** The
        // renderer's foreign filter compares a note edge's *table*, because `SOUND-INV-017`
        // leaves a release no node to compare — so a foreign node address on a note-on would
        // be stamped with this table's occurrence and pass that filter. This is the last
        // point at which the node is present to check, and `render_offline` reaches it
        // without going through `CompiledEventScheduler::prepare`.
        //
        // A foreign **parameter** slot is deliberately not refused here. The renderer still
        // filters and counts it, which is the documented post-swap behaviour ADR-0043 wants
        // and which `lowering` asserts; the scheduler's own stricter check on that payload is
        // separate and unchanged.
        match event.payload() {
            CompiledPayload::SetParameter { .. } => {}
            CompiledPayload::NoteOn { slot } => {
                if slot.plan() != expected {
                    return Err(SchedulePrepareError::ForeignPlan {
                        event_index,
                        expected,
                        actual: slot.plan(),
                    });
                }
                if compiled_producer.is_none() {
                    return Err(SchedulePrepareError::NoCompiledNoteProducer { event_index });
                }
                sounding.push(slot);
            }
            CompiledPayload::NoteOff { slot } => {
                if slot.plan() != expected {
                    return Err(SchedulePrepareError::ForeignPlan {
                        event_index,
                        expected,
                        actual: slot.plan(),
                    });
                }
                let Some(position) = sounding.iter().rposition(|node| *node == slot) else {
                    return Err(SchedulePrepareError::UnmatchedRelease { event_index });
                };
                let _ = sounding.remove(position);
            }
        }
    }

    // Pass two: mint. **A release frees its index here**, which is what makes a producer's
    // declared range its *polyphony* rather than the note count of a whole piece. The
    // renderer does not see this: it keeps its own registry, written by the events in the
    // order it applies them, so an index reissued below is still resolvable at both of the
    // occurrences that used it.
    let mut open: Vec<crate::identity::NoteIdentity> = Vec::new();
    let mut open_nodes: Vec<crate::plan::NoteSlot> = Vec::new();
    // Commit-or-discard: every `?` below drops this copy, and the renderer's own minter is
    // untouched until the assignment after the loop.
    let mut minter = renderer.minter_mut().working_copy();
    let mut stamped = Vec::with_capacity(events.len());
    for (event_index, event) in events.iter().copied().enumerate() {
        let payload = match event.payload() {
            CompiledPayload::SetParameter { slot, value } => {
                EventPayload::SetParameter { slot, value }
            }
            CompiledPayload::NoteOn { slot } => {
                let Some(producer) = compiled_producer else {
                    return Err(SchedulePrepareError::NoCompiledNoteProducer { event_index });
                };
                let identity = minter.mint(producer, slot).map_err(|source| {
                    SchedulePrepareError::Identity {
                        event_index,
                        source,
                    }
                })?;
                open.push(identity);
                open_nodes.push(slot);
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On { slot },
                }
            }
            CompiledPayload::NoteOff { slot } => {
                // Pass one proved this pairing exists, so the failure branch is unreachable
                // rather than tolerated — and it is written as a refusal anyway, because a
                // silent `continue` would drop an event ADR-0001 clause 16 forbids dropping.
                let Some(position) = open_nodes.iter().rposition(|node| *node == slot) else {
                    return Err(SchedulePrepareError::UnmatchedRelease { event_index });
                };
                let _ = open_nodes.remove(position);
                let identity = open.remove(position);
                minter.release(identity);
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                }
            }
        };
        stamped.push(TimedEvent::new(
            EventEnvelope::new(epoch, event.time(), TimeSource::Compiled),
            payload,
        ));
    }

    // Committed only now that nothing can refuse.
    *renderer.minter_mut() = minter;
    Ok(stamped)
}
