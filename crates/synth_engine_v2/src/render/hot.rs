//! The render loop: the only code in this crate that runs on the audio thread.
//!
//! Nothing here may allocate, lock, perform I/O, log, or panic. Every buffer it touches
//! was allocated by `PreparedRenderer::prepare`, the event scratch is preallocated to
//! the capacity admission approved, and the one sort it performs is in place over that
//! scratch. `tests/render_loop_purity.rs` reads this file and fails on the constructs
//! that would break any of those rules; the counting-allocator test covers the
//! allocation half behaviourally.
//!
//! Keeping it in its own file is what makes the check meaningful: `prepare` legitimately
//! allocates, and a scan over one file holding both could only ever be a scan with
//! exceptions.

use super::NoteEdge;
use super::{
    AudioBlockMut, DueEvent, EventPayload, PendingCounts, PreparedRenderer, Renderer,
    ResolvedTarget, TimedEvents,
};
use crate::diagnostics::RenderError;
use crate::node::kernels;
use crate::node::kernels::TimedControl;
use crate::plan::{ControlRate, PlanOp};
use crate::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, TimeSource};

impl PreparedRenderer {
    /// Silence the output and end the epoch.
    ///
    /// ADR-0021 part 3's shape, shared by the two terminal faults: output silence,
    /// both carries invalidated, `needs_reprepare` published, nothing allocated.
    fn fault(&mut self, output: &mut AudioBlockMut<'_>) {
        output.samples.fill(0.0);
        self.carry_frames = 0;
        self.output_carry.fill(0.0);
        self.input_carry.fill(0.0);
        self.diagnostics.set_needs_reprepare();
    }

    /// Take ADR-0046 clause 7's terminal response from outside the render loop.
    ///
    /// The publication arbiter detects a share overrun or an over-full batch *before* the
    /// renderer is called, so the response has to be reachable from there. It is the same
    /// [`Self::fault`] the loop's own two terminal cases take, exposed to the crate rather
    /// than duplicated: a second implementation of "end the stream" is exactly how the two
    /// would drift.
    ///
    /// The counter is not decoration. Clause 7 asks the terminal response to increment an
    /// **attributable** counter, because a stream that ended for a contract violation and
    /// one that ended for any other reason are indistinguishable without it — and the
    /// violation is the only one of the two a producer can be told to fix.
    ///
    /// After this the epoch is over. `render`'s first check returns
    /// [`RenderError::NeedsReprepare`] and silences every later callback.
    pub(crate) fn terminal_fault(&mut self, output: &mut AudioBlockMut<'_>) {
        self.diagnostics.count_publication_fault();
        self.fault(output);
    }

    /// How many quanta a call for `frames` frames will render.
    ///
    /// Public because a caller has to know it: Phase 1's event span covers the quanta
    /// the call renders, and the carry means that is not simply `frames / Q`. The
    /// scheduler that would work this out for the caller is Phase 3's.
    pub const fn quanta_needed_for(&self, frames: usize) -> usize {
        if self.carry_frames >= frames {
            0
        } else {
            (frames - self.carry_frames).div_ceil(QUANTUM_FRAMES as usize)
        }
    }

