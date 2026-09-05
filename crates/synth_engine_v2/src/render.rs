//! The renderer: the host-driven boundary and the quantum splitting contract.
//!
//! ADR-0001 is what this module implements. The renderer never processes a partial
//! quantum, control is evaluated exactly once per quantum, and a caller block of any
//! size up to the profile's maximum is served from a carry that was primed with `Q`
//! frames of silence at stream start. Partition invariance follows by construction
//! rather than by test: the sequence of quanta does not depend on how the caller
//! chops its requests.
//!
//! # What the render loop may not do
//!
//! No heap allocation, no lock, no I/O, no logging. Every buffer is allocated at
//! preparation, the event scratch is preallocated to the capacity admission approved,
//! and the one sort the loop performs is in place. A caller precondition returns a
//! [`RenderError`]; a `debug_assert` cannot define release behaviour because it
//! compiles out of the build that runs.

use crate::diagnostics::{CompileError, DiagnosticsReport, RenderError};
use crate::node::kernels::{NodeState, TimedControl};
use crate::plan::{CompiledPlan, PlanOp};
use crate::quantities::{ChannelLayout, EventCount, ParameterValue, RecordCount};
use crate::time::{FrameCount, QUANTUM_FRAMES, SampleTime, StreamAnchor, StreamEpoch, TimeSource};

/// How many bytes preparing a renderer will allocate for one call's event
/// resolution, given a plan's capacities.
///
/// It lives next to the allocation it describes and is called by admission, so the two
/// cannot drift: an earlier revision computed the scratch budget from the audio buffers
/// alone, so a raised `max_events_per_quantum` or a large `maximum_block_size` could be
/// reported as fitting and then allocate far past the budget at preparation.
#[must_use]
pub fn event_scratch_bytes(
    max_events_per_quantum: EventCount,
    maximum_block_size: FrameCount,
) -> u64 {
    let quantum = u64::from(QUANTUM_FRAMES);
    let quanta_per_call = maximum_block_size
        .as_u64()
        .div_ceil(quantum)
        .saturating_add(1);
    let events = u64::from(max_events_per_quantum.get()).saturating_mul(quanta_per_call);
    events
        .saturating_mul(size_of::<DueEvent>() as u64)
        .saturating_add(quanta_per_call.saturating_mul(size_of::<u32>() as u64))
}

/// How many bytes preparing a renderer will allocate for the note-identity halves,
/// given a plan's admitted producer partition.
///
/// Both halves are sized by the same partition, and **both hold the ranges**: the minter has
/// one slot and the registry one entry per admitted index, and each keeps its own copy of the
/// per-producer spans. The registry needs them because ADR-0050 clause 5 scopes its mass
/// release by producer, and a budget that charged one copy would report a ceiling preparation
/// then allocates past. It lives next to the allocation it describes for the reason
/// [`event_scratch_bytes`] does — a budget computed anywhere else drifts from what
/// preparation takes.
#[must_use]
pub fn identity_bytes(producer_ranges: &[crate::quantities::HeldNoteCount]) -> u64 {
    let mut indices = 0_u64;
    for range in producer_ranges {
        indices = indices.saturating_add(u64::from(range.get()));
    }
    let per_index = (size_of::<crate::identity::Slot>()
        + size_of::<Option<crate::identity::LiveNote>>()) as u64;
    // Two range tables, one per half.
    let spans = (producer_ranges.len() as u64)
        .saturating_mul(size_of::<crate::identity::Range>() as u64)
        .saturating_mul(2);
    indices.saturating_mul(per_index).saturating_add(spans)
}

/// How many bytes preparing a renderer will allocate for the sample-positioned
/// control scratch, given a plan's per-quantum event capacity and how many records it
/// schedules.
///
/// Beside the allocation for the same reason [`event_scratch_bytes`] is: admission has to
/// state what preparation takes, and two formulas for one allocation drift. One quantum's
/// events bound how many sample-positioned changes that quantum can carry, and the two
/// index tables are one entry per scheduled record plus a terminator.
#[must_use]
pub fn timed_control_scratch_bytes(
    max_events_per_quantum: EventCount,
    scheduled_records: RecordCount,
    identity_indices: crate::quantities::HeldNoteCount,
    writes_per_note: crate::quantities::WritesPerNote,
) -> u64 {
    // One quantum's events **times what one of them can expand to**, plus the notes an
    // activation can end at a boundary.
    //
    // `SOUND-INV-021` makes a note-on more than one control write — its gate and the
    // magnitudes describing the note that gate starts — and the invariant requires admission
    // to charge for the expansion. A budget of one write per event is overrun by the first
    // note whose scope declares a pitch and a velocity destination, so the worst case is
    // every event in the quantum being the plan's widest note-on.
    //
    // ADR-0050 clause 5's mass release lowers a gate per note it ends, and a gate reaches a
    // kernel only as a sample-positioned control — so those changes land in this scratch
    // beside the quantum's own. They are not events and are charged to no share, which is
    // exactly why they need room here rather than there: an activation that ended more
    // notes than a quantum admits events would otherwise have nowhere to put them. A
    // release expands to no magnitudes, so it is not multiplied.
    let controls = u64::from(max_events_per_quantum.get())
        .saturating_mul(u64::from(writes_per_note.get()))
        .saturating_add(u64::from(identity_indices.get()))
        .saturating_mul(size_of::<TimedControl>() as u64);
    // And the queue those gate-downs wait in between adoption and the boundary quantum: one
    // control and one node index per identity the partition holds. Preparation allocates
    // both, so a budget that charged only the scratch above would report a ceiling
    // preparation then allocates past — which is admission passing a plan it should refuse.
    // An independent review found the two vectors uncharged.
    let adoption_gates = u64::from(identity_indices.get())
        .saturating_mul((size_of::<TimedControl>() + size_of::<usize>()) as u64);
    let index = u64::from(scheduled_records.get())
        .saturating_mul(2)
        .saturating_add(1)
        .saturating_mul(size_of::<u32>() as u64);
    controls
        .saturating_add(adoption_gates)
        .saturating_add(index)
}

