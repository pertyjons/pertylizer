//! A fixed initial live-source set for a two-source MIDI1 mixed adapter.
//!
//! The raw connection generation is the stable identity at this boundary.
//! The bound ordinals are stream-local routing keys; they never identify a
//! returning raw connection. Each raw ID resolves through its generation.

use thiserror::Error;

use crate::host::ConnectionGeneration;

/// One of the two sources admitted for an initial mixed MIDI1 stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(super) enum MixedBoundSource {
    First,
    Second,
}

impl MixedBoundSource {
    /// The private mixed occurrence table's logical source identifier.
    pub(super) const fn ordinal(self) -> u64 {
        match self {
            Self::First => 1,
            Self::Second => 2,
        }
    }
}

/// Why a raw connection cannot be admitted to the bound source set.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(super) enum MixedSourceSetError {
    #[error("the two mixed source connections must be distinct")]
    Duplicate,
    #[error("the raw connection is outside the mixed source set")]
    Unbound,
}

/// The two raw connection identities accepted by an initial MIDI1 mixed host.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub(super) struct MixedSourceSet {
    first: ConnectionGeneration,
    second: ConnectionGeneration,
}

impl MixedSourceSet {
    /// Each admitted source can send a bend on each of MIDI1's 16 channels.
    pub(super) const MAX_SCOPES: usize = 2 * 16;

    pub(super) fn prepare(
        first: ConnectionGeneration,
        second: ConnectionGeneration,
    ) -> Result<Self, MixedSourceSetError> {
        if first == second {
            return Err(MixedSourceSetError::Duplicate);
        }
        Ok(Self { first, second })
    }

    pub(super) const fn generation(self, source: MixedBoundSource) -> ConnectionGeneration {
        match source {
            MixedBoundSource::First => self.first,
            MixedBoundSource::Second => self.second,
        }
    }

    pub(super) fn resolve(
        self,
        generation: ConnectionGeneration,
    ) -> Result<MixedBoundSource, MixedSourceSetError> {
        if generation == self.first {
            Ok(MixedBoundSource::First)
        } else if generation == self.second {
            Ok(MixedBoundSource::Second)
        } else {
            Err(MixedSourceSetError::Unbound)
        }
    }
}

#[cfg(all(test, feature = "simulated-ingress"))]
mod tests {
    use super::*;
    use crate::host::issue_capture_source_generation;

    #[test]
    fn source_binding_rejects_duplicates_and_an_outsider() {
        let first = issue_capture_source_generation().unwrap();
        let second = issue_capture_source_generation().unwrap();
        let outsider = issue_capture_source_generation().unwrap();
        assert_eq!(
            MixedSourceSet::prepare(first, first).unwrap_err(),
            MixedSourceSetError::Duplicate
        );
        let bound = MixedSourceSet::prepare(first, second).unwrap();
        assert_eq!(bound.resolve(first), Ok(MixedBoundSource::First));
        assert_eq!(bound.resolve(second), Ok(MixedBoundSource::Second));
        assert_eq!(bound.resolve(outsider), Err(MixedSourceSetError::Unbound));
        assert_eq!(bound.generation(MixedBoundSource::First), first);
        assert_eq!(bound.generation(MixedBoundSource::Second), second);
        assert_eq!(MixedSourceSet::MAX_SCOPES, 32);
    }
}
