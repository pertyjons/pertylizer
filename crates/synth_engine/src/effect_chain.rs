//! Effect chain processing for per-instrument insert effects.
//!
//! Each instrument owns its own EffectChain for independent effect processing.
//! This enables loading patches that only affect that instrument's sound.
//!
//! ## Design: Unified ChainSlot
//!
//! The chain uses a unified `ChainSlot` enum that can hold either effects or
//! visualizers. This enables flexible ordering where you can place a visualizer
//! (e.g., Oscilloscope) anywhere in the chain - between effects, before them,
//! or after them.

use std::sync::Arc;

use crate::commands::ModuleId;
use crate::visualizers::VisualizationBuffer;
use synth_core::ModuleType;
use synth_core::{AudioBuffer, AudioEffect, ProcessContext, SampleRate};

// Maximum buffer size in samples (mono) — the engine-wide block ceiling.
use synth_core::MAX_BLOCK_SIZE as MAX_BUFFER_SIZE;

// ============================================================================
// EnabledState - Descriptive enum for on/off state
// ============================================================================

/// Enabled state for effects and visualizers.
///
/// Using an enum instead of bool makes the code more readable:
/// `EnabledState::Active` vs `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnabledState {
    /// The slot is active and will process audio.
    #[default]
    Active,
    /// The slot is bypassed and will not process audio.
    Bypassed,
}

impl EnabledState {
    /// Check if the slot is active (enabled).
    #[inline]
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the slot is bypassed (disabled).
    #[inline]
    #[must_use]
    pub const fn is_bypassed(self) -> bool {
        matches!(self, Self::Bypassed)
    }

    /// Toggle the enabled state.
    #[inline]
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Active => Self::Bypassed,
            Self::Bypassed => Self::Active,
        }
    }
}

impl From<bool> for EnabledState {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::Active
        } else {
            Self::Bypassed
        }
    }
}

/// Effect slot in the effect chain.
pub struct EffectSlot {
    /// Unique module ID.
    pub module_id: ModuleId,
    /// The effect processor.
    pub effect: Box<dyn AudioEffect>,
    /// Module type for type-safe lookup.
    pub module_type: ModuleType,
    /// Whether the effect is active or bypassed.
    pub state: EnabledState,
}

/// Visualizer slot with shared buffer.
pub struct VisualizerSlot {
    /// Unique module ID.
    pub module_id: ModuleId,
    /// The visualizer processor.
    pub visualizer: Box<dyn AudioEffect>,
    /// Shared buffer for visualization data.
    pub buffer: Arc<VisualizationBuffer>,
    /// Whether the visualizer is active or bypassed.
    pub state: EnabledState,
}

/// A slot in the effect chain that can be either an effect or a visualizer.
///
/// This unification allows flexible ordering - you can place an Oscilloscope
/// between a Delay and a Reverb to see the signal at that point.
pub enum ChainSlot {
    /// An audio effect that modifies the signal
    Effect(EffectSlot),
    /// A visualizer that captures audio data without modifying it
    Visualizer(VisualizerSlot),
}

impl ChainSlot {
    /// Get the module ID of this slot.
    #[must_use]
    pub fn module_id(&self) -> ModuleId {
        match self {
            Self::Effect(slot) => slot.module_id,
            Self::Visualizer(slot) => slot.module_id,
        }
    }

    /// Get the enabled state of this slot.
    #[must_use]
    pub fn state(&self) -> EnabledState {
        match self {
            Self::Effect(slot) => slot.state,
            Self::Visualizer(slot) => slot.state,
        }
    }

    /// Check if this slot is active (enabled).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state().is_active()
    }

    /// Set enabled state.
    pub fn set_state(&mut self, state: EnabledState) {
        match self {
            Self::Effect(slot) => slot.state = state,
            Self::Visualizer(slot) => slot.state = state,
        }
    }

    /// Set enabled state from boolean (for backwards compatibility).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.set_state(EnabledState::from(enabled));
    }
}

/// Per-instrument effect chain that manages insert effects and visualizers.
///
/// Each instrument has its own EffectChain, enabling independent effect
/// processing. This ensures that loading a patch only affects that
/// instrument's sound, solving collision issues.
///
/// Effects and visualizers can be freely intermixed in the chain for
/// flexible signal monitoring at any point.
pub struct EffectChain {
    /// Unified slots for effects and visualizers in processing order
    slots: Vec<ChainSlot>,
    /// Working buffer for effect processing
    working_buffer: AudioBuffer,
}