/// A caller's output block: interleaved samples the renderer writes.
#[derive(Debug)]
#[must_use]
pub struct AudioBlockMut<'a> {
    samples: &'a mut [f32],
    frames: usize,
    layout: ChannelLayout,
}

impl<'a> AudioBlockMut<'a> {
    /// Split the block at a frame, giving two blocks over the same buffer.
    ///
    /// ADR-0050 clause 4's mechanism. A host block whose quanta straddle an activation
    /// boundary is rendered as two calls, adopting between them, and this is what makes the
    /// two calls possible without copying: the same samples, described as two spans.
    ///
    /// That the audio is unchanged by the split is not an assumption — it is ADR-0001's
    /// partition invariance, which the four-partition test already asserts and which this
    /// only reuses.
    ///
    /// A split outside the block gives the block **back** rather than consuming it, because
    /// a zero-length half is not a call: the renderer would be asked to serve nothing while
    /// the caller still had its whole block to fill. Returning `Err(self)` is what lets the
    /// one caller fall through to rendering it whole without a second borrow.
    pub fn split_at_frame(self, frames: usize) -> Result<(Self, Self), Self> {
        if frames == 0 || frames >= self.frames {
            return Err(self);
        }
        let channels = self.layout.channels();
        let Some(cut) = frames.checked_mul(channels) else {
            return Err(self);
        };
        let rest = self.frames - frames;
        let layout = self.layout;
        let (head, tail) = self.samples.split_at_mut(cut);
        Ok((
            Self {
                samples: head,
                frames,
                layout,
            },
            Self {
                samples: tail,
                frames: rest,
                layout,
            },
        ))
    }

    /// Borrow the same block again, for a caller that must keep it after lending it.
    ///
    /// ADR-0050 clause 4's split path needs both: it hands each half to a renderer call and
    /// must still be able to silence both if either call faults, because the terminal
    /// contract is silence over the **complete** callback rather than over the span the
    /// fault happened in.
    pub fn reborrow(&mut self) -> AudioBlockMut<'_> {
        AudioBlockMut {
            samples: self.samples,
            frames: self.frames,
            layout: self.layout,
        }
    }

    /// Write silence over the whole block.
    ///
    /// A fill rather than a loop, and no allocation: this is reachable from the audio
    /// thread's terminal path, where the alternative to silence is playing whatever the
    /// caller's buffer happened to hold.
    pub fn silence(&mut self) {
        self.samples.fill(0.0);
    }

    /// Wrap a caller's buffer, checking that its shape is what it claims.
    ///
    /// The check is here rather than in the render loop so that the loop can index
    /// without branching on shape, and so that a wrong shape is a caller error with a
    /// diagnostic rather than a panic on the audio thread.
    pub fn new(
        samples: &'a mut [f32],
        frames: usize,
        layout: ChannelLayout,
    ) -> Result<Self, RenderError> {
        let needed = frames.saturating_mul(layout.channels());
        if samples.len() != needed {
            return Err(RenderError::OutputBufferShape {
                samples: samples.len(),
                frames,
                layout,
                needed,
            });
        }
        Ok(Self {
            samples,
            frames,
            layout,
        })
    }

    /// How many frames the caller asked for.
    pub const fn frames(&self) -> usize {
        self.frames
    }

    /// The block's channel layout.
    pub const fn layout(&self) -> ChannelLayout {
        self.layout
    }
}

/// What every event carries in addition to its payload.
///
/// ADR-0032 clause 17. It is `Copy` and fixed-size, so it crosses a lock-free queue
/// under the audio thread's rules, and the `epoch` is what makes an event from a dead
/// stream detectable rather than merely unlikely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct EventEnvelope {
    epoch: StreamEpoch,
    time: SampleTime,
    source: TimeSource,
}

