//! Compiled-event scheduling at the renderer's engine-time boundary.
//!
//! Phase 3 starts here, deliberately on the engine side of ADR-0022. A compiled event
//! already carries the current epoch's [`SampleTime`]; this module does not observe a
//! host clock, compensate latency, or assign `Hardware`/`Arrival` provenance. It rejects
//! an over-full absolute quantum during preparation, then turns the sorted compiled list
//! into a bounded event span for each actual host call. Full compiled-share and
//! sliding-window admission remain later Phase 3 work.
//!
//! Preparation validates order and plan identity and allocates the stamped list. The
//! [`CompiledEventScheduler::render`] path only advances a cursor over that storage and
//! delegates one borrowed slice to the renderer. Its source lives in `schedule/hot.rs`
//! so the real-time purity test scans it alongside the renderer and node kernels.

use thiserror::Error;

use crate::plan::PlanId;
use crate::quantities::EventCount;
use crate::render::{EventEnvelope, EventPayload, PreparedRenderer, TimedEvent};
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
    payload: EventPayload,
}

impl CompiledEvent {
    /// An event at an exact engine time.
    pub const fn new(time: SampleTime, payload: EventPayload) -> Self {
        Self { time, payload }
    }

    /// The event's exact engine time.
    pub const fn time(self) -> SampleTime {
        self.time
    }

    /// What the event does.
    pub const fn payload(self) -> EventPayload {
        self.payload
    }
}

/// Why a compiled schedule could not be prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SchedulePrepareError {
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
}

/// A prepared, epoch-stamped compiled list and its release cursor.
#[derive(Debug)]
#[must_use]
pub struct CompiledEventScheduler {
    epoch: StreamEpoch,
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
        renderer: &PreparedRenderer,
        events: &[CompiledEvent],
    ) -> Result<Self, SchedulePrepareError> {
        let expected = renderer.plan().id();
        let max_events_per_quantum = renderer.plan().max_events_per_quantum();
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
        let mut stamped = Vec::with_capacity(events.len());
        for event in events.iter().copied() {
            stamped.push(TimedEvent::new(
                EventEnvelope::new(epoch, event.time(), TimeSource::Compiled),
                event.payload(),
            ));
        }

        Ok(Self {
            epoch,
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

const fn payload_plan(payload: EventPayload) -> PlanId {
    match payload {
        EventPayload::SetParameter { slot, .. } => slot.plan(),
        EventPayload::Note { slot, .. } => slot.plan(),
    }
}

#[path = "schedule/hot.rs"]
mod hot;
