//! The executable host boundary: reservation precedes activation and source closure
//! precedes acknowledgement. Low-level queue helpers do not confer either authority.
use super::*;
use super::{
    archive::RetainedRuns,
    prepare::PreparedAttempt,
    source::{SourceInbox, SourceProducer, SourceQueueId},
};

#[must_use]
pub struct ManagedRun {
    control: LiveControl,
    inboxes: [SourceInbox; 2],
    generations: [ConnectionGeneration; 2],
}

pub struct UnpublishedSound {
    pub audition: Option<(
        super::audition::AuditionControl,
        super::audition::AuditionAudio,
    )>,
    pub metronome: Option<super::metronome::MetronomeAudio>,
}
impl UnpublishedSound {
    fn describe(&self) -> String {
        format!(
            "audition={}, metronome={}",
            self.audition.is_some(),
            self.metronome.is_some()
        )
    }
}
pub enum StartOwners {
    Prepared(Box<PreparedAttempt>),
    Split(
        Box<(InputCaptureControl, InputCaptureAudio, InputCaptureHalt)>,
        Box<UnpublishedSound>,
    ),
    Core(
        Box<synth_engine_v2::host::input::InputSplitError>,
        Box<UnpublishedSound>,
    ),
    Host(Box<(LiveControl, LiveAudio)>),
}
#[derive(Error)]
#[error("managed attempt preparation refused: {message}")]
pub struct StartFailure {
    message: String,
    pub owners: StartOwners,
}
impl std::fmt::Debug for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let owner = match &self.owners {
            StartOwners::Prepared(prepared) => format!("prepared {:?}", prepared.epoch()),
            StartOwners::Split(owners, sound) => {
                let (control, audio, halt) = &**owners;
                // All three remain owned; inspect neither as an acknowledgement.
                let _retained_signal = halt;
                format!(
                    "split {:?}, {} bytes, {}",
                    audio.acknowledged().epoch,
                    control.bytes().get(),
                    sound.describe()
                )
            }
            StartOwners::Core(error, sound) => format!(
                "core split refusal: {}, {}",
                error.error(),
                sound.describe()
            ),
            StartOwners::Host(owners) => format!("host split at {:?}", owners.1.clock()),
        };
        f.debug_struct("StartFailure")
            .field("message", &self.message)
            .field("owner", &owner)
            .finish()
    }
}

pub enum FinishFailure {
    Pending(Box<(ManagedRun, LiveAudio)>, HostError),
    Reunion(ReuniteError),
    Retained(Box<InputCaptureSession>, String),
}
impl std::fmt::Debug for FinishFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending(owners, error) => f
                .debug_tuple("Pending")
                .field(&owners.1.clock())
                .field(error)
                .finish(),
            Self::Reunion(ReuniteError::Pending(owners)) => f
                .debug_tuple("ReunionPending")
                .field(&owners.1.clock())
                .finish(),
            Self::Reunion(ReuniteError::Core(error, _audition, _metronome)) => {
                f.debug_tuple("ReunionCore").field(error).finish()
            }
            Self::Reunion(ReuniteError::Audition(owner, audition, _metronome, error)) => f
                .debug_tuple("AuditionReunion")
                .field(&owner.acknowledged())
                .field(&audition.clock())
                .field(error)
                .finish(),
            Self::Retained(owner, error) => f
                .debug_tuple("Retained")
                .field(&owner.acknowledged())
                .field(error)
                .finish(),
        }
    }
}

impl ManagedRun {
    pub fn publish_live_plan(
        &mut self,
        graph: &synth_engine_v2::ir::GraphIr,
    ) -> Result<synth_engine_v2::plan::PlanId, HostError> {
        let audition = self
            .control
            .audition
            .as_mut()
            .ok_or(synth_engine_v2::host::live::LiveInputError::Configuration)?;
        Ok(audition.swaps.publish(graph)?)
    }
    pub fn collect_live_plans(&mut self) -> usize {
        self.control
            .audition
            .as_mut()
            .map_or(0, |audition| audition.swaps.collect())
    }