impl EventEnvelope {
    /// Stamp an event.
    pub const fn new(epoch: StreamEpoch, time: SampleTime, source: TimeSource) -> Self {
        Self {
            epoch,
            time,
            source,
        }
    }

    /// The epoch the event was stamped in.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// When the event says it happens.
    ///
    /// **Immutable.** Nothing the renderer does rewrites it: a clamp moves the
    /// *render position*, and the declared sample survives, so a report can say how
    /// far an event was displaced and a recorded performance is never quantized
    /// forward by an overload.
    pub const fn time(&self) -> SampleTime {
        self.time
    }

    /// Where the timestamp came from.
    pub const fn source(&self) -> TimeSource {
        self.source
    }
}

/// Which way a note edge goes.
///
/// An **edge**, not a level: a note is played or let go, and ADR-0001 clause 14 names both
/// as sample-positioned effects. Carrying a float here instead would invite the question
/// of what a gate of `0.5` means, which no caller has ever needed to ask.
///
/// The on edge carries the two magnitudes `SOUND-INV-021` gives it and no others. A
/// release carries neither: what a note's key and velocity describe is the note the gate
/// starts, and release velocity is Phase 6's expression model.
///
/// **Not `Eq`**, because a velocity is a float. It is `PartialEq` like every other payload
/// in this module, and nothing compares note edges for identity — that is what the
/// occurrence is for.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub enum NoteEdge {
    /// The note is played, on this compiled node.
    ///
    /// The node rides on this edge and **only** this edge. `SOUND-INV-017` keeps it off the
    /// release rather than carrying it there and requiring agreement: an event whose
    /// identity names one occurrence and whose node names another has no safe reading, so
    /// the case is made unrepresentable instead of adjudicated.
    On {
        /// Which compiled node is played.
        slot: crate::plan::NoteSlot,
        /// Which keyboard position the note names.
        ///
        /// A **key**, not a frequency: ADR-0025 selects a pre-tuning event contract, so what
        /// frequency this is belongs to the prepared tuning the plan resolves it through.
        key: crate::quantities::KeyIdentity,
        /// How hard it was struck.
        velocity: crate::quantities::NoteVelocity,
    },
    /// The note is let go. The occurrence knows its node; nothing here has to.
    Off,
}

impl NoteEdge {
    /// The control value this edge sets.
    ///
    /// Any value above zero raises a gate and zero lowers it, so an edge is one of exactly
    /// two values. Named constants rather than a fallible conversion, because the audio
    /// thread has no way to report a failure and no fallback here would be honest.
    pub const fn value(self) -> ParameterValue {
        match self {
            Self::On { .. } => ParameterValue::ONE,
            Self::Off => ParameterValue::ZERO,
        }
    }
}

/// What an event does.
///
/// **When** it takes effect is not this enum's to say. ADR-0001 clause 14, as ADR-0043
/// restated it, splits on the *effect*: a sample-positioned one — note-on, note-off, gate,
/// retrigger — occurs at the sample its **render position** names, while a control-rate
/// response begins at the first quantum boundary at or after that position, under clause
/// 13's causality rule. The render position is the declared sample unless the preserving
/// late clamp moves a genuinely late event to the first not-yet-rendered boundary. The node
/// kind declares which of the two each of its controls is, admission compiles that into the
/// target, and the renderer reads it. So addressing a gate as a parameter and playing its
/// node as a note reach the same control under the same timing law, and neither payload can
/// be used to escape the other's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventPayload {
    /// Set one compiled parameter slot.
    ///
    /// The slot, not the `(node, parameter)` pair: [`CompiledPlan::resolve_parameter`]
    /// turns an address into one **off the audio thread**, so the render loop indexes
    /// instead of searching, and an address the plan does not have is caught where a
    /// caller can still be told about it.
    SetParameter {
        /// Which compiled parameter.
        slot: crate::plan::ParameterSlot,
        /// The new value, validated where it was built.
        value: ParameterValue,
    },
    /// Play or let go of one compiled node.
    ///
    /// The note-side twin of [`Self::SetParameter`], resolved by
    /// [`CompiledPlan::resolve_note`] under the same rule. It names the node rather than
    /// one of its controls: what being played *means* belongs to the node kind, which is
    /// what lets a caller play a voice without knowing the graph inside it.
    Note {
        /// Which occurrence this edge belongs to.
        ///
        /// `SOUND-INV-017`: the occurrence is the sole authority for which note an event
        /// resolves to. Both edges carry it; only the on edge also names a node.
        identity: crate::identity::NoteIdentity,
        /// Which way the edge goes.
        edge: NoteEdge,
    },
    /// ADR-0058: fade the voice this occurrence sounds on to silence over `frames`, from
    /// this event's position. The occurrence is ended as it is applied; the note that takes
    /// the voice follows as a [`Self::Reset`] and a [`Self::Note`] at the position the fade
    /// completes. Stamped by preparation, off the audio thread, never by a producer.
    Fade {
        /// The occurrence whose voice is taken.
        identity: crate::identity::NoteIdentity,
        /// How long the fade runs.
        frames: FrameCount,
    },
    /// ADR-0058: restore every step of the voice instance this occurrence's index names to
    /// its prepared state at this event's position, so the note that follows attacks from
    /// silence on a fresh instance. Stamped by preparation, as [`Self::Fade`] is.
    Reset {
        /// The occurrence about to start on the instance, whose index names it.
        identity: crate::identity::NoteIdentity,
    },
}

