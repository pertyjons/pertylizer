//! Module-agnostic host for a rack of YAMS control scripts.
//!
//! A [`ScriptHost`] co-locates the immutable `Arc<BoundScript>` slots with their
//! per-voice mutable [`RegisterFile`] state, so **any** module can host scripts
//! by embedding one — not just the Mod Matrix. Because the host lives *in* the
//! module, each per-voice module clone (via `PolyModule::box_clone`) carries its
//! own state; voice cloning and stealing reuse the module's own registers
//! instead of a special case kept on the `Voice`.
//!
//! Source resolution stays outside the host (the engine reads the graph and
//! fills a `sources` slice), which dissolves the `&graph` / `&mut regs` borrow
//! conflict: resolve first, then [`eval_slot`](ScriptHost::eval_slot) borrows one
//! slot's script immutably and its registers mutably at once (disjoint arrays).

use std::sync::Arc;

use crate::script::{BoundScript, EvalContext, RegisterFile};

/// Script slots per host rack. The Mod Matrix maps its
/// [`MAX_MOD_MATRIX_SLOTS`](crate::MAX_MOD_MATRIX_SLOTS) routings 1:1 onto these;
/// the Script module (Phase 2) uses the first few as output ports.
pub const SCRIPT_HOST_SLOTS: usize = 16;

/// Global PRNG salt shared by every control script. Folded with a
/// per-(voice, module, slot) index so simultaneous voices and slots stay
/// decorrelated while a retrigger of the same voice re-seeds identically
/// (deterministic). See [`ScriptHost::set_voice_index`].
pub const SCRIPT_PRNG_SEED: u64 = 0x59A5_5EED_2C0D_E001;

/// A rack of [`SCRIPT_HOST_SLOTS`] script slots plus their per-voice state.
///
/// Embedded in a script-hosting module. `slots` and `regs` are kept as separate
/// arrays (not a single `[(Option<Arc>, RegisterFile)]`) so
/// [`eval_slot`](Self::eval_slot) can split-borrow one slot's immutable script
/// and its mutable registers simultaneously.
#[derive(Debug, Clone)]
pub struct ScriptHost {
    /// Immutable compiled scripts, shared across voices behind an `Arc`.
    slots: [Option<Arc<BoundScript>>; SCRIPT_HOST_SLOTS],
    /// Per-voice mutable state, one [`RegisterFile`] per slot.
    regs: [RegisterFile; SCRIPT_HOST_SLOTS],
    /// Stable per-(voice, module) seed base, set at voice allocation and used to
    /// re-seed `regs` on note-on. Survives voice stealing (it is not derived from
    /// transient note state).
    voice_index: u32,
}

impl ScriptHost {
    /// A fresh, empty rack (no scripts, zeroed state, voice index 0).
    #[must_use]
    pub fn new() -> Self {
        Self {
            // `Option<Arc<_>>` is not `Copy`, so the `[x; N]` form is unavailable.
            slots: std::array::from_fn(|_| None),
            regs: std::array::from_fn(|slot| RegisterFile::new(slot as u32, SCRIPT_PRNG_SEED)),
            voice_index: 0,
        }
    }

    /// Set the stable per-(voice, module) seed base and re-seed every slot's PRNG.
    ///
    /// Called once per voice at allocation (after the voice id is assigned). The
    /// caller folds the voice id with the module identity so two different
    /// script-hosting modules in the same voice still decorrelate.
    pub fn set_voice_index(&mut self, voice_index: u32) {
        self.voice_index = voice_index;
        self.reseed();
    }

    /// Reset all slot state for a new note: zero the state cells and re-seed each
    /// PRNG from the stored `voice_index` (deterministic retrigger, decorrelated
    /// voices). Call from the host module's `note_on`.
    pub fn note_on(&mut self) {
        self.reseed();
    }

    /// Re-seed every slot's [`RegisterFile`] from the stored `voice_index`.
    fn reseed(&mut self) {
        let base = self.voice_index;
        for (slot, regs) in self.regs.iter_mut().enumerate() {
            let idx = base
                .wrapping_mul(SCRIPT_HOST_SLOTS as u32)
                .wrapping_add(slot as u32);
            regs.reset(idx, SCRIPT_PRNG_SEED);
        }
    }

