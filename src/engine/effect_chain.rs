//! Effect chain processing for the synthesizer engine.
//!
//! Manages effects and visualizers in a processing chain.

use std::sync::Arc;

use crate::effects::{Chorus, Delay, Distortion, Reverb};
use crate::engine::commands::ModuleId;
use crate::engine::params::ModuleType;
use crate::modules::{AudioBuffer, EffectModule, ProcessContext};
use crate::visualizers::VisualizationBuffer;

/// Effect slot in the effect chain.
pub struct EffectSlot {
    /// Unique module ID
    pub module_id: ModuleId,
    /// The effect processor
    pub effect: Box<dyn EffectModule>,
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
    pub visualizer: Box<dyn EffectModule>,
    /// Shared buffer for visualization data
    pub buffer: Arc<VisualizationBuffer>,
    /// Whether the visualizer is enabled
    pub enabled: bool,
}

/// Effect chain that manages effects and visualizers.
pub struct EffectChain {
    /// Effect slots
    effects: Vec<EffectSlot>,
    /// Visualizer slots
    visualizers: Vec<VisualizerSlot>,
    /// Working buffer for effect processing
    effect_buffer: AudioBuffer,
}

impl EffectChain {
    /// Create a new effect chain with default effects.
    pub fn new() -> Self {
        let effects = vec![
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Chorus, 1),
                effect: Box::new(Chorus::new()),
                module_type: ModuleType::Chorus,
                enabled: false,
            },
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Delay, 1),
                effect: Box::new(Delay::new()),
                module_type: ModuleType::Delay,
                enabled: false,
            },
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Reverb, 1),
                effect: Box::new(Reverb::new()),
                module_type: ModuleType::Reverb,
                enabled: false,
            },
            EffectSlot {
                module_id: ModuleId::new(ModuleType::Distortion, 1),
                effect: Box::new(Distortion::new()),
                module_type: ModuleType::Distortion,
                enabled: false,
            },
        ];

        Self {
            effects,
            visualizers: Vec::new(),
            effect_buffer: AudioBuffer::new(512),
        }
    }

    /// Create an empty effect chain.
    pub fn empty() -> Self {
        Self {
            effects: Vec::new(),
            visualizers: Vec::new(),
            effect_buffer: AudioBuffer::new(512),
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
    pub fn add_effect(&mut self, id: ModuleId, mut effect: Box<dyn EffectModule>, sample_rate: f32) {
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
        visualizer: Box<dyn EffectModule>,
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
        self.effect_buffer.resize(context.samples * 2);

        for slot in &mut self.effects {
            if slot.enabled {
                // Copy mix to effect buffer
                self.effect_buffer.copy_from(mix_buffer);

                // Process effect (in-place)
                slot.effect.process(
                    self.effect_buffer.as_slice(),
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
