//! MSEG (Multi-Stage Envelope Generator) parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, NormalizedValue, Seconds, TimeScale};

/// MSEG parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MsegParam {
    /// Number of active segments (1-16).
    SegmentCount(u8),
    /// Sustain segment index (0-15, where envelope holds until note-off).
    SustainSegment(u8),
    /// Loop start segment index.
    LoopStart(u8),
    /// Loop end segment index.
    LoopEnd(u8),
    /// Whether looping is enabled.
    LoopEnabled(bool),
    /// Global time scale multiplier.
    TimeScale(TimeScale),
    /// Segment time for a specific segment (index encoded in value).
    SegmentTime(u8, Seconds),
    /// Segment level for a specific segment (index encoded in value).
    SegmentLevel(u8, NormalizedValue),
    /// Segment curve for a specific segment (-1=log, 0=linear, +1=exp).
    SegmentCurve(u8, BipolarValue),
}

impl Default for MsegParam {
    fn default() -> Self {
        Self::SegmentCount(4)
    }
}