    /// Resolve the span into [`Self::event_scratch`], or reject the call.
    ///
    /// Nothing is committed until this returns: the counters it would move are
    /// accumulated locally, so a rejected call leaves the renderer exactly as it was.
    fn resolve_events(
        &mut self,
        events: TimedEvents<'_>,
        quanta: usize,
    ) -> Result<PendingCounts, RenderError> {
        // Bounded work first. Every later branch may `continue`, so without this the
        // audio thread's work would be a function of what the producer sent rather than
        // of a declared capacity.
        let presented = events.as_slice().len();
        let admissible = self.event_scratch.len();
        if presented > admissible {
            return Err(RenderError::EventSpanTooLarge {
                presented,
                admissible,
            });
        }

        self.scratch_len = 0;
        self.quantum_counts.fill(0);
        let mut pending = PendingCounts::default();

        let first_quantum = self.clock.quantum_index();
        // With no quantum to render, there is no quantum an event could belong to.
        let last_quantum = first_quantum.saturating_add(quanta.max(1) as u64) - 1;
        let horizon_end = self
            .clock
            .checked_add(self.plan.forward_event_horizon())
            .ok();
        let capacity = self.plan.max_events_per_quantum().get();

        for (index, event) in events.as_slice().iter().enumerate() {
            let envelope = event.envelope();

            // ADR-0032 clause 20: an event from another epoch is discarded and
            // counted, never interpreted in this one.
            if envelope.epoch() != self.epoch {
                pending.stale_epoch = pending.stale_epoch.saturating_add(1);
                continue;
            }

            // A slot from another plan is the same class of thing as a stale epoch, and
            // is filtered here for the same reason: after a plan swap, in-flight events
            // are *ordinary*. Recognising them later, in `apply`, would let five of them
            // in one quantum fail the call against a capacity of four — turning the
            // documented post-swap case into a render failure.
            let foreign = match event.payload() {
                EventPayload::SetParameter { slot, .. } => slot.plan() != self.plan.id(),
                // A note edge's provenance is its **identity's table**, not a node address:
                // `SOUND-INV-017` removes the node from the release, so there is no slot to
                // compare. An identity from another table is the same class of stale as a
                // foreign slot and is filtered here for the same reason.
                EventPayload::Note { identity, .. } => identity.table() != self.live_notes.id(),
            };
            if foreign {
                pending.foreign_slot = pending.foreign_slot.saturating_add(1);
                continue;
            }

            // ADR-0032 clause 21: the forward horizon binds ingress provenance only.
            // A compiled list spans the whole piece, and measuring it against a
            // horizon meant for live input would reject most of a song.
            if envelope.source().is_ingress()
                && horizon_end.is_some_and(|end| envelope.time() > end)
            {
                pending.out_of_horizon = pending.out_of_horizon.saturating_add(1);
                continue;
            }

            if envelope.source() == TimeSource::Arrival {
                pending.arrival_stamped = pending.arrival_stamped.saturating_add(1);
            }

            // ADR-0001 clause 16: an event whose timestamp falls in an
            // already-rendered quantum is late. It is clamped forward to the first
            // not-yet-rendered quantum boundary — which is where the clock stands —
            // and counted. The stamp itself is untouched.
            let position = if envelope.time() < self.clock {
                pending.late = pending.late.saturating_add(1);
                self.clock
            } else {
                envelope.time()
            };

            let quantum = position.quantum_index();
            if quantum < first_quantum || quantum > last_quantum || quanta == 0 {
                return Err(RenderError::EventOutsideCallSpan {
                    position: position.as_u64(),
                    event_quantum: quantum,
                    first_quantum,
                    last_quantum,
                });
            }

            let slot = (quantum - first_quantum) as usize;
            let tally =
                self.quantum_counts
                    .get_mut(slot)
                    .ok_or(RenderError::EventOutsideCallSpan {
                        position: position.as_u64(),
                        event_quantum: quantum,
                        first_quantum,
                        last_quantum,
                    })?;
            *tally = tally.saturating_add(1);
            if *tally > capacity {
                return Err(RenderError::QuantumEventOverflow {
                    quantum_index: quantum,
                    requested: *tally,
                    available: capacity,
                });
            }

            // The scratch is sized to `max_events_per_quantum` times the quanta a call can
            // render, so a full scratch means the per-quantum tally above should already
            // have refused. Refusing here as well is what keeps the write below a write
            // rather than a growth.
            let Some(slot) = self.event_scratch.get_mut(self.scratch_len) else {
                return Err(RenderError::QuantumEventOverflow {
                    quantum_index: quantum,
                    requested: capacity.saturating_add(1),
                    available: capacity,
                });
            };
            *slot = DueEvent {
                position,
                // Resolved below, once the whole span is in render order.
                target: None,
                // Four billion events in one span is unreachable, and a saturated arrival
                // index only affects tie order among events at one position.
                arrival: u32::try_from(index).unwrap_or(u32::MAX),
                payload: event.payload(),
            };
            self.scratch_len += 1;
        }

        // Ascending render position, ties by arrival order. An in-place sort over the live
        // prefix of preallocated storage: the candidate set is bounded by a declared
        // capacity, which is what the real-time rule asks for — it constrains the work,
        // not the mechanism.
        if let Some(live) = self.event_scratch.get_mut(..self.scratch_len) {
            live.sort_unstable_by_key(|event| (event.position, event.arrival));
        }

        self.resolve_nodes(&mut pending);

        Ok(pending)
    }

