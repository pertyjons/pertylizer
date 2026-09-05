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

    /// The same terminal response for an activation displacement that leaves engine time.
    ///
    /// **Its own counter**, because publication did not fail — the condition is detected
    /// before a window is even opened, and reporting it as a publication fault would tell a
    /// reader to look for a share overrun that never happened. An independent review found
    /// the misattribution. The counter is attributable for the reason clause 7 gives, and
    /// the condition is terminal because the shift is fixed at adoption and the event is
    /// fixed in the list: nothing a caller can do clears it.
    pub(crate) fn terminal_displacement_fault(&mut self, output: &mut AudioBlockMut<'_>) {
        self.diagnostics.count_displacement_fault();
        self.fault(output);
    }

    /// Adopt an activation at `effective`, which must be a quantum boundary at or after the
    /// clock.
    ///
    /// ADR-0050 clause 3: **an infallible move**. Everything that can refuse happened at the
    /// offer, so there is no branch here that can fail — a refusal discovered at the boundary
    /// would have to roll back a partly applied state set or fault the stream, and the atomic
    /// set exists to avoid the first while the second would turn a caller mistake into a
    /// terminal fault.
    ///
    /// Two things move: the anchor, which every quantum's plan position is derived from, and
    /// the notes the replaced producers are sounding. Neither carry is touched (clause 2):
    /// the output carry holds audio for engine samples strictly before the clock, and
    /// `effective` is at or after it, so that audio was rendered under the mapping that was
    /// in force when it was rendered.
    ///
    /// The mass release is clause 5's, and its registry half is here. It ends what is
    /// **sounding**; the allocator's own set is different and the control handled it before
    /// this candidate was stamped. Scoping the clear to a producer range is safe because the
    /// incoming schedule's events have reached no render call yet — not because of any order
    /// inside a call, which is a claim an earlier revision of this contract made and which is
    /// false.
    pub(crate) fn adopt(
        &mut self,
        activation: &mut crate::transport::TransportActivation,
        effective: SampleTime,
        late: bool,
        cursor: usize,
        shift: crate::time::FrameCount,
    ) {
        // The scalars of clause 3's atomic set, which have no storage to be swapped into.
        // Taken before the anchor moves, because the anchor is one of them.
        activation.retired = Some(crate::transport::RetiredState {
            anchor: self.anchor,
            cursor,
            shift,
        });
        self.anchor = crate::time::StreamAnchor::new(effective, activation.position);
        // Clause 1's lateness, decided by the caller from `requested < clock` at the call
        // that adopts. **Not** `effective > requested`, which is true of every request that
        // does not land on a quantum boundary and would make the counter report snapping.
        if late {
            self.diagnostics.count_late_activation();
        }

        // `SOUND-INV-024`: an activation never ramps. The catch-up that follows this
        // adoption writes every prepared target, and each slot takes that write as a step
        // whatever its policy — seeded with current equal to target and nothing remaining.
        for slot in &mut self.parameter_slots {
            slot.seed();
        }

        self.adoption_gate_len = 0;
        for index in 0..activation.producers.len() {
            let Some(producer) = activation.producers.get(index).copied() else {
                break;
            };
            // **No range lookup here.** `note_producers` omits the empty producers off-thread,
            // which is what bounds this walk by the admitted identity span rather than by the
            // declaration count. A defensive re-check kept here indexed the range table by
            // the loop counter — and once the list is compacted that counter is a *position*,
            // not a `ProducerId`, so it could read a different producer's range and skip a
            // real release, leaving that producer's notes sounding across the activation. An
            // independent review found it. One authority on the scope is the fix; a second
            // one that has to be kept in step is how they come to disagree.
            let ended = self.live_notes.release_all(
                crate::identity::ReleaseScope::Producer(producer),
                &mut activation.ended,
            );
            let mut reported = 0_u32;
            while reported < ended.get() {
                let Some(Some(note)) = activation.ended.get(reported as usize).copied() else {
                    break;
                };
                reported = reported.saturating_add(1);
                let Some(target) = self.plan.note_targets().get(note.note.index()).copied() else {
                    continue;
                };
                let Some(slot) = self.adoption_gates.get_mut(self.adoption_gate_len) else {
                    break;
                };
                *slot = TimedControl {
                    // The boundary itself. A gate lowered here is lowered at the first
                    // sample the new mapping governs, which is what "at the boundary" means
                    // for a sample-positioned effect.
                    offset: crate::time::QuantumOffset::ZERO,
                    control: target.control,
                    value: crate::quantities::ParameterValue::ZERO,
                };
                let Some(row) = self.voice_row(target.parameter.index(), note.index) else {
                    continue;
                };
                self.adoption_gate_len = self.adoption_gate_len.saturating_add(1);
                if let Some(entry) = self.adoption_gate_slots.get_mut(self.adoption_gate_len - 1) {
                    *entry = row;
                }
            }
        }
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
                EventPayload::Note { identity, .. }
                | EventPayload::Fade { identity, .. }
                | EventPayload::Reset { identity } => identity.table() != self.live_notes.id(),
            };
            if foreign {
                pending.foreign_slot = pending.foreign_slot.saturating_add(1);
                continue;
            }

            // **The forward horizon is no longer evaluated here.** `HOST-INV-013` says it is
            // evaluated exactly once, at the boundary admitting into bounded source storage,
            // and the maintainer settled on 2026-09-01 that the boundary is
            // `PerformanceIngress::admit` rather than this loop. Four ways of having both
            // sites were tried in the ingress slice and each was refused by independent
            // review; a merge-gate review then found that keeping only *this* one cannot
            // work, because the drain releases an entry only once the publication reaches it,
            // so a far-future event never arrives here while it is still far-future.
            //
            // The accepted cost is stated rather than implied: a caller-assembled span whose
            // events carry ingress provenance now meets only the span check. It is still
            // bounded — an event outside the callback's window has no row to land in — but it
            // is not measured against the horizon.
            //
            // ADR-0032 clause 21 is unaffected and is why the check was never applied to
            // compiled provenance: a compiled list spans the whole piece, and measuring it
            // against a horizon meant for live input would reject most of a song.

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
            // sample-positioned, so the whole payload is the other pass's — and so are a
            // steal's fade and reset (ADR-0058).
            EventPayload::Note { .. } | EventPayload::Fade { .. } | EventPayload::Reset { .. } => {}
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
                // `SOUND-INV-023`: an override-layer write, composed through the slot —
                // the modulation in force stays in force — and `SOUND-INV-024`: the
                // resolved value is the slot's new target, which its segment reaches over
                // the declared policy and the kernel reads per frame from the slot's
                // buffer. Nothing is written to node state here; the state no longer
                // carries a quantum-rate control at all. `P06-S001`: the write fans out
                // over the parameter's group, one row per voice instance.
                let rows = target.instances.get() as usize;
                for row in slot.index()..slot.index().saturating_add(rows) {
                    if let Some(composed) = self.parameter_slots.get_mut(row) {
                        let _ = composed.write_override(value);
                    }
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

        // An adopted activation's gate-downs belong to the quantum at its boundary, which is
        // this one: adoption runs between renderer calls, so the first collection after it is
        // the first quantum the new mapping governs. They are counted with the quantum's own
        // changes so that the prefix sum sizes every node's run once.
        for index in 0..self.adoption_gate_len {
            let Some(slot) = self.adoption_gate_slots.get(index).copied() else {
                break;
            };
            let Some(node) = self.slot_node(slot) else {
                continue;
            };
            if let Some(count) = self.control_starts.get_mut(node + 1) {
                *count = count.saturating_add(1);
            }
        }

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
                // Counted at `node + 1`, so the prefix sum below turns the counts into
                // starts in place: entry `n` becomes where node `n`'s run begins. A write
                // that fans out over a group counts one per row (`P06-S001`).
                for row in target.slot..target.slot.saturating_add(target.rows) {
                    if let Some(node) = self.slot_node(row)
                        && let Some(count) = self.control_starts.get_mut(node + 1)
                    {
                        *count = count.saturating_add(1);
                    }
                }
            }
            // `SOUND-INV-021`: a note-on is a gate **and** the magnitudes describing the
            // note it starts, so it is due more than one write — and they land on nodes
            // other than the gate's, which is why the count is per magnitude rather than a
            // multiple of the gate's. Counted in the same window as pass two writes in, so
            // the two cannot disagree about how much room a node's run needs.
            match event.payload {
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On { slot, .. },
                } => {
                    for magnitude in self.plan.note_magnitudes_of(slot) {
                        if let Some(row) =
                            self.voice_row(magnitude.parameter.index(), identity.index())
                            && let Some(node) = self.slot_node(row)
                            && let Some(count) = self.control_starts.get_mut(node + 1)
                        {
                            *count = count.saturating_add(1);
                        }
                    }
                }
                // ADR-0058: a fade is one control per voice-sum step of the instance, a
                // reset one per instance step. Counted here so pass two has the room.
                EventPayload::Fade { identity, .. } => {
                    let Some(instance) = self.steal_instance(identity) else {
                        continue;
                    };
                    for first in self.plan.sum_groups() {
                        let node = first.index().saturating_add(instance);
                        if let Some(count) = self.control_starts.get_mut(node + 1) {
                            *count = count.saturating_add(1);
                        }
                    }
                }
                EventPayload::Reset { identity } => {
                    let Some(instance) = self.steal_instance(identity) else {
                        continue;
                    };
                    for first in self.plan.instance_groups() {
                        let node = first.index().saturating_add(instance);
                        if let Some(count) = self.control_starts.get_mut(node + 1) {
                            *count = count.saturating_add(1);
                        }
                    }
                }
                _ => {}
            }
        }
        *cursor = last;

        let mut running = 0_u32;
        for start in &mut self.control_starts {
            running = running.saturating_add(*start);
            *start = running;
        }

        // The adoption gates first, because they sit at offset zero and a run has to come out
        // ascending by offset. Every event in this quantum is at or after that boundary.
        for index in 0..self.adoption_gate_len {
            let (Some(slot), Some(gate)) = (
                self.adoption_gate_slots.get(index).copied(),
                self.adoption_gates.get(index).copied(),
            ) else {
                break;
            };
            self.push_timed_control(slot, gate.offset, gate.value);
        }
        // Spent: they belong to the boundary quantum and to no other.
        self.adoption_gate_len = 0;

        // Pass two: each change written where its node's run has room. Within a node the
        // writes happen in the order the scratch holds them, which is ascending position,
        // so each run comes out ascending by offset without a second sort.
        let mut index = window;
        while index < last {
            let Some(event) = self.event_scratch.get(index).copied() else {
                break;
            };
            index += 1;
            // The magnitudes go in **at the note's own offset**, which is what
            // `SOUND-INV-021`'s "a gate raised at the same sample must see them already
            // applied" requires: a kernel applies every control due at a frame before it
            // writes that frame, so a magnitude one sample later would leave the note's first
            // frame carrying the previous note's pitch. That placement is what
            // `a_notes_magnitudes_are_in_force_on_the_sample_its_gate_rises` asserts, and a
            // one-frame displacement fails it.
            //
            // They are written **before** the gate, and that order is currently not
            // observable — measured, not assumed: moving this block after the gate write
            // fails no test. It cannot be, because the two land at one offset and the
            // envelope's gate law does not read the velocity; the multiplier is applied where
            // the sample is emitted, after every control due at that frame. The order is kept
            // because it costs nothing and it is the one that stays correct if a kernel ever
            // does read a magnitude while handling its own edge.
            if let EventPayload::Note {
                identity,
                edge:
                    NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
            } = event.payload
            {
                for position in 0..self.plan.note_magnitudes_of(slot).len() {
                    let Some(magnitude) = self.plan.note_magnitudes_of(slot).get(position).copied()
                    else {
                        break;
                    };
                    let Some(value) = self.plan.magnitude_value(&magnitude, key, velocity) else {
                        continue;
                    };
                    // The row of the note's own voice instance (`P06-S001`).
                    let Some(row) = self.voice_row(magnitude.parameter.index(), identity.index())
                    else {
                        continue;
                    };
                    self.push_timed_control(row, event.position.quantum_offset(), value);
                }
            }
            // ADR-0058's two loop-reserved controls, on the instance the identity's index
            // names. Neither is a parameter write, so neither passes through a slot: a fade
            // and a reset are the loop's own operations on a voice, not values a caller set.
            match event.payload {
                EventPayload::Fade { identity, frames } => {
                    let Some(instance) = self.steal_instance(identity) else {
                        continue;
                    };
                    let value = crate::quantities::ParameterValue::from_frames(frames);
                    for position in 0..self.plan.sum_groups().len() {
                        let Some(first) = self.plan.sum_groups().get(position).copied() else {
                            break;
                        };
                        let node = first.index().saturating_add(instance);
                        self.push_node_control(
                            node,
                            event.position.quantum_offset(),
                            kernels::ControlIndex::FADE_OUT,
                            value,
                        );
                    }
                }
                EventPayload::Reset { identity } => {
                    let Some(instance) = self.steal_instance(identity) else {
                        continue;
                    };
                    for position in 0..self.plan.instance_groups().len() {
                        let Some(first) = self.plan.instance_groups().get(position).copied() else {
                            break;
                        };
                        let node = first.index().saturating_add(instance);
                        self.push_node_control(
                            node,
                            event.position.quantum_offset(),
                            kernels::ControlIndex::RESET,
                            crate::quantities::ParameterValue::ZERO,
                        );
                    }
                }
                _ => {}
            }
            let (Some(target), Some(value)) = (event.target, self.timed_value(event.payload))
            else {
                continue;
            };
            for row in target.slot..target.slot.saturating_add(target.rows) {
                self.push_timed_control(row, event.position.quantum_offset(), value);
            }
        }
    }

    /// The row of a parameter group that a note's voice instance owns: the group's first
    /// row plus the identity's index, which is the instance the occurrence sounds on
    /// (`P06-S001`). A group of one row — a played node outside the voice scope — is shared
    /// by every occurrence, as it was before instancing. `None` where the index is outside
    /// a group of several — an identity the plan's partition does not hold — so nothing is
    /// written rather than another voice's row.
    fn voice_row(&self, first: usize, index: u16) -> Option<usize> {
        let target = self.plan.parameter_targets().get(first)?;
        let instances = target.instances.get() as usize;
        if instances <= 1 {
            return Some(first);
        }
        let voice = usize::from(index);
        (voice < instances).then_some(first.saturating_add(voice))
    }

    /// The voice instance a steal's identity names, or `None` where the index lies outside
    /// the plan's partition — the same refusal `voice_row` makes, so a bad index selects no
    /// step rather than one scheduled after the last instance. An independent read found the
    /// index taken unchecked.
    fn steal_instance(&self, identity: crate::identity::NoteIdentity) -> Option<usize> {
        let instance = usize::from(identity.index());
        (instance < self.plan.voice_instances().get() as usize).then_some(instance)
    }

    /// The node a parameter slot's control belongs to, from the target table.
    fn slot_node(&self, slot: usize) -> Option<usize> {
        self.plan
            .parameter_targets()
            .get(slot)
            .map(|target| target.node.index())
    }

    /// Append one control change to a node's run, composed through its parameter slot.
    ///
    /// The one place a run is written, so the gate, the magnitudes `SOUND-INV-021` adds
    /// beside it and an adopted activation's gate-downs are placed by the same rule — and
    /// the counting sort's two passes stay symmetric, because a second inline copy of this
    /// is how a run comes to be filled past what pass one counted. It is also the one place
    /// a sample-positioned write meets `SOUND-INV-023`'s slot: the value a caller sent is
    /// the override layer, and what the kernel is handed is the slot's resolved value, so a
    /// modulation in force on the destination is composed into a note's pitch as it is
    /// into an automated one. Every step is bounds-checked; a slot the plan has no row for
    /// or a full run writes nothing rather than panicking on the audio thread.
    fn push_timed_control(
        &mut self,
        slot: usize,
        offset: crate::time::QuantumOffset,
        value: crate::quantities::ParameterValue,
    ) {
        let (Some(target), Some(composed)) = (
            self.plan.parameter_targets().get(slot).copied(),
            self.parameter_slots.get_mut(slot),
        ) else {
            return;
        };
        let (node, control) = (target.node.index(), target.control);
        let value = composed.write_override(value);
        self.push_node_control(node, offset, control, value);
    }

    /// Append one control change to a node's run, by node.
    ///
    /// The tail of [`Self::push_timed_control`], and the whole of a loop-reserved control's
    /// path (ADR-0058's fade and reset): those address a step rather than a parameter, so
    /// there is no slot to compose through and nothing a caller could have set. Every step is
    /// bounds-checked; a node the run table has no entry for or a full run writes nothing.
    fn push_node_control(
        &mut self,
        node: usize,
        offset: crate::time::QuantumOffset,
        control: kernels::ControlIndex,
        value: crate::quantities::ParameterValue,
    ) {
        let (Some(base), Some(filled)) = (
            self.control_starts.get(node).copied(),
            self.control_fill.get(node).copied(),
        ) else {
            return;
        };
        let Some(slot) = self
            .timed_controls
            .get_mut(base.saturating_add(filled) as usize)
        else {
            return;
        };
        *slot = TimedControl {
            offset,
            control,
            value,
        };
        if let Some(fill) = self.control_fill.get_mut(node) {
            *fill = fill.saturating_add(1);
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
                    edge: NoteEdge::On { slot, .. },
                } => {
                    // The on edge names what is played; the registry records it against the
                    // occurrence so the release does not have to carry it.
                    self.live_notes.admit(identity, slot);
                    self.note_target(slot, identity)
                }
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                } => match self.live_notes.release(identity) {
                    Some(slot) => self.note_target(slot, identity),
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
                        slot: slot.index(),
                        rows: row.instances.get() as usize,
                    }),
                // ADR-0058 clause 5: the taken note is ended as its fade begins. Its own
                // release, should one reach the loop, is then an orphan; the compiled path
                // drops it at preparation and counts it there. The fade and the reset land
                // through the instance groups rather than a target, so neither has one.
                EventPayload::Fade { identity, .. } => {
                    // A fade whose identity names no live note is as much an orphan as a
                    // release that does, and is counted as one: preparation stamps a fade
                    // against a note it holds live, so only a stale or foreign one gets here.
                    if self.live_notes.release(identity).is_none() {
                        pending.orphan_note = pending.orphan_note.saturating_add(1);
                        pending.last_orphan_note = Some(identity);
                    }
                    None
                }
                EventPayload::Reset { .. } => None,
            };
            if let Some(slot) = self.event_scratch.get_mut(index) {
                slot.target = target;
            }
        }
    }

    /// Where a note slot's edge lands: the gate control's row for the note's own voice
    /// instance (`P06-S001`).
    fn note_target(
        &self,
        slot: crate::plan::NoteSlot,
        identity: crate::identity::NoteIdentity,
    ) -> Option<ResolvedTarget> {
        let row = self.plan.note_targets().get(slot.index())?;
        let voice_row = self.voice_row(row.parameter.index(), identity.index())?;
        Some(ResolvedTarget {
            slot: voice_row,
            rows: 1,
        })
    }

    /// What it moves it to.
    fn timed_value(&self, payload: EventPayload) -> Option<crate::quantities::ParameterValue> {
        match payload {
            EventPayload::Note { edge, .. } => Some(edge.value()),
            EventPayload::SetParameter { value, .. } => Some(value),
            EventPayload::Fade { .. } | EventPayload::Reset { .. } => None,
        }
    }

    /// Render exactly one quantum and append it to the output carry.
    fn render_quantum(&mut self) -> Result<(), RenderError> {
        let quantum = QUANTUM_FRAMES as usize;
        let quantum_start = self.clock;

        // The plan position of this quantum's first sample. Anchoring is the only
        // place engine time and plan time meet.
        let plan_start = self.plan_position_of(quantum_start);

        // `SOUND-INV-024`: every quantum-rate slot advances **before** any kernel reads, one
        // add per frame into its buffer. The one place a segment moves.
        for index in 0..self.parameter_slots.len() {
            let Some(offset) = self.ramp_offsets.get(index).copied() else {
                break;
            };
            let (Some(slot), Some(buffer)) = (
                self.parameter_slots.get_mut(index),
                self.ramp_buffers
                    .get_mut(offset..offset.saturating_add(quantum)),
            ) else {
                continue;
            };
            slot.advance(buffer);
        }

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
                    let Some(prepared) = self.plan.prepared_nodes().get(step.prepared().index())
                    else {
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
                    let (Some(ramp_start), Some(ramp_end)) = (
                        self.ramp_starts.get(step.node().index()).copied(),
                        self.ramp_starts.get(step.node().index() + 1).copied(),
                    ) else {
                        continue;
                    };
                    let ramps = self.ramp_buffers.get(ramp_start..ramp_end).unwrap_or(&[]);
                    let Some(mut io) = kernels::bind(
                        &mut self.buffers,
                        self.plan.regions(),
                        step,
                        plan_start,
                        gates,
                        ramps,
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
        output: AudioBlockMut<'_>,
        events: TimedEvents<'_>,
    ) -> Result<(), RenderError> {
        self.render_observed(output, events, None)
    }
}

impl PreparedRenderer {
    /// Render one caller block, pushing each rendered quantum of every subscribed tap into
    /// the observers' rings (`HOST-INV-023`).
    ///
    /// The store is a parameter for the reason the ingress store is: the host owns it, and
    /// the renderer neither allocates nor keeps it. With `None` this is [`Renderer::render`]
    /// exactly, and with a store the audio is the same — the push is a copy out of the arena
    /// after the schedule walk, which `tests/observation.rs` holds bit for bit.
    pub fn render_observed(
        &mut self,
        mut output: AudioBlockMut<'_>,
        events: TimedEvents<'_>,
        mut observers: Option<&mut crate::observe::ObservationSubscriptions>,
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
            // `SOUND-INV-022`'s taps, read by `HOST-INV-023`'s subscribers: after the walk,
            // the tapped regions hold this quantum, and ADR-0005 clause 6 kept them so.
            if let Some(observers) = observers.as_deref_mut() {
                observers.push_quantum(
                    self.plan.id(),
                    self.plan.taps(),
                    self.plan.regions(),
                    &self.buffers,
                );
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