/// One stamped event.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct TimedEvent {
    envelope: EventEnvelope,
    payload: EventPayload,
}

impl TimedEvent {
    /// An event.
    pub const fn new(envelope: EventEnvelope, payload: EventPayload) -> Self {
        Self { envelope, payload }
    }

    /// The stamp.
    pub const fn envelope(&self) -> EventEnvelope {
        self.envelope
    }

    /// The payload.
    pub const fn payload(&self) -> EventPayload {
        self.payload
    }
}

/// The events one render call is presented with.
///
/// A **prevalidated bounded span**, per the host-profile specification's Phase 1–2
/// presentation rule. The renderer groups these by the same absolute quantum boundaries
/// it renders on, independently of how the caller partitions blocks, and rejects the
/// call if any one quantum exceeds `max_events_per_quantum`. Phase 3 instead presents
/// only sealed, admitted batches for the imminent render call. The span's **total** is
/// not a limit — one call may cover several quanta.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct TimedEvents<'a> {
    events: &'a [TimedEvent],
}

impl<'a> TimedEvents<'a> {
    /// No events.
    pub const EMPTY: Self = Self { events: &[] };

    /// A span.
    pub const fn new(events: &'a [TimedEvent]) -> Self {
        Self { events }
    }

    /// The events, in the order the caller presented them.
    pub const fn as_slice(&self) -> &'a [TimedEvent] {
        self.events
    }
}

/// The host-driven renderer boundary.
pub trait Renderer {
    /// Render one caller block.
    ///
    /// Fallible on purpose: an over-full quantum, an oversized callback, and an
    /// exhausted clock are all conditions a release build has to define, and a
    /// `debug_assert` defines none of them.
    fn render(
        &mut self,
        output: AudioBlockMut<'_>,
        events: TimedEvents<'_>,
    ) -> Result<(), RenderError>;
}

/// One event, resolved to a render position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DueEvent {
    pub(crate) position: SampleTime,
    pub(crate) arrival: u32,
    pub(crate) payload: EventPayload,
    /// The node and control this event moves, resolved once.
    ///
    /// **Resolved before the quantum passes and cached, rather than resolved in each.** A
    /// note edge's node comes from the live-note registry, which the same walk mutates, so a
    /// second resolution would read a different registry and could disagree with the first —
    /// and the two passes agreeing is what makes the counts they produce describe the writes
    /// they perform. `None` is an event with no sample-positioned effect: an orphan release,
    /// or a parameter whose target is control-rate.
    pub(crate) target: Option<ResolvedTarget>,
}

/// Where a sample-positioned event's effect lands, resolved once per call.
///
/// The index of the **parameter slot** it writes, since `P05-S007a`: the node and the
/// control are the target table's row at that index, and the slot state the write is
/// composed through is the parallel table's. One index for both is what keeps a write from
/// reaching a kernel by a path the slot does not see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolvedTarget {
    pub(crate) slot: usize,
    /// How many consecutive rows the write covers: a parameter's whole group for a
    /// `SetParameter`, one row — the note's own voice — for a note edge (`P06-S001`).
    pub(crate) rows: usize,
}

impl DueEvent {
    /// The value the scratch is filled with at preparation.
    ///
    /// Never read: `scratch_len` bounds every read, and the fill exists so the buffer can
    /// be allocated to its full length once. That is what makes growth *impossible* in
    /// the loop rather than merely unlikely — a `Vec::push` with spare capacity does not
    /// allocate, but a `Vec::push` is still a call that can.
    const FILL: Self = Self {
        position: SampleTime::ZERO,
        arrival: 0,
        target: None,
        payload: EventPayload::SetParameter {
            // Never read: `scratch_len` bounds every read of the scratch. The identity
            // is the first a process can issue, so even if it were read it would name a
            // plan this renderer does not hold.
            slot: crate::plan::ParameterSlot::new(crate::plan::PlanId::FILL, 0),
            value: ParameterValue::ZERO,
        },
    };
}

/// Counters a call accumulates before committing them.
///
/// Held locally so that a call which fails validation leaves the published counters
/// untouched: a rejected call must not move renderer state, and the diagnostics report
/// is renderer state.
#[derive(Debug, Clone, Copy, Default)]
struct PendingCounts {
    late: u32,
    stale_epoch: u32,
    foreign_slot: u32,
    arrival_stamped: u32,
    orphan_note: u32,
    /// The most recent orphan's occurrence, for the report's attribution.
    last_orphan_note: Option<crate::identity::NoteIdentity>,
}