    /// Apply every event at or before `boundary`, starting at `cursor`.
    ///
    /// Returns the new cursor. The scratch is sorted, so the due events are a prefix
    /// and no scan repeats work.
    fn apply_control_events(&mut self, boundary: SampleTime, cursor: usize) -> usize {
        let mut next = cursor;
        while next < self.scratch_len {
            let Some(event) = self.event_scratch.get(next) else {
                break;
            };
            if event.position > boundary {
                break;
            }
            let payload = event.payload;
            self.apply(payload);
            next += 1;
        }
        next
    }

    /// Apply one event's **quantum-rate** effect, if it has one.
    ///
    /// A sample-positioned target is skipped here and collected by
    /// [`Self::collect_timed_controls`] instead, which is what keeps ADR-0001 clause 14's
    /// split a property of the target rather than of the payload: a gate addressed as a
    /// parameter lands on its sample exactly as a note does.
    fn apply(&mut self, payload: EventPayload) {
        match payload {
            // A note has no quantum-rate effect at all. Every edge it carries is
            // sample-positioned, so the whole payload is the other pass's.
            EventPayload::Note { .. } => {}
            EventPayload::SetParameter { slot, value } => {
                // A slot indexes **one** plan's target table, and one from another plan
                // was already refused and counted during resolution — before it could
                // reach a capacity check. Nothing that arrives here belongs to another
                // plan.
                let Some(target) = self.plan.parameter_targets().get(slot.index()).copied() else {
                    return;
                };
                if matches!(target.rate, ControlRate::Sample) {
                    return;
                }
                // A `ParameterValue` is finite by construction, so this assignment cannot
                // poison a phase accumulator — which is why the type exists rather than a
                // check here, where no diagnostic could be produced. What the control
                // *means* is the node state's, which is the last place it is known.
                if let Some(state) = self.node_states.get_mut(target.node.index()) {
                    state.set_control(target.control, value);
                }
            }
        }
    }