impl EffectChain {
    /// Create a new empty effect chain.
    ///
    /// Effects and visualizers are added dynamically when loading patches or via GUI.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            // Pre-allocate for max stereo interleaved size to avoid
            // heap allocation in the real-time audio thread process().
            working_buffer: AudioBuffer::new(MAX_BUFFER_SIZE * 2),
        }
    }

    /// Find an effect slot by its module type.
    pub fn find_effect_by_type(&mut self, module_type: ModuleType) -> Option<&mut EffectSlot> {
        self.slots.iter_mut().find_map(|slot| {
            if let ChainSlot::Effect(effect_slot) = slot
                && effect_slot.module_type == module_type
            {
                return Some(effect_slot);
            }
            None
        })
    }

    /// Find an effect slot by its module ID.
    pub fn find_effect_by_id(&mut self, module_id: ModuleId) -> Option<&mut EffectSlot> {
        self.slots.iter_mut().find_map(|slot| {
            if let ChainSlot::Effect(effect_slot) = slot
                && effect_slot.module_id == module_id
            {
                return Some(effect_slot);
            }
            None
        })
    }

    /// Add an effect instance to the end of the chain.
    pub fn add_effect(
        &mut self,
        id: ModuleId,
        mut effect: Box<dyn AudioEffect>,
        sample_rate: SampleRate,
    ) {
        effect.set_sample_rate(sample_rate);
        let module_type = effect.module_type();

        self.slots.push(ChainSlot::Effect(EffectSlot {
            module_id: id,
            module_type,
            effect,
            state: EnabledState::Active,
        }));
    }

    /// Remove an effect by ID.
    pub fn remove_effect(&mut self, id: ModuleId) {
        self.slots
            .retain(|slot| !matches!(slot, ChainSlot::Effect(e) if e.module_id == id));
    }

    /// Add a visualizer to the end of the chain.
    pub fn add_visualizer(
        &mut self,
        id: ModuleId,
        visualizer: Box<dyn AudioEffect>,
        buffer: Arc<VisualizationBuffer>,
    ) {
        self.slots.push(ChainSlot::Visualizer(VisualizerSlot {
            module_id: id,
            visualizer,
            buffer,
            state: EnabledState::Active,
        }));
    }

    /// Remove a visualizer by ID.
    pub fn remove_visualizer(&mut self, id: ModuleId) {
        self.slots
            .retain(|slot| !matches!(slot, ChainSlot::Visualizer(v) if v.module_id == id));
    }

    /// Clear all effects and visualizers.
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    /// Check if the chain is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Reset all effects (visualizers don't need reset).
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            if let ChainSlot::Effect(effect_slot) = slot {
                effect_slot.effect.reset();
            }
        }
    }

    /// Get mutable access to all slots.
    pub fn slots_mut(&mut self) -> &mut Vec<ChainSlot> {
        &mut self.slots
    }

    /// Get immutable access to all slots.
    pub fn slots(&self) -> &[ChainSlot] {
        &self.slots
    }

    /// Get the ordered list of **effect** module IDs in the chain (processing
    /// order). Visualizers are deliberately excluded: they are passive taps that
    /// always observe the final post-chain signal regardless of their slot
    /// position (see `process_visualizers`), so they are not part of the chain's
    /// ordering discipline and must not be grouped with effects by the GUI.
    #[must_use]
    pub fn slot_order(&self) -> Vec<ModuleId> {
        self.slots
            .iter()
            .filter(|s| matches!(s, ChainSlot::Effect(_)))
            .map(ChainSlot::module_id)
            .collect()
    }

    /// Move a slot one step earlier in the chain (toward index 0).
    /// Returns `true` if the move was performed, `false` if already first or not found.
    pub fn move_slot_up(&mut self, id: ModuleId) -> bool {
        if let Some(idx) = self.slots.iter().position(|s| s.module_id() == id)
            && idx > 0
        {
            self.slots.swap(idx, idx - 1);
            return true;
        }
        false
    }

    /// Move a slot one step later in the chain (toward end).
    /// Returns `true` if the move was performed, `false` if already last or not found.
    pub fn move_slot_down(&mut self, id: ModuleId) -> bool {
        if let Some(idx) = self.slots.iter().position(|s| s.module_id() == id)
            && idx + 1 < self.slots.len()
        {
            self.slots.swap(idx, idx + 1);
            return true;
        }
        false
    }

    /// Reorder the chain so listed IDs appear first, in the given order.
    ///
    /// Slots whose IDs appear in `order` are moved to the front in that order.
    /// Slots not mentioned keep their existing relative order at the end.
    /// Unknown IDs in `order` are ignored.
    pub fn set_slot_order(&mut self, order: &[ModuleId]) {
        let mut new_slots: Vec<ChainSlot> = Vec::with_capacity(self.slots.len());
        for id in order {
            if let Some(pos) = self.slots.iter().position(|s| s.module_id() == *id) {
                new_slots.push(self.slots.remove(pos));
            }
        }
        new_slots.append(&mut self.slots);
        self.slots = new_slots;
    }

    /// Process the effect chain (effects only, visualizers are skipped).
    ///
    /// The `mix_buffer` contains interleaved stereo audio and is modified in place.
    /// Visualizers are processed separately via `process_visualizers()` so they
    /// can run after the effect chain to show the final signal.
    pub fn process(&mut self, mix_buffer: &mut AudioBuffer, context: &ProcessContext<'_>) {
        // Resize to active frame count. This never exceeds MAX_BUFFER_SIZE * 2
        // which was pre-allocated in the constructor, so no heap allocation occurs.
        debug_assert!(
            context.samples.as_usize() * 2 <= MAX_BUFFER_SIZE * 2,
            "Buffer size {} exceeds pre-allocated capacity {}",
            context.samples.as_usize() * 2,
            MAX_BUFFER_SIZE * 2
        );
        self.working_buffer.resize(context.samples.as_usize() * 2);

        for slot in &mut self.slots {
            match slot {
                ChainSlot::Effect(effect_slot) => {
                    if effect_slot.state.is_active() {
                        // Copy mix to working buffer
                        self.working_buffer.copy_from(mix_buffer);

                        // Process effect (in-place)
                        effect_slot.effect.process(
                            self.working_buffer.as_slice(),
                            mix_buffer.as_mut_slice(),
                            context,
                        );
                    }
                }
                ChainSlot::Visualizer(_) => {
                    // Visualizers are processed via process_visualizers()
                    continue;
                }
            }
        }
    }

    /// Process visualizers only (called after the effect chain to capture the final signal).
    ///
    /// Visualizers capture audio data without modifying the signal.
    pub fn process_visualizers(&mut self, mix_buffer: &AudioBuffer) {
        for slot in &mut self.slots {
            if let ChainSlot::Visualizer(viz_slot) = slot
                && viz_slot.state.is_active()
            {
                viz_slot.buffer.write_interleaved(mix_buffer.as_slice());
                viz_slot
                    .buffer
                    .update_levels_interleaved(mix_buffer.as_slice());
            }
        }
    }
}