/// An admitted plan, prepared for one stream epoch.
#[derive(Debug)]
#[must_use]
pub struct PreparedRenderer {
    /// Shared with [`crate::stream::StreamControl`] rather than copied.
    ///
    /// A compiled plan is immutable for the life of a stream (`SOUND-INV-003`), and both
    /// halves read it: the control to build and place a schedule, the renderer to run one.
    /// One allocation shared is what keeps the resource report honest — a second copy would
    /// be memory admission never accounted for — and the `Arc` is cloned once at
    /// preparation, never on the audio thread.
    plan: std::sync::Arc<CompiledPlan>,
    epoch: StreamEpoch,
    anchor: StreamAnchor,
    clock: SampleTime,
    channels: usize,
    /// Per-source buffers, `Q` frames each, back to back.
    buffers: Vec<f32>,
    /// Interleaved output frames waiting to be served.
    output_carry: Vec<f32>,
    /// How many frames of [`Self::output_carry`] are live.
    carry_frames: usize,
    /// Which occurrences are sounding right now, and on which node.
    ///
    /// **The audio thread's half**, and separate from the minter for the reason
    /// [`crate::identity::LiveNotes`] gives: stamping runs ahead of the render, so the
    /// minter's state is not what is sounding when an event is applied. The minter itself
    /// is [`crate::stream::StreamControl`]'s — ADR-0050 clause 9 gives the two halves
    /// different owners so that a schedule can be built while this one renders.
    live_notes: crate::identity::LiveNotes,
    /// ADR-0001 clause 5's input carry.
    ///
    /// Prepared at `maximum_block_size + Q` frames and **not read in this phase**: no
    /// Phase 1 node consumes live input, and the render signature has no input block
    /// to append. It is allocated because the contract sizes it and the report
    /// accounts for it, and the phase that adds an input-consuming node is the one
    /// that starts reading it. Preparing it here is what keeps that phase from
    /// discovering it needs an allocation on the audio thread.
    input_carry: Vec<f32>,
    node_states: Vec<NodeState>,
    /// `SOUND-INV-023`'s parameter slots, one per row of the plan's target table and
    /// parallel to it by construction. Every write the loop makes to a control — a
    /// `SetParameter`, a note's gate or magnitude, an adopted activation's gate-down — is
    /// composed through the slot at the same index before it reaches node state or a
    /// kernel's control run.
    parameter_slots: Vec<slot::SlotState>,
    /// `SOUND-INV-024`'s control buffers: one quantum of values per quantum-rate slot,
    /// written by the slot's advance before the schedule walk and read per frame by the
    /// kernel. Sample-positioned slots have none; their writes land as timed controls.
    ramp_buffers: Vec<f32>,
    /// Where each quantum-rate slot's buffer starts in [`Self::ramp_buffers`], by slot
    /// index; `usize::MAX` for a slot that has none.
    ramp_offsets: Vec<usize>,
    /// Where each node's run of buffers starts, plus a terminator — the slice a kernel is
    /// handed as its ramps, in its declaration's control order.
    ramp_starts: Vec<usize>,
    /// Whether the plan writes the carry at all, decided once at preparation so the
    /// loop does not re-derive a topology fact per quantum.
    has_output: bool,
    /// Fixed-length scratch for one call's resolved events.
    ///
    /// Allocated to `max_events_per_quantum` times the quanta a call can render, and
    /// written by index — never grown. [`Self::scratch_len`] is how much of it is live.
    event_scratch: Vec<DueEvent>,
    scratch_len: usize,
    quantum_counts: Vec<u32>,
    /// One quantum's sample-positioned control changes, grouped by the node they move.
    ///
    /// Rebuilt before each quantum renders and never grown. Its length is one quantum's
    /// declared event capacity **plus** the identity partition, because an activation's mass
    /// release adds a gate-down per note it ends and those are not events.
    timed_controls: Vec<TimedControl>,
    /// The gate-downs an adopted activation owes, waiting for the quantum at its boundary.
    ///
    /// ADR-0050 clause 5 applies the mass release as one bounded operation rather than as
    /// one event per voice, so these never reach the arbiter and are charged to no share.
    /// They are written here at adoption and merged into the next quantum's controls, which
    /// is the only place a kernel reads a gate.
    ///
    /// Preallocated to the identity partition — the most notes that can be sounding at once
    /// — and written by index. `adoption_gate_len` says how much is live.
    adoption_gates: Vec<TimedControl>,
    /// Which parameter slot each queued gate-down writes, in the same order.
    ///
    /// Beside the control rather than inside it because [`TimedControl`] is what a kernel is
    /// handed and a kernel already knows which node it is: adding a field there would put a
    /// value in the hot slice that nothing reads. The slot rather than the node, because the
    /// slot's target row names the node and the write is composed through the slot.
    adoption_gate_slots: Vec<usize>,
    adoption_gate_len: usize,
    /// Where each node's run of [`Self::timed_controls`] starts, plus a terminator.
    ///
    /// `control_starts[n] .. control_starts[n + 1]` is node `n`'s slice, which is what
    /// lets the schedule walk hand a kernel its own edges by indexing rather than by
    /// searching or by relying on the order node slots appear in.
    control_starts: Vec<u32>,
    /// How far each node's run has been filled, while it is being built.
    control_fill: Vec<u32>,
    diagnostics: DiagnosticsReport,
}

