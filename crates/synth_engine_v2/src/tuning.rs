//! The prepared tuning a plan resolves a key through.
//!
//! [ADR-0025](../../plans/v2/decisions/ADR-0025-tuning-representation-and-ownership.md) selects
//! a pre-tuning event contract: a note names a key, and what frequency that key is belongs
//! here. `SOUND-INV-021` states what this must do.
//!
//! # Why this is a table rather than a formula
//!
//! Equal temperament is a formula; a Scala definition is not, and the point of the decision is
//! that V2 renders whichever the project selects. A table is also what makes the lookup
//! real-time legal — one index, no transcendental, no branch.
//!
//! `SOUND-INV-013` permits it: "A dependency may still supply a value, a table, or a
//! mathematical primitive that is not a kernel." The table comes from `synth_core`, which owns
//! the authored definitions and the Scala parser; what this crate owns is the validation, the
//! digest, and the resolution.
//!
//! # Total by construction, and what that does and does not buy
//!
//! `SOUND-INV-021` requires a prepared tuning to answer for every key in `0..=127`, because a
//! node that cannot resolve one has no safe answer on the audio thread — it can neither
//! allocate a diagnostic nor choose a frequency. `synth_core::TuningTable` is `[Hertz; 128]`,
//! so totality is **structural**: there is always an answer.
//!
//! What preparation adds is that every answer is **usable** — finite and above zero. What it
//! cannot add is that every answer was **authored**. `TuningTable` carries no record of which
//! keys a definition actually mapped, and `from_scala` extrapolates an entry for every key from
//! the reference, so a KBM that maps one key arrives here as 128 finite frequencies. An
//! independent review found this module claiming to refuse a partial mapping; it cannot, and
//! `preparation_validates_values_and_not_definedness` records the gap with a measurement rather
//! than leaving it to be discovered.
//!
//! Completing a partial definition therefore belongs to the authored model, which is Phase
//! 10A's. Nothing in Phase 4 supplies a Scala definition at all — it uses
//! [`PreparedTuning::equal_temperament`], which is total by derivation — so the gap is
//! recorded rather than reachable here.

use synth_core::tuning::TuningTable;

use crate::quantities::{Frequency, KeyIdentity};
use thiserror::Error;

/// A prepared tuning that refused to be built.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum TuningError {
    /// One key resolves to a frequency the renderer cannot use.
    ///
    /// A Scala definition can produce a non-finite or non-positive value through an arithmetic
    /// path the parser does not police. Reaching a phase accumulator, a `NaN` is unrecoverable
    /// — every later sample is one — and a frequency at or below zero is a note that does not
    /// sound or runs backwards, neither of which a key is asking for.
    #[error("{key} resolves to {frequency} Hz, which is not a usable frequency")]
    KeyNotUsable {
        /// The key whose entry was refused.
        key: KeyIdentity,
        /// What it resolved to.
        frequency: f32,
    },
}

/// A digest of a prepared tuning's content.
///
/// `SOUND-INV-021` requires derivation to be deterministic and to carry a digest, so that two
/// preparations of one definition are the same table and a report can say so. Computed over the
/// bit patterns of all 128 frequencies, so two tables agree exactly or not at all — a
/// tolerance here would make two audibly different scales report as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use]
pub struct TuningDigest(u64);

impl TuningDigest {
    /// The raw digest.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for TuningDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tuning {:016x}", self.0)
    }
}

/// The immutable key-to-frequency mapping one execution scope resolves through.
///
/// Shared rather than copied: `SOUND-INV-021` requires every node of one scope to reference the
/// same one, so a scope cannot resolve two keys two ways.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct PreparedTuning {
    frequencies: [Frequency; 128],
    digest: TuningDigest,
}

impl PreparedTuning {
    /// Prepare an authored table, refusing any key whose frequency is unusable.
    ///
    /// Runs off the audio thread. Every later resolution is an index, which is what makes the
    /// refusal worth doing here: this is the last place a diagnostic can be produced.
    ///
    /// It validates values, not definedness — see the module header for why it cannot do the
    /// second, and who owns it instead.
    pub fn prepare(table: &TuningTable) -> Result<Self, TuningError> {
        // `Frequency::ZERO` is a placeholder the loop below overwrites for every entry; the
        // array is fully written before it is read, and the compiler cannot see that without
        // an initial value.
        let mut frequencies = [Frequency::ZERO; 128];
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for index in 0..128_u8 {
            let key = KeyIdentity::new(index).unwrap_or(KeyIdentity::LOWEST);
            let hz = table
                .note_to_freq(synth_core::MidiNote::new(index))
                .as_f32();
            if !hz.is_finite() || hz <= 0.0 {
                return Err(TuningError::KeyNotUsable { key, frequency: hz });
            }
            frequencies[key.as_index()] =
                Frequency::new(hz).map_err(|_| TuningError::KeyNotUsable { key, frequency: hz })?;
            // FNV-1a over the bit pattern. Deterministic, allocation-free, and it needs no
            // dependency — `sha2` is a dev-dependency here and a digest that only exists in
            // test builds is not one the report can carry.
            for byte in hz.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        Ok(Self {
            frequencies,
            digest: TuningDigest(hash),
        })
    }

    /// Standard twelve-tone equal temperament, for a plan that selects no tuning.
    ///
    /// Not a default in the sense of a fallback: a plan states which tuning it uses, and this
    /// is the one nearly every project states. It is built through `prepare` like any other, so
    /// it carries a real digest rather than a special case.
    pub fn equal_temperament() -> Result<Self, TuningError> {
        Self::prepare(&TuningTable::equal_temperament())
    }

    /// The frequency a key resolves to.
    ///
    /// **Real-time legal**: one bounds-free index into a fixed array, because `KeyIdentity` is
    /// `0..=127` by construction and the array is 128 long. There is no failure to report and
    /// so no `Result` to unwrap on the audio thread.
    pub const fn frequency_of(&self, key: KeyIdentity) -> Frequency {
        self.frequencies[key.as_index()]
    }

    /// What this table is, for the resource report and for comparing two preparations.
    pub const fn digest(&self) -> TuningDigest {
        self.digest
    }

    /// The bytes one prepared table occupies.
    ///
    /// Charged **once** to a plan's immutable prepared total however many nodes reference it,
    /// which is what makes a second scale visible in the report as something other than a
    /// second node.
    #[must_use]
    pub const fn prepared_bytes() -> u64 {
        size_of::<Self>() as u64
    }
}
