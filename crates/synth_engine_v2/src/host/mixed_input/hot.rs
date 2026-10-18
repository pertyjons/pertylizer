//! Immutable raw proof access and internal atomic-copy reconstruction.
use super::{ConnectionGeneration, MixedRawPublication, SampleTime};

impl MixedRawPublication {
    pub(crate) const fn from_mailbox(
        sources: [ConnectionGeneration; 2],
        through: SampleTime,
        interrupted: bool,
    ) -> Self {
        Self {
            sources,
            through,
            interrupted,
        }
    }
    /// A classified late interruption exempts raw coverage, never sound coverage.
    pub const fn is_interrupted(self) -> bool {
        self.interrupted
    }
    pub(crate) fn ready_through(self, sound: SampleTime) -> SampleTime {
        if self.interrupted {
            sound
        } else {
            sound.min(self.through)
        }
    }
    pub const fn through(self) -> SampleTime {
        self.through
    }
    pub(crate) const fn sources(self) -> [ConnectionGeneration; 2] {
        self.sources
    }
}
