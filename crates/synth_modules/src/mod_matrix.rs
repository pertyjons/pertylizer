//! Mod Matrix module — routes modulation sources to destinations.
//!
//! The mod matrix provides up to 16 modulation slots arranged in a selectable grid
//! (1x1, 2x2, 3x3, 4x4). Each slot has:
//! - A source (LFO, envelope, velocity, note number, aftertouch, mod wheel, pitch bend)
//! - A destination (osc pitch, filter cutoff, amp level, etc.)
//! - A bipolar amount (-1.0 to +1.0)
//!
//! A slot with Source=None is effectively inactive.
//! The actual modulation is applied by `Voice` before graph processing.
//! This module stores the routing configuration and caches source values.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, BipolarValue, Describable, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, Param, ParameterDescriptor, PolyModule, PortName, ProcessContext, WidgetHint,
};
use synth_core::{
    DestAddr, MAX_MOD_MATRIX_SLOTS, ModDestination, ModMatrixGridSize, ModMatrixParam, ModRouting,
    ModSource,
};
use synth_core::{MidiNote, SampleRate, Velocity};

/// Modulation result from one active slot.
#[derive(Debug, Clone, Copy)]
pub struct ModulationOutput {
    pub destination: DestAddr,
    pub value: f32,
}

/// Mod Matrix — grid-based modulation routing (up to 16 slots).
///
/// Lives in the voice graph as a `PolyModule` for parameter storage and GUI,
/// but `process()` is a no-op. The actual modulation logic runs from `Voice`.
#[derive(Clone)]
pub struct ModMatrix {
    slots: [ModRouting; MAX_MOD_MATRIX_SLOTS],
    grid_size: ModMatrixGridSize,
    /// Cached source values, indexed by `ModSource::index()`.
    source_values: [f32; 16],
}

impl ModMatrix {
    pub fn new() -> Self {
        Self {
            slots: [ModRouting::default(); MAX_MOD_MATRIX_SLOTS],
            grid_size: ModMatrixGridSize::default(),
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
    /// Iterates every slot — the grid no longer gates processing (the routing
    /// list is presented dynamically in the GUI). Returns an iterator of
    /// (destination, scaled value) for active slots.
    pub fn calculate_modulations(&self) -> impl Iterator<Item = ModulationOutput> + '_ {
        self.slots.iter().filter_map(|slot| {
            if !slot.enabled || matches!(slot.source, ModSource::None) {
                return None;
            }
            let destination = slot.destination?;
            let src_idx = slot.source.index();
            let src_value = if src_idx < self.source_values.len() {
                self.source_values[src_idx]
            } else {
                0.0
            };
            let scaled = src_value * slot.amount.as_f32();
            Some(ModulationOutput {
                destination,
                value: scaled,
            })
        })
    }

    /// Get a slot reference.
    pub fn slot(&self, index: usize) -> Option<&ModRouting> {
        self.slots.get(index)
    }
}

impl Default for ModMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ModMatrix {
    #[allow(clippy::too_many_lines)]
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("mod_matrix", "Mod Matrix")
            .description("Grid-based modulation routing matrix (up to 16 slots)")
            .category(ModuleCategory::Utility)
            .tag("modulation")
            .tag("routing");

        // Grid size selector (first parameter)
        desc = desc.parameter(
            ParameterDescriptor::choice(
                "grid_size",
                Param::ModMatrix(ModMatrixParam::GridSize(ModMatrixGridSize::default())),
                "Grid Size".to_string(),
                ModMatrixGridSize::to_choices(),
            )
            .description("Number of modulation slots (grid dimensions)".to_string()),
        );

        // Add parameters for each slot (source, destination, amount)
        for i in 0..MAX_MOD_MATRIX_SLOTS {
            let slot = i as u8;
            let slot_num = i + 1;

            desc = desc.parameter(
                ParameterDescriptor::choice(
                    format!("slot_{slot_num}_source"),
                    Param::ModMatrix(ModMatrixParam::SlotSource(slot, ModSource::None)),
                    format!("Slot {slot_num} Source"),
                    ModSource::to_choices(),
                )
                .description(format!("Modulation source for slot {slot_num}")),
            );

            desc = desc.parameter(
                ParameterDescriptor::choice(
                    format!("slot_{slot_num}_dest"),
                    Param::ModMatrix(ModMatrixParam::SlotDestination(slot, None)),
                    format!("Slot {slot_num} Dest"),
                    ModDestination::to_choices(),
                )
                .description(format!("Modulation destination for slot {slot_num}")),
            );

            desc = desc.parameter(
                ParameterDescriptor::float(
                    format!("slot_{slot_num}_amount"),
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
                    format!("slot_{slot_num}_enabled"),
                    Param::ModMatrix(ModMatrixParam::SlotEnabled(slot, true)),
                    format!("Slot {slot_num} Enabled"),
                )
                .range(0.0, 1.0)
                .default(1.0)
                .description(format!("Enable/disable slot {slot_num}")),
            );
        }

        desc
    }
}

