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
//! Preparation then places every admitted position at the transport's anchor, pairs and
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
use crate::stream::StreamControl;
use crate::time::{Located, PlanPosition, SampleTime, StreamEpoch, TimeSource};
use crate::transport::ActivationRefused;

/// One exact event produced by compiled plan time.
///
/// It has no epoch yet because preparation issues that identity. The prepared scheduler
/// stamps every event with the stream's epoch and `Compiled` provenance before the
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
        /// The keyboard position the note names, per `SOUND-INV-021`.
        key: crate::quantities::KeyIdentity,
        /// How hard it was struck.
        velocity: crate::quantities::NoteVelocity,
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
    /// loop interval, so the four loop-shaped variants — the two malformed-interval ones,
    /// [`AdmissionError::LoopWindowOverShare`] and
    /// [`AdmissionError::LoopPolyphonyOverProducer`] — describe checks this path does not
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
    /// This stream already has a schedule, and a stream has exactly one.
    ///
    /// Two schedulers prepared from one control would each hold their own exchange and their
    /// own copy of the sequence in force, so a candidate superseding that sequence could be
    /// accepted by **both** — and since adoption is infallible, both would then adopt. The
    /// first-adopted-wins rule `SOUND-INV-018` states is a property of one exchange, and an
    /// independent review found that a second scheduler silently created a second one.
    ///
    /// A schedule is replaced by an **activation**, which is what ADR-0050 exists to
    /// provide; preparing another scheduler is not the mechanism for it.
    #[error("this stream already has a schedule; a replacement is an activation")]
    SchedulerExists,

    /// A candidate holds a snapshot of the minter that this stamping would strand.
    ///
    /// ADR-0050 clause 3 stamps a candidate against a copy of the authoritative table and
    /// promotes that copy at collection. A stamping committed in between advances generations
    /// the copy has never seen, and promotion would then **rewind** them — after which a
    /// later note could be given an identity that is already live. `SOUND-INV-017`'s
    /// never-reused generation is the invariant that forbids it, and refusing here is what
    /// keeps it true: the copy is what makes an abandoned candidate free, and the price is
    /// that the authoritative table stands still while one is outstanding.
    #[error("{outstanding} activation candidates are outstanding, so the minter may not move")]
    CandidateOutstanding {
        /// How many candidates have been built and neither withdrawn nor collected.
        outstanding: usize,
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

    /// An admitted position lies before the stream's anchor.
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
    /// An activation's displacement moved an event past the end of engine time.
    ///
    /// ADR-0050 clause 1 displaces every placed event by `effective - requested`, and a
    /// far-future event under a large shift can leave the representable range. It is an
    /// error rather than the end of the list: a scan that stopped here would silently drop
    /// that event and every one after it while the cursor never reached completion, which
    /// ADR-0001 clause 16 forbids and which no counter would show.
    #[error("an event at {time} displaced by {shift} has no representable engine time")]
    EventTimeUnrepresentable {
        /// The event's stamped time, before displacement.
        time: SampleTime,
        /// The activation shift in force.
        shift: crate::time::FrameCount,
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

    /// An ingress store this stream never adopted was supplied to the drain.
    ///
    /// The off-thread half marks a store when it adopts it, and this reads that mark rather
    /// than keeping a second latch: two latches can disagree, and a caller offering into
    /// store A while rendering store B wedged the stream permanently.
    ///
    /// **Refusing strands nothing**, because every offer goes through the control that sets
    /// the mark, so an unadopted store is empty. What it prevents is that empty store's zero
    /// counters overwriting the adopted one's totals in the report, since the drain mirrors
    /// them rather than accumulating.
    #[error("ingress store {store} was not adopted by stream {stream}")]
    UnadoptedIngressStore {
        /// The store that was supplied.
        store: crate::ingress::IngressStoreId,
        /// The stream that was asked to drain it.
        stream: crate::time::StreamEpoch,
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
    /// The activation sequence in force for this stream.
    ///
    /// Changes at adoption alone (clause 6), which is what lets an abandoned candidate
    /// consume nothing and what orders two candidates built against one value.
    in_force: crate::transport::ActivationSequence,
    /// ADR-0050's single-slot exchange, in both directions.
    exchange: crate::transport::Exchange,
    /// Whether an adopted activation still owes the session share its mass-release charge.
    ///
    /// ADR-0050 clause 5's release runs inside adoption, between renderer calls, where no
    /// publication window is open. The charge therefore waits for the first call that
    /// renders a quantum — which is not always the next call, because one served entirely
    /// from the carry opens a window with no rows at all.
    owed_release_charge: bool,
    /// ADR-0051 clause 1's catch-up batch, swapped in at adoption and spent once.
    ///
    /// The buffer is kept after it is spent so adoption can swap into it again without
    /// allocating on the audio thread; `catch_up_len` is what says how much of it is live.
    catch_up: Vec<crate::render::TimedEvent>,
    catch_up_len: usize,
    /// How far the placed schedule is displaced from where it was stamped.
    ///
    /// ADR-0050 clause 1. A candidate is stamped against the time it **requested**, and
    /// adoption may snap that forward to a quantum boundary; every placed event is then
    /// displaced by the same `effective - requested`. The displacement is uniform, so an
    /// `O(1)` value reproduces it and no per-event pass runs on the audio thread — which is
    /// the whole reason it is stored rather than applied by rewriting the list.
    ///
    /// This is a **placement**, not a clamp: an event's engine time is a function of its plan
    /// position and the anchor, and the anchor moved. The published stamp is the time the
    /// event genuinely happens.
    ///
    /// A [`FrameCount`](crate::time::FrameCount) rather than a `u64`: it is a distance in
    /// frames that persists in the stream's state and is added to a [`SampleTime`], so the
    /// unit belongs in the type rather than in the name.
    shift: crate::time::FrameCount,
}

impl CompiledEventScheduler {
    /// Place an admitted stream at the transport's anchor and stamp it.
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
    /// **The anchor comes from the control**, which ADR-0050 clause 9 makes its owner. An
    /// earlier revision took it as an argument so that a caller could state a post-seek
    /// pairing the renderer did not yet hold; with an owner there is nothing for a caller to
    /// state, and the class of error that argument existed to catch cannot arise.
    ///
    /// **The renderer is not a parameter, and that is the point of clause 9.** An earlier
    /// revision took `&PreparedRenderer` to read the plan and the epoch from it; an
    /// independent review established that this splits the ownership in name only, because
    /// an off-thread builder cannot hold a shared borrow of a value the audio thread
    /// mutates. The control carries the plan and the epoch, so nothing here touches the
    /// audio-thread half.
    ///
    /// A schedule prepared against one stream and rendered by another is refused where it
    /// becomes wrong rather than here: the stamped events carry this control's epoch, and
    /// `render` refuses a renderer whose epoch differs. Checking it at preparation would
    /// need the renderer back, which is what this signature exists to avoid.
    pub fn prepare(
        control: &mut StreamControl,
        stream: &AdmittedCompiledStream,
    ) -> Result<Self, SchedulePrepareError> {
        let expected = control.plan_id();
        if stream.plan() != expected {
            return Err(SchedulePrepareError::ForeignStream {
                expected,
                actual: stream.plan(),
            });
        }

        // The **transport's** pairing, from the half that owns it. ADR-0050 clause 9 puts the
        // anchor on the control side, so this is read rather than supplied — a caller cannot
        // state a pairing it does not hold, which removes the whole class of error the
        // explicit argument existed to catch.
        let anchor = control.anchor();

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

        let epoch = control.epoch();
        // The **compiled share**, not the per-quantum cap: ADR-0046 clause 1 partitions the
        // cap across six producers, and this producer spends its own. Carried so the audio
        // thread can bound one call's window without reaching for the profile.
        let max_events_per_quantum = control.plan().compiled_event_share();

        // One schedule per stream, and it is the exchange that makes this a rule rather than
        // a preference: two schedulers are two exchanges, and one candidate accepted by each
        // would be adopted twice.
        if control.has_scheduler() {
            return Err(SchedulePrepareError::SchedulerExists);
        }

        let stamped = stamp_compiled(control, &placed)?;
        control.scheduler_prepared();
        Ok(Self {
            epoch,
            arbiter: None,
            max_events_per_quantum,
            events: stamped,
            next: 0,
            // The stream's first and only schedule, so the transport state it starts from is
            // the stream's own beginning. An earlier revision inherited these from the
            // control to make a *second* scheduler behave; the rule above is what replaced
            // that, because two schedulers were never one stream.
            in_force: crate::transport::ActivationSequence::INITIAL,
            exchange: crate::transport::Exchange::Empty,
            owed_release_charge: false,
            catch_up: Vec::new(),
            catch_up_len: 0,
            shift: crate::time::FrameCount::ZERO,
        })
    }

    /// Whether every compiled event has been presented successfully.
    pub const fn is_complete(&self) -> bool {
        self.next == self.events.len()
    }

    /// The activation sequence this stream is running under.
    pub const fn in_force(&self) -> crate::transport::ActivationSequence {
        self.in_force
    }

    /// Offer an activation, which the next boundary at or after its time adopts.
    ///
    /// **Every refusal is here**, which is what makes adoption infallible. There are six, and
    /// this is the order they are decided in: a schedule paired with another stream's
    /// renderer, a stream that has already faulted, a stale epoch, unsupported loop playback,
    /// an occupied exchange slot, and a superseded sequence. The last two are in that order
    /// deliberately — an
    /// uncollected retirement is *why* a candidate is superseded, so reporting the
    /// consequence would send a reader after a racing seek when the fix is that the
    /// off-thread half has not collected. Each leaves the stream running on the state in
    /// force, and the candidate comes back so the control can withdraw it — which means
    /// dropping the working copy it stamped against and restoring nothing because nothing
    /// was taken.
    ///
    /// **Five of the six are counted; the pairing is not**, and that is why it is decided
    /// first: the counters belong to the stream that was offered to, and a renderer that is
    /// not this schedule's half is not that stream. Counting matters because a stream that
    /// silently declines every seek and one that adopts them are otherwise
    /// indistinguishable.
    pub fn offer(
        &mut self,
        renderer: &mut PreparedRenderer,
        activation: Box<crate::transport::TransportActivation>,
    ) -> Result<
        (),
        (
            Box<crate::transport::TransportActivation>,
            ActivationRefused,
        ),
    > {
        // The pairing first, because everything below writes to the renderer's counters and
        // a foreign renderer's counters are another stream's report.
        if self.epoch != renderer.epoch() {
            let refusal = ActivationRefused::ForeignRenderer {
                schedule: self.epoch,
                renderer: renderer.epoch(),
            };
            return Err((activation, refusal));
        }
        // A faulted epoch adopts nothing ever again, so accepting a candidate into it would
        // trap the candidate: adoption never comes, `collect` never yields it, and the control
        // cannot withdraw a value it no longer holds.
        if renderer.diagnostics().needs_reprepare() {
            renderer.count_refused_activation();
            return Err((activation, ActivationRefused::StreamFaulted));
        }
        if activation.epoch != self.epoch {
            renderer.count_refused_activation();
            let refusal = ActivationRefused::StaleEpoch {
                candidate: activation.epoch,
                stream: self.epoch,
            };
            return Err((activation, refusal));
        }
        // The interval has already passed its off-thread density and polyphony checks, but
        // no runtime wrap exists. Accepting it would expose a loop as active while the
        // schedule continues beyond its end. This refusal is the executable pull-forward
        // guard on the deferred sample-exact wrap obligation.
        if let Some(interval) = activation.loop_interval() {
            renderer.count_refused_activation();
            let refusal = ActivationRefused::LoopPlaybackUnsupported {
                start: interval.start(),
                end: interval.end(),
            };
            return Err((activation, refusal));
        }
        // **The slot is asked before the sequence**, and the order is the diagnostic rather
        // than an arbitrary choice. An uncollected retired value is *why* the control's idea
        // of what is in force is stale, so a candidate built in that window is superseded as
        // a consequence. Reporting the consequence would send a reader to look for a racing
        // seek when the actual fix is that the off-thread half has not collected — and it
        // would leave `RetiredUncollected` unreachable, which is a rule nobody checks.
        if let Some(refusal) = self.exchange.occupied() {
            renderer.count_refused_activation();
            return Err((activation, refusal));
        }
        if activation.supersedes != self.in_force {
            renderer.count_refused_activation();
            let refusal = ActivationRefused::Superseded {
                supersedes: activation.supersedes,
                in_force: self.in_force,
            };
            return Err((activation, refusal));
        }
        // ADR-0050 clause 1's lateness, decided **here**: the clock has already passed the
        // time this candidate names, so building it took longer than the time it asked for.
        // This is the only moment the question has a stable answer — by the call that adopts,
        // the clock stands on the boundary and every off-grid request looks late.
        let mut activation = activation;
        activation.late = activation.requested() < renderer.clock();
        self.exchange = crate::transport::Exchange::Pending(activation);
        Ok(())
    }

    /// Take the retired activation, if adoption has produced one.
    ///
    /// The off-thread half calls this. Until it does, the slot is occupied and a further
    /// offer is refused — which is `HOST-INV-022`'s backpressure and the signal that this
    /// half has fallen behind. Collecting is also what tells the control that adoption
    /// happened, and therefore when to promote the working copy it has been holding.
    pub fn collect(&mut self) -> Option<Box<crate::transport::TransportActivation>> {
        match std::mem::take(&mut self.exchange) {
            crate::transport::Exchange::Retired(retired) => Some(retired),
            other => {
                self.exchange = other;
                None
            }
        }
    }
}

const fn payload_plan(payload: CompiledPayload) -> PlanId {
    match payload {
        CompiledPayload::SetParameter { slot, .. } => slot.plan(),
        CompiledPayload::NoteOn { slot, .. } | CompiledPayload::NoteOff { slot } => slot.plan(),
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
/// The epoch is the **stream's own**, read from the control rather than taken as an
/// argument. Taking one from the caller would let a list be stamped with another stream's
/// epoch: it would succeed, reserve occurrences here, and then be discarded event by event
/// as stale — reserving a producer's whole range against a render that never happens. The
/// plan comes from the same place, so there is nothing for a caller to pair wrongly: the
/// control holds both, and ADR-0050 clause 9 is why it holds the plan at all.
///
/// The occurrences the list leaves **outstanding** — the note-ons it never paired — are
/// recorded on the control rather than returned. Those are what the schedule still reserves,
/// and ADR-0050 clause 3 is what consumes them, from a value the audio thread does not own.
///
/// **All or nothing.** A refused list leaves the minter exactly as it found it. Pairing,
/// provenance and producer presence are all decided before the first mint — but minting can
/// fail on its own, because a list can pair correctly and still hold more notes at once than
/// the producer's range admits, and a check for that before the first mint would have to
/// reimplement allocation. So the minting pass works on a copy and assigns it back only on
/// success. Anything less leaves occurrences reserved for a list that was never returned,
/// and the next attempt fails as false over-emission.
pub fn stamp_compiled(
    control: &mut StreamControl,
    events: &[CompiledEvent],
) -> Result<Vec<TimedEvent>, SchedulePrepareError> {
    // Refused before anything is read: a candidate outstanding means a snapshot of this
    // minter exists that promotion will install, and any generation this stamping advanced
    // would be rewound by it.
    let outstanding = control.outstanding_candidates();
    if outstanding > 0 {
        return Err(SchedulePrepareError::CandidateOutstanding { outstanding });
    }

    let epoch = control.epoch();
    // Commit-or-discard against the control's own table, which is what [`stamp_into`] does
    // not decide: it works on whatever minter it is handed, and this is the caller that
    // hands it the authoritative one.
    let plan = std::sync::Arc::clone(control.plan_arc());
    let mut minter = control.minter_mut().working_copy();
    let (stamped, outstanding) = stamp_into(&mut minter, &plan, epoch, events)?;
    *control.minter_mut() = minter;
    // What this list left live, recorded where a replacement can reach it without borrowing
    // the audio thread's schedule.
    control.add_outstanding(&outstanding);
    Ok(stamped)
}

/// The same stamping, against a minter the caller owns.
///
/// ADR-0050 clause 3's candidate build is the other caller: it stamps against a **working
/// copy** with the outgoing schedule's reservations already released, so that abandoning the
/// candidate costs nothing and the two schedules never compete for a producer's range.
///
/// **This does not commit.** Whatever it mints stays in the minter it was given, and what
/// happens to that minter is the caller's decision — which is exactly the difference between
/// the two entry points.
pub fn stamp_into(
    minter: &mut crate::identity::IdentityTable,
    plan: &CompiledPlan,
    epoch: StreamEpoch,
    events: &[CompiledEvent],
) -> Result<(Vec<TimedEvent>, Vec<crate::identity::NoteIdentity>), SchedulePrepareError> {
    let expected = plan.id();
    let compiled_producer = plan.compiled_note_producer();

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
            CompiledPayload::NoteOn { slot, .. } => {
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
    let mut stamped = Vec::with_capacity(events.len());
    for (event_index, event) in events.iter().copied().enumerate() {
        let payload = match event.payload() {
            CompiledPayload::SetParameter { slot, value } => {
                EventPayload::SetParameter { slot, value }
            }
            CompiledPayload::NoteOn {
                slot,
                key,
                velocity,
            } => {
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
                    edge: NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
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

    // `open` is what the list never paired, and it is exactly what this schedule still
    // reserves in the allocator: every paired release already freed its index during the
    // pass above. ADR-0050 clause 3 has the control release this set into the working copy
    // it stamps a replacement against, so the outgoing schedule and the candidate never
    // compete for a producer's range.
    Ok((stamped, open))
}
