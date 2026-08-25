//! EVD-0016's deterministic host-clock simulator.
//!
//! This is an evidence harness, not the Phase 3 ingress implementation. ADR-0022
//! forbids a production mapper before its evidence is accepted, so the candidate
//! below remains private to this example. It exercises the contract with integer
//! host nanoseconds, controllable device drift, callback timestamp disturbance,
//! host-block partitions, arrival delay, and stream replacement.

use std::error::Error;

use synth_core::audio::DeviceSampleRate;
use synth_engine_v2::time::{
    FrameCount, QUANTUM_FRAMES, SampleTime, StreamEpoch, TimeError, TimeSource, issue_epoch,
};
use thiserror::Error;

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const PARTS_PER_MILLION: i128 = 1_000_000;
const DECLARED_MAXIMUM_DRIFT_PPM: i32 = 1_500;
const DECLARED_TIMESTAMP_NOISE_NS: u64 = 500_000;
const DECLARED_ARRIVAL_UNCERTAINTY_FRAMES: u64 = 256;

const WHOLE_BLOCK: &[u64] = &[4_096];
const BLOCKS_256: &[u64] = &[256; 16];
const BLOCKS_64: &[u64] = &[64; 64];
const IRREGULAR_BLOCKS: &[u64] = &[17, 511, 3, 64, 1_024, 1, 700, 256, 63, 1_457];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
struct HostNanoseconds(i128);

impl HostNanoseconds {
    const ZERO: Self = Self(0);

    const fn new(nanoseconds: i128) -> Self {
        Self(nanoseconds)
    }

    const fn difference(self, earlier: Self) -> HostNanosecondDelta {
        HostNanosecondDelta(self.0 - earlier.0)
    }

