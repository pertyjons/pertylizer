//! Common raw-first input boundary for ADR-0078's non-shipping combined host.
//! Source FIFOs and concrete queue ownership remain with the host. This module
//! makes the routing order and ADR-0079 outcome classification shared library laws;
//! it does not enable a physical adapter or qualify a production capacity profile.
mod hot;
use super::{
    input::{InputCaptureControl, InputError, InputEventId, InputObservation, InputOutcome},
    session::{LoopSessionError, SessionError, SessionSourceOutcome},
};
use crate::{
    host::{
        ConnectionGeneration, input::SimulatedInputClock,
        session::loop_transfer::LoopTransferPacket,
    },
    stream::MixedTransferProducer,
    time::SampleTime,
};
use crate::{
    recording::notes::{CaptureDisposition, NoteCaptureError},
    stream::{
        MixedBridgeControl, MixedBridgeError, MixedSourceFault, MixedSourceHandoff,
        MixedSourcePacketId, MixedStaged, MixedTransferSources,
    },
};
use thiserror::Error;

/// Raw queue coverage has source-generation lifetime, so it survives a sound-owner
/// reset. Only successful FIFO queue sends can advance this opaque proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedRawPublication {
    sources: [ConnectionGeneration; 2],
    through: SampleTime,
    interrupted: bool,
}

/// Off-thread tracker around the host's real raw FIFO producer. Retain every refused
/// packet and retry it before preparing or sending a younger packet. As with the
/// mixed queue contract, the producer must retain each accepted packet exactly once.
#[derive(Debug)]
#[must_use]
pub struct MixedRawPublisher {
    sources: [ConnectionGeneration; 2],
    clocks: [SimulatedInputClock; 2],
    frontiers: [SampleTime; 2],
}
#[derive(Debug, Error)]
pub enum MixedRawSendRefusal {
    #[error("raw FIFO is full; retain and retry this packet first")]
    Full(LoopTransferPacket),
    #[error("raw publication binding or observation refused: {error}")]
    Invalid {
        packet: LoopTransferPacket,
        error: InputError,
    },
}
impl MixedRawPublisher {
    /// Bind to the raw owner's two distinct source generations, off-thread.
    ///
    /// # Errors
    /// Missing, duplicate or unconfigured sources.
    pub fn new(
        raw: &InputCaptureControl,
        sources: MixedTransferSources,
    ) -> Result<Self, InputError> {
        let sources = [sources.first(), sources.second()];
        let clocks = [
            raw.input(sources[0])
                .and_then(|input| input.clock())
                .ok_or(InputError::Stale)?,
            raw.input(sources[1])
                .and_then(|input| input.clock())
                .ok_or(InputError::Stale)?,
        ];
        Ok(Self {
            sources,
            clocks,
            frontiers: [SampleTime::ZERO; 2],
        })
    }
    /// Send the next raw packet. Coverage advances AFTER successful queue push, never
    /// merely after raw admission or packet preparation. This does not own retries.
    ///
    /// # Errors
    /// FIFO backpressure or changed raw binding; the exact packet is returned.
    pub fn send<P: MixedTransferProducer<LoopTransferPacket>>(
        &mut self,
        raw: &InputCaptureControl,
        queue: &mut P,
        packet: LoopTransferPacket,
    ) -> Result<(), MixedRawSendRefusal> {
        for (source, clock) in self.sources.iter().zip(self.clocks) {
            if raw.input(*source).and_then(|input| input.clock()) != Some(clock) {
                return Err(MixedRawSendRefusal::Invalid {
                    packet,
                    error: InputError::Stale,
                });
            }
        }
        let frontier = if let Some((id, InputObservation::Frontier { tick }, clock)) =
            raw.packet_observation(&packet)
        {
            let Some(source) = self
                .sources
                .iter()
                .position(|source| *source == id.generation())
            else {
                return Err(MixedRawSendRefusal::Invalid {
                    packet,
                    error: InputError::Stale,
                });
            };
            let at = match clock.map(tick) {
                Ok(at) => at,
                Err(error) => return Err(MixedRawSendRefusal::Invalid { packet, error }),
            };
            Some((source, at))
        } else {
            None
        };
        queue.try_push(packet).map_err(MixedRawSendRefusal::Full)?;
        if let Some((source, at)) = frontier
            && let Some(frontier) = self.frontiers.get_mut(source)
        {
            *frontier = at;
        }
        Ok(())
    }
    /// Both sources' actually queued complete prefix, independent of the sound arm.
    pub fn publication(&self) -> MixedRawPublication {
        MixedRawPublication {
            sources: self.sources,
            through: self.frontiers[0].min(self.frontiers[1]),
            interrupted: false,
        }
    }
    /// Carry a real classified raw-only interruption to the callback without
    /// inventing further queued raw coverage. Sound still requires its own proof.
    ///
    /// # Errors
    /// Raw has not halted late, or the policy belongs to different sources.
    pub fn interrupted_publication(
        &self,
        raw: &InputCaptureControl,
        policy: &MixedRawPolicy,
    ) -> Result<MixedRawPublication, InputError> {
        if !raw.is_halted()
            || policy.sources != self.sources
            || policy.decision() != Some(MixedRawHaltClass::Late)
            || self
                .sources
                .iter()
                .any(|source| raw.input(*source).is_none())
        {
            return Err(InputError::State);
        }
        Ok(MixedRawPublication {
            interrupted: true,
            ..self.publication()
        })
    }
}

