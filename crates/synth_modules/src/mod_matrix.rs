//! Mod Matrix module — routes modulation sources to destinations.
//!
//! The mod matrix provides 8 modulation slots. Each slot has:
//! - A source (LFO, envelope, velocity, note number, aftertouch, mod wheel, pitch bend)
//! - A destination (osc pitch, filter cutoff, amp level, etc.)
//! - A bipolar amount (-1.0 to +1.0)
//! - An enable toggle
//!
//! The actual modulation is applied by `Voice` before graph processing.
//! This module stores the routing configuration and caches source values.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, BipolarValue, Describable, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, Param, ParameterDescriptor, PolyModule, ProcessContext, WidgetHint,
};
use synth_core::{MOD_MATRIX_SLOTS, ModDestination, ModMatrixParam, ModSource};
use synth_core::{MidiNote, SampleRate, Velocity};

/// A single modulation routing slot.
#[derive(Debug, Clone, Copy)]
pub struct ModSlot {
    pub source: ModSource,
    pub destination: ModDestination,
    pub amount: BipolarValue,
    pub enabled: bool,
}

impl Default for ModSlot {
    fn default() -> Self {
        Self {
            source: ModSource::None,
            destination: ModDestination::None,
            amount: BipolarValue::CENTER,
            enabled: true,
        }
    }
}

/// Modulation result from one active slot.
#[derive(Debug, Clone, Copy)]
pub struct ModulationOutput {
    pub destination: ModDestination,
    pub value: f32,
}

/// Mod Matrix — 8-slot modulation routing.
///
/// Lives in the voice graph as a `PolyModule` for parameter storage and GUI,
/// but `process()` is a no-op. The actual modulation logic runs from `Voice`.
#[derive(Clone)]
pub struct ModMatrix {
    slots: [ModSlot; MOD_MATRIX_SLOTS],
    /// Cached source values, indexed by `ModSource::index()`.
    source_values: [f32; 16],
}

impl ModMatrix {
    pub fn new() -> Self {
        Self {
            slots: [ModSlot::default(); MOD_MATRIX_SLOTS],
            source_values: [0.0; 16],
        }
    }

    /// Update a cached source value.
    pub fn update_source(&mut self, source: ModSource, value: f32) {
        let idx = source.index();
        if idx < self.source_values.len() {
            self.source_values[idx] = value;
        }
    }

    /// Calculate all active modulations.
    ///
    /// Returns an iterator of (destination, scaled value) for active slots.
    pub fn calculate_modulations(&self) -> impl Iterator<Item = ModulationOutput> + '_ {
        self.slots.iter().filter_map(|slot| {
            if !slot.enabled {
                return None;
            }
            if matches!(slot.source, ModSource::None)
                || matches!(slot.destination, ModDestination::None)
            {
                return None;
            }
            let src_idx = slot.source.index();
            let src_value = if src_idx < self.source_values.len() {
                self.source_values[src_idx]
            } else {
                0.0
            };
            let scaled = src_value * slot.amount.as_f32();
            Some(ModulationOutput {
                destination: slot.destination,
                value: scaled,
            })
        })
    }

    /// Get a slot reference.
    pub fn slot(&self, index: usize) -> Option<&ModSlot> {
        self.slots.get(index)
    }
}

impl Default for ModMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ModMatrix {
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("mod_matrix", "Mod Matrix")
            .description("8-slot modulation routing matrix")
            .category(ModuleCategory::Utility)
            .tag("modulation")
            .tag("routing");

        // Add parameters for each slot (source, destination, amount, enabled)
        for i in 0..MOD_MATRIX_SLOTS {
            let slot = i as u8;
            let slot_num = i + 1;

            desc = desc.parameter(
                ParameterDescriptor::choice(
                    Param::ModMatrix(ModMatrixParam::SlotSource(slot, ModSource::None)),
                    format!("Slot {slot_num} Source"),
                    ModSource::to_choices(),
                )
                .description(format!("Modulation source for slot {slot_num}")),
            );

            desc = desc.parameter(
                ParameterDescriptor::choice(
                    Param::ModMatrix(ModMatrixParam::SlotDestination(slot, ModDestination::None)),
                    format!("Slot {slot_num} Dest"),
                    ModDestination::to_choices(),
                )
                .description(format!("Modulation destination for slot {slot_num}")),
            );

            desc = desc.parameter(
                ParameterDescriptor::float(
                    Param::ModMatrix(ModMatrixParam::SlotAmount(slot, BipolarValue::CENTER)),
                    format!("Slot {slot_num} Amount"),
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob)
                .description(format!("Modulation amount for slot {slot_num}")),
            );

            desc = desc.parameter(
                ParameterDescriptor::float(
                    Param::ModMatrix(ModMatrixParam::SlotEnabled(slot, true)),
                    format!("Slot {slot_num} On"),
                )
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Toggle)
                .description(format!("Enable slot {slot_num}"))
                .modulatable(false),
            );
        }

        desc
    }
}