    pub fn start(
        archive: &mut RetainedRuns,
        prepared: PreparedAttempt,
    ) -> Result<(Self, LiveAudio, [SourceProducer; 2]), Box<StartFailure>> {
        if let Err(error) = archive.admit(&prepared) {
            return Err(Box::new(StartFailure {
                message: error.to_string(),
                owners: StartOwners::Prepared(Box::new(prepared)),
            }));
        }
        let epoch = prepared.epoch();
        let generations = prepared.generations;
        let (mut control, audio, halt) = match prepared.owner.split(PreparedBytes::measured(65536))
        {
            Ok(owners) => owners,
            Err(error) => {
                let message = match archive.cancel_preparation(epoch) {
                    Ok(()) => error.to_string(),
                    Err(cancel) => format!("{error}; reservation cancellation: {cancel}"),
                };
                return Err(Box::new(StartFailure {
                    message,
                    owners: StartOwners::Core(
                        error,
                        Box::new(UnpublishedSound {
                            audition: prepared.audition,
                            metronome: prepared.metronome,
                        }),
                    ),
                }));
            }
        };
        for generation in generations {
            if let Err(error) = control.start_input(generation) {
                let message = match archive.cancel_preparation(epoch) {
                    Ok(()) => error.to_string(),
                    Err(cancel) => format!("{error}; reservation cancellation: {cancel}"),
                };
                return Err(Box::new(StartFailure {
                    message,
                    owners: StartOwners::Split(
                        Box::new((control, audio, halt)),
                        Box::new(UnpublishedSound {
                            audition: prepared.audition,
                            metronome: prepared.metronome,
                        }),
                    ),
                }));
            }
        }
        let (mut control, mut audio) = LiveControl::attach(control, audio, halt);
        audio.metronome = prepared.metronome;
        if let Some((live_control, live_audio)) = prepared.audition {
            control.audition = Some(live_control);
            audio.audition = Some(live_audio);
        }
        if let Err(error) = control.pump() {
            let message = match archive.cancel_preparation(epoch) {
                Ok(()) => error.to_string(),
                Err(cancel) => format!("{error}; reservation cancellation: {cancel}"),
            };
            return Err(Box::new(StartFailure {
                message,
                owners: StartOwners::Host(Box::new((control, audio))),
            }));
        }
        let (first, first_inbox) =
            SourceInbox::prepare(generations[0], control.halt_handle(), prepared.clocks[0]);
        let (second, second_inbox) =
            SourceInbox::prepare(generations[1], control.halt_handle(), prepared.clocks[1]);
        Ok((
            Self {
                control,
                inboxes: [first_inbox, second_inbox],
                generations,
            },
            audio,
            [first, second],
        ))
    }