    const fn shifted(self, delta: HostNanosecondDelta) -> Self {
        Self(self.0 + delta.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct HostNanosecondDelta(i128);

impl HostNanosecondDelta {
    const ZERO: Self = Self(0);

    const fn new(nanoseconds: i128) -> Self {
        Self(nanoseconds)
    }

    const fn unsigned_abs(self) -> u128 {
        self.0.unsigned_abs()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct ClockDriftPpm(i32);

impl ClockDriftPpm {
    const ZERO: Self = Self(0);

    const fn new(parts_per_million: i32) -> Self {
        Self(parts_per_million)
    }

    const fn as_i128(self) -> i128 {
        self.0 as i128
    }

    const fn unsigned_abs(self) -> u128 {
        self.0.unsigned_abs() as u128
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct TimestampNoiseBound(HostNanosecondDelta);

impl TimestampNoiseBound {
    const NONE: Self = Self(HostNanosecondDelta::ZERO);

    const fn new(nanoseconds: u64) -> Self {
        Self(HostNanosecondDelta::new(nanoseconds as i128))
    }

    const fn as_nanoseconds(self) -> u128 {
        self.0.unsigned_abs()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct ArrivalDelay(HostNanosecondDelta);

impl ArrivalDelay {
    const fn from_microseconds(microseconds: u64) -> Self {
        Self(HostNanosecondDelta::new(microseconds as i128 * 1_000))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct HostLatency(FrameCount);

impl HostLatency {
    const fn new(frames: u64) -> Self {
        Self(FrameCount::new(frames))
    }

    const fn total_live_output(self) -> Option<FrameCount> {
        self.0.checked_add(FrameCount::QUANTUM)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Calibration {
    host: HostNanoseconds,
    time: SampleTime,
    timestamp_noise: TimestampNoiseBound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HarnessStamp {
    epoch: StreamEpoch,
    time: SampleTime,
    source: TimeSource,
    uncertainty: FrameCount,
    pre_epoch_clamped: bool,
}

#[derive(Debug, Clone, Copy)]
struct CallbackMapper {
    epoch: StreamEpoch,
    rate: DeviceSampleRate,
    calibration: Calibration,
    maximum_drift: ClockDriftPpm,
}

impl CallbackMapper {
    fn prepared(
        epoch: StreamEpoch,
        rate: DeviceSampleRate,
        preparation_host_time: HostNanoseconds,
        maximum_drift: ClockDriftPpm,
    ) -> Self {
        Self {
            epoch,
            rate,
            calibration: Calibration {
                host: preparation_host_time,
                time: SampleTime::ZERO,
                timestamp_noise: TimestampNoiseBound::NONE,
            },
            maximum_drift,
        }
    }

    fn observe_callback(
        &mut self,
        host: HostNanoseconds,
        time: SampleTime,
        timestamp_noise: TimestampNoiseBound,
    ) -> Result<(), HarnessError> {
        if host < self.calibration.host || time < self.calibration.time {
            return Err(HarnessError::NonMonotoneCalibration);
        }
        self.calibration = Calibration {
            host,
            time,
            timestamp_noise,
        };
        Ok(())
    }

    fn map_hardware(self, hardware_time: HostNanoseconds) -> Result<HarnessStamp, HarnessError> {
        self.map(hardware_time, TimeSource::Hardware)
    }

    fn map_arrival(
        self,
        arrival_time: HostNanoseconds,
        adapter_uncertainty: FrameCount,
    ) -> Result<HarnessStamp, HarnessError> {
        let mut stamp = self.map(arrival_time, TimeSource::Arrival)?;
        stamp.uncertainty = stamp
            .uncertainty
            .checked_add(adapter_uncertainty)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        Ok(stamp)
    }

    fn map(
        self,
        host_time: HostNanoseconds,
        source: TimeSource,
    ) -> Result<HarnessStamp, HarnessError> {
        let delta = host_time.difference(self.calibration.host);
        let frame_delta = rounded_frames(delta, self.rate)?;
        let raw_time = i128::from(self.calibration.time.as_u64()) + i128::from(frame_delta);
        let (time, pre_epoch_clamped) = if raw_time < 0 {
            (SampleTime::ZERO, true)
        } else {
            let frames = u64::try_from(raw_time).map_err(|_| HarnessError::MappedTimeOverflow)?;
            (SampleTime::new(frames), false)
        };
        Ok(HarnessStamp {
            epoch: self.epoch,
            time,
            source,
            uncertainty: mapping_uncertainty(
                delta,
                self.rate,
                self.maximum_drift,
                self.calibration.timestamp_noise,
            )?,
            pre_epoch_clamped,
        })
    }
}

#[derive(Debug, Error)]
enum HarnessError {
    #[error(transparent)]
    Time(#[from] TimeError),
    #[error("the simulated device rate is not positive")]
    InvalidDeviceRate,
    #[error("division by zero in the evidence harness")]
    ZeroDivisor,
    #[error("integer arithmetic overflowed in the evidence harness")]
    ArithmeticOverflow,
    #[error("a mapped frame delta does not fit i64")]
    FrameDeltaOverflow,
    #[error("a mapped sample time does not fit u64")]
    MappedTimeOverflow,
    #[error("a callback calibration moved backwards")]
    NonMonotoneCalibration,
    #[error("the F1 negative control was insensitive: observed {observed_frames} frames")]
    InsensitiveNegativeControl { observed_frames: u64 },
    #[error("case {case} mapped sample {observed}, expected {expected}")]
    WrongMapping {
        case: &'static str,
        observed: u64,
        expected: u64,
    },
    #[error("case {case} observed {error_frames} frames of error outside {uncertainty_frames}")]
    UncoveredUncertainty {
        case: &'static str,
        error_frames: u64,
        uncertainty_frames: u64,
    },
    #[error("partition {partition} produced a different quiet-case stamp")]
    PartitionDependent { partition: &'static str },
    #[error("an already accepted stamp changed after calibration")]
    AcceptedStampMoved,
    #[error("the accepted-stamp mutation control was insensitive")]
    InsensitiveAcceptedStampControl,
    #[error("the candidate acceptance ledger lost an accepted event")]
    AcceptedEventMissing,
    #[error("a stale epoch was accepted after stream replacement")]
    StaleEpochAccepted,
    #[error("stream replacement retained the old stream's calibration")]
    CalibrationSurvivedReplacement,
    #[error("the retained-calibration mutation control was insensitive")]
    InsensitiveCalibrationResetControl,
    #[error("stream replacement did not issue a strictly newer epoch")]
    EpochNotIncreasing,
    #[error("the pre-epoch case did not clamp and report its provenance")]
    MissingPreEpochClamp,
    #[error("live latency did not retain both the host and Q contributors")]
    IncorrectLatencyAccounting,
    #[error("live latency accounting moved a causal event earlier")]
    CausalEventMovedEarlier,
    #[error("the causal-event mutation control was insensitive")]
    InsensitiveCausalEventControl,
    #[error("arrival fallback error was not covered by its paired reference")]
    ArrivalUncertaintyUncovered,
}

fn rounded_frames(delta: HostNanosecondDelta, rate: DeviceSampleRate) -> Result<i64, HarnessError> {
    let numerator = delta
        .0
        .checked_mul(i128::from(rate.as_u32()))
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let magnitude = numerator.unsigned_abs();
    let denominator = NANOS_PER_SECOND as u128;
    let rounded = magnitude
        .checked_add(denominator / 2)
        .ok_or(HarnessError::ArithmeticOverflow)?
        / denominator;
    let signed = i128::try_from(rounded).map_err(|_| HarnessError::FrameDeltaOverflow)?;
    let signed = if numerator < 0 { -signed } else { signed };
    i64::try_from(signed).map_err(|_| HarnessError::FrameDeltaOverflow)
}

fn mapping_uncertainty(
    extrapolation: HostNanosecondDelta,
    rate: DeviceSampleRate,
    maximum_drift: ClockDriftPpm,
    timestamp_noise: TimestampNoiseBound,
) -> Result<FrameCount, HarnessError> {
    let rate = u128::from(rate.as_u32());
    let noise_numerator = timestamp_noise
        .as_nanoseconds()
        .checked_mul(rate)
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let noise_frames = div_ceil(noise_numerator, NANOS_PER_SECOND as u128)?;

    let drift_numerator = extrapolation
        .unsigned_abs()
        .checked_mul(rate)
        .and_then(|value| value.checked_mul(maximum_drift.unsigned_abs()))
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let drift_denominator = (NANOS_PER_SECOND as u128)
        .checked_mul(PARTS_PER_MILLION as u128)
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let drift_frames = div_ceil(drift_numerator, drift_denominator)?;

    // One frame covers round-to-nearest and the integer-nanosecond construction
    // of the simulated device clock. The two modeled errors are rounded outward.
    let total = noise_frames
        .checked_add(drift_frames)
        .and_then(|value| value.checked_add(1))
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let frames = u64::try_from(total).map_err(|_| HarnessError::ArithmeticOverflow)?;
    Ok(FrameCount::new(frames))
}

fn div_ceil(numerator: u128, denominator: u128) -> Result<u128, HarnessError> {
    if denominator == 0 {
        return Err(HarnessError::ZeroDivisor);
    }
    numerator
        .checked_add(denominator - 1)
        .map(|value| value / denominator)
        .ok_or(HarnessError::ArithmeticOverflow)
}

fn host_time_for_frame(
    frame: u64,
    rate: DeviceSampleRate,
    drift: ClockDriftPpm,
) -> Result<HostNanoseconds, HarnessError> {
    let scaled_rate = i128::from(rate.as_u32())
        .checked_mul(
            PARTS_PER_MILLION
                .checked_add(drift.as_i128())
                .ok_or(HarnessError::ArithmeticOverflow)?,
        )
        .ok_or(HarnessError::ArithmeticOverflow)?;
    if scaled_rate <= 0 {
        return Err(HarnessError::InvalidDeviceRate);
    }
    let numerator = i128::from(frame)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_mul(PARTS_PER_MILLION))
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let rounded = numerator
        .checked_add(scaled_rate / 2)
        .ok_or(HarnessError::ArithmeticOverflow)?
        / scaled_rate;
    Ok(HostNanoseconds::new(rounded))
}

fn deterministic_noise(
    callback_index: usize,
    bound: TimestampNoiseBound,
) -> Result<HostNanosecondDelta, HarnessError> {
    const PATTERN: [i8; 8] = [-8, 5, -3, 8, -1, 2, -6, 4];
    let bound =
        i128::try_from(bound.as_nanoseconds()).map_err(|_| HarnessError::ArithmeticOverflow)?;
    let scaled = bound * i128::from(PATTERN[callback_index % PATTERN.len()]);
    Ok(HostNanosecondDelta::new(scaled / 8))
}

fn static_mapper_negative_control() -> Result<MatrixResult, HarnessError> {
    let rate = DeviceSampleRate::DVD_QUALITY;
    let epoch = issue_epoch()?;
    let elapsed_seconds = 30_u64 * 60;
    let actual_frames = u64::from(rate.as_u32())
        .checked_mul(elapsed_seconds)
        .and_then(|value| value.checked_mul(10_001))
        .map(|value| value / 10_000)
        .ok_or(HarnessError::ArithmeticOverflow)?;
    let static_mapper =
        CallbackMapper::prepared(epoch, rate, HostNanoseconds::ZERO, ClockDriftPpm::ZERO);
    let event_host = HostNanoseconds::new(
        i128::from(elapsed_seconds)
            .checked_mul(NANOS_PER_SECOND)
            .ok_or(HarnessError::ArithmeticOverflow)?,
    );
    let mapped_frames = static_mapper.map_hardware(event_host)?.time.as_u64();
    let error = actual_frames.abs_diff(mapped_frames);
    if error < 8_000 {
        return Err(HarnessError::InsensitiveNegativeControl {
            observed_frames: error,
        });
    }
    let mut result = MatrixResult::new();
    result.observe(error, 0)?;
    Ok(result)
}

struct QuietPartitionResult {
    mapped_times: Vec<u64>,
    measurements: MatrixResult,
}

fn quiet_partition_result(
    partition_name: &'static str,
    blocks: &[u64],
) -> Result<QuietPartitionResult, HarnessError> {
    const EVENTS: [u64; 9] = [0, 1, 63, 64, 255, 256, 1_023, 2_048, 4_095];
    let rate = DeviceSampleRate::DVD_QUALITY;
    let epoch = issue_epoch()?;
    let mut mapper =
        CallbackMapper::prepared(epoch, rate, HostNanoseconds::ZERO, ClockDriftPpm::ZERO);
    let mut mapped = Vec::with_capacity(EVENTS.len());
    let mut measurements = MatrixResult::new();
    let mut callback_start = 0_u64;
    for block in blocks {
        let callback_host = host_time_for_frame(callback_start, rate, ClockDriftPpm::ZERO)?;
        mapper.observe_callback(
            callback_host,
            SampleTime::new(callback_start),
            TimestampNoiseBound::NONE,
        )?;
        let callback_end = callback_start
            .checked_add(*block)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        for event in EVENTS {
            if event >= callback_start && event < callback_end {
                let host = host_time_for_frame(event, rate, ClockDriftPpm::ZERO)?;
                let stamp = mapper.map_hardware(host)?;
                if stamp.time.as_u64() != event {
                    return Err(HarnessError::WrongMapping {
                        case: partition_name,
                        observed: stamp.time.as_u64(),
                        expected: event,
                    });
                }
                measurements.observe(
                    stamp.time.as_u64().abs_diff(event),
                    stamp.uncertainty.as_u64(),
                )?;
                mapped.push(stamp.time.as_u64());
            }
        }
        callback_start = callback_end;
    }
    if callback_start != 4_096 {
        return Err(HarnessError::WrongMapping {
            case: partition_name,
            observed: callback_start,
            expected: 4_096,
        });
    }
    Ok(QuietPartitionResult {
        mapped_times: mapped,
        measurements,
    })
}

fn quiet_partition_control() -> Result<MatrixResult, HarnessError> {
    let reference = quiet_partition_result("4096", WHOLE_BLOCK)?;
    let mut aggregate = reference.measurements;
    for (name, blocks) in [
        ("16x256", BLOCKS_256),
        ("64x64", BLOCKS_64),
        ("irregular", IRREGULAR_BLOCKS),
    ] {
        let candidate = quiet_partition_result(name, blocks)?;
        if candidate.mapped_times != reference.mapped_times {
            return Err(HarnessError::PartitionDependent { partition: name });
        }
        aggregate.merge(candidate.measurements)?;
    }
    Ok(aggregate)
}

#[derive(Debug, Clone, Copy)]
struct MatrixResult {
    observations: u64,
    maximum_error: FrameCount,
    maximum_uncertainty: FrameCount,
}

impl MatrixResult {
    const fn new() -> Self {
        Self {
            observations: 0,
            maximum_error: FrameCount::ZERO,
            maximum_uncertainty: FrameCount::ZERO,
        }
    }

    fn observe(&mut self, error: u64, uncertainty: u64) -> Result<(), HarnessError> {
        self.observations = self
            .observations
            .checked_add(1)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        self.maximum_error = FrameCount::new(self.maximum_error.as_u64().max(error));
        self.maximum_uncertainty =
            FrameCount::new(self.maximum_uncertainty.as_u64().max(uncertainty));
        Ok(())
    }

    fn merge(&mut self, other: Self) -> Result<(), HarnessError> {
        self.observations = self
            .observations
            .checked_add(other.observations)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        self.maximum_error = self.maximum_error.max(other.maximum_error);
        self.maximum_uncertainty = self.maximum_uncertainty.max(other.maximum_uncertainty);
        Ok(())
    }
}

fn run_matrix_case(
    rate: DeviceSampleRate,
    drift: ClockDriftPpm,
    noise_bound: TimestampNoiseBound,
    blocks: &[u64],
    total_frames: u64,
) -> Result<MatrixResult, HarnessError> {
    let epoch = issue_epoch()?;
    let mut mapper = CallbackMapper::prepared(
        epoch,
        rate,
        HostNanoseconds::ZERO,
        ClockDriftPpm::new(DECLARED_MAXIMUM_DRIFT_PPM),
    );
    let mut callback_start = 0_u64;
    let mut observations = 0_u64;
    let mut maximum_error = 0_u64;
    let mut maximum_uncertainty = 0_u64;
    let mut previous_observed_host = HostNanoseconds::ZERO;

    let mut callback_index = 0_usize;
    while callback_start < total_frames {
        let declared_block = blocks[callback_index % blocks.len()];
        let block = declared_block.min(total_frames - callback_start);
        let true_callback_host = host_time_for_frame(callback_start, rate, drift)?;
        let disturbed_callback_host =
            true_callback_host.shifted(deterministic_noise(callback_index, noise_bound)?);
        // CPAL's public contract makes callback instants monotone. A bounded
        // timestamp-error fixture must preserve that property or it would be an
        // F7 input, not a calibration-noise case.
        let observed_callback_host = disturbed_callback_host.max(previous_observed_host);
        mapper.observe_callback(
            observed_callback_host,
            SampleTime::new(callback_start),
            TimestampNoiseBound::new(DECLARED_TIMESTAMP_NOISE_NS),
        )?;
        previous_observed_host = observed_callback_host;

        let last = block.saturating_sub(1);
        for offset in [0, block / 2, last] {
            let expected = callback_start
                .checked_add(offset)
                .ok_or(HarnessError::ArithmeticOverflow)?;
            let event_host = host_time_for_frame(expected, rate, drift)?;
            let stamp = mapper.map_hardware(event_host)?;
            let error = stamp.time.as_u64().abs_diff(expected);
            if error > stamp.uncertainty.as_u64() {
                return Err(HarnessError::UncoveredUncertainty {
                    case: "drift/noise matrix",
                    error_frames: error,
                    uncertainty_frames: stamp.uncertainty.as_u64(),
                });
            }
            maximum_error = maximum_error.max(error);
            maximum_uncertainty = maximum_uncertainty.max(stamp.uncertainty.as_u64());
            observations += 1;
        }
        callback_start = callback_start
            .checked_add(block)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        callback_index = callback_index
            .checked_add(1)
            .ok_or(HarnessError::ArithmeticOverflow)?;
    }

    Ok(MatrixResult {
        observations,
        maximum_error: FrameCount::new(maximum_error),
        maximum_uncertainty: FrameCount::new(maximum_uncertainty),
    })
}

fn same_accepted_identity(left: HarnessStamp, right: HarnessStamp) -> bool {
    left.epoch == right.epoch && left.time == right.time && left.source == right.source
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct AcceptedEventIndex(usize);

struct CandidateIngress {
    mapper: CallbackMapper,
    accepted: Vec<HarnessStamp>,
}

impl CandidateIngress {
    fn prepared(
        epoch: StreamEpoch,
        rate: DeviceSampleRate,
        preparation_host_time: HostNanoseconds,
        maximum_drift: ClockDriftPpm,
    ) -> Self {
        Self {
            mapper: CallbackMapper::prepared(epoch, rate, preparation_host_time, maximum_drift),
            accepted: Vec::new(),
        }
    }

    fn accept_hardware(
        &mut self,
        hardware_time: HostNanoseconds,
    ) -> Result<AcceptedEventIndex, HarnessError> {
        let stamp = self.mapper.map_hardware(hardware_time)?;
        let index = AcceptedEventIndex(self.accepted.len());
        self.accepted.push(stamp);
        Ok(index)
    }

    fn retained(&self, index: AcceptedEventIndex) -> Result<HarnessStamp, HarnessError> {
        self.accepted
            .get(index.0)
            .copied()
            .ok_or(HarnessError::AcceptedEventMissing)
    }

    fn observe_callback(
        &mut self,
        host: HostNanoseconds,
        time: SampleTime,
        timestamp_noise: TimestampNoiseBound,
    ) -> Result<(), HarnessError> {
        self.mapper.observe_callback(host, time, timestamp_noise)
    }
}

fn calibration_does_not_move_accepted_stamp() -> Result<(MatrixResult, MatrixResult), HarnessError>
{
    let rate = DeviceSampleRate::DVD_QUALITY;
    let epoch = issue_epoch()?;
    let mut ingress = CandidateIngress::prepared(
        epoch,
        rate,
        HostNanoseconds::ZERO,
        ClockDriftPpm::new(DECLARED_MAXIMUM_DRIFT_PPM),
    );
    let event_host = host_time_for_frame(8_192, rate, ClockDriftPpm::new(1_000))?;
    let accepted_index = ingress.accept_hardware(event_host)?;
    let accepted = ingress.retained(accepted_index)?;
    ingress.observe_callback(
        host_time_for_frame(4_096, rate, ClockDriftPpm::new(1_000))?,
        SampleTime::new(4_096),
        TimestampNoiseBound::new(DECLARED_TIMESTAMP_NOISE_NS),
    )?;
    let retained = ingress.retained(accepted_index)?;
    if !same_accepted_identity(retained, accepted) {
        return Err(HarnessError::AcceptedStampMoved);
    }
    let mut retained_result = MatrixResult::new();
    retained_result.observe(
        retained.time.as_u64().abs_diff(accepted.time.as_u64()),
        retained.uncertainty.as_u64(),
    )?;

    // Negative control: the forbidden policy recomputes an already accepted
    // stamp after calibration. The identity checker must detect the movement.
    let remapped = ingress.mapper.map_hardware(event_host)?;
    let mutation_error = remapped.time.as_u64().abs_diff(accepted.time.as_u64());
    if mutation_error == 0 || same_accepted_identity(remapped, accepted) {
        return Err(HarnessError::InsensitiveAcceptedStampControl);
    }
    let mut mutation_result = MatrixResult::new();
    mutation_result.observe(mutation_error, 0)?;
    Ok((retained_result, mutation_result))
}

struct EpochAdmission {
    current: StreamEpoch,
    stale_events: u64,
}

struct CandidateEpoch {
    mapper: CallbackMapper,
    admission: EpochAdmission,
}

impl CandidateEpoch {
    fn prepared(
        epoch: StreamEpoch,
        rate: DeviceSampleRate,
        preparation_host_time: HostNanoseconds,
    ) -> Self {
        Self {
            mapper: CallbackMapper::prepared(
                epoch,
                rate,
                preparation_host_time,
                ClockDriftPpm::ZERO,
            ),
            admission: EpochAdmission::new(epoch),
        }
    }

    fn replace(
        &self,
        rate: DeviceSampleRate,
        preparation_host_time: HostNanoseconds,
    ) -> Result<Self, HarnessError> {
        let epoch = issue_epoch()?;
        if epoch <= self.mapper.epoch {
            return Err(HarnessError::EpochNotIncreasing);
        }
        Ok(Self::prepared(epoch, rate, preparation_host_time))
    }

    fn admit(&mut self, stamp: HarnessStamp) -> bool {
        self.admission.admit(stamp)
    }
}

impl EpochAdmission {
    const fn new(current: StreamEpoch) -> Self {
        Self {
            current,
            stale_events: 0,
        }
    }

    fn admit(&mut self, stamp: HarnessStamp) -> bool {
        if stamp.epoch == self.current {
            true
        } else {
            self.stale_events = self.stale_events.saturating_add(1);
            false
        }
    }
}

fn pre_epoch_and_replacement_controls() -> Result<(MatrixResult, MatrixResult), HarnessError> {
    let mut result = MatrixResult::new();
    let rate = DeviceSampleRate::DVD_QUALITY;
    let first_epoch = issue_epoch()?;
    let preparation_host = HostNanoseconds::new(NANOS_PER_SECOND);
    let mut first = CandidateEpoch::prepared(first_epoch, rate, preparation_host);
    let early = first
        .mapper
        .map_hardware(preparation_host.shifted(HostNanosecondDelta::new(-500_000)))?;
    if early.time != SampleTime::ZERO
        || !early.pre_epoch_clamped
        || early.source != TimeSource::Hardware
    {
        return Err(HarnessError::MissingPreEpochClamp);
    }
    result.observe(early.time.as_u64(), early.uncertainty.as_u64())?;

    first.mapper.observe_callback(
        host_time_for_frame(4_096, rate, ClockDriftPpm::ZERO)?
            .shifted(HostNanosecondDelta::new(NANOS_PER_SECOND)),
        SampleTime::new(4_096),
        TimestampNoiseBound::new(DECLARED_TIMESTAMP_NOISE_NS),
    )?;
    let stale = first.mapper.map_hardware(preparation_host)?;
    let replacement_host = HostNanoseconds::new(NANOS_PER_SECOND * 5);
    let mut second = first.replace(rate, replacement_host)?;
    let second_epoch = second.mapper.epoch;
    if second.admit(stale) || second.admission.stale_events != 1 {
        return Err(HarnessError::StaleEpochAccepted);
    }
    result.observe(0, stale.uncertainty.as_u64())?;
    let second_origin = second.mapper.map_hardware(replacement_host)?;
    if second_origin.epoch != second_epoch
        || second_origin.time != SampleTime::ZERO
        || second.mapper.calibration
            != (Calibration {
                host: replacement_host,
                time: SampleTime::ZERO,
                timestamp_noise: TimestampNoiseBound::NONE,
            })
    {
        return Err(HarnessError::CalibrationSurvivedReplacement);
    }
    if !second.admit(second_origin) || second.admission.stale_events != 1 {
        return Err(HarnessError::StaleEpochAccepted);
    }
    result.observe(
        second_origin.time.as_u64(),
        second_origin.uncertainty.as_u64(),
    )?;

    // Negative control: changing only the epoch while retaining the first
    // stream's calibrated anchor must not map the replacement origin to zero.
    let mut retained_calibration = first.mapper;
    retained_calibration.epoch = second_epoch;
    let falsely_reused = retained_calibration.map_hardware(replacement_host)?;
    if falsely_reused.time == SampleTime::ZERO {
        return Err(HarnessError::InsensitiveCalibrationResetControl);
    }
    let mut mutation_result = MatrixResult::new();
    mutation_result.observe(falsely_reused.time.as_u64(), 0)?;
    Ok((result, mutation_result))
}

struct LiveTiming {
    event: AcceptedEventIndex,
    reported_output_latency: FrameCount,
}

fn account_live_timing(
    ingress: &mut CandidateIngress,
    hardware_time: HostNanoseconds,
    host_latency: HostLatency,
) -> Result<LiveTiming, HarnessError> {
    let event = ingress.accept_hardware(hardware_time)?;
    let reported_output_latency = host_latency
        .total_live_output()
        .ok_or(HarnessError::IncorrectLatencyAccounting)?;
    Ok(LiveTiming {
        event,
        reported_output_latency,
    })
}

fn latency_and_arrival_controls() -> Result<(MatrixResult, MatrixResult), HarnessError> {
    let mut candidate_result = MatrixResult::new();
    let mut mutation_result = MatrixResult::new();
    let host_latency = HostLatency::new(192);
    let Some(total_latency) = host_latency.total_live_output() else {
        return Err(HarnessError::IncorrectLatencyAccounting);
    };
    if total_latency.as_u64() != 192 + u64::from(QUANTUM_FRAMES) {
        return Err(HarnessError::IncorrectLatencyAccounting);
    }

    let rate = DeviceSampleRate::DVD_QUALITY;
    let epoch = issue_epoch()?;
    let mut ingress =
        CandidateIngress::prepared(epoch, rate, HostNanoseconds::ZERO, ClockDriftPpm::ZERO);
    let hardware_host = host_time_for_frame(1_024, rate, ClockDriftPpm::ZERO)?;
    let monitored = account_live_timing(&mut ingress, hardware_host, host_latency)?;
    let hardware = ingress.retained(monitored.event)?;
    if monitored.reported_output_latency != total_latency
        || ingress.retained(monitored.event)? != hardware
    {
        return Err(HarnessError::CausalEventMovedEarlier);
    }
    candidate_result.observe(0, hardware.uncertainty.as_u64())?;

    // Negative control: subtracting the declared live latency would move the
    // causal event earlier. The identity check must reject that policy.
    let moved_time = SampleTime::new(
        hardware
            .time
            .as_u64()
            .saturating_sub(total_latency.as_u64()),
    );
    let moved = HarnessStamp {
        time: moved_time,
        ..hardware
    };
    let mutation_error = moved.time.as_u64().abs_diff(hardware.time.as_u64());
    if mutation_error == 0 || same_accepted_identity(moved, hardware) {
        return Err(HarnessError::InsensitiveCausalEventControl);
    }
    mutation_result.observe(mutation_error, 0)?;

    let declared_arrival_uncertainty = FrameCount::new(DECLARED_ARRIVAL_UNCERTAINTY_FRAMES);
    for delay in [
        ArrivalDelay::from_microseconds(0),
        ArrivalDelay::from_microseconds(100),
        ArrivalDelay::from_microseconds(1_000),
        ArrivalDelay::from_microseconds(5_000),
    ] {
        let arrival = ingress
            .mapper
            .map_arrival(hardware_host.shifted(delay.0), declared_arrival_uncertainty)?;
        let error = arrival.time.as_u64().abs_diff(hardware.time.as_u64());
        if error > arrival.uncertainty.as_u64() {
            return Err(HarnessError::ArrivalUncertaintyUncovered);
        }
        candidate_result.observe(error, arrival.uncertainty.as_u64())?;
    }
    Ok((candidate_result, mutation_result))
}

fn main() -> Result<(), Box<dyn Error>> {
    println!(
        "case,rate_hz,drift_ppm,noise_ns,partition,observations,max_error_frames,max_uncertainty_frames"
    );

    let negative = static_mapper_negative_control()?;
    println!(
        "F1-static-control,48000,100,0,static,{},{},{}",
        negative.observations,
        negative.maximum_error.as_u64(),
        negative.maximum_uncertainty.as_u64()
    );

    let quiet = quiet_partition_control()?;
    println!(
        "F2-quiet-partitions,48000,0,0,all,{},{},{}",
        quiet.observations,
        quiet.maximum_error.as_u64(),
        quiet.maximum_uncertainty.as_u64()
    );

    let rates = [8_000, 44_100, 48_000, 96_000, 192_000];
    let drifts = [-1_000, -100, -20, 0, 20, 100, 1_000];
    let noise_bounds = [0, 20_000, 250_000];
    let partitions = [
        ("4096", WHOLE_BLOCK),
        ("16x256", BLOCKS_256),
        ("64x64", BLOCKS_64),
        ("irregular", IRREGULAR_BLOCKS),
    ];
    for rate_hz in rates {
        for drift_ppm in drifts {
            for noise_ns in noise_bounds {
                for (partition_name, blocks) in partitions {
                    let result = run_matrix_case(
                        DeviceSampleRate::new(rate_hz),
                        ClockDriftPpm::new(drift_ppm),
                        TimestampNoiseBound::new(noise_ns),
                        blocks,
                        4_096,
                    )?;
                    println!(
                        "matrix,{rate_hz},{drift_ppm},{noise_ns},{partition_name},{},{},{}",
                        result.observations,
                        result.maximum_error.as_u64(),
                        result.maximum_uncertainty.as_u64()
                    );
                }
            }
        }
    }

    let long_frames = u64::from(DeviceSampleRate::DVD_QUALITY.as_u32()) * 30 * 60;
    for drift_ppm in [-1_000, 1_000] {
        for (partition_name, blocks) in partitions {
            let result = run_matrix_case(
                DeviceSampleRate::DVD_QUALITY,
                ClockDriftPpm::new(drift_ppm),
                TimestampNoiseBound::new(250_000),
                blocks,
                long_frames,
            )?;
            println!(
                "F3-long-horizon,48000,{drift_ppm},250000,{partition_name},{},{},{}",
                result.observations,
                result.maximum_error.as_u64(),
                result.maximum_uncertainty.as_u64()
            );
        }
    }

    let (retained, accepted_mutation) = calibration_does_not_move_accepted_stamp()?;
    println!(
        "F5-retained-stamp,48000,1000,500000,all,{},{},{}",
        retained.observations,
        retained.maximum_error.as_u64(),
        retained.maximum_uncertainty.as_u64()
    );
    println!(
        "F5-remap-mutation-detected,48000,1000,500000,all,{},{},{}",
        accepted_mutation.observations,
        accepted_mutation.maximum_error.as_u64(),
        accepted_mutation.maximum_uncertainty.as_u64()
    );

    let (epoch_controls, epoch_mutation) = pre_epoch_and_replacement_controls()?;
    println!(
        "F8-epoch-controls,48000,0,0,all,{},{},{}",
        epoch_controls.observations,
        epoch_controls.maximum_error.as_u64(),
        epoch_controls.maximum_uncertainty.as_u64()
    );
    println!(
        "F8-retained-calibration-mutation-detected,48000,0,0,all,{},{},{}",
        epoch_mutation.observations,
        epoch_mutation.maximum_error.as_u64(),
        epoch_mutation.maximum_uncertainty.as_u64()
    );

    let (latency_arrival, latency_mutation) = latency_and_arrival_controls()?;
    println!(
        "F9-arrival-controls,48000,0,0,all,{},{},{}",
        latency_arrival.observations,
        latency_arrival.maximum_error.as_u64(),
        latency_arrival.maximum_uncertainty.as_u64()
    );
    println!(
        "F9-latency-subtraction-mutation-detected,48000,0,0,all,{},{},{}",
        latency_mutation.observations,
        latency_mutation.maximum_error.as_u64(),
        latency_mutation.maximum_uncertainty.as_u64()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_simulator_control_still_passes() {
        if let Err(error) = super::main() {
            panic!("EVD-0016 simulator control failed: {error}");
        }
    }
}