impl PreparedRenderer {
    /// Prepare an admitted plan for one already-issued stream epoch.
    ///
    /// Everything the render loop touches is allocated here. The output carry is
    /// primed with `Q` frames of silence, which is ADR-0001 clause 6 and the reason
    /// clause 5's loop can serve any `N` — including `N < Q` and any irregular
    /// sequence — without rendering a quantum whose input has not arrived.
    ///
    /// **Crate-private, and it takes the epoch and the table identity rather than issuing
    /// them.** [`crate::stream::StreamControl::open`] is the public door, and it is what
    /// keeps the two halves of one stream from being mismatched: the registry built here
    /// must answer to the minter the control holds, because the renderer's foreign filter
    /// compares an occurrence's table against this registry's.
    pub(crate) fn prepare(
        plan: std::sync::Arc<CompiledPlan>,
        anchor: StreamAnchor,
        epoch: StreamEpoch,
        minter: crate::identity::TableId,
    ) -> Result<Self, CompileError> {
        let quantum = QUANTUM_FRAMES as usize;
        let channels = plan.channel_layout().channels();

        let max_block =
            plan.maximum_block_size()
                .as_usize()
                .ok_or(CompileError::ReportUnitMismatch {
                    field: crate::report::ResourceField::MaximumBlockSize,
                })?;
        let carry_frames_capacity = max_block.saturating_add(quantum);

        // One state per prepared node, built from the prepared record so the two tables
        // are parallel by construction rather than by two counters agreeing.
        // One state per **scheduled step**, from the prepared record the step names: since
        // `P06-S001` a voice-scope node has one prepared record and one state per instance,
        // so the two tables are no longer parallel — the step is what pairs them.
        let steps = plan
            .ops()
            .iter()
            .filter(|op| matches!(op, PlanOp::Node(_)))
            .count();
        let mut node_states: Vec<NodeState> = vec![NodeState::Stateless; steps];
        for op in plan.ops() {
            if let PlanOp::Node(step) = op
                && let (Some(prepared), Some(state)) = (
                    plan.prepared_nodes().get(step.prepared().index()),
                    node_states.get_mut(step.node().index()),
                )
            {
                *state = NodeState::initial(prepared);
            }
        }
        // One slot per target row, from the row itself, for the same reason.
        let parameter_slots: Vec<slot::SlotState> = plan
            .parameter_targets()
            .iter()
            .map(|target| {
                slot::SlotState::prepared(target.law, target.unit, target.smoothing, target.base)
            })
            .collect();
        // One quantum of values per quantum-rate slot, laid out node by node in target
        // order — which is declaration order, because the compiler pushes a node's controls
        // contiguously — so a node's ramps are one slice.
        let mut ramp_offsets = Vec::with_capacity(parameter_slots.len());
        let mut ramp_starts = vec![0_usize; node_states.len().saturating_add(1)];
        let mut running = 0_usize;
        for target in plan.parameter_targets() {
            match target.rate {
                crate::plan::ControlRate::Quantum => {
                    ramp_offsets.push(running);
                    running = running.saturating_add(quantum);
                }
                crate::plan::ControlRate::Sample => ramp_offsets.push(usize::MAX),
            }
            if let Some(next) = ramp_starts.get_mut(target.node.index().saturating_add(1)) {
                *next = running;
            }
        }
        // A node with no quantum-rate control inherits the previous node's end, so every
        // node's slice is well-formed and empty where it has nothing.
        for index in 1..ramp_starts.len() {
            let previous = ramp_starts.get(index - 1).copied().unwrap_or(0);
            if let Some(start) = ramp_starts.get_mut(index) {
                *start = (*start).max(previous);
            }
        }
        let ramp_buffers = vec![0.0_f32; running];

        // A call renders at most one quantum more than its frame count spans, so this
        // bounds both the per-quantum tally and the event scratch.
        let quanta_per_call = max_block.div_ceil(quantum).saturating_add(1);
        let events_per_quantum = plan.max_events_per_quantum().as_usize().unwrap_or(0);
        // What the widest of this plan's events writes: a note-on's expansion, gate included,
        // or a sample-positioned parameter write fanned out over the instances of the widest
        // group such a write can address (`P06-S001`).
        let writes_per_note = plan
            .max_writes_per_note()
            .fanned_out(plan.sample_positioned_fan_out())
            .widest(plan.steal_expansion())
            .get() as usize;
        // One index entry per scheduled record, from the table the renderer already keeps
        // one state per — so the two cannot be counted differently.
        let records = node_states.len();

        // The identity partition, which bounds how many notes one activation can end and so
        // how many gate-downs a boundary can owe. Summed from the same declaration both
        // identity halves are sized by.
        let identity_indices: usize = plan
            .note_producer_ranges()
            .iter()
            .map(|range| range.get() as usize)
            .sum();

        let mut output_carry = vec![0.0; carry_frames_capacity.saturating_mul(channels)];
        output_carry.fill(0.0);

        // ADR-0047 clause 3's identity partition, from what admission copied in. A plan with
        // no note-on producers gets an empty partition, which is right: nothing can start a
        // note, so nothing can name an occurrence. The minting half is the control's; this
        // is the registry that answers to it.
        let live_notes =
            crate::identity::LiveNotes::for_ranges(minter, plan.note_producer_ranges())?;

        let has_output = plan
            .ops()
            .iter()
            .any(|op| matches!(op, PlanOp::Output { .. }));

        Ok(Self {
            // ADR-0041 clause 13: the arena is one allocation of variable-width regions,
            // sized by the extent the assignment reached rather than by a count of
            // uniform slots.
            buffers: vec![0.0; plan.arena_samples()],
            output_carry,
            // Primed: `Q` frames of silence are already available to serve.
            carry_frames: quantum,
            live_notes,
            input_carry: vec![0.0; carry_frames_capacity.saturating_mul(channels)],
            node_states,
            parameter_slots,
            ramp_buffers,
            ramp_offsets,
            ramp_starts,
            has_output,
            event_scratch: vec![DueEvent::FILL; events_per_quantum.saturating_mul(quanta_per_call)],
            scratch_len: 0,
            quantum_counts: vec![0; quanta_per_call],
            // ADR-0001 clause 14's storage. One quantum at a time, because an edge is
            // applied inside the quantum it falls in and nothing outlives that. Sized on
            // what a note-on **expands** to since `SOUND-INV-021`, from the plan's own
            // figure, so preparation takes exactly what admission charged.
            timed_controls: vec![
                TimedControl::FILL;
                events_per_quantum
                    .saturating_mul(writes_per_note)
                    .saturating_add(identity_indices)
            ],
            adoption_gates: vec![TimedControl::FILL; identity_indices],
            adoption_gate_slots: vec![0; identity_indices],
            adoption_gate_len: 0,
            control_starts: vec![0; records.saturating_add(1)],
            control_fill: vec![0; records],
            plan,
            epoch,
            anchor,
            clock: SampleTime::ZERO,
            channels,
            diagnostics: DiagnosticsReport::default(),
        })
    }