    pub fn command(
        &mut self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<LoopTransferId, HostError> {
        self.control.command(at, command)
    }
    pub fn halt_handle(&self) -> InputCaptureHalt {
        self.control.halt_handle()
    }

    #[cfg(test)]
    pub(crate) fn source_discontinuity(
        &self,
        port: usize,
    ) -> Option<synth_engine_v2::host::input::InputDiscontinuity> {
        let generation = *self.generations.get(port)?;
        self.control.core.input(generation)?.discontinuity()
    }

    /// Admission and execution have distinct identified outcomes. Both are reported.
    #[cfg(test)]
    pub fn service(
        &mut self,
        mut input: impl FnMut(InputOfferResult),
        command: impl FnMut(LoopTransferId, HostOutcome),
        receipt: impl FnMut(InputReceipt),
    ) -> Result<(), HostError> {
        self.service_identified(|_, result| input(result), command, receipt)
    }

    /// The queue ID identifies an accepted source occurrence before raw admission.
    /// Pre-ring terminal refusals have no queue ID.
    pub fn service_identified(
        &mut self,
        mut input: impl FnMut(Option<SourceQueueId>, InputOfferResult),
        mut command: impl FnMut(LoopTransferId, HostOutcome),
        mut receipt: impl FnMut(InputReceipt),
    ) -> Result<(), HostError> {
        for inbox in &mut self.inboxes {
            inbox.record_failure(&mut self.control, |result| input(None, result));
        }
        for inbox in &mut self.inboxes {
            inbox.service_identified(&mut self.control, &mut input);
        }
        while self.control.has_completions() {
            if let Some((id, outcome)) = self.control.collect()? {
                command(id, outcome);
            }
        }
        for generation in self.generations {
            while let Some(outcome) = self
                .control
                .collect_input(generation)
                .map_err(InputCaptureError::from)?
            {
                receipt(outcome);
            }
        }
        self.control.pump()
    }

    pub fn collect_audition(
        &mut self,
    ) -> Option<(
        synth_engine_v2::host::live::AuditionId,
        synth_engine_v2::host::live::AuditionOutcome,
    )> {
        self.control.collect_audition()
    }

    /// The producer's unique endpoint can return only after its producer has stopped.
    /// Refuse closure while its ring or terminal failure awaits service.
    pub fn close_source(&mut self, producer: SourceProducer) -> Result<(), SourceProducer> {
        let mut producer = producer;
        for inbox in &mut self.inboxes {
            match inbox.close(producer) {
                Ok(()) => return Ok(()),
                Err(value) => producer = value,
            }
        }
        Err(producer)
    }

    /// The caller has joined backend callback access before passing its audio owner.
    /// A failed finish retains unresolved core ownership and its archive reservation.
    /// Empty, closed producer endpoints need no further acknowledgement after reunion.
    pub fn finish(
        mut self,
        mut audio: LiveAudio,
        interrupted: bool,
        archive: &mut RetainedRuns,
        mut command: impl FnMut(LoopTransferId, HostOutcome),
        mut receipt: impl FnMut(InputReceipt),
        audition: impl FnMut(
            synth_engine_v2::host::live::AuditionId,
            synth_engine_v2::host::live::AuditionOutcome,
        ),
    ) -> Result<(), Box<FinishFailure>> {
        if self.inboxes.iter().any(|inbox| !inbox.is_closed()) {
            return Err(Box::new(FinishFailure::Pending(
                Box::new((self, audio)),
                HostError::SourcesOpen,
            )));
        }
        let finish = if interrupted {
            self.control.recover(&mut audio, &mut command)
        } else {
            self.control.finish_after_join(&mut audio, &mut command)
        };
        if let Err(error) = finish {
            return Err(Box::new(FinishFailure::Pending(
                Box::new((self, audio)),
                error,
            )));
        }
        for generation in self.generations {
            loop {
                match self.control.collect_input(generation) {
                    Ok(Some(outcome)) => receipt(outcome),
                    Ok(None) => break,
                    Err(error) => {
                        return Err(Box::new(FinishFailure::Pending(
                            Box::new((self, audio)),
                            HostError::Input(error.into()),
                        )));
                    }
                }
            }
        }
        let mut owner = self
            .control
            .reunite(audio, audition)
            .map_err(|error| Box::new(FinishFailure::Reunion(error)))?;
        let result = (|| {
            if !interrupted {
                owner.finalize()?;
                owner.close_completed()?;
            }
            for generation in self.generations {
                while let Some(outcome) = owner.collect_input(generation)? {
                    receipt(outcome);
                }
                owner.acknowledge_input_quiescence(generation)?;
            }
            owner.finalize()
        })();
        if let Err(error) = result {
            return Err(Box::new(FinishFailure::Retained(
                Box::new(owner),
                error.to_string(),
            )));
        }
        archive.retain(owner).map_err(|error| {
            let (owner, error) = *error;
            Box::new(FinishFailure::Retained(Box::new(owner), error.to_string()))
        })
    }
}