/// Scope of an actual collected raw outcome. Terminal always dominates late,
/// irrespective of completion order. The host retains the original receipt/error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedRawHaltClass {
    Late,
    Terminal,
}
impl MixedRawHaltClass {
    /// Combine classifications without ever reviving a terminal host.
    #[must_use]
    pub const fn combine(self, other: Self) -> Self {
        if matches!(self, Self::Terminal) || matches!(other, Self::Terminal) {
            Self::Terminal
        } else {
            Self::Late
        }
    }
}
/// Classify a retained raw receipt, not an admission attempt or an inferred cause.
pub fn classify_raw_outcome(outcome: &InputOutcome) -> Option<MixedRawHaltClass> {
    match outcome {
        InputOutcome::Delivered(SessionSourceOutcome::Published(published))
            if published.capture == CaptureDisposition::Late =>
        {
            Some(MixedRawHaltClass::Late)
        }
        InputOutcome::Delivered(SessionSourceOutcome::Refused(NoteCaptureError::PastBoundary))
        | InputOutcome::Refused(LoopSessionError::Session(SessionError::SourceOrder)) => {
            Some(MixedRawHaltClass::Late)
        }
        InputOutcome::Delivered(SessionSourceOutcome::Published(published))
            if matches!(published.capture, CaptureDisposition::Stopped(_)) =>
        {
            Some(MixedRawHaltClass::Terminal)
        }
        InputOutcome::Delivered(SessionSourceOutcome::Refused(_)) | InputOutcome::Refused(_) => {
            Some(MixedRawHaltClass::Terminal)
        }
        _ => None,
    }
}

/// Monotone classification, populated only from collected outcomes. A render or
/// preparation failure must additionally call `terminal_failure`; those causes
/// override even an earlier classified late receipt.
#[derive(Debug)]
#[must_use]
pub struct MixedRawPolicy {
    sources: [ConnectionGeneration; 2],
    decision: Option<MixedRawHaltClass>,
}
impl MixedRawPolicy {
    /// Bind outcome classification to this raw owner's distinct source generations.
    ///
    /// # Errors
    /// A missing or unconfigured raw source.
    pub fn new(
        raw: &InputCaptureControl,
        sources: MixedTransferSources,
    ) -> Result<Self, InputError> {
        let sources = [sources.first(), sources.second()];
        if sources
            .iter()
            .any(|source| raw.input(*source).and_then(|input| input.clock()).is_none())
        {
            return Err(InputError::Attachment);
        }
        Ok(Self {
            sources,
            decision: None,
        })
    }
    /// The decisive outcome class observed so far; the host owns exact diagnostics.
    pub const fn decision(&self) -> Option<MixedRawHaltClass> {
        self.decision
    }
    /// Observe the exact outcome before the host stores its original receipt.
    ///
    /// # Errors
    /// A foreign outcome source. It makes the classification terminal rather than
    /// allowing another capture owner's late result to authorize raw-only routing.
    pub fn observe(
        &mut self,
        source: ConnectionGeneration,
        outcome: &InputOutcome,
    ) -> Result<(), InputError> {
        if !self.sources.contains(&source) {
            self.terminal_failure();
            return Err(InputError::Stale);
        }
        if let Some(class) = classify_raw_outcome(outcome) {
            self.decision = Some(self.decision.map_or(class, |prior| prior.combine(class)));
        }
        Ok(())
    }
    /// A raw preparation/render error, or a self-halt with no decisive receipt.
    pub fn terminal_failure(&mut self) {
        self.decision = Some(MixedRawHaltClass::Terminal);
    }
}

/// Sound handling for the original source-ring identity, independent of raw IDs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MixedRoutedSound {
    Message(Result<MixedStaged, MixedBridgeError>),
    Frontier(Result<(), MixedBridgeError>),
}
/// Exactly what raw admitted before sound received the same observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedRawAdmission {
    id: InputEventId,
    matched_onset: Option<InputEventId>,
}
impl MixedRawAdmission {
    pub const fn id(self) -> InputEventId {
        self.id
    }
    pub const fn matched_onset(self) -> Option<InputEventId> {
        self.matched_onset
    }
}
/// A consumed route. `raw == None` means a classified raw-only interruption;
/// sound still receives the original packet, including subsequent exact key-ups.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct MixedInputReceipt {
    pub raw: Option<MixedRawAdmission>,
    pub sound: MixedRoutedSound,
}