    /// This stream's epoch.
    pub const fn epoch(&self) -> StreamEpoch {
        self.epoch
    }

    /// Write one parameter slot's modulation sum, in its law's units.
    ///
    /// **`P05-S007a`'s seam, off the audio thread.** `SOUND-INV-023`'s modulation layer has
    /// no producer until Phase 7's modulation edges, and this is what stands in for one so
    /// that the layer's composition is tested rather than claimed: a modulation in force
    /// survives an override write and an activation's catch-up, and reaches the kernel
    /// composed. A quantum-rate target retargets its segment now, as an `apply` of a write
    /// would, and the kernel reads the result per frame from the slot's buffer; a
    /// sample-positioned target takes it at its next positioned write, which is the only
    /// path a value has to such a kernel. A slot index the plan has no row for writes
    /// nothing.
    ///
    /// Compiled for tests only, which is what a seam with no production caller is: the
    /// attribute comes off with Phase 7's first modulator.
    #[cfg(test)]
    pub(crate) fn modulate(
        &mut self,
        slot: crate::plan::ParameterSlot,
        sum: crate::node::ModulationSum,
    ) {
        if let Some(state) = self.parameter_slots.get_mut(slot.index()) {
            let _ = state.modulate(sum);
        }
    }

    /// Override one slot's declared smoothing policy with a segment length in frames.
    ///
    /// Test-only, beside [`Self::modulate`] and for the same reason: no declaration smooths
    /// yet, and `SOUND-INV-024`'s facts are tested through this rather than claimed.
    #[cfg(test)]
    pub(crate) fn smooth_over(&mut self, slot: crate::plan::ParameterSlot, frames: u32) {
        if let Some(state) = self.parameter_slots.get_mut(slot.index()) {
            state.smooth_over(frames);
        }
    }

    /// The bytes preparation holds for the parameter slots and their control buffers, for
    /// the test that compares the charge with what is held.
    #[cfg(test)]
    pub(crate) fn slot_bytes_held(&self) -> usize {
        self.parameter_slots
            .len()
            .saturating_mul(size_of::<slot::SlotState>())
            .saturating_add(self.ramp_buffers.len().saturating_mul(size_of::<f32>()))
            .saturating_add(self.ramp_offsets.len().saturating_mul(size_of::<usize>()))
    }

