//! Compiled-event scheduling at the renderer's engine-time boundary.
//!
//! Phase 3 starts here, deliberately on the engine side of ADR-0022. A compiled event
//! already carries the current epoch's [`SampleTime`]; this module does not observe a
//! host clock, compensate latency, or assign `Hardware`/`Arrival` provenance. It rejects
//! a quantum over the compiled producer's own share during preparation, then turns the
//! sorted compiled list into a bounded event span for each actual host call. Sliding-window
//! admission over every anchor phase remains later Phase 3 work.
//!
//! Preparation validates order and plan identity and allocates the stamped list. The
//! [`CompiledEventScheduler::render`] path advances a cursor over that storage and
//! **publishes** the due span through ADR-0046's arbiter — it does not hand the renderer a
//! borrowed slice, which would make clause 2's "the only normal path that constructs
//! renderer input" false for this producer. Its source lives in `schedule/hot.rs` so the
//! real-time purity test scans it alongside the renderer, the arbiter and node kernels.

use thiserror::Error;

use crate::plan::PlanId;
use crate::quantities::EventCount;
use crate::render::{EventEnvelope, EventPayload, NoteEdge, PreparedRenderer, TimedEvent};
use crate::time::{SampleTime, StreamEpoch, TimeSource};

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

    /// The compiled list is not monotone in engine time.
    #[error("compiled event {event_index} at {time} precedes the previous event at {previous}")]
    EventsOutOfOrder {
        /// Position in the supplied event list.
        event_index: usize,
        /// The preceding event's time.
        previous: SampleTime,
        /// The rejected event's time.
        time: SampleTime,
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
    /// One absolute quantum exceeds the renderer's prepared event capacity.
    #[error(
        "compiled quantum beginning at {quantum_start} contains more than its admitted \
         {admissible}"
    )]
    QuantumTooDense {
        /// First engine-time position in the over-full quantum.
        quantum_start: SampleTime,
        /// Maximum event count admitted for one quantum.
        admissible: EventCount,
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
    /// Validate and stamp a compiled list for `renderer`.
    ///
    /// Equal times preserve caller order. Descending time is refused instead of sorted,
    /// because ADR-0023 has not yet selected a same-sample ordering policy and this layer
    /// must not invent one as a side effect of preparation. A statically over-full
    /// absolute quantum is also refused here, before the schedule reaches the audio
    /// thread.
    pub fn prepare(
        renderer: &mut PreparedRenderer,
        events: &[CompiledEvent],
    ) -> Result<Self, SchedulePrepareError> {
        let expected = renderer.plan().id();
        // The **compiled share**, not the per-quantum cap. ADR-0046 clause 1 partitions the
        // cap across six producers, and clause 3 makes a compiled runtime miss a producer
        // defect rather than a load condition — so a schedule that would overrun its share
        // at publication has to be refused here, where a caller can still be told.
        let max_events_per_quantum = renderer.plan().compiled_event_share();
        let mut previous = None;
        let mut current_quantum = None;
        let mut events_in_quantum = 0_u32;

        for (event_index, event) in events.iter().copied().enumerate() {
            if let Some(previous_time) = previous
                && event.time() < previous_time
            {
                return Err(SchedulePrepareError::EventsOutOfOrder {
                    event_index,
                    previous: previous_time,
                    time: event.time(),
                });
            }
            previous = Some(event.time());

            let actual = payload_plan(event.payload());
            if actual != expected {
                return Err(SchedulePrepareError::ForeignPlan {
                    event_index,
                    expected,
                    actual,
                });
            }

            let event_quantum = event.time().quantum_index();
            if current_quantum == Some(event_quantum) {
                if events_in_quantum == max_events_per_quantum.get() {
                    return Err(SchedulePrepareError::QuantumTooDense {
                        quantum_start: event.time().quantum_start(),
                        admissible: max_events_per_quantum,
                    });
                }
                events_in_quantum += 1;
            } else {
                current_quantum = Some(event_quantum);
                events_in_quantum = 1;
            }
        }

        let epoch = renderer.epoch();

        let stamped = stamp_compiled(renderer, events)?;

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