/// A terminal route refusal retains the original observation. A host records it,
/// calls `MixedRawPolicy::terminal_failure` before collecting later receipts, and
/// requests its shared/raw halts. `Halted` leaves the source-ring head uncommitted.
#[derive(Debug, Error)]
pub enum MixedInputRefusal {
    #[error("raw admission refused {observation:?}: {error}; {fault}")]
    Raw {
        observation: InputObservation,
        error: InputError,
        fault: MixedBridgeError,
    },
    #[error("raw metadata for admitted {id:?} refused: {error}; {fault}")]
    Metadata {
        observation: InputObservation,
        id: InputEventId,
        error: InputError,
        fault: MixedBridgeError,
    },
    #[error("raw owner halted without a raw-only late classification: {fault}")]
    Halted {
        observation: InputObservation,
        fault: MixedBridgeError,
    },
}

// A private endpoint keeps the real offer/then-halt sampling order testable without
// scheduler races or callback hooks. Production uses the capture owner's own methods.
trait RawOfferEndpoint {
    fn offer(
        &mut self,
        source: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<InputEventId, (InputObservation, InputError)>;
    fn halted(&self) -> bool;
}
impl RawOfferEndpoint for InputCaptureControl {
    fn offer(
        &mut self,
        source: ConnectionGeneration,
        observation: InputObservation,
    ) -> Result<InputEventId, (InputObservation, InputError)> {
        self.offer_observation(source, observation)
    }
    fn halted(&self) -> bool {
        self.is_halted()
    }
}
fn offer_raw(
    raw: &mut impl RawOfferEndpoint,
    source: ConnectionGeneration,
    observation: InputObservation,
    decision: Option<MixedRawHaltClass>,
) -> Result<Option<InputEventId>, RawOfferRefusal> {
    let offered = raw.offer(source, observation);
    classify_raw_offer(offered, raw.halted(), decision)
}

/// Classify the offer result with the halt sampled AFTER the offer. A State
/// refusal caused by audio closing raw must never consume or blame a source head.
#[derive(Debug, PartialEq)]
enum RawOfferRefusal {
    SelfHalt(InputObservation),
    Admission(InputObservation, InputError),
}
fn classify_raw_offer(
    offered: Result<InputEventId, (InputObservation, InputError)>,
    halted_after_offer: bool,
    decision: Option<MixedRawHaltClass>,
) -> Result<Option<InputEventId>, RawOfferRefusal> {
    match offered {
        Ok(id) => Ok(Some(id)),
        Err((observation, InputError::State)) if halted_after_offer => {
            if decision == Some(MixedRawHaltClass::Late) {
                Ok(None)
            } else {
                Err(RawOfferRefusal::SelfHalt(observation))
            }
        }
        Err((observation, error)) => Err(RawOfferRefusal::Admission(observation, error)),
    }
}

impl MixedBridgeControl {
    /// Consume one ordered source-ring observation through the shared raw-first law.
    /// Collect and classify available raw outcomes before calling this. While raw
    /// runs, its admission completes before sound staging. A classified late raw
    /// interruption skips raw for BOTH sources and keeps sound's own FIFO alive.
    /// Sound-stage shortages are retained per-occurrence outcomes, never raw faults.
    /// This method runs off the callback; the host retains all returned diagnostics.
    ///
    /// # Errors
    /// Raw admission/metadata failure or an unclassified/terminal raw halt. Sound
    /// receives no observation on that path, and the bridge publishes its terminal
    /// signal. The original and any already-admitted raw ID remain in the refusal.
    pub fn route_raw_first(
        &mut self,
        raw: &mut InputCaptureControl,
        policy: &MixedRawPolicy,
        packet: MixedSourcePacketId,
        observation: InputObservation,
    ) -> Result<MixedInputReceipt, Box<MixedInputRefusal>> {
        if let Some(fault) = self.fault() {
            return Err(Box::new(MixedInputRefusal::Halted {
                observation,
                fault: MixedBridgeError::Halted(fault),
            }));
        }
        if policy.sources.iter().any(|source| {
            self.lane_charge(*source).is_none()
                || raw.input(*source).and_then(|input| input.clock()).is_none()
        }) {
            return Err(Box::new(MixedInputRefusal::Halted {
                observation,
                fault: self.report_raw_self_halt(),
            }));
        }
        if policy.decision() == Some(MixedRawHaltClass::Terminal)
            || (raw.is_halted() && policy.decision() != Some(MixedRawHaltClass::Late))
        {
            return Err(Box::new(MixedInputRefusal::Halted {
                observation,
                fault: self.report_raw_self_halt(),
            }));
        }
        let source = packet.connection();
        if self.lane_charge(source).is_none() {
            let error = InputError::Stale;
            return Err(Box::new(MixedInputRefusal::Raw {
                observation,
                error,
                fault: self.report_source_fault(source, MixedSourceFault::RawAdmission(error)),
            }));
        }
        let at = raw
            .input(source)
            .and_then(|input| input.clock())
            .ok_or(InputError::Stale)
            .and_then(|clock| {
                let tick = match observation {
                    InputObservation::Message { tick, .. }
                    | InputObservation::Frontier { tick } => tick,
                };
                clock.map(tick)
            })
            .map_err(|error| {
                Box::new(MixedInputRefusal::Raw {
                    observation,
                    error,
                    fault: self.report_source_fault(source, MixedSourceFault::RawAdmission(error)),
                })
            })?;
        let admission = if raw.is_halted() && policy.decision() == Some(MixedRawHaltClass::Late) {
            None
        } else {
            match offer_raw(raw, source, observation, policy.decision()) {
                Ok(Some(id)) => Some(MixedRawAdmission {
                    id,
                    matched_onset: raw.matched_onset(id).map_err(|error| {
                        Box::new(MixedInputRefusal::Metadata {
                            observation,
                            id,
                            error,
                            fault: self
                                .report_source_fault(source, MixedSourceFault::RawAdmission(error)),
                        })
                    })?,
                }),
                Ok(None) => None,
                Err(RawOfferRefusal::SelfHalt(observation)) => {
                    return Err(Box::new(MixedInputRefusal::Halted {
                        observation,
                        fault: self.report_raw_self_halt(),
                    }));
                }
                Err(RawOfferRefusal::Admission(observation, error)) => {
                    return Err(Box::new(MixedInputRefusal::Raw {
                        observation,
                        error,
                        fault: self
                            .report_source_fault(source, MixedSourceFault::RawAdmission(error)),
                    }));
                }
            }
        };
        let sound = match observation {
            InputObservation::Message { input, .. } => {
                MixedRoutedSound::Message(self.stage(MixedSourceHandoff { packet, at, input }))
            }
            InputObservation::Frontier { .. } => {
                MixedRoutedSound::Frontier(self.advance_frontier(packet, at))
            }
        };
        Ok(MixedInputReceipt {
            raw: admission,
            sound,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::input::InputTick;

    #[test]
    fn a_raw_halt_racing_the_offer_is_source_neutral_until_classified_late() {
        let observation = InputObservation::Frontier {
            tick: InputTick::new(128),
        };
        // Inject the exact post-offer state of an audio self-halt after preflight.
        // No scheduler timing is needed: State and the now-published halt are the
        // observable boundary, while the decisive receipt can still be in flight.
        for decision in [None, Some(MixedRawHaltClass::Terminal)] {
            assert_eq!(
                classify_raw_offer(Err((observation, InputError::State)), true, decision),
                Err(RawOfferRefusal::SelfHalt(observation)),
            );
        }
        assert_eq!(
            classify_raw_offer(
                Err((observation, InputError::State)),
                true,
                Some(MixedRawHaltClass::Late)
            ),
            Ok(None),
        );
        assert_eq!(
            classify_raw_offer(Err((observation, InputError::State)), false, None),
            Err(RawOfferRefusal::Admission(observation, InputError::State)),
        );
    }
    #[test]
    fn the_routing_offer_samples_halt_after_the_capture_can_close() {
        struct ClosingRaw {
            halted: bool,
        }
        impl RawOfferEndpoint for ClosingRaw {
            fn offer(
                &mut self,
                _source: ConnectionGeneration,
                observation: InputObservation,
            ) -> Result<InputEventId, (InputObservation, InputError)> {
                assert!(!self.halted, "raw ran before this offer");
                self.halted = true;
                Err((observation, InputError::State))
            }
            fn halted(&self) -> bool {
                self.halted
            }
        }
        let source = crate::host::issue_capture_source_generation().expect("source");
        let observation = InputObservation::Frontier {
            tick: InputTick::new(128),
        };
        for decision in [
            None,
            Some(MixedRawHaltClass::Terminal),
            Some(MixedRawHaltClass::Late),
        ] {
            let mut raw = ClosingRaw { halted: false };
            let expected = if decision == Some(MixedRawHaltClass::Late) {
                Ok(None)
            } else {
                Err(RawOfferRefusal::SelfHalt(observation))
            };
            assert_eq!(offer_raw(&mut raw, source, observation, decision), expected);
        }
    }
}
