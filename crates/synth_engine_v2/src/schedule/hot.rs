//! The compiled scheduler's audio-thread path.
//!
//! Like `render/hot.rs`, this file may not allocate, lock, perform I/O, log, or panic. The
//! prepared list is immutable; only its cursor advances after a successful render.
//!
//! **Compiled events reach the renderer through the arbiter, not around it.** An earlier
//! revision handed the renderer a borrowed slice of the prepared list directly. That made
//! ADR-0046 clause 2's "the only normal path that constructs renderer input" false for the
//! one producer Phase 2 already had, and it made an equivalence test between this path and
//! a published one meaningless — the two would not share a boundary. Every due event is now
//! charged to `ProducerClass::Compiled` and the sealed batch is what the renderer sees.

use super::{CompiledEventScheduler, ScheduledRenderError};
use crate::publish::{ProducerClass, PublicationArbiter};
use crate::render::{AudioBlockMut, EventEnvelope, PreparedRenderer, Renderer, TimedEvent};
use crate::time::{FrameCount, QUANTUM_FRAMES, SampleTime};

/// The first quantum boundary at or after `time`, or `None` where engine time has none.
///
/// The build refuses a request with no boundary, so the `None` here is unreachable for an
/// accepted candidate — it is a refusal rather than an index because the audio thread has no
/// way to report a panic.
fn snap(time: SampleTime) -> Option<SampleTime> {
    let quantum = u64::from(QUANTUM_FRAMES);
    let remainder = time.as_u64() % quantum;
    if remainder == 0 {
        return Some(time);
    }
    time.as_u64()
        .checked_add(quantum - remainder)
        .map(SampleTime::new)
}

/// How many quanta the old mapping still owns before `boundary`.
///
/// Zero means the boundary is the clock, so the whole call belongs to the new mapping.
fn boundary_quanta(clock: SampleTime, boundary: SampleTime) -> usize {
    let Some(frames) = boundary.as_u64().checked_sub(clock.as_u64()) else {
        return 0;
    };
    usize::try_from(frames / u64::from(QUANTUM_FRAMES)).unwrap_or(usize::MAX)
}

impl CompiledEventScheduler {
    /// The placed event at `index`, displaced by the activation shift.
    ///
    /// ADR-0050 clause 1's uniform offset, applied where an event is read rather than by
    /// rewriting the list: the displacement is the same for every event, so reproducing it
    /// costs one addition and the audio thread never walks the schedule to re-place it.
    ///
    /// Every read of the schedule goes through here — the missed-event check, the window
    /// scan and the charge — because a shift applied at some reads and not others would put
    /// the window and the events it selects on different timelines.
    fn placed(&self, index: usize) -> Result<Option<TimedEvent>, ScheduledRenderError> {
        let Some(event) = self.events.get(index).copied() else {
            return Ok(None);
        };
        match self.displaced(event) {
            Some(placed) => Ok(Some(placed)),
            // **Not** the end of the list. Returning `None` here would stop every scan at
            // this event and leave it, and everything after it, unpublished while the cursor
            // never reached completion — a silent drop with no counter to show it.
            //
            // **And it is terminal**, which is what makes the behaviour defined rather than
            // merely reported. The shift is fixed at adoption and the event is fixed in the
            // list, so the condition cannot clear: a retryable error here would hand the
            // caller the same refusal forever. An earlier revision argued the branch was
            // unreachable because it needs a clock past `2^63`; an independent review was
            // right that a representable clock is not an unreachable one, and a defined
            // response is worth more than the argument. The branch is defensive and has no
            // test, which is stated rather than covered by a bound that does not hold.
            None => Err(ScheduledRenderError::EventTimeUnrepresentable {
                time: event.envelope().time(),
                shift: self.shift,
            }),
        }
    }

    /// One event moved by the activation shift.
    ///
    /// One function for one displacement. Clause 7's catch-up batch will be published under
    /// the same shift when it lands — it is stamped against the requested time exactly as the
    /// schedule is — and an earlier revision that applied the displacement at some reads and
    /// not others named a quantum the arbiter's window did not cover, which faulted an
    /// otherwise valid seek.
    fn displaced(&self, event: TimedEvent) -> Option<TimedEvent> {
        if self.shift == FrameCount::ZERO {
            return Some(event);
        }
        let envelope = event.envelope();
        let time = envelope.time().as_u64().checked_add(self.shift.as_u64())?;
        Some(TimedEvent::new(
            EventEnvelope::new(envelope.epoch(), SampleTime::new(time), envelope.source()),
            event.payload(),
        ))
    }