    /// Group this quantum's sample-positioned control changes by the node they move.
    ///
    /// ADR-0001 clause 14 in one pass. `cursor` walks [`Self::event_scratch`] — which
    /// [`Self::resolve_events`] left sorted by `(position, arrival)` — and advances
    /// monotonically across the quanta of one call, so the total work is linear in the
    /// span rather than quadratic in it. Every admitted position is at or after the call's
    /// first quantum boundary, because a late one was already clamped forward to the
    /// clock, and the clock only ever stands on a boundary.
    ///
    /// A counting sort rather than a second sort of the scratch: the events arrive in
    /// position order and are needed in node order, and counting is what turns one into
    /// the other by index alone. It also removes any dependence on the order node slots
    /// appear in the schedule, which is an invariant of lowering rather than of the plan.
    fn collect_timed_controls(&mut self, cursor: &mut usize) {
        self.control_starts.fill(0);
        self.control_fill.fill(0);

        let Ok(end) = self.clock.checked_add(FrameCount::QUANTUM) else {
            // The clock is one quantum from exhausting, so this call is about to fail on
            // the advance. Nothing is collected, and the refusal happens where it can be
            // reported.
            return;
        };

        // Pass one: how many changes each node is due. The window is the same in both
        // passes, so the second cannot see an event the first did not count.
        let window = *cursor;
        let mut last = window;
        while let Some(event) = self.event_scratch.get(last) {
            if last >= self.scratch_len || event.position >= end {
                break;
            }
            last += 1;
            if let Some(target) = event.target {
                let node = target.node;
                // Counted at `node + 1`, so the prefix sum below turns the counts into
                // starts in place: entry `n` becomes where node `n`'s run begins.
                if let Some(count) = self.control_starts.get_mut(node + 1) {
                    *count = count.saturating_add(1);
                }
            }
        }
        *cursor = last;

        let mut running = 0_u32;
        for start in &mut self.control_starts {
            running = running.saturating_add(*start);
            *start = running;
        }

        // Pass two: each change written where its node's run has room. Within a node the
        // writes happen in the order the scratch holds them, which is ascending position,
        // so each run comes out ascending by offset without a second sort.
        let mut index = window;
        while index < last {
            let Some(event) = self.event_scratch.get(index).copied() else {
                break;
            };
            index += 1;
            let (Some(target), Some(value)) = (event.target, self.timed_value(event.payload))
            else {
                continue;
            };
            let (node, control) = (target.node, target.control);
            let (Some(base), Some(filled)) = (
                self.control_starts.get(node).copied(),
                self.control_fill.get(node).copied(),
            ) else {
                continue;
            };
            let Some(slot) = self
                .timed_controls
                .get_mut(base.saturating_add(filled) as usize)
            else {
                continue;
            };
            *slot = TimedControl {
                offset: event.position.quantum_offset(),
                control,
                value,
            };
            if let Some(fill) = self.control_fill.get_mut(node) {
                *fill = fill.saturating_add(1);
            }
        }
    }

