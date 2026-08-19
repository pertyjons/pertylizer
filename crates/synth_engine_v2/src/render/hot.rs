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

use super::{
    AudioBlockMut, DueEvent, EventPayload, PendingCounts, PreparedRenderer, Renderer, TimedEvents,
};
use crate::diagnostics::RenderError;
use crate::node::kernels;
use crate::plan::PlanOp;
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
        for count in &mut self.quantum_counts {
            *count = 0;
        }
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
            match event.payload() {
                EventPayload::SetParameter { slot, .. } if slot.plan() != self.plan.id() => {
                    pending.foreign_slot = pending.foreign_slot.saturating_add(1);
                    continue;
                }
                EventPayload::SetParameter { .. } => {}
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

    fn apply(&mut self, payload: EventPayload) {
        match payload {
            EventPayload::SetParameter { slot, value } => {
                // A slot indexes **one** plan's target table, and one from another plan
                // was already refused and counted during resolution — before it could
                // reach a capacity check. Nothing that arrives here belongs to another
                // plan.
                let Some(target) = self.plan.parameter_targets().get(slot.index()).copied() else {
                    return;
                };
                // A `ParameterValue` is finite by construction, so this assignment cannot
                // poison a phase accumulator — which is why the type exists rather than a
                // check here, where no diagnostic could be produced. What the control
                // *means* is the node state's, which is the last place it is known.
                // Both records, because a control change is a node operation like a
                // render is: what a value *means* can depend on the prepared data, and an
                // envelope's release derives its motion from the duration it was
                // prepared with.
                let Some(prepared) = self.plan.prepared_nodes().get(target.node.index()) else {
                    return;
                };
                if let Some(state) = self.node_states.get_mut(target.node.index()) {
                    state.set_control(prepared, target.control, value);
                }
            }
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
                    let Some(mut io) =
                        kernels::bind(&mut self.buffers, self.plan.regions(), step, plan_start)
                    else {
                        continue;
                    };
                    (step.kernel())(prepared, state, &mut io);
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
        for _ in 0..quanta {
            let boundary = self.clock;
            cursor = self.apply_control_events(boundary, cursor);
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