    /// Render one actual host block with the prepared, bounded events its quanta contain.
    ///
    /// The arbiter is a parameter rather than owned state because ADR-0046 clause 2 admits
    /// exactly **one** of them: every producer for a stream publishes into the same store,
    /// and a scheduler holding its own would be a second arbiter wearing another name.
    pub fn render(
        &mut self,
        renderer: &mut PreparedRenderer,
        arbiter: &mut PublicationArbiter,
        output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        self.render_with_ingress(renderer, arbiter, None, output)
    }

    /// Render one actual host block, draining a live ingress store into the same pass.
    ///
    /// The store is a parameter for the reason the arbiter is: ADR-0046 clause 2 admits one
    /// publication pass per call, and every producer for the stream publishes into it. A
    /// scheduler owning the live store would make the compiled producer the owner of
    /// another producer's storage.
    ///
    /// Additive rather than a change to [`Self::render`], so a stream with no live producer
    /// keeps the signature it had; passing `None` is exactly what that stream is.
    pub fn render_with_ingress(
        &mut self,
        renderer: &mut PreparedRenderer,
        arbiter: &mut PublicationArbiter,
        ingress: Option<&mut crate::ingress::PerformanceIngress>,
        output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        // **Every refusal that leaves the stream untouched happens before any adoption.**
        // Adoption moves the anchor, the live-note registry, the schedule and the exchange,
        // and each of these three says the call should not have happened at all: a crossed
        // pair of stream halves, a retry after the epoch faulted, a second arbiter. Deciding
        // them after adopting would mutate the stream on the way to the same error, so a
        // caller that corrected its mistake and retried would find a stream that had already
        // seeked. An independent review found exactly that in the boundary-at-the-clock
        // branch below, which adopts before rendering anything.
        if self.epoch != renderer.epoch() {
            return Err(ScheduledRenderError::EpochMismatch {
                schedule: self.epoch,
                renderer: renderer.epoch(),
            });
        }

        // A faulted epoch publishes nothing further and adopts nothing either. Delegating
        // rather than filling here: `PreparedRenderer::render`'s own first check silences the
        // block and returns the same error, and duplicating that would be a second place for
        // the two to disagree about what a dead epoch looks like.
        if renderer.diagnostics().needs_reprepare() {
            return renderer
                .render(output, crate::render::TimedEvents::EMPTY)
                .map_err(ScheduledRenderError::Render);
        }

        // The arbiter this schedule adopted, latched on first use. A second store would pass
        // every capacity bound while restarting the high-water history.
        match self.arbiter {
            Some(latched) if latched != arbiter.id() => {
                return Err(ScheduledRenderError::ForeignArbiter {
                    latched,
                    offered: arbiter.id(),
                });
            }
            Some(_) => {}
            None => self.arbiter = Some(arbiter.id()),
        }

        // And the ingress store, **verified rather than latched**. The off-thread half marks
        // a store when it adopts it, and this reads that one mark: two independent latches
        // could disagree, and a caller offering into store A while rendering store B wedged
        // the stream — offers reached only A while drains accepted only B, permanently. An
        // independent review found it.
        //
        // A store this stream never adopted is refused, and refusing it strands nothing:
        // every offer goes through the control, which is what sets the mark, so an unadopted
        // store is necessarily empty. What the refusal does prevent is its zero counters
        // overwriting the adopted store's totals, since the drain mirrors rather than adds.
        if let Some(store) = ingress.as_deref()
            && store.adopted_by() != Some(self.epoch)
        {
            return Err(ScheduledRenderError::UnadoptedIngressStore {
                store: store.id(),
                stream: self.epoch,
            });
        }

        // **And the block itself, for the same reason.** `PreparedRenderer::render` refuses a
        // layout the plan does not render and faults an oversized one, and both decisions are
        // made after this method would already have adopted. A layout mismatch is retryable,
        // so a caller that corrected it would find a stream that had silently seeked and
        // charged its release. Delegating rather than deciding here keeps one authority on
        // the block's shape; publication is skipped because a call that cannot render must
        // not spend a charge either.
        if output.layout() != renderer.plan().channel_layout()
            || output.frames() as u64 > renderer.plan().maximum_block_size().as_u64()
        {
            return renderer
                .render(output, crate::render::TimedEvents::EMPTY)
                .map_err(ScheduledRenderError::Render);
        }

        // ADR-0050 clause 4. When a pending activation's boundary falls strictly inside the
        // quanta this block would render, the block is rendered as **two** calls with the
        // adoption between them. That is free rather than clever: ADR-0001's partition
        // invariance is exactly the property that the same frames rendered as two calls
        // produce the same audio as one.
        //
        // Where to cut is determined rather than chosen. With `c` the live carry and `k` the
        // number of quanta the old mapping still owns, the crossing case is `frames > c + kQ`
        // and the first call is `c + kQ` frames — the largest request that renders precisely
        // `k` quanta, because `quanta_needed_for(f)` is `ceil((f - c) / Q)`.
        let split = match self.pending_boundary(renderer) {
            Some((boundary, late)) => {
                let quanta = renderer.quanta_needed_for(output.frames());
                let owned = boundary_quanta(renderer.clock(), boundary);
                if owned == 0 && quanta > 0 {
                    // The boundary is the clock itself: adopt before rendering anything, so
                    // the whole block runs under the new mapping.
                    //
                    // **`quanta > 0` is what keeps an adoption's debt payable in the call
                    // that incurs it.** The release charge is published by this call's own
                    // publication pass, and a window of no quanta has no row for it; adopting
                    // anyway leaves it owing, and a second activation adopted at the same
                    // clock then coalesces two release operations into one charge. An
                    // independent review found it, and clause 7's catch-up will owe its
                    // quantum the same way.
                    //
                    // Waiting costs nothing observable, and — unlike the first repair, which
                    // skipped the **whole** activation while any debt was outstanding —
                    // nothing that depends on the partition either. A call that renders no
                    // quantum writes no new audio and does not move the clock, so the next
                    // call computes the same boundary. A second review found that first
                    // repair delaying an already-accepted candidate by a whole callback,
                    // which made its effective point a function of the host's block size:
                    // the one property this slice exists to guarantee.
                    self.adopt_pending(renderer, boundary, late);
                    None
                } else if owned < quanta {
                    Some((
                        boundary,
                        late,
                        renderer
                            .carry_frames()
                            .saturating_add(owned.saturating_mul(QUANTUM_FRAMES as usize)),
                    ))
                } else {
                    None
                }
            }
            None => None,
        };

        // Reborrowed rather than moved, because the split path renders twice and each
        // sub-call is its own publication pass: the store must be drained by both, and a
        // moved `&mut` would serve only the first.
        let mut ingress = ingress;
        let whole = match split {
            Some((boundary, late, cut)) => match output.split_at_frame(cut) {
                Ok((mut head, mut tail)) => {
                    let mut outcome =
                        self.render_one(renderer, arbiter, ingress.as_deref_mut(), head.reborrow());
                    if outcome.is_ok() {
                        self.adopt_pending(renderer, boundary, late);
                        outcome = self.render_one(
                            renderer,
                            arbiter,
                            ingress.as_deref_mut(),
                            tail.reborrow(),
                        );
                    }
                    // **The terminal contract is silence over the complete callback**, and a
                    // sub-call can only silence its own half. After a tail fault the head
                    // holds audio that was rendered successfully; after a head fault the tail
                    // holds whatever the caller's buffer held. Both are frames the host is
                    // about to play, so the fault clears the block it was given rather than
                    // the span it happened in. An independent review found the tail case.
                    if outcome.is_err() && renderer.diagnostics().needs_reprepare() {
                        head.silence();
                        tail.silence();
                    }
                    return outcome;
                }
                // A cut outside the block cannot happen while `owned < quanta` holds. It is
                // written as a fall-through rather than an index because the audio thread has
                // no way to report a panic, and the block that comes back is the same one.
                Err(whole) => whole,
            },
            None => output,
        };
        self.render_one(renderer, arbiter, ingress, whole)
    }

