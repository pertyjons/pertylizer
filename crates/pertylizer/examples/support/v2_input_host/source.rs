//! Concrete nonblocking producer custody; the merger resolves every popped observation.
use super::*;
use synth_engine_v2::host::input::{InputTick, SimulatedInputClock};
use synth_engine_v2::recording::notes::Midi1Event;

#[path = "../../../../synth_engine_v2/tests/support/source_custody.rs"]
mod source_custody;
#[path = "../../../../synth_engine_v2/tests/support/source_queue.rs"]
mod source_queue;
#[allow(
    unused_imports,
    reason = "preserve the example host's source type paths"
)]
pub use source_custody::{
    SOURCE_BACKING_CELLS, SOURCE_CELLS, SourceAccepted, SourceAttemptId, SourceHandoff,
    SourceHeadError, SourceInbox, SourceOwnedSendError, SourceProducer, SourceQueueId,
    SourceQueueStamp, SourceRetiredAttempt, SourceRetryToken, SourceSendError,
};

pub const SOURCE_OUTSTANDING: synth_engine_v2::quantities::EventCount =
    synth_engine_v2::quantities::EventCount::measured(64);

impl SourceInbox {
    /// Register a terminal producer fault before any source queue is serviced.
    pub fn record_failure(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(SourceOfferReport),
    ) {
        if let Some((observation, reason)) = self.unresolved_failure() {
            match control.source_failed_before_ring(self.generation(), observation, reason) {
                Ok(()) => self.resolve_failure(),
                Err(error) => {
                    receive(SourceOfferReport::new(
                        Err(InputOfferError::Refused(observation, reason)),
                        AuditionPacketCustody::NotQueued,
                        Some(error),
                    ));
                    self.resolve_failure();
                }
            }
        }
    }

    /// A fixed prefix, including refusals after closure. A refusal returns the
    /// original; an accepted ID with a fault must never be retried.
    #[cfg(test)]
    pub fn service(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(InputOfferResult),
    ) {
        self.service_identified(control, |_, result| receive(result));
    }

    /// A queue ID accompanies every popped observation, including raw refusals.
    /// A pre-ring failure has no queue ID.
    #[cfg(test)]
    pub fn service_identified(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(Option<SourceQueueId>, InputOfferResult),
    ) {
        self.service_custody_identified(control, |id, report| receive(id, report.result));
    }

    /// Preserve the raw result and exact audition-packet custody for each pop.
    #[cfg(test)]
    pub fn service_custody_identified(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(Option<SourceQueueId>, InputOfferReport),
    ) {
        self.service_handoff_identified(control, |handoff, report| {
            assert!(
                report.attribution_error.is_none(),
                "test-only projection cannot discard a source attribution failure"
            );
            receive(handoff.map(|item| item.stamp().queue()), report.offer);
        });
    }

    /// A queued original and its validated nominal engine time remain available
    /// beside the raw offer result. A pre-ring failure has no queued handoff.
    /// A head waits only when completed raw receipts can free its needed cells;
    /// the same head is retried after those receipts are collected.
    pub fn service_handoff_identified(
        &mut self,
        control: &mut LiveControl,
        mut receive: impl FnMut(Option<SourceHandoff>, SourceOfferReport),
    ) {
        self.record_failure(control, |report| receive(None, report));
        let prefix = self.occupied_len();
        for _ in 0..prefix {
            let Some(head) = self.peek_handoff() else {
                break;
            };
            if matches!(
                control.preflight_source_head(
                    self.generation(),
                    head.stamp().queue(),
                    head.observation(),
                ),
                Ok(InputCustodyPreflight::Await(_))
            ) {
                break;
            }
            let report = control.offer_source_queued(
                self.generation(),
                head.stamp().queue(),
                head.observation(),
            );
            match self.commit_handoff(head) {
                Ok(committed) => receive(Some(committed), report),
                Err(error) => {
                    // The sole consumer cannot change the head across this
                    // call. Preserve the raw report and halt further offers.
                    control.halt.request_invalid();
                    eprintln!("source head commit failed after raw offer: {error}");
                    receive(Some(head), report);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
impl SourceProducer {
    pub(super) fn set_serial_for_test(&mut self, serial: u64) {
        self.set_serial_in_core_for_test(serial);
    }

    pub(super) fn set_attempt_serial_for_test(&mut self, serial: u64) {
        self.set_attempt_serial_in_core_for_test(serial);
    }
}
