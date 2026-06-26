//! Mod Matrix module — routes modulation sources to destinations.
//!
//! Holds a list of up to [`MAX_MOD_MATRIX_SLOTS`] routings. Each routing is a
//! source ([`SrcAddr`] — a macro or a module output/param) scaled by a bipolar
//! amount into a destination ([`DestAddr`](synth_core::DestAddr) — any
//! modulatable param on any module), plus an enabled flag. A routing with no
//! source or destination is inactive.
//!
//! This module only stores the routing configuration (a no-op `process()`); the
//! `Voice` reads it via [`PolyModule::mod_routings`], resolves each source, and
//! applies the scaled offsets before graph processing.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::script::{BoundScript, EvalContext, ScriptHost};
use synth_core::{
    AudioBuffer, BipolarValue, Describable, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, Param, ParameterDescriptor, PolyModule, PortName, ProcessContext, WidgetHint,
};
use synth_core::{
    MAX_MOD_MATRIX_SLOTS, ModDestination, ModMatrixGridSize, ModMatrixParam, ModRouting, ModSource,
};
use synth_core::{MidiNote, SampleRate, Velocity};

// The Mod Matrix maps its routings 1:1 onto the embedded `ScriptHost`'s slots
// (slot `i` ↔ host slot `i`), so the host must have at least as many slots.
// Enforce the coupling at compile time — diverging the two constants would
// otherwise silently drop the high routings' scripts.
const _: () = assert!(MAX_MOD_MATRIX_SLOTS <= synth_core::script::SCRIPT_HOST_SLOTS);

/// Mod Matrix — a dynamic list of modulation routings.
///
/// Lives in the voice graph as a `PolyModule` for parameter storage and GUI,
/// but `process()` is a no-op. The actual modulation — resolving each routing's
/// source (macro / module output / param) and applying the scaled offset to the
/// destination — runs from `Voice`, which reads the routing list via
/// [`mod_routings`](PolyModule::mod_routings).
#[derive(Clone)]
pub struct ModMatrix {
    slots: [ModRouting; MAX_MOD_MATRIX_SLOTS],
    /// Per-slot compiled control scripts (Step 2) plus their per-voice state,
    /// parallel to `slots`. When `host.slots()[i]` is `Some`, slot `i`'s offset is
    /// the script's output instead of the scalar `source × amount` (decision #1).
    /// The `Arc<BoundScript>`s are shared immutably; cloning the module (per voice)
    /// bumps the refcount and gives the clone its own register state.
    host: ScriptHost,
    grid_size: ModMatrixGridSize,
}

