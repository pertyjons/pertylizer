//! Test-only callback observations. The caller prepares output and retains logs
//! outside `observe`. Clock reads bracket the callback inside the allocator scope;
//! allocator-scope setup and collection are excluded from the retained duration.
use std::time::{Duration, Instant};

use synth_engine_v2::quantities::SampleRate;

#[derive(Clone, Copy, Debug)]
pub(super) struct Observation {
    pub duration: Duration,
    pub allocations: allocation_counter::AllocationInfo,
}

pub(super) fn observe<T>(callback: impl FnOnce() -> T) -> (T, Observation) {
    let mut result = None;
    let mut duration = Duration::ZERO;
    let allocations = allocation_counter::measure(|| {
        let start = Instant::now();
        result = Some(callback());
        duration = start.elapsed();
    });
    (
        result.expect("the callback returned"),
        Observation {
            duration,
            allocations,
        },
    )
}

/// Quarter of a fixed 64-frame callback period, rounded down to nanoseconds.
pub(super) fn budget(rate: SampleRate) -> Duration {
    let hertz = match rate.as_f32() {
        44_100.0 => 44_100,
        48_000.0 => 48_000,
        96_000.0 => 96_000,
        192_000.0 => 192_000,
        _ => panic!("rate outside the frozen capacity candidate"),
    };
    Duration::from_nanos(16_000_000_000 / hertz)
}

/// Audio-side handoff APIs have the same no-allocation/no-free contract as
/// render, even when this serial harness calls them between callbacks.
pub(super) fn without_heap<T>(operation: impl FnOnce() -> T) -> T {
    let (result, observation) = observe(operation);
    assert_eq!(observation.allocations.count_total, 0);
    assert_eq!(observation.allocations.count_current, 0);
    assert_eq!(observation.allocations.bytes_current, 0);
    result
}

/// Nearest-rank p99.9; sort the caller's retained durations outside the callback.
pub(super) fn p999(durations: &mut [Duration]) -> Option<Duration> {
    durations.sort_unstable();
    let rank = durations.len().checked_mul(999)?.div_ceil(1_000);
    durations.get(rank.checked_sub(1)?).copied()
}

#[test]
fn allocation_observer_detects_transient_allocation_inside_the_callback() {
    let (_, empty) = observe(|| std::hint::black_box(7));
    assert_eq!(empty.allocations.count_total, 0);
    let (_, allocated) = observe(|| {
        let buffer = std::hint::black_box(vec![0_u8; 4_096]);
        std::hint::black_box(buffer.len())
    });
    assert!(allocated.allocations.count_total > 0);
    assert!(allocated.allocations.bytes_total >= 4_096);
    assert_eq!(allocated.allocations.bytes_current, 0);
    let buffer = std::hint::black_box(vec![0_u8; 4_096]);
    let (_, freed) = observe(|| drop(buffer));
    assert_eq!(freed.allocations.count_total, 0);
    assert!(freed.allocations.count_current < 0);
    assert!(freed.allocations.bytes_current < 0);
}

#[test]
fn intentionally_delayed_callback_fails_every_candidate_deadline() {
    let (_, delayed) = observe(|| {
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(1) {
            std::hint::spin_loop();
        }
    });
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        assert!(delayed.duration > budget(SampleRate::new(hz).expect("candidate rate")));
    }
}

#[test]
fn percentile_uses_nearest_rank_without_including_warmup() {
    let mut measured: Vec<_> = (1..=10_000).rev().map(Duration::from_nanos).collect();
    assert_eq!(p999(&mut measured), Some(Duration::from_nanos(9_990)));
    assert_eq!(p999(&mut []), None);
}