    /// Resolve every admitted event's target **once**, in the order the call will apply them.
    ///
    /// This is the only place the live-note registry is written, and it runs after the sort
    /// so the walk is in render order. That matters for one case and it is not exotic: a
    /// producer that plays a note, releases it and plays another may have both occurrences
    /// on the same index, and only a walk in application order sees the first end before the
    /// second begins. Resolving inside the per-quantum passes instead would read a registry
    /// those same passes were mutating, and the two passes would stop agreeing — which is
    /// what makes the counts the first produces describe the writes the second performs.
    ///
    /// An orphan release is counted here rather than silently skipped: `SOUND-INV-017`
    /// requires an identity naming no live note to be *refused and counted*, and this is
    /// where the refusal happens.
    fn resolve_nodes(&mut self, pending: &mut PendingCounts) {
        for index in 0..self.scratch_len {
            let Some(event) = self.event_scratch.get(index).copied() else {
                break;
            };
            let target = match event.payload {
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On { slot },
                } => {
                    // The on edge names what is played; the registry records it against the
                    // occurrence so the release does not have to carry it.
                    self.live_notes.admit(identity, slot);
                    self.note_target(slot)
                }
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                } => match self.live_notes.release(identity) {
                    Some(slot) => self.note_target(slot),
                    None => {
                        pending.orphan_note = pending.orphan_note.saturating_add(1);
                        pending.last_orphan_note = Some(identity);
                        None
                    }
                },
                EventPayload::SetParameter { slot, .. } => self
                    .plan
                    .parameter_targets()
                    .get(slot.index())
                    .filter(|row| matches!(row.rate, ControlRate::Sample))
                    .map(|row| ResolvedTarget {
                        node: row.node.index(),
                        control: row.control,
                    }),
            };
            if let Some(slot) = self.event_scratch.get_mut(index) {
                slot.target = target;
            }
        }
    }

    /// Where a note slot's edge lands.
    fn note_target(&self, slot: crate::plan::NoteSlot) -> Option<ResolvedTarget> {
        self.plan
            .note_targets()
            .get(slot.index())
            .map(|row| ResolvedTarget {
                node: row.node.index(),
                control: row.control,
            })
    }

    /// What it moves it to.
    fn timed_value(&self, payload: EventPayload) -> Option<crate::quantities::ParameterValue> {
        match payload {
            EventPayload::Note { edge, .. } => Some(edge.value()),
            EventPayload::SetParameter { value, .. } => Some(value),
        }
    }

    /// Render exactly one quantum and append it to the output carry.
    fn render_quantum(&mut self) -> Result<(), RenderError> {
        let quantum = QUANTUM_FRAMES as usize;
        let quantum_start = self.clock;

        // The plan position of this quantum's first sample. Anchoring is the only
        // place engine time and plan time meet.
        let plan_start = self.plan_position_of(quantum_start);

        for index in 0..self.plan.ops().len() {
            // Borrowed, not copied. A step is the widest thing the schedule holds and a
            // prepared record is the widest variant of its enum; copying either per node
            // per quantum would be work proportional to the representation rather than to
            // the audio. The three fields below are disjoint, which is what lets one loop
            // hold a shared borrow of the plan while it writes state and buffers.
            let Some(op) = self.plan.ops().get(index) else {
                break;
            };
            match op {
                // The whole schedule walk, for every node kind there will ever be. What
                // runs is a function pointer the compiler resolved; this loop does not
                // know what kind of node it just ran, which is ADR-0004 clause 2 — adding
                // a node adds a kernel and a registry entry, and adds nothing here.
                PlanOp::Node(step) => {
                    let Some(prepared) = self.plan.prepared_nodes().get(step.node().index()) else {
                        continue;
                    };
                    let Some(state) = self.node_states.get_mut(step.node().index()) else {
                        continue;
                    };
                    // Resolved before the arena is borrowed mutably: these are two
                    // disjoint fields of one renderer, and taking the slice first is what
                    // lets the borrow checker see that.
                    let (Some(start), Some(end)) = (
                        self.control_starts.get(step.node().index()).copied(),
                        self.control_starts.get(step.node().index() + 1).copied(),
                    ) else {
                        continue;
                    };
                    let gates = self
                        .timed_controls
                        .get(start as usize..end as usize)
                        .unwrap_or(&[]);
                    let Some(mut io) = kernels::bind(
                        &mut self.buffers,
                        self.plan.regions(),
                        step,
                        plan_start,
                        gates,
                    ) else {
                        continue;
                    };
                    step.kernel().run(prepared, state, &mut io);
                }
                PlanOp::Output { source } => {
                    // ADR-0041 clause 11: the plan's output signal already has the
                    // stream's layout and its arrangement, so the boundary is **one
                    // contiguous copy** rather than a channel-strided write per channel.
                    // The transpose the planar renderer performed here is what the
                    // conversion removes, and EVD-0010 measured that loop's index
                    // arithmetic at 45.8 ns per quantum over a frame-strided form.
                    let Some(region) = self.plan.region(*source) else {
                        continue;
                    };
                    let start = self.carry_frames * self.channels;
                    let samples = quantum * self.channels;
                    let (Some(from), Some(into)) = (
                        self.buffers.get(region.offset()..region.offset() + samples),
                        self.output_carry.get_mut(start..start + samples),
                    ) else {
                        continue;
                    };
                    into.copy_from_slice(from);
                }
            }
        }

        // A plan with no output operation renders silence rather than leaving the
        // carry holding whatever the previous quantum left there.
        if !self.has_output {
            let start = self.carry_frames * self.channels;
            let end = start + quantum * self.channels;
            if let Some(region) = self.output_carry.get_mut(start..end) {
                region.fill(0.0);
            }
        }

        self.carry_frames += quantum;
        self.clock = self
            .clock
            .checked_advance_quantum()
            .map_err(RenderError::ClockExhausted)?;
        Ok(())
    }

    /// The plan position of an engine time, or `None` where the anchor does not reach
    /// it.
    fn plan_position_of(&self, time: SampleTime) -> Option<PlanPosition> {
        let Ok(delta) = time.difference(self.anchor.time()) else {
            return None;
        };
        if delta.is_negative() {
            return None;
        }
        self.anchor
            .position()
            .checked_add(FrameCount::new(delta.as_i64() as u64))
            .ok()
    }
}