impl ModMatrix {
    pub fn new() -> Self {
        Self {
            slots: [ModRouting::default(); MAX_MOD_MATRIX_SLOTS],
            host: ScriptHost::new(),
            grid_size: ModMatrixGridSize::default(),
        }
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
            .width(synth_core::ModuleWidth::Large)
            .description("Modulation matrix — a list of source → destination routings")
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
                    Param::ModMatrix(ModMatrixParam::SlotSource(slot, None)),
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
                // The matrix's own routing config — not a self-referential mod target.
                .modulatable(false)
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
                // Boolean toggle (the matrix's own routing config): not modulatable.
                .modulatable(false)
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
                        ModMatrixParam::SlotSource(_, src) => {
                            // The param now carries the source address directly.
                            self.slots[slot].source = src;
                        }
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
                        ModMatrixParam::SlotSource(_, _) => {
                            self.slots[slot].source.map_or(0, |a| a.legacy_index()) as f32
                        }
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
        // No per-block state: routings are config, applied by Voice.
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Re-seed each slot's control-script state for the new note (zero state
        // cells, re-seed PRNG from the stored voice index — deterministic
        // retrigger, decorrelated voices).
        self.host.note_on();
    }
    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, _sample_rate: SampleRate) {}

    fn mod_routings(&self) -> Option<&[ModRouting]> {
        Some(&self.slots)
    }

    fn scripts(&self) -> Option<&[Option<Arc<BoundScript>>]> {
        Some(self.host.slots())
    }

    fn set_script(
        &mut self,
        slot: usize,
        script: Option<Arc<BoundScript>>,
    ) -> Option<Arc<BoundScript>> {
        // Return the replaced script rather than dropping it: this runs on the
        // audio thread, so its (possibly final) `free()` must happen on the main
        // thread instead. The engine routes the returned `Arc` to a trash channel.
        self.host.set_slot(slot, script)
    }

    fn eval_script_slot(&mut self, slot: usize, sources: &[f32], ctx: &EvalContext) -> Option<f32> {
        self.host.eval_slot(slot, sources, ctx)
    }

    fn set_voice_index(&mut self, voice_index: u32) {
        self.host.set_voice_index(voice_index);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::{DestAddr, MacroSource, ModDestination, SrcAddr};

    #[test]
    fn starts_with_unconfigured_routings() {
        let mm = ModMatrix::new();
        for i in 0..MAX_MOD_MATRIX_SLOTS {
            let slot = mm.slot(i).unwrap();
            assert_eq!(slot.source, None);
            assert_eq!(slot.destination, None);
        }
    }

    /// Legacy enum source/dest set via `set_param` are stored as addresses;
    /// `mod_routings` exposes them for the Voice; `get_params` round-trips the
    /// legacy enum for the GUI combo.
    #[test]
    fn legacy_enum_source_dest_become_addresses() {
        let mut mm = ModMatrix::new();
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotSource(
            0,
            SrcAddr::from_mod_source(ModSource::Velocity),
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotDestination(
            0,
            DestAddr::from_mod_destination(ModDestination::FilterCutoff(0)),
        )));
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotAmount(
            0,
            BipolarValue::new(0.5),
        )));

        let routings = mm.mod_routings().unwrap();
        assert_eq!(
            routings[0].source,
            Some(SrcAddr::Macro(MacroSource::Velocity))
        );
        assert_eq!(
            routings[0].destination,
            Some(DestAddr::new(ModuleType::Filter, 1, "cutoff"))
        );
        assert!((routings[0].amount.as_f32() - 0.5).abs() < 1e-6);
    }

    /// A slot's compiled control script is stored beside the routing, read back
    /// via `scripts`, cleared with `None`, and survives a `box_clone` (each
    /// voice shares the same `Arc`, never a deep copy).
    #[test]
    fn slot_script_is_stored_cleared_and_cloned() {
        use std::sync::Arc;
        use synth_core::script::{BoundScript, CompiledScript, Op};

        let mut mm = ModMatrix::new();
        assert!(mm.scripts().unwrap().iter().all(Option::is_none));

        // A trivial `out = 0.5` program (one constant, no sources/state).
        let script = Arc::new(BoundScript::new(
            CompiledScript::new(vec![Op::PushConst(0)], vec![0.5], 0, 0),
            Vec::new(),
            "out = 0.5".to_string(),
        ));
        assert!(mm.set_script(2, Some(Arc::clone(&script))).is_none());

        let scripts = mm.scripts().unwrap();
        assert!(scripts[0].is_none());
        assert_eq!(
            scripts[2].as_deref().map(|b| b.source.as_str()),
            Some("out = 0.5")
        );

        // box_clone shares the Arc (refcount rises, no deep copy).
        let cloned = mm.box_clone();
        assert_eq!(
            cloned.scripts().unwrap()[2]
                .as_deref()
                .map(|b| b.source.as_str()),
            Some("out = 0.5")
        );
        assert!(Arc::strong_count(&script) >= 3);

        // Clearing removes it and returns the old script for deferred drop.
        assert!(mm.set_script(2, None).is_some());
        assert!(mm.scripts().unwrap()[2].is_none());
        // Out-of-range slot is a safe no-op; the script is handed straight back.
        assert!(mm.set_script(999, Some(script)).is_some());
    }

    /// A source that the legacy enum could not express — a third LFO — is now a
    /// valid module-addressed source.
    #[test]
    fn third_lfo_source_is_addressable() {
        let mut mm = ModMatrix::new();
        // Directly set an address-based routing (as MCP / a picker would).
        mm.set_param(Param::ModMatrix(ModMatrixParam::SlotDestination(
            0,
            Some(DestAddr::new(ModuleType::Filter, 1, "cutoff")),
        )));
        // Source is set internally via the routing; verify the addressable form
        // resolves through SrcAddr.
        assert_eq!(
            SrcAddr::parse("lfo-3.out"),
            Some(SrcAddr::module(ModuleType::Lfo, 3, "out"))
        );
    }
}