    /// The effective point of the pending activation, if there is one.
    ///
    /// ADR-0050 clause 1: the first quantum boundary at or after `max(requested, clock)`. The
    /// clock always stands on a boundary, so the result is never behind it — which is what
    /// makes clause 2's statement about the carry true.
    /// Returns the point and whether the activation is **late**, which are two different
    /// questions decided from the same comparison.
    ///
    /// Lateness is **not** decided here: `offer` recorded it, because that is the moment
    /// clause 1's rule is about and the only one at which it has a stable answer.
    fn pending_boundary(&self, renderer: &PreparedRenderer) -> Option<(SampleTime, bool)> {
        let crate::transport::Exchange::Pending(activation) = &self.exchange else {
            return None;
        };
        let boundary = snap(activation.requested().max(renderer.clock()))?;
        Some((boundary, activation.late))
    }

    /// Adopt the pending activation, putting the retired one in the slot it came from.
    ///
    /// Infallible by construction (clause 3): the slot holds a candidate whose epoch and
    /// sequence were checked at the offer, and every allocation it needs travels with it.
    fn adopt_pending(&mut self, renderer: &mut PreparedRenderer, boundary: SampleTime, late: bool) {
        let crate::transport::Exchange::Pending(mut activation) =
            core::mem::take(&mut self.exchange)
        else {
            return;
        };
        renderer.adopt(&mut activation, boundary, late, self.next, self.shift);

        // **Swapped, not rebuilt.** The candidate's vectors become the scheduler's live ones
        // and the scheduler's become the candidate's, so the box the candidate arrived in
        // carries the retired state back for the control to free. Constructing a second
        // value here would allocate, which the audio thread may not do — the purity scan
        // caught exactly that in the first version.
        //
        // The arbiter identity is deliberately **kept**: ADR-0046 clause 2 admits exactly
        // one per stream, so an adopted schedule inherits the latch rather than starting
        // unlatched. The candidate's **outstanding set and minter copy are not swapped
        // either**, for the opposite reason: they describe the schedule that is now in
        // force, and the control takes them when it collects.
        core::mem::swap(&mut self.events, &mut activation.events);
        // Clause 1's uniform displacement. The candidate was stamped against the time it
        // requested; the boundary may be later, and the whole stream moves with it.
        self.shift = FrameCount::new(
            boundary
                .as_u64()
                .saturating_sub(activation.requested.as_u64()),
        );
        self.next = 0;
        self.in_force = activation.sequence;
        // Clause 5's mass release ran inside `adopt`, and it owes the session share one
        // charge at the boundary quantum. Recorded rather than charged here because
        // publication belongs to the next call, which is the one that opens a window.
        core::mem::swap(&mut self.catch_up, &mut activation.catch_up);
        self.catch_up_len = self.catch_up.len();
        self.owed_release_charge = true;
        activation.effective = Some(boundary);
        self.exchange = crate::transport::Exchange::Retired(activation);
    }