impl Renderer for PreparedRenderer {
    fn render(
        &mut self,
        mut output: AudioBlockMut<'_>,
        events: TimedEvents<'_>,
    ) -> Result<(), RenderError> {
        if self.diagnostics.needs_reprepare() {
            output.samples.fill(0.0);
            return Err(RenderError::NeedsReprepare);
        }

        if output.layout() != self.plan.channel_layout() {
            let needed = output.frames().saturating_mul(self.channels);
            return Err(RenderError::OutputBufferShape {
                samples: output.samples.len(),
                frames: output.frames(),
                layout: self.plan.channel_layout(),
                needed,
            });
        }

        let frames = output.frames();
        let maximum = self.plan.maximum_block_size();
        if frames as u64 > maximum.as_u64() {
            // ADR-0021 part 3: a terminal stream-contract fault. The engine makes no
            // claim that the old input epoch continues.
            self.diagnostics.count_oversized_callback();
            self.fault(&mut output);
            return Err(RenderError::OversizedCallback {
                frames: FrameCount::new(frames as u64),
                maximum,
            });
        }

        let quanta = self.quanta_needed_for(frames);
        let pending = self.resolve_events(events, quanta)?;

        // Committed only now that the call cannot be rejected.
        for _ in 0..pending.stale_epoch {
            self.diagnostics.count_stale_epoch_event();
        }
        for _ in 0..pending.foreign_slot {
            self.diagnostics.count_foreign_slot_event();
        }
        // The identity is the most recent orphan's; ADR-0047 clause 4 wants one named, and
        // the count is what says how many there were.
        if let Some(identity) = pending.last_orphan_note {
            for _ in 0..pending.orphan_note {
                self.diagnostics.count_orphan_note_event(identity);
            }
        }
        for _ in 0..pending.out_of_horizon {
            self.diagnostics.count_out_of_horizon_event();
        }
        for _ in 0..pending.arrival_stamped {
            self.diagnostics.count_arrival_stamped_event();
        }
        for _ in 0..pending.late {
            self.diagnostics.count_late_event();
        }

        let mut cursor = 0;
        let mut timed = 0;
        for _ in 0..quanta {
            let boundary = self.clock;
            cursor = self.apply_control_events(boundary, cursor);
            // After the boundary controls and before the quantum: the edges are a property
            // of the samples about to be written, and the kernel that writes them is what
            // places each one.
            self.collect_timed_controls(&mut timed);
            if let Err(error) = self.render_quantum() {
                if matches!(error, RenderError::ClockExhausted(_)) {
                    self.diagnostics.count_clock_exhaustion();
                    self.fault(&mut output);
                }
                return Err(error);
            }
        }

        // Whatever is left sits inside the last quantum this call rendered, so its
        // control-rate response belongs to the *next* quantum — which the next call
        // renders. Applying it now is applying it at that boundary, because nothing
        // renders in between, and the scratch is cleared on the next call: an earlier
        // revision left these events unapplied, which lost every control change that fell
        // inside a call's final quantum and made automation a function of how the host
        // partitioned its callbacks. That is precisely what ADR-0001 exists to prevent.
        self.apply_control_events(SampleTime::new(u64::MAX), cursor);

        let samples = frames * self.channels;
        if let Some(source) = self.output_carry.get(..samples) {
            output.samples.copy_from_slice(source);
        }
        let live = self.carry_frames * self.channels;
        self.output_carry.copy_within(samples..live, 0);
        self.carry_frames -= frames;
        Ok(())
    }
}
