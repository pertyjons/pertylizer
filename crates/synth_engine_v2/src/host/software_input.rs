//! Logical software connections for the experimental Arrival-stamped mixed route.
//! No native endpoint or timestamp mapping is opened or inferred here.
use super::{HostError, NEXT_GENERATION, issue_generation};
use crate::stream::{MixedTransferSources, MixedTransferSourcesError};
use std::sync::atomic::AtomicU64;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SoftwareInputError {
    #[error(transparent)]
    Host(#[from] HostError),
    #[error(transparent)]
    Sources(#[from] MixedTransferSourcesError),
}

/// Retain these logical connections until their source releases and owner teardown
/// have settled. Reopening always receives new process-local generations. They
/// authorize software Arrival provenance, never hardware-clock provenance.
#[derive(Debug)]
#[must_use]
pub struct SoftwareInputPair {
    sources: MixedTransferSources,
}
impl SoftwareInputPair {
    pub fn open() -> Result<Self, SoftwareInputError> {
        Self::open_with_counter(&NEXT_GENERATION)
    }
    fn open_with_counter(counter: &AtomicU64) -> Result<Self, SoftwareInputError> {
        let first = issue_generation(counter)?;
        let second = issue_generation(counter)?;
        Ok(Self {
            sources: MixedTransferSources::new(first, second)?,
        })
    }
    pub const fn sources(&self) -> MixedTransferSources {
        self.sources
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    #[test]
    fn software_connections_are_distinct_and_reopening_never_reuses_them() {
        let issuer = AtomicU64::new(0);
        let first = SoftwareInputPair::open_with_counter(&issuer).unwrap();
        let second = SoftwareInputPair::open_with_counter(&issuer).unwrap();
        let sources = [
            first.sources.first(),
            first.sources.second(),
            second.sources.first(),
            second.sources.second(),
        ];
        for (index, source) in sources.iter().enumerate() {
            assert!(!sources[..index].contains(source));
        }
    }
    #[test]
    fn a_partial_pair_at_exhaustion_returns_no_owner_and_cannot_reuse_a_generation() {
        let issuer = AtomicU64::new(u64::MAX - 1);
        for _ in 0..2 {
            assert!(matches!(
                SoftwareInputPair::open_with_counter(&issuer),
                Err(SoftwareInputError::Host(HostError::GenerationExhausted))
            ));
            assert_eq!(issuer.load(Ordering::Relaxed), u64::MAX);
        }
    }
}