impl PolyModule for ModMatrix {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        _outputs: &mut HashMap<String, AudioBuffer>,
        _context: &ProcessContext,
    ) {
        // No-op: modulation is applied by Voice, not in the graph processing chain.
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ModMatrix(mm_param) = param {
            let slot = mm_param.slot() as usize;
            if slot >= MOD_MATRIX_SLOTS {
                return;
            }
            match mm_param {
                ModMatrixParam::SlotSource(_, src) => self.slots[slot].source = src,
                ModMatrixParam::SlotDestination(_, dst) => self.slots[slot].destination = dst,
                ModMatrixParam::SlotAmount(_, amt) => self.slots[slot].amount = amt,
                ModMatrixParam::SlotEnabled(_, en) => self.slots[slot].enabled = en,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ModMatrix(mm_param) = param {
            let slot = mm_param.slot() as usize;
            if slot >= MOD_MATRIX_SLOTS {
                return None;
            }
            Some(match mm_param {
                ModMatrixParam::SlotSource(_, _) => self.slots[slot].source.index() as f32,
                ModMatrixParam::SlotDestination(_, _) => {
                    self.slots[slot].destination.index() as f32
                }
                ModMatrixParam::SlotAmount(_, _) => self.slots[slot].amount.as_f32(),
                ModMatrixParam::SlotEnabled(_, _) => {
                    if self.slots[slot].enabled {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        let mut params = Vec::with_capacity(MOD_MATRIX_SLOTS * 4);
        for i in 0..MOD_MATRIX_SLOTS {
            let slot = i as u8;
            params.push(Param::ModMatrix(ModMatrixParam::SlotSource(
                slot,
                self.slots[i].source,
            )));
            params.push(Param::ModMatrix(ModMatrixParam::SlotDestination(
                slot,
                self.slots[i].destination,
            )));
            params.push(Param::ModMatrix(ModMatrixParam::SlotAmount(
                slot,
                self.slots[i].amount,
            )));
            params.push(Param::ModMatrix(ModMatrixParam::SlotEnabled(
                slot,
                self.slots[i].enabled,
            )));
        }
        params
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::ModMatrix
    }

    fn reset(&mut self) {
        self.source_values.fill(0.0);
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, _sample_rate: SampleRate) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mod_matrix_creation() {
        let mm = ModMatrix::new();
        for i in 0..MOD_MATRIX_SLOTS {
            let slot = mm.slot(i).unwrap();
            assert_eq!(slot.source, ModSource::None);
            assert_eq!(slot.destination, ModDestination::None);
        }
    }

    #[test]
    fn test_mod_matrix_routing() {
        let mut mm = ModMatrix::new();

        // Set up slot 0: Velocity -> Filter Cutoff, amount 0.5
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotSource(
            0,
            ModSource::Velocity,
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotDestination(
            0,
            ModDestination::FilterCutoff(0),
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotAmount(
            0,
            BipolarValue::new(0.5),
        )));

        // Update velocity source
        mm.update_source(ModSource::Velocity, 0.8);

        // Calculate modulations
        let mods: Vec<_> = mm.calculate_modulations().collect();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].destination, ModDestination::FilterCutoff(0));
        assert!((mods[0].value - 0.4).abs() < 0.001); // 0.8 * 0.5 = 0.4
    }

    #[test]
    fn test_disabled_slot_not_in_output() {
        let mut mm = ModMatrix::new();

        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotSource(
            0,
            ModSource::Velocity,
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotDestination(
            0,
            ModDestination::FilterCutoff(0),
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotEnabled(0, false)));

        mm.update_source(ModSource::Velocity, 1.0);

        let mods: Vec<_> = mm.calculate_modulations().collect();
        assert_eq!(mods.len(), 0);
    }
}