    /// One renderer call: publish the due span and render it.
    fn render_one(
        &mut self,
        renderer: &mut PreparedRenderer,
        arbiter: &mut PublicationArbiter,
        ingress: Option<&mut crate::ingress::PerformanceIngress>,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), ScheduledRenderError> {
        // The epoch, the faulted-stream and the arbiter checks are `render`'s, taken before
        // it could adopt. Repeating them here would be a second authority on the same three
        // questions, and the split path reaches this twice with nothing between the calls
        // that could change any of their answers.
        let clock = renderer.clock();
        let head = match self.placed(self.next) {
            Ok(event) => event,
            Err(error) => {
                renderer.terminal_displacement_fault(&mut output);
                return Err(error);
            }
        };
        if let Some(event) = head
            && event.envelope().time() < clock
        {
            return Err(ScheduledRenderError::MissedEvent {
                event: event.envelope().time(),
                clock,
            });
        }

        let quanta = renderer.quanta_needed_for(output.frames());
        let admissible = self
            .max_events_per_quantum
            .as_usize()
            .unwrap_or(0)
            .saturating_mul(quanta);
        let Ok(quantum_count) = u64::try_from(quanta) else {
            return Err(ScheduledRenderError::CallSpanUnrepresentable { clock });
        };
        let Some(end_quantum) = clock.quantum_index().checked_add(quantum_count) else {
            return Err(ScheduledRenderError::CallSpanUnrepresentable { clock });
        };

        let mut end = self.next;
        loop {
            let event = match self.placed(end) {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => {
                    renderer.terminal_displacement_fault(&mut output);
                    return Err(error);
                }
            };
            if event.envelope().time().quantum_index() >= end_quantum {
                break;
            }
            if end.saturating_sub(self.next) >= admissible {
                return Err(ScheduledRenderError::EventWindowTooDense {
                    clock,
                    presented_at_least: admissible.saturating_add(1),
                    admissible,
                });
            }
            end = end.saturating_add(1);
        }

        // Publication first, and a failure here ends the stream rather than the call.
        // Clause 7 is explicit that a share overrun is a contract violation and not a load
        // condition, so there is nothing to retry and the terminal response is taken before
        // the caller learns about it.
        let mut publication = match arbiter.open(clock, quanta) {
            Ok(publication) => publication,
            Err(fault) => {
                renderer.terminal_fault(&mut output);
                return Err(ScheduledRenderError::Publication(fault));
            }
        };

        // **What an adoption owes is published in the first call that renders a quantum,
        // which is not always the first call after it.** A call served entirely from the
        // carry opens a zero-quantum window, and a charge into one has no row to land in;
        // charging anyway faulted an ordinary seek, which an independent review found. The
        // debt is not spent until it is paid, so it simply waits for the next call.
        if quanta > 0 {
            // ADR-0050 clause 5's boundary mass release: **one** operation charged to the
            // session share, never one event per voice. It ended its notes inside `adopt`;
            // this is the accounting, and without it the share's occupancy and high-water
            // mark are blind to an operation that admission reserved room for.
            if self.owed_release_charge {
                if let Err(fault) = publication.charge_operation(ProducerClass::Session, clock) {
                    renderer.terminal_fault(&mut output);
                    return Err(ScheduledRenderError::Publication(fault));
                }
                self.owed_release_charge = false;
            }

            // ADR-0051 clause 1's catch-up, **after** the boundary gate-downs and **before**
            // the new stream's own events at the same sample. The batch is the state already
            // in force at the destination; the stream is what happens from there, so the other
            // order would let a stale restoration overwrite the new timeline's first event.
            // Charged to the session share, and displaced by the same shift the schedule
            // carries.
            let mut restored = 0;
            while restored < self.catch_up_len {
                let Some(stamped) = self.catch_up.get(restored).copied() else {
                    break;
                };
                let Some(event) = self.displaced(stamped) else {
                    renderer.terminal_displacement_fault(&mut output);
                    return Err(ScheduledRenderError::EventTimeUnrepresentable {
                        time: stamped.envelope().time(),
                        shift: self.shift,
                    });
                };
                if let Err(fault) = publication.charge(ProducerClass::Session, event) {
                    renderer.terminal_fault(&mut output);
                    return Err(ScheduledRenderError::Publication(fault));
                }
                restored = restored.saturating_add(1);
            }
            // Spent: a catch-up restores a destination once, not on every call after it.
            self.catch_up_len = 0;
        }

        let mut index = self.next;
        while index < end {
            let event = match self.placed(index) {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => {
                    renderer.terminal_displacement_fault(&mut output);
                    return Err(error);
                }
            };
            if let Err(fault) = publication.charge(ProducerClass::Compiled, event) {
                renderer.terminal_fault(&mut output);
                return Err(ScheduledRenderError::Publication(fault));
            }
            index = index.saturating_add(1);
        }

        // **Live ingress is drained last**, which is ADR-0023's declared order: session and
        // transport first, because they are the state already in force at this sample;
        // compiled second, because it is the timeline; live last, because a performer acts
        // on top of it and at a tie the second write is what the quantum renders.
        //
        // The store is drained even in a window of no quanta, because the drain is also
        // where the horizon's reference clock is recorded. It charges nothing there — a
        // window with no rows reaches no destination.
        if let Some(store) = ingress
            && let Err(fault) =
                store.drain_into(&mut publication, renderer.diagnostics_mut(), clock)
        {
            renderer.terminal_fault(&mut output);
            return Err(ScheduledRenderError::Publication(fault));
        }

        let batch = publication.seal();
        if let Err(error) = renderer.render(output, batch.events()) {
            return Err(ScheduledRenderError::Render(error));
        }
        self.next = end;
        Ok(())
    }
}
