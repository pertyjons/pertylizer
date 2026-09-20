//! Declared synthetic clocks, never inferred physical timestamps (ADR-0069).
use super::InputError;
use crate::time::{FrameCount, SampleTime, StreamEpoch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct InputTick(u64);
impl InputTick {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InputTickSpan(u64);
impl InputTickSpan {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// A positive rational number of engine frames per synthetic source tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InputRate {
    frames: FrameCount,
    ticks: InputTickSpan,
}
impl InputRate {
    pub fn new(frames: FrameCount, ticks: InputTickSpan) -> Result<Self, InputError> {
        if frames == FrameCount::ZERO || ticks.0 == 0 {
            return Err(InputError::ClockRange);
        }
        Ok(Self { frames, ticks })
    }
}

/// Immutable oracle parameters. Each connection declares its own tick origin and
/// rational rate. The epoch and frame origin belong to the simulated engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SimulatedInputClock {
    pub(super) epoch: StreamEpoch,
    pub(super) origin: SampleTime,
    pub(super) tick_origin: InputTick,
    rate: InputRate,
    uncertainty: InputTickSpan,
}
impl SimulatedInputClock {
    pub const fn new(
        epoch: StreamEpoch,
        origin: SampleTime,
        tick_origin: InputTick,
        rate: InputRate,
        uncertainty: InputTickSpan,
    ) -> Self {
        Self {
            epoch,
            origin,
            tick_origin,
            rate,
            uncertainty,
        }
    }
    pub const fn epoch(self) -> StreamEpoch {
        self.epoch
    }
    pub const fn origin(self) -> SampleTime {
        self.origin
    }
    pub const fn uncertainty(self) -> InputTickSpan {
        self.uncertainty
    }

    fn bucket_wide(self, tick: u64) -> Result<u128, InputError> {
        let elapsed = tick
            .checked_sub(self.tick_origin.0)
            .ok_or(InputError::ClockRange)?;
        let frames = u128::from(elapsed)
            .checked_mul(u128::from(self.rate.frames.as_u64()))
            .ok_or(InputError::ClockRange)?
            / u128::from(self.rate.ticks.0);
        frames
            .checked_add(u128::from(self.origin.as_u64()))
            .ok_or(InputError::ClockRange)
    }
    fn bucket(self, tick: u64) -> Result<SampleTime, InputError> {
        let value = u64::try_from(self.bucket_wide(tick)?).map_err(|_| InputError::ClockRange)?;
        Ok(SampleTime::new(value))
    }

    /// Quality attribution only: intersect the possible tick interval with the
    /// oracle's representable domain, then with representable engine frames.
    /// Admission still rejects the original out-of-domain observation; these
    /// clipped diagnostic bounds never become a CaptureStamp or accepted input.
    pub(super) fn quality_bounds(self, tick: InputTick) -> Option<(SampleTime, SampleTime)> {
        let low = tick
            .0
            .saturating_sub(self.uncertainty.0)
            .max(self.tick_origin.0);
        let high = tick.0.saturating_add(self.uncertainty.0);
        if low > high {
            return None;
        }
        let low = u64::try_from(self.bucket_wide(low).ok()?).ok()?;
        let high = self.bucket_wide(high).ok()?.min(u128::from(u64::MAX));
        let high = u64::try_from(high).ok()?;
        Some((SampleTime::new(low), SampleTime::new(high)))
    }

    /// Floor defines half-open synthetic frame buckets. Exact means the entire
    /// inclusive tick-uncertainty interval selects the same bucket.
    pub fn map(self, tick: InputTick) -> Result<SampleTime, InputError> {
        let (low, high) = self.bounds(tick)?;
        if low != high {
            return Err(InputError::Uncertain);
        }
        Ok(low)
    }

    pub(super) fn bounds(self, tick: InputTick) -> Result<(SampleTime, SampleTime), InputError> {
        let low = tick
            .0
            .checked_sub(self.uncertainty.0)
            .ok_or(InputError::ClockRange)?;
        let high = tick
            .0
            .checked_add(self.uncertainty.0)
            .ok_or(InputError::ClockRange)?;
        let low = self.bucket(low)?;
        let high = self.bucket(high)?;
        Ok((low, high))
    }
}
