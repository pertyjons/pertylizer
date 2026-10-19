//! Borrowed declarations read by the bounded mixed producers.
use super::{InternalProducerDeclaration, RuntimeParameterEntry, RuntimeParameterProgram};
use crate::{
    quantities::{EventCount, ParameterValue},
    time::FrameCount,
};

impl RuntimeParameterEntry {
    pub const fn phase(self) -> FrameCount {
        self.phase
    }
    pub const fn value(self) -> ParameterValue {
        self.value
    }
}
impl RuntimeParameterProgram {
    pub const fn period(&self) -> FrameCount {
        self.period
    }
    pub fn entries(&self) -> &[RuntimeParameterEntry] {
        &self.entries
    }
    /// Includes a fresh owner's extra latest-value burst; holds and retained future are zero.
    pub const fn destination_occupancy(&self) -> EventCount {
        self.bound
    }
}
impl InternalProducerDeclaration {
    pub const fn period(self) -> FrameCount {
        self.period
    }
    pub const fn phase(self) -> FrameCount {
        self.phase
    }
}