impl PolyModule for ModMatrix {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        _outputs: &mut HashMap<PortName, AudioBuffer>,
        _context: &ProcessContext,
    ) {
        // No-op: modulation is applied by Voice, not in the graph processing chain.
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ModMatrix(mm_param) = param {
            match mm_param {
                ModMatrixParam::GridSize(gs) => self.grid_size = gs,
                ModMatrixParam::SlotSource(_, _)
                | ModMatrixParam::SlotDestination(_, _)
                | ModMatrixParam::SlotAmount(_, _)
                | ModMatrixParam::SlotEnabled(_, _) => {
                    let slot = mm_param.slot() as usize;
                    if slot >= MAX_MOD_MATRIX_SLOTS {
                        return;
                    }
                    match mm_param {
                        ModMatrixParam::SlotSource(_, src) => self.slots[slot].source = src,
                        ModMatrixParam::SlotDestination(_, dst) => {
                            // The param now carries the address directly.
                            self.slots[slot].destination = dst;
                        }
                        ModMatrixParam::SlotAmount(_, amt) => self.slots[slot].amount = amt,
                        ModMatrixParam::SlotEnabled(_, en) => self.slots[slot].enabled = en,
                        ModMatrixParam::GridSize(_) => {}
                    }
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ModMatrix(mm_param) = param {
            match mm_param {
                ModMatrixParam::GridSize(_) =>
                {
                    #[allow(clippy::cast_precision_loss)]
                    Some(self.grid_size.index() as f32)
                }
                _ => {
                    let slot = mm_param.slot() as usize;
                    if slot >= MAX_MOD_MATRIX_SLOTS {
                        return None;
                    }
                    #[allow(clippy::cast_precision_loss)]
                    Some(match mm_param {
                        ModMatrixParam::SlotSource(_, _) => self.slots[slot].source.index() as f32,
                        ModMatrixParam::SlotDestination(_, _) => {
                            // Report the legacy enum index for the enum-combo GUI.
                            self.slots[slot].destination.map_or(0, |d| d.legacy_index()) as f32
                        }
                        ModMatrixParam::SlotAmount(_, _) => self.slots[slot].amount.as_f32(),
                        ModMatrixParam::SlotEnabled(_, _) => {
                            if self.slots[slot].enabled {
                                1.0
                            } else {
                                0.0
                            }
                        }
                        ModMatrixParam::GridSize(_) => 0.0, // unreachable
                    })
                }
            }
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        let mut params = Vec::with_capacity(1 + MAX_MOD_MATRIX_SLOTS * 4);
        params.push(Param::ModMatrix(ModMatrixParam::GridSize(self.grid_size)));
        for i in 0..MAX_MOD_MATRIX_SLOTS {
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

    fn mod_routings(&self) -> Option<&[ModRouting]> {
        Some(&self.slots)
    }

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
        for i in 0..MAX_MOD_MATRIX_SLOTS {
            let slot = mm.slot(i).unwrap();
            assert_eq!(slot.source, ModSource::None);
            assert_eq!(slot.destination, None);
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
            DestAddr::from_mod_destination(ModDestination::FilterCutoff(0)),
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
        assert_eq!(
            mods[0].destination,
            DestAddr::new(ModuleType::Filter, 1, "cutoff")
        );
        assert!((mods[0].value - 0.4).abs() < 0.001); // 0.8 * 0.5 = 0.4
    }

    #[test]
    fn test_none_source_slot_not_in_output() {
        let mm = ModMatrix::new();

        // All slots have Source=None by default, so no modulations
        let mods: Vec<_> = mm.calculate_modulations().collect();
        assert_eq!(mods.len(), 0);
    }

    /// The grid no longer gates processing: every configured slot is active
    /// regardless of the (vestigial) GridSize. A configured slot beyond a small
    /// grid is now processed (the accepted behavior change of removing the grid).
    #[test]
    fn test_all_configured_slots_processed_regardless_of_grid() {
        let mut mm = ModMatrix::new();

        // A leftover 1x1 grid from an old project — must not gate anything.
        mm.set_param(Param::ModMatrix(ModMatrixParam::GridSize(
            ModMatrixGridSize::Grid1x1,
        )));

        // Slot 0 and slot 1 both configured.
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotSource(
            0,
            ModSource::Velocity,
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotDestination(
            0,
            DestAddr::from_mod_destination(ModDestination::FilterCutoff(0)),
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotAmount(
            0,
            BipolarValue::new(0.5),
        )));

        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotSource(
            1,
            ModSource::ModWheel,
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotDestination(
            1,
            DestAddr::from_mod_destination(ModDestination::AmpLevel(0)),
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotAmount(
            1,
            BipolarValue::new(1.0),
        )));

        mm.update_source(ModSource::Velocity, 1.0);
        mm.update_source(ModSource::ModWheel, 1.0);

        // Both slots produce output even though the grid is 1x1.
        let mods: Vec<_> = mm.calculate_modulations().collect();
        assert_eq!(mods.len(), 2);
    }

    #[test]
    fn test_grid_size_param() {
        let mut mm = ModMatrix::new();

        // Default is 2x2
        let gs = mm
            .get_param(&Param::ModMatrix(ModMatrixParam::GridSize(
                ModMatrixGridSize::default(),
            )))
            .unwrap();
        assert_eq!(gs as usize, ModMatrixGridSize::Grid2x2.index());

        // Set to 3x3
        mm.set_param(Param::ModMatrix(ModMatrixParam::GridSize(
            ModMatrixGridSize::Grid3x3,
        )));
        let gs = mm
            .get_param(&Param::ModMatrix(ModMatrixParam::GridSize(
                ModMatrixGridSize::default(),
            )))
            .unwrap();
        assert_eq!(gs as usize, ModMatrixGridSize::Grid3x3.index());
    }
}
