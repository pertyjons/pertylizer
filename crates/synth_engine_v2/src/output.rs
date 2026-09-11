//! The terminating voice's output policy (`SOUND-INV-037`).

/// Limiting applied after a voice's master gain and constant-power pan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum OutputLimiting {
    /// V1's enabled soft knee, with its fixed -0.3 dB threshold.
    SoftKnee,
    /// V1's disabled-limiter behavior: hard clamp to -1 through 1.
    HardClamp,
    /// Explicit float-headroom policy: preserve values beyond full scale.
    Unbounded,
}
