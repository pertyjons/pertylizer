//! The parameter slot: where a parameter's layers become the one value a kernel reads.
//!
//! `SOUND-INV-023` in code. A slot holds the layers — the stored base, an override that
//! replaces it, the modulation sum — and composes them under its declared law, then the
//! type's clamp, into a resolved value. It is the only place a law's arithmetic runs: a
//! kernel is handed the result and composes nothing, and `render_loop_purity` holds
//! `node/kernels.rs` to that by scan. The module is inside the render loop's scanned
//! region because it runs on the audio thread — an override write arrives as an event
//! and is composed where it is applied.
//!
//! What is **not** here yet, by `P05-S007a`'s cut: `SOUND-INV-024`'s segment. Every
//! resolved value is a step today; the segment is `P05-S007b`'s and lands in this struct.
//! Nor is a modulator: the sum is written by [`SlotState::modulate`], which Phase 7's
//! modulation edges will call per quantum and which a crate-private test seam calls
//! today, so that the override-leaves-modulation-in-force property is a tested fact
//! rather than a claim about code with no caller.

use crate::node::{ModulationLaw, ModulationSum, ParameterUnit};
use crate::quantities::ParameterValue;

/// One addressable parameter's layers and law.
///
/// `Copy` and three values wide beyond the two enums, because the renderer keeps one per
/// parameter target in a table it indexes on the audio thread, prepared once. The values
/// keep their types — a validated parameter value stays one, and the sum is a
/// [`ModulationSum`] — because they persist; only the law's arithmetic is over raw floats.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub(crate) struct SlotState {
    law: ModulationLaw,
    unit: ParameterUnit,
    /// The stored base: the value the node was prepared with.
    base: ParameterValue,
    /// The override layer, which replaces the base while present. A caller's
    /// `SetParameter`, a note's gate or magnitude, and `SOUND-INV-018`'s catch-up all
    /// write here, and only here: none of them is a modulator.
    override_value: Option<ParameterValue>,
    /// The modulation sum `m`, in the law's units; the law's identity until a modulator
    /// writes it.
    modulation: ModulationSum,
}

impl SlotState {
    /// A slot at rest: base in place, no override, the law's identity as its sum.
    pub(crate) const fn prepared(
        law: ModulationLaw,
        unit: ParameterUnit,
        base: ParameterValue,
    ) -> Self {
        Self {
            law,
            unit,
            base,
            override_value: None,
            modulation: law.identity(),
        }
    }

    /// An override-layer write, and the value the kernel reads as a result.
    ///
    /// Replaces the base only — the modulation stays in force, which is the clause an
    /// automated pitch still bends under.
    pub(crate) fn write_override(&mut self, value: ParameterValue) -> ParameterValue {
        self.override_value = Some(value);
        self.resolved()
    }

    /// The modulation sum, and the value the kernel reads as a result.
    ///
    /// Crate-private and compiled for tests only: Phase 7's modulators are its callers,
    /// and until then the render tests are, through `PreparedRenderer::modulate`.
    #[cfg(test)]
    pub(crate) fn modulate(&mut self, sum: ModulationSum) -> ParameterValue {
        self.modulation = sum;
        self.resolved()
    }

    /// The layers composed: the override where present, else the base; the law; the clamp.
    pub(crate) fn resolved(&self) -> ParameterValue {
        let base = self.override_value.unwrap_or(self.base);
        let composed = self.law.resolve(base, self.modulation);
        ParameterValue::saturating(self.unit.hold_to_domain(composed))
    }
}

impl ParameterUnit {
    /// The type's clamp, which `SOUND-INV-023` applies after the law's arithmetic.
    ///
    /// Exactly the domain the quantity type states and nothing narrower: a level is held to
    /// `[0, 1]` because [`crate::quantities::NormalizedLevel`] refuses anything else, and a
    /// frequency, an amplitude and a gate are any finite value because their types are. The
    /// finite bound itself is [`ParameterValue::saturating`]'s, applied by the slot after
    /// this. `f32::clamp` panics only on an inverted or `NaN` range, and both bounds here are
    /// literals — the purity scan holds every `clamp` in the region to literal bounds for
    /// exactly that reason.
    pub(crate) fn hold_to_domain(self, value: f32) -> f32 {
        match self {
            Self::NormalizedLevel => value.clamp(0.0, 1.0),
            Self::Hertz | Self::LinearAmplitude | Self::Gate => value,
        }
    }
}

impl ModulationLaw {
    /// The record's arithmetic, over a base `b` and a modulation sum `m` in the law's units.
    /// Returns the raw intermediate: the type's clamp and the finite bound follow in
    /// [`SlotState::resolved`], which is the only caller.
    ///
    /// Exactly the eight expressions ADR-0007 clause 1 states, in the order it states them,
    /// and nothing else: no per-target scale, no second threshold, no smoothing. `resolve`
    /// is defined here rather than beside the enum so that the one place a law is applied is
    /// inside the scanned region, and the scan can hold that the kernels' file never calls
    /// it.
    pub(crate) fn resolve(self, base: ParameterValue, modulation: ModulationSum) -> f32 {
        let (base, modulation) = (base.as_f32(), modulation.as_f32());
        match self {
            Self::NormalizedAdditive => (base + modulation).clamp(0.0, 1.0),
            Self::BipolarAdditive => (base + modulation).clamp(-1.0, 1.0),
            Self::SemitoneAdditive => base * (modulation / 12.0).exp2(),
            Self::DecibelAdditive => base * 10.0_f32.powf(modulation / 20.0),
            Self::PhysicalLinearAdditive => base + modulation,
            Self::MultiplicativeGain => base * modulation,
            Self::ThresholdedBoolean => {
                if base + modulation >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::NotModulatable => base,
        }
    }
}
