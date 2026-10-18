//! Copy-only publication errors; formatting is never called by preflight.
use crate::time::{SampleTime, TimeError};
use thiserror::Error;

/// Why an update was ignored. Rejection does not replace the previous certificate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedPublicationRefusal {
    /// Another bridge or transfer arm issued the certificate.
    #[error("publication belongs to another bridge or active arm")]
    Foreign,
    /// The complete paired sound/raw proof would regress its ready prefix.
    #[error("publication would regress the ready prefix")]
    Regressed,
}

/// A strict host's first missing-prefix reason, retained after a later refill.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedPublicationFault {
    /// No certificate valid for the active transfer arm was available.
    #[error("no publication for the active mixed arm")]
    MissingArm,
    /// A callback would open a quantum beyond the complete sound/raw prefix.
    #[error("publication reaches {ready}, callback requires {required}")]
    Unprepared {
        ready: SampleTime,
        required: SampleTime,
    },
    /// Clock arithmetic exhausted its finite representation.
    #[error(transparent)]
    Clock(#[from] TimeError),
}