    /// The installed scripts, indexed by slot (`None` = empty slot).
    #[must_use]
    pub fn slots(&self) -> &[Option<Arc<BoundScript>>] {
        &self.slots
    }

    /// Install (or clear, with `None`) one slot's script, **returning** the script
    /// it replaced so the caller can drop it off the audio thread (the install
    /// itself runs on the audio thread via the command drain). Out-of-range slots
    /// are a no-op that hands the script straight back.
    #[must_use = "the replaced script must be dropped off the audio thread"]
    pub fn set_slot(
        &mut self,
        slot: usize,
        script: Option<Arc<BoundScript>>,
    ) -> Option<Arc<BoundScript>> {
        match self.slots.get_mut(slot) {
            Some(cell) => std::mem::replace(cell, script),
            None => script,
        }
    }

    /// Evaluate one slot from externally-resolved `sources` plus the slot's own
    /// persistent state, returning the sanitized output. `None` if the slot is
    /// empty or out of range. Real-time safe (no allocation, no locks).
    pub fn eval_slot(&mut self, slot: usize, sources: &[f32], ctx: &EvalContext) -> Option<f32> {
        // Split borrow: `slots[slot]` (immutable script) and `regs[slot]`
        // (mutable state) are distinct arrays, so both borrows coexist.
        let script = self.slots.get(slot)?.as_ref()?;
        let regs = self.regs.get_mut(slot)?;
        Some(script.script.eval(sources, regs, ctx))
    }
}

impl Default for ScriptHost {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::{CompiledScript, Op};

    fn const_script(v: f32) -> Arc<BoundScript> {
        Arc::new(BoundScript::new(
            CompiledScript::new(vec![Op::PushConst(0)], vec![v], 0, 0),
            Vec::new(),
            format!("out = {v}"),
        ))
    }

    #[test]
    fn set_slot_returns_replaced_and_evaluates() {
        let mut host = ScriptHost::new();
        assert!(host.slots().iter().all(Option::is_none));

        // First install returns nothing (slot was empty).
        assert!(host.set_slot(3, Some(const_script(0.5))).is_none());
        let ctx = EvalContext::new(750.0);
        assert_eq!(host.eval_slot(3, &[], &ctx), Some(0.5));
        // Empty slot evaluates to None.
        assert_eq!(host.eval_slot(0, &[], &ctx), None);

        // Re-install returns the previous script for deferred drop.
        let replaced = host.set_slot(3, Some(const_script(0.25)));
        assert_eq!(
            replaced.as_deref().map(|b| b.source.as_str()),
            Some("out = 0.5")
        );
        assert_eq!(host.eval_slot(3, &[], &ctx), Some(0.25));

        // Out-of-range slot hands the script straight back, never panics.
        assert!(
            host.set_slot(SCRIPT_HOST_SLOTS, Some(const_script(1.0)))
                .is_some()
        );
    }

    #[test]
    fn voice_index_decorrelates_then_note_on_redetermines() {
        // A `rand`-driven slot: different voice indices must diverge; a note-on
        // re-seed must reproduce the same first value (deterministic retrigger).
        let rand_script = Arc::new(BoundScript::new(
            CompiledScript::new(
                vec![Op::PushConst(0), Op::PushConst(1), Op::Rand],
                vec![0.0, 1.0],
                0,
                0,
            ),
            Vec::new(),
            "out = rand()".to_string(),
        ));
        let ctx = EvalContext::new(750.0);

        let mut a = ScriptHost::new();
        let mut b = ScriptHost::new();
        let _ = a.set_slot(0, Some(Arc::clone(&rand_script)));
        let _ = b.set_slot(0, Some(rand_script));
        a.set_voice_index(1);
        b.set_voice_index(2);

        let a0 = a.eval_slot(0, &[], &ctx).unwrap();
        let b0 = b.eval_slot(0, &[], &ctx).unwrap();
        assert!(a0 != b0, "distinct voice indices must decorrelate");

        a.note_on();
        let a0_again = a.eval_slot(0, &[], &ctx).unwrap();
        assert!(
            (a0 - a0_again).abs() < 1e-6,
            "note-on re-seed is deterministic"
        );
    }
}
