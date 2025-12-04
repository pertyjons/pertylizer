//! Effect chain processing for per-instrument insert effects.
//!
//! Each instrument owns its own EffectChain for independent effect processing.
//! This enables loading patches that only affect that instrument's sound.

use std::sync::Arc;

use crate::engine::commands::ModuleId;
use crate::engine::params::ModuleType;
use crate::modules::{AudioBuffer, AudioEffect, ProcessContext};
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

/// Per-instrument effect chain that manages insert effects and visualizers.
///
/// Each instrument has its own EffectChain, enabling independent effect
/// processing. This ensures that loading a patch only affects that
/// instrument's sound, solving collision issues.
pub struct EffectChain {
    /// Effect slots
    effects: Vec<EffectSlot>,
    /// Visualizer slots
    visualizers: Vec<VisualizerSlot>,
    /// Working buffer for effect processing
    working_buffer: AudioBuffer,
}

impl EffectChain {
    /// Create a new empty effect chain.
    ///
    /// Effects are added dynamically when loading patches or via GUI.
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
            visualizers: Vec::new(),
            working_buffer: AudioBuffer::new(512),
        }
    }

    /// Find an effect slot by its module type.
    pub fn find_effect_by_type(&mut self, module_type: ModuleType) -> Option<&mut EffectSlot> {
        self.effects
            .iter_mut()
            .find(|slot| slot.module_type == module_type)
    }

    /// Find an effect slot by its module ID.
    pub fn find_effect_by_id(&mut self, module_id: ModuleId) -> Option<&mut EffectSlot> {
        self.effects
            .iter_mut()
            .find(|slot| slot.module_id == module_id)
    }

    /// Add an effect instance.
    pub fn add_effect(&mut self, id: ModuleId, mut effect: Box<dyn AudioEffect>, sample_rate: f32) {
        effect.set_sample_rate(sample_rate);
        let module_type = effect.module_type();

        self.effects.push(EffectSlot {
            module_id: id,
            module_type,
            effect,
            enabled: true,
        });
    }

    /// Remove an effect by ID.
    pub fn remove_effect(&mut self, id: ModuleId) {
        self.effects.retain(|slot| slot.module_id != id);
    }

    /// Add a visualizer.
    pub fn add_visualizer(
        &mut self,
        id: ModuleId,
        visualizer: Box<dyn AudioEffect>,
        buffer: Arc<VisualizationBuffer>,
    ) {
        self.visualizers.push(VisualizerSlot {
            module_id: id,
            visualizer,
            buffer,
            enabled: true,
        });
    }

    /// Remove a visualizer by ID.
    pub fn remove_visualizer(&mut self, id: ModuleId) {
        self.visualizers.retain(|slot| slot.module_id != id);
    }

    /// Clear all effects and visualizers.
    pub fn clear(&mut self) {
        self.effects.clear();
        self.visualizers.clear();
    }

    /// Reset all effects.
    pub fn reset(&mut self) {
        for slot in &mut self.effects {
            slot.effect.reset();
        }
    }

    /// Get mutable access to effects.
    pub fn effects_mut(&mut self) -> &mut Vec<EffectSlot> {
        &mut self.effects
    }

    /// Process the effect chain.
    ///
    /// The `mix_buffer` contains interleaved stereo audio and is modified in place.
    pub fn process(&mut self, mix_buffer: &mut AudioBuffer, context: &ProcessContext) {
        self.working_buffer.resize(context.samples * 2);

        for slot in &mut self.effects {
            if slot.enabled {
                // Copy mix to working buffer
                self.working_buffer.copy_from(mix_buffer);

                // Process effect (in-place)
                slot.effect.process(
                    self.working_buffer.as_slice(),
                    mix_buffer.as_mut_slice(),
                    context,
                );
            }
        }

        // Process visualizers (they don't modify the signal, just capture it)
        self.process_visualizers(mix_buffer);
    }

    /// Process visualizers - capture audio data for display.
    /// Uses interleaved methods to avoid heap allocation in audio thread.
    fn process_visualizers(&mut self, mix_buffer: &AudioBuffer) {
        for slot in &mut self.visualizers {
            if !slot.enabled {
                continue;
            }

            // Write directly from interleaved mix buffer - NO ALLOCATION!
            slot.buffer.write_interleaved(mix_buffer.as_slice());
            slot.buffer.update_levels_interleaved(mix_buffer.as_slice());
        }
    }
}

impl Default for EffectChain {
    fn default() -> Self {
        Self::new()
    }
}
