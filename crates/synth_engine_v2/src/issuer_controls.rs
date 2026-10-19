//! Injected local counters exercise actual issuer arithmetic without spending global IDs.
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn assert_u64_issuer_exhaustion(issue: fn(&AtomicU64) -> Option<u64>) {
    let start = AtomicU64::new(1);
    assert_eq!(issue(&start), Some(1));
    assert_eq!(issue(&start), Some(2));
    let counter = AtomicU64::new(u64::MAX - 8);
    let start_together = std::sync::Barrier::new(32);
    let mut issued = std::thread::scope(|scope| {
        let joins: Vec<_> = (0..32)
            .map(|_| {
                scope.spawn(|| {
                    start_together.wait();
                    issue(&counter)
                })
            })
            .collect();
        joins
            .into_iter()
            .filter_map(|join| join.join().expect("issuer thread"))
            .collect::<Vec<_>>()
    });
    issued.sort_unstable();
    assert_eq!(issued, (u64::MAX - 8..u64::MAX).collect::<Vec<_>>());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    for _ in 0..32 {
        assert_eq!(issue(&counter), None);
    }
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}
