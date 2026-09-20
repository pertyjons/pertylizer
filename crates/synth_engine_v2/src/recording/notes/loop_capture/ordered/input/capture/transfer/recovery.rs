//! Off-thread preparation and owning reunion, including deferred quality attribution.
use super::*;
use thiserror::Error;

#[derive(Error)]
#[error("input split refused: {error}")]
pub struct InputSplitError {
    session: InputCaptureSession,
    error: InputCaptureError,
}
impl std::fmt::Debug for InputSplitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSplitError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl InputSplitError {
    pub const fn error(&self) -> &InputCaptureError {
        &self.error
    }
    #[must_use = "The returned session retains input and recording custody"]
    pub fn into_parts(self) -> (InputCaptureSession, InputCaptureError) {
        (self.session, self.error)
    }
}

enum RetainedOwners {
    Split {
        control: InputCaptureControl,
        audio: InputCaptureAudio,
    },
    Reunited(InputCaptureSession),
}

/// A quality reconciliation failure keeps its serial owner opaque until retry succeeds.
#[derive(Error)]
#[error("input reunion refused: {error}")]
pub struct InputReuniteError {
    owners: RetainedOwners,
    error: InputCaptureError,
}
impl std::fmt::Debug for InputReuniteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputReuniteError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
impl InputReuniteError {
    pub const fn error(&self) -> &InputCaptureError {
        &self.error
    }
    /// Recover split owners to resolve outstanding transfers. A joined quality
    /// refusal instead stays opaque; retry must reconcile it before result access.
    pub fn into_split(self) -> Result<(InputCaptureControl, InputCaptureAudio), Box<Self>> {
        match self.owners {
            RetainedOwners::Split { control, audio } => Ok((control, audio)),
            RetainedOwners::Reunited(_) => Err(Box::new(self)),
        }
    }
    pub fn retry(self) -> Result<InputCaptureSession, Box<Self>> {
        match self.owners {
            RetainedOwners::Split { control, audio } => audio.reunite(control),
            RetainedOwners::Reunited(session) => finish_reunion(session),
        }
    }
}

impl InputCaptureSession {
    /// Split only a fresh composition with Ready inputs. Concrete queue backing
    /// is separately budgeted and retained by the host through callback join.
    pub fn split(
        self,
        budget: PreparedBytes,
    ) -> Result<(InputCaptureControl, InputCaptureAudio, InputCaptureHalt), Box<InputSplitError>>
    {
        if let Err(error) = Self::validate(&self.session, &self.inputs, self.bytes) {
            return Err(Box::new(InputSplitError {
                session: self,
                error,
            }));
        }
        // Conservative wrapper charge in addition to existing input/composition
        // storage. 256 bytes cover the atomic payload, Arc counters and padding.
        let extra = u64::try_from(
            size_of::<InputCaptureControl>()
                + size_of::<InputCaptureAudio>()
                + size_of::<InputCaptureHalt>()
                - size_of::<LoopRecordingControl>()
                - size_of::<LoopRecordingAudio>()
                + 256,
        );
        let Ok(extra) = extra else {
            return Err(Box::new(InputSplitError {
                session: self,
                error: InputError::Layout.into(),
            }));
        };
        let Some(core_budget) = budget.get().checked_sub(extra) else {
            return Err(Box::new(InputSplitError {
                session: self,
                error: InputError::ByteBudget {
                    required: PreparedBytes::measured(extra),
                    available: budget,
                }
                .into(),
            }));
        };
        let (core, audio) = match self.session.split(PreparedBytes::measured(core_budget)) {
            Ok(owners) => owners,
            Err(error) => {
                let (session, error) = error.into_parts();
                return Err(Box::new(InputSplitError {
                    session: Self { session, ..self },
                    error: error.into(),
                }));
            }
        };
        // The core admitted within budget-extra, so this sum cannot overflow.
        let bytes = PreparedBytes::measured(core.transfer_bytes().get() + extra);
        let halt = InputCaptureHalt {
            signal: Arc::new(AtomicU8::new(0)),
        };
        Ok((
            InputCaptureControl {
                core,
                inputs: self.inputs,
                halt: halt.clone(),
                closed: false,
                serial_bytes: self.bytes,
                bytes,
            },
            InputCaptureAudio {
                core: audio,
                halt: halt.clone(),
            },
            halt,
        ))
    }
}

impl InputCaptureAudio {
    /// Call only after callback access joins. Resolve packets/completions first;
    /// source quiescence remains a separate acknowledgement after reunion.
    pub fn reunite(
        mut self,
        control: InputCaptureControl,
    ) -> Result<InputCaptureSession, Box<InputReuniteError>> {
        if let Err(error) = self.synchronize_halt() {
            return Err(Box::new(InputReuniteError {
                owners: RetainedOwners::Split {
                    control,
                    audio: self,
                },
                error: error.into(),
            }));
        }
        match self.core.reunite(control.core) {
            Ok(session) => finish_reunion(InputCaptureSession {
                session,
                inputs: control.inputs,
                closed: false,
                bytes: control.serial_bytes,
            }),
            Err(error) => {
                let (core, audio, error) = error.into_parts();
                Err(Box::new(InputReuniteError {
                    owners: RetainedOwners::Split {
                        control: InputCaptureControl { core, ..control },
                        audio: Self {
                            core: audio,
                            ..self
                        },
                    },
                    error: error.into(),
                }))
            }
        }
    }
}

fn finish_reunion(
    mut session: InputCaptureSession,
) -> Result<InputCaptureSession, Box<InputReuniteError>> {
    // Do not close the composition until its retained diagnostic has been applied:
    // synchronize_failure attributes quality before setting the closed flag.
    if let Err(error) = session.synchronize_failure() {
        return Err(Box::new(InputReuniteError {
            owners: RetainedOwners::Reunited(session),
            error,
        }));
    }
    if session.session.commands.closed {
        session.close_inputs();
    }
    Ok(session)
}
