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

use crate::engine::commands::ModuleId;
use crate::engine::params::ModuleType;
use crate::modules::{AudioBuffer, AudioEffect, ProcessContext, SampleRate};
use crate::visualizers::VisualizationBuffer;

/// Effect slot in the effect chain.
pub struct EffectSlot {
    /// Unique module ID
    pub module_id: ModuleId,
    /// The effect processor
    pub effect: Box<dyn AudioEffect>,
    /// Module type for type-safe lookup
    pub module_type: ModuleType,
    /// Whether the effect is enabled
    pub enabled: bool,
}

/// Visualizer slot with shared buffer.
pub struct VisualizerSlot {
    /// Unique module ID
    pub module_id: ModuleId,
    /// The visualizer processor
    pub visualizer: Box<dyn AudioEffect>,
    /// Shared buffer for visualization data
    pub buffer: Arc<VisualizationBuffer>,
    /// Whether the visualizer is enabled
    pub enabled: bool,
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
    pub fn module_id(&self) -> ModuleId {
        match self {
            Self::Effect(slot) => slot.module_id,
            Self::Visualizer(slot) => slot.module_id,
        }
    }

    /// Check if this slot is enabled.
    pub fn is_enabled(&self) -> bool {
        match self {
            Self::Effect(slot) => slot.enabled,
            Self::Visualizer(slot) => slot.enabled,
        }
    }

    /// Set enabled state.
    pub fn set_enabled(&mut self, enabled: bool) {
        match self {
            Self::Effect(slot) => slot.enabled = enabled,
            Self::Visualizer(slot) => slot.enabled = enabled,
        }
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
            working_buffer: AudioBuffer::new(512),
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
            enabled: true,
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
            enabled: true,
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

    /// Process the effect chain.
    ///
    /// The `mix_buffer` contains interleaved stereo audio and is modified in place.
    /// Effects modify the signal; visualizers only capture it without modification.
    pub fn process(&mut self, mix_buffer: &mut AudioBuffer, context: &ProcessContext) {
        self.working_buffer.resize(context.samples.as_usize() * 2);

        for slot in &mut self.slots {
            match slot {
                ChainSlot::Effect(effect_slot) => {
                    if effect_slot.enabled {
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
                ChainSlot::Visualizer(viz_slot) => {
                    if viz_slot.enabled {
                        // Visualizers capture audio data but don't modify the signal
                        viz_slot.buffer.write_interleaved(mix_buffer.as_slice());
                        viz_slot
                            .buffer
                            .update_levels_interleaved(mix_buffer.as_slice());
                    }
                }
            }
        }
    }
}

impl Default for EffectChain {
    fn default() -> Self {
        Self::new()
    }
}
