//! The bound mixed audio half's quantum-boundary operation.
//! The renderer and ended-note storage are prepared off-thread. This path does
//! not allocate, lock, perform I/O, log or drop an owning candidate.

use super::MixedStreamAudio;
use crate::time::{PlanPosition, SampleTime};

impl MixedStreamAudio {
    /// Move only the compiled musical mapping at the current render boundary.
    /// Scratch was sized from the bound compiled span off-thread. The returned
    /// anchor belongs with the retired compiled list; a later schedule owner
    /// must also publish complete scoped restoration in this same quantum.
    #[allow(dead_code)] // No mixed audio-side scheduler calls this yet.
    pub(crate) fn adopt_compiled_boundary(
        &mut self,
        effective: SampleTime,
        position: PlanPosition,
    ) -> Result<crate::render::MixedBoundaryAdopted, crate::render::MixedBoundaryReleaseError> {
        self.renderer.adopt_mixed_compiled_boundary(
            self.partition.compiled_producer(),
            effective,
            position,
            &mut self.compiled_ended,
        )
    }
}