    /// One tap's samples as the last rendered quantum left them: the region the tapped
    /// node wrote, which ADR-0005 clause 6 kept live to the end of the quantum.
    ///
    /// Test-only until `HOST-INV-023`'s subscription reads it; that consumer is Phase 9's
    /// live host or Phase 10E's facade, and this is the read it will make.
    #[cfg(test)]
    pub(crate) fn tap_block(&self, slot: crate::plan::TapSlot) -> &[f32] {
        if slot.plan() != self.plan.id() {
            return &[];
        }
        self.plan
            .taps()
            .get(slot.index())
            .and_then(|tap| self.plan.region(tap.region))
            .and_then(|region| self.buffers.get(region.offset()..region.end()))
            .unwrap_or(&[])
    }

    /// Every state record, by node slot — for a test reading what an instance's kernel kept.
    #[cfg(test)]
    pub(crate) fn node_states(&self) -> &[NodeState] {
        &self.node_states
    }

    /// The bytes preparation holds for the per-node run table of those buffers.
    #[cfg(test)]
    pub(crate) fn ramp_table_bytes_held(&self) -> usize {
        self.ramp_starts.len().saturating_mul(size_of::<usize>())
    }

    /// The render clock: input frames consumed so far.
    pub const fn clock(&self) -> SampleTime {
        self.clock
    }

    /// How many rendered frames are waiting to be served.
    ///
    /// Crate-private, and its one caller is ADR-0050 clause 4's split: the frame a crossing
    /// host block is cut at is `carry + kQ`, which is the largest request that renders
    /// exactly `k` quanta. Between calls this is at most `Q` — a call renders the fewest
    /// quanta that cover its request, so the remainder is under one quantum, and the stream
    /// starts with exactly `Q`. The `maximum_block_size + Q` the buffer is sized to is the
    /// peak reached *inside* a call, which is why adoption belongs between them.
    pub(crate) const fn carry_frames(&self) -> usize {
        self.carry_frames
    }

    /// Count an activation the scheduler refused at its offer.
    ///
    /// The refusal itself belongs to the scheduler, which owns the exchange; the counters
    /// belong to the renderer, which owns the report. Exposing the increment rather than the
    /// report keeps that split intact.
    pub(crate) fn count_refused_activation(&mut self) {
        self.diagnostics.count_refused_activation();
    }

    /// How many scheduled records preparation sized its index tables for.
    ///
    /// The other input to [`timed_control_scratch_bytes`], read back so a budget check has
    /// both of them from the object rather than one from the object and one from a guess.
    pub fn prepared_record_count(&self) -> RecordCount {
        RecordCount::measured(u32::try_from(self.control_fill.len()).unwrap_or(u32::MAX))
    }

    /// What the sample-positioned control scratch actually holds, in bytes.
    ///
    /// The symmetric reader of [`timed_control_scratch_bytes`], which is what admission
    /// charges a plan for it. The two are written apart on purpose: a test that compared the
    /// budget with a restatement of its own formula would pass however wrong the formula was,
    /// and both terms this covers were once missing from it — the identity extension, and
    /// then the two buffers an adopted activation's gate-downs wait in.
    #[must_use]
    pub fn control_scratch_bytes(&self) -> usize {
        self.timed_controls
            .len()
            .saturating_add(self.adoption_gates.len())
            .saturating_mul(size_of::<TimedControl>())
            .saturating_add(self.adoption_gate_slots.len() * size_of::<usize>())
            .saturating_add(
                self.control_starts
                    .len()
                    .saturating_add(self.control_fill.len())
                    * size_of::<u32>(),
            )
    }

    /// The counters this stream has accumulated.
    /// The report, for the one producer that writes counts this renderer cannot observe.
    ///
    /// The live boundary drops **before acceptance**, on the producing half, so those counts
    /// exist only on the ingress store. `HOST-INV-009` requires them to reach this report,
    /// and there is no other path from one to the other.
    pub(crate) const fn diagnostics_mut(&mut self) -> &mut DiagnosticsReport {
        &mut self.diagnostics
    }

    pub const fn diagnostics(&self) -> &DiagnosticsReport {
        &self.diagnostics
    }

    /// The latency this stream adds, which is a constant `Q` frames.
    pub fn added_latency(&self) -> FrameCount {
        self.plan.added_latency()
    }

    /// The plan being rendered.
    pub fn plan(&self) -> &CompiledPlan {
        &self.plan
    }
}

/// The hot path: everything that runs on the audio thread.
///
/// It is a separate file so that the real-time rules have an unambiguous region to be
/// checked against. `tests/render_loop_purity.rs` reads it and fails on a lock, an I/O
/// call, a logging macro, a panicking accessor, or an allocating construct — none of
/// which the counting-allocator test can see for the three that do not allocate.
#[path = "render/hot.rs"]
mod hot;

/// The parameter slot, inside the scanned region for the reason its header gives.
#[path = "render/slot.rs"]
pub(crate) mod slot;

#[cfg(test)]
#[path = "tests/render_scratch.rs"]
mod scratch_tests;

#[cfg(test)]
#[path = "tests/parameter_slot.rs"]
mod parameter_slot_tests;

#[cfg(test)]
#[path = "tests/taps.rs"]
mod tap_tests;
