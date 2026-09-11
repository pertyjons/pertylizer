//! Required capture configuration under HOST-INV-020. No implicit production defaults.

use super::{ProfileError, nonzero};
use crate::quantities::{
    CapturePassCount, CaptureResultCount, CaptureSourceCount, PreparedBytes, ProjectionTickCount,
    TrackedInputNoteCount,
};
use crate::time::FrameCount;

/// External construction input. Every field must be supplied, even when its first
/// consumer (audio, projection or looping) is not enabled yet. This is not serialized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureLimitsInput {
    pub max_tracked_input_notes: TrackedInputNoteCount,
    pub max_capture_sources: CaptureSourceCount,
    pub max_capture_passes: CapturePassCount,
    pub max_pending_capture_results: CaptureResultCount,
    pub max_capture_bytes: PreparedBytes,
    pub max_audio_capture_frames: FrameCount,
    pub max_projection_ticks: ProjectionTickCount,
    pub capture_lateness_allowance: FrameCount,
}

/// Validated additional fields in `RecordingLimits`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct CaptureLimits(CaptureLimitsInput);

impl CaptureLimits {
    /// Validate positivity of the seven capacity settings. RecordingLimits checks the
    /// tracker against H; storage preparation checks representability and aggregate bytes.
    /// Zero lateness is valid for an explicitly exact simulated source.
    pub fn new(input: CaptureLimitsInput) -> Result<Self, ProfileError> {
        for (field, amount) in [
            (
                "max_tracked_input_notes",
                u64::from(input.max_tracked_input_notes.get()),
            ),
            (
                "max_capture_sources",
                u64::from(input.max_capture_sources.get()),
            ),
            (
                "max_capture_passes",
                u64::from(input.max_capture_passes.get()),
            ),
            (
                "max_pending_capture_results",
                u64::from(input.max_pending_capture_results.get()),
            ),
            ("max_capture_bytes", input.max_capture_bytes.get()),
            (
                "max_audio_capture_frames",
                input.max_audio_capture_frames.as_u64(),
            ),
            ("max_projection_ticks", input.max_projection_ticks.get()),
        ] {
            nonzero(field, amount)?;
        }
        Ok(Self(input))
    }

    pub const fn max_tracked_input_notes(self) -> TrackedInputNoteCount {
        self.0.max_tracked_input_notes
    }
    pub const fn max_capture_sources(self) -> CaptureSourceCount {
        self.0.max_capture_sources
    }
    pub const fn max_capture_passes(self) -> CapturePassCount {
        self.0.max_capture_passes
    }
    pub const fn max_pending_capture_results(self) -> CaptureResultCount {
        self.0.max_pending_capture_results
    }
    pub const fn max_capture_bytes(self) -> PreparedBytes {
        self.0.max_capture_bytes
    }
    pub const fn max_audio_capture_frames(self) -> FrameCount {
        self.0.max_audio_capture_frames
    }
    pub const fn max_projection_ticks(self) -> ProjectionTickCount {
        self.0.max_projection_ticks
    }
    pub const fn capture_lateness_allowance(self) -> FrameCount {
        self.0.capture_lateness_allowance
    }
}
