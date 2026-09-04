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
//! `SOUND-INV-024`'s segment lives here too, since `P05-S007b`: a resolved value is the
//! segment's **target**, the slot advances toward it by one add per frame into a buffer the
//! kernel reads per frame, and the segment's last frame reads exactly the target. Every
//! declared policy is `Smoothing::None` today, so every write is still a step; the
//! mechanism is exercised through a crate-private, test-only policy seam until a
//! declaration smooths. What is **not** here: a modulator. The sum is written by
//! [`SlotState::modulate`], which Phase 7's modulation edges will call per quantum and
//! which a test seam calls today, so that the override-leaves-modulation-in-force
//! property is a tested fact rather than a claim about code with no caller.

use crate::node::{ModulationLaw, ModulationSum, ParameterUnit, Smoothing};
use crate::quantities::ParameterValue;
use crate::time::QUANTUM_FRAMES;

/// The bytes one quantum-rate slot's control buffer takes: one `f32` per frame of a quantum.
pub(crate) const RAMP_BUFFER_BYTES: u64 = (QUANTUM_FRAMES as u64) * (size_of::<f32>() as u64);

/// One addressable parameter's layers, law and segment.
///
/// `Copy`, because the renderer keeps one per parameter target in a table it indexes on
/// the audio thread, prepared once. The values keep their types — a validated parameter
/// value stays one, and the sum is a [`ModulationSum`] — because they persist; only the
/// law's arithmetic and the segment's add are over raw floats.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub(crate) struct SlotState {
    law: ModulationLaw,
    unit: ParameterUnit,
    /// The segment length a retarget takes, from the declaration's policy.
    frames: u32,
    /// The segment: what the last frame read, where it is going, how far it has to go, and
    /// the add per frame that takes it there. `remaining == 0` is a step already taken.
    value: ParameterValue,
    target: ParameterValue,
    increment: f32,
    remaining: u32,
    /// Set by an adoption and spent by the next write: `SOUND-INV-018`'s catch-up is that
    /// write for every slot, and an activation never ramps.
    seed_next: bool,
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
    /// A slot at rest: base in place, no override, the law's identity as its sum, and the
    /// segment standing on the base with nothing remaining.
    pub(crate) fn prepared(
        law: ModulationLaw,
        unit: ParameterUnit,
        smoothing: Smoothing,
        base: ParameterValue,
    ) -> Self {
        let mut slot = Self {
            law,
            unit,
            frames: smoothing.frames(),
            value: base,
            target: base,
            increment: 0.0,
            remaining: 0,
            seed_next: false,
            base,
            override_value: None,
            modulation: law.identity(),
        };
        // Through the same composition every later value takes, so a base outside its
        // law's domain — which admission does not refuse — starts clamped as it will read.
        let resolved = slot.resolved();
        slot.value = resolved;
        slot.target = resolved;
        slot
    }

    /// An override-layer write, and the value the kernel reads as a result **now**: the
    /// segment's current value, which is the new target only under a `None` policy.
    ///
    /// Replaces the base only — the modulation stays in force, which is the clause an
    /// automated pitch still bends under.
    pub(crate) fn write_override(&mut self, value: ParameterValue) -> ParameterValue {
        self.override_value = Some(value);
        self.retarget()
    }

    /// Re-derive the resolved value and make it the segment's target.
    ///
    /// ADR-0006 clause 3: a retarget continues from the **current** value, never from the
    /// previous target, so a write mid-segment cannot jump. A seeded slot — one an adoption
    /// marked — takes the value as a step and clears the mark, which is what makes an
    /// activation's catch-up land in force on its first frame. The add per frame is fixed
    /// here so the loop's advance is one add.
    fn retarget(&mut self) -> ParameterValue {
        let target = self.resolved();
        self.target = target;
        let seeded = core::mem::take(&mut self.seed_next);
        if self.frames == 0 || seeded {
            self.value = target;
            self.remaining = 0;
            self.increment = 0.0;
        } else {
            self.remaining = self.frames;
            // In `f64`: two legal finite endpoints of opposite sign can be further apart
            // than `f32` holds, and an overflowed delta would make the first advance
            // saturate to the target instead of ramping. The per-frame add is narrowed
            // once the division has brought it back into range; only a one-frame segment
            // between such endpoints cannot be, and its one frame is the target by rule.
            let delta = f64::from(target.as_f32()) - f64::from(self.value.as_f32());
            self.increment = (delta / f64::from(self.frames)) as f32;
        }
        self.value
    }

    /// Mark the slot so that its next write is a step: an adoption's catch-up seeds the
    /// segment with current equal to target and nothing remaining (`SOUND-INV-024`).
    pub(crate) fn seed(&mut self) {
        self.seed_next = true;
    }

    /// Advance the segment through one quantum, writing the value each frame reads.
    ///
    /// **Before the kernel reads**: frame `k` of a segment of `N` reads
    /// `start + (target − start) × (k + 1) / N`, as one add per frame, and the segment's
    /// last frame reads exactly the target rather than the sum's rounding of it — V1's
    /// own filter convention. Every later frame holds the target. A slot with nothing
    /// remaining fills its value.
    pub(crate) fn advance(&mut self, out: &mut [f32]) {
        for frame in out.iter_mut() {
            if self.remaining > 0 {
                self.remaining -= 1;
                self.value = if self.remaining == 0 {
                    self.target
                } else {
                    ParameterValue::saturating(self.value.as_f32() + self.increment)
                };
            }
            *frame = self.value.as_f32();
        }
    }

    /// What the kernel reads on the next frame, without advancing.
    #[cfg(test)]
    pub(crate) const fn current(&self) -> ParameterValue {
        self.value
    }

    /// Override the declared policy with a segment length in frames.
    ///
    /// Test-only, until a declaration smooths: every declared policy is `None`, and this is
    /// what lets the segment's own facts be tested rather than claimed.
    #[cfg(test)]
    pub(crate) fn smooth_over(&mut self, frames: u32) {
        self.frames = frames;
    }

    /// The modulation sum, and the value the kernel reads as a result.
    ///
    /// Crate-private and compiled for tests only: Phase 7's modulators are its callers,
    /// and until then the render tests are, through `PreparedRenderer::modulate`.
    #[cfg(test)]
    pub(crate) fn modulate(&mut self, sum: ModulationSum) -> ParameterValue {
        self.modulation = sum;
        self.retarget()
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