impl Default for EffectChain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleRate;

    fn make_id(module_type: ModuleType, instance: u16) -> ModuleId {
        ModuleId::new(module_type, instance)
    }

    fn populate(chain: &mut EffectChain, ids: &[ModuleId]) {
        let sr = SampleRate::new(44100.0);
        for id in ids {
            chain.add_effect(*id, Box::new(synth_modules::Chorus::new()), sr);
        }
    }

    #[test]
    fn set_slot_order_reorders_to_requested_sequence() {
        let a = make_id(ModuleType::Chorus, 1);
        let b = make_id(ModuleType::Chorus, 2);
        let c = make_id(ModuleType::Chorus, 3);

        let mut chain = EffectChain::new();
        populate(&mut chain, &[a, b, c]);
        assert_eq!(chain.slot_order(), vec![a, b, c]);

        chain.set_slot_order(&[c, a, b]);
        assert_eq!(chain.slot_order(), vec![c, a, b]);
    }

    #[test]
    fn set_slot_order_appends_unmentioned_slots() {
        let a = make_id(ModuleType::Chorus, 1);
        let b = make_id(ModuleType::Chorus, 2);
        let c = make_id(ModuleType::Chorus, 3);

        let mut chain = EffectChain::new();
        populate(&mut chain, &[a, b, c]);

        // Only mention c; a and b should remain in their original order at end.
        chain.set_slot_order(&[c]);
        assert_eq!(chain.slot_order(), vec![c, a, b]);
    }

    #[test]
    fn set_slot_order_ignores_unknown_ids() {
        let a = make_id(ModuleType::Chorus, 1);
        let b = make_id(ModuleType::Chorus, 2);
        let unknown = make_id(ModuleType::Chorus, 99);

        let mut chain = EffectChain::new();
        populate(&mut chain, &[a, b]);

        chain.set_slot_order(&[unknown, b, a]);
        assert_eq!(chain.slot_order(), vec![b, a]);
    }

    #[test]
    fn set_slot_order_on_empty_chain_is_noop() {
        let mut chain = EffectChain::new();
        chain.set_slot_order(&[make_id(ModuleType::Chorus, 1)]);
        assert!(chain.is_empty());
    }
}
