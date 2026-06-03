//! Deterministic, RT-safe hashing for audio-thread randomness.
//!
//! The audio path never calls an RNG (no state, no syscalls); anything that
//! needs "randomness" hashes stable inputs (tick, note id, step index) through
//! this finalizer instead, so playback and offline renders are reproducible.

/// Golden-ratio odd constant — the SplitMix64 increment.
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64 finalizer: a deterministic 64-bit mix. Pure arithmetic, RT-safe,
/// reproducible for a given seed. Mixes structured input thoroughly, so
/// callers may feed a plain xor of their key fields.
///
/// Shared by the sequencer-side probability roll (`synth_engine`) and the
/// arpeggiator's `Random` mode (`synth_sequencer`) — one copy, so their
/// determinism guarantees can never drift apart.
#[must_use]
pub fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(SPLITMIX_GAMMA);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic pseudo-random value in `[0, 1)` from a 64-bit seed
/// ([`splitmix64`] reduced to a unit float).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn splitmix64_unit(seed: u64) -> f32 {
    // Top 24 bits → [0, 1); 2^24 = 16_777_216 (exactly representable in f32).
    ((splitmix64(seed) >> 40) as f32) / 16_777_216.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_is_in_range_and_reproducible() {
        for seed in [0u64, 1, 42, 1_000_000, u64::MAX, SPLITMIX_GAMMA] {
            let v = splitmix64_unit(seed);
            assert!((0.0..1.0).contains(&v), "seed {seed} → {v} out of [0,1)");
            assert_eq!(v, splitmix64_unit(seed), "must be reproducible");
        }
    }

    #[test]
    fn mix_decorrelates_adjacent_seeds() {
        assert_ne!(splitmix64(1), splitmix64(2));
        assert_ne!(splitmix64(0), 0);
    }
}
