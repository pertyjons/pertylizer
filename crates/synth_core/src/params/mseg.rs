//! MSEG (Multi-Stage Envelope Generator) parameter types.

use serde::{Deserialize, Serialize};

use crate::types::{BipolarValue, NormalizedValue, Seconds};

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
    TimeScale(NormalizedValue),
    /// Segment time for a specific segment (index encoded in value).
    SegmentTime(u8, Seconds),
    /// Segment level for a specific segment (index encoded in value).
    SegmentLevel(u8, NormalizedValue),
    /// Segment curve for a specific segment (-1=log, 0=linear, +1=exp).
    SegmentCurve(u8, BipolarValue),
}

impl MsegParam {
    pub fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SegmentCount(_), Self::SegmentCount(_))
            | (Self::SustainSegment(_), Self::SustainSegment(_))
            | (Self::LoopStart(_), Self::LoopStart(_))
            | (Self::LoopEnd(_), Self::LoopEnd(_))
            | (Self::LoopEnabled(_), Self::LoopEnabled(_))
            | (Self::TimeScale(_), Self::TimeScale(_)) => true,
            (Self::SegmentTime(a, _), Self::SegmentTime(b, _)) => a == b,
            (Self::SegmentLevel(a, _), Self::SegmentLevel(b, _)) => a == b,
            (Self::SegmentCurve(a, _), Self::SegmentCurve(b, _)) => a == b,
            _ => false,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SegmentCount(_) => "Segments",
            Self::SustainSegment(_) => "Sustain Seg",
            Self::LoopStart(_) => "Loop Start",
            Self::LoopEnd(_) => "Loop End",
            Self::LoopEnabled(_) => "Loop",
            Self::TimeScale(_) => "Time Scale",
            Self::SegmentTime(_, _) => "Seg Time",
            Self::SegmentLevel(_, _) => "Seg Level",
            Self::SegmentCurve(_, _) => "Seg Curve",
        }
    }

    pub fn as_f32(&self) -> f32 {
        match self {
            Self::SegmentCount(n) => *n as f32,
            Self::SustainSegment(n) | Self::LoopStart(n) | Self::LoopEnd(n) => *n as f32,
            Self::LoopEnabled(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::TimeScale(v) => v.as_f32(),
            Self::SegmentTime(_, t) => t.as_f32(),
            Self::SegmentLevel(_, v) => v.as_f32(),
            Self::SegmentCurve(_, v) => v.as_f32(),
        }
    }

    pub fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::SegmentCount(_) => Self::SegmentCount((value as u8).clamp(1, 16)),
            Self::SustainSegment(_) => Self::SustainSegment((value as u8).min(15)),
            Self::LoopStart(_) => Self::LoopStart((value as u8).min(15)),
            Self::LoopEnd(_) => Self::LoopEnd((value as u8).min(15)),
            Self::LoopEnabled(_) => Self::LoopEnabled(value > 0.5),
            Self::TimeScale(_) => Self::TimeScale(NormalizedValue::new(value)),
            Self::SegmentTime(idx, _) => Self::SegmentTime(*idx, Seconds::new(value)),
            Self::SegmentLevel(idx, _) => Self::SegmentLevel(*idx, NormalizedValue::new(value)),
            Self::SegmentCurve(idx, _) => Self::SegmentCurve(*idx, BipolarValue::new(value)),
        }
    }
}

impl Default for MsegParam {
    fn default() -> Self {
        Self::SegmentCount(4)
    }
}
