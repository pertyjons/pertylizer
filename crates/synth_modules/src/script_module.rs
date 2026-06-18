//! Script module — a rack of YAMS control scripts as a first-class graph node.
//!
//! Unlike the Mod Matrix (whose scripts produce parameter *offsets* applied by
//! the `Voice`), the Script module exposes each slot's `out` as a real **output
//! port** (`out1`..`out8`). That makes a scripted control signal reusable: it is
//! computed once and can feed many destinations — referenced as a Mod-Matrix
//! source (`scr-1.out1`), wired into a port (FM/CV input), or read by another
//! script (`src x = scr-1.out1`), so scripts chain.
//!
//! Evaluation is driven by the `Voice` (it resolves each script's address-based
//! sources from the graph, then calls [`PolyModule::eval_script_slot`], which
//! this module overrides to **cache** the result). `process()` then fills each
//! output buffer with its slot's cached value — over the *entire* block, so a
//! port consumer reads a clean signal, not just `buffer[0]`.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::script::{BoundScript, EvalContext, ScriptHost};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleType, Param,
    PolyModule, PortDescriptor, PortName, ProcessContext,
};
use synth_core::{MidiNote, SampleRate, Velocity};

/// Number of script slots exposed as output ports (`out1`..`out8`). Backed by
/// the first 8 slots of the embedded [`ScriptHost`].
pub const SCRIPT_MODULE_OUTPUTS: usize = 8;

/// A rack of up to [`SCRIPT_MODULE_OUTPUTS`] scripted control signals, each on
/// its own output port.
#[derive(Clone)]
pub struct ScriptModule {
    /// The compiled scripts + per-voice state (Phase 0 host).
    host: ScriptHost,
    /// Last evaluated output per slot, filled by [`Self::eval_script_slot`] and
    /// broadcast across the buffer in [`process`](PolyModule::process).
    outputs: [f32; SCRIPT_MODULE_OUTPUTS],
    /// Interned `out1`..`out8` port names, computed once off the audio thread
    /// (`PortName::intern` locks, so it must not run in `process()`).
    port_names: [PortName; SCRIPT_MODULE_OUTPUTS],
}

impl ScriptModule {
    #[must_use]
    pub fn new() -> Self {
        Self {
            host: ScriptHost::new(),
            outputs: [0.0; SCRIPT_MODULE_OUTPUTS],
            port_names: std::array::from_fn(|i| PortName::intern(&format!("out{}", i + 1))),
        }
    }
}

impl Default for ScriptModule {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ScriptModule {
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("script", "Script")
            .description(
                "Scripted control signals — up to 8 YAMS slots, each on its own output port",
            )
            .category(ModuleCategory::Utility)
            .tag("modulation")
            .tag("script");
        for i in 0..SCRIPT_MODULE_OUTPUTS {
            let n = i + 1;
            desc = desc.port(
                PortDescriptor::audio_output(format!("out{n}"), format!("Out {n}"))
                    .description(format!("Output of script slot {n}")),
            );
        }
        desc
    }
}

impl PolyModule for ScriptModule {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        _context: &ProcessContext,
    ) {
        // The slots were already evaluated for this block by the Voice (which
        // resolved their sources from the graph); broadcast each cached value
        // across the whole output buffer so a port consumer sees a steady signal.
        for slot in 0..SCRIPT_MODULE_OUTPUTS {
            if let Some(buf) = outputs.get_mut(&self.port_names[slot]) {
                buf.as_mut_slice().fill(self.outputs[slot]);
            }
        }
    }

    fn set_param(&mut self, _param: Param) {
        // No numeric parameters — the scripts are installed via `set_script`.
    }

    fn get_param(&self, _param: &Param) -> Option<f32> {
        None
    }

    fn get_params(&self) -> Vec<Param> {
        Vec::new()
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Script
    }

    fn reset(&mut self) {
        self.outputs = [0.0; SCRIPT_MODULE_OUTPUTS];
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Re-seed each slot's script state for the new note (decision #4).
        self.host.note_on();
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, _sample_rate: SampleRate) {}

    fn scripts(&self) -> Option<&[Option<Arc<BoundScript>>]> {
        Some(self.host.slots())
    }

    fn set_script(
        &mut self,
        slot: usize,
        script: Option<Arc<BoundScript>>,
    ) -> Option<Arc<BoundScript>> {
        // Clearing a slot must also drop its cached output, otherwise the port
        // would keep emitting the last evaluated value (the Voice no longer
        // evaluates an empty slot).
        if script.is_none()
            && let Some(cell) = self.outputs.get_mut(slot)
        {
            *cell = 0.0;
        }
        self.host.set_slot(slot, script)
    }

    fn eval_script_slot(&mut self, slot: usize, sources: &[f32], ctx: &EvalContext) -> Option<f32> {
        let out = self.host.eval_slot(slot, sources, ctx);
        if let Some(v) = out
            && let Some(cell) = self.outputs.get_mut(slot)
        {
            *cell = v;
        }
        out
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
    use synth_core::script::{CompiledScript, Op};

    fn const_script(v: f32) -> Arc<BoundScript> {
        Arc::new(BoundScript::new(
            CompiledScript::new(vec![Op::PushConst(0)], vec![v], 0, 0),
            Vec::new(),
            format!("out = {v}"),
        ))
    }

    #[test]
    fn descriptor_declares_eight_output_ports() {
        let m = ScriptModule::new();
        let desc = m.descriptor();
        let outs = desc
            .ports
            .iter()
            .filter(|p| p.label.starts_with("Out "))
            .count();
        assert_eq!(outs, SCRIPT_MODULE_OUTPUTS);
    }

    #[test]
    fn eval_caches_and_process_broadcasts_over_whole_buffer() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(0, Some(const_script(0.7)));
        let ctx = EvalContext::new(750.0);

        // Voice-driven eval caches the slot output.
        assert_eq!(m.eval_script_slot(0, &[], &ctx), Some(0.7));

        // process() fills the entire out1 buffer with the cached value.
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::intern("out1"), AudioBuffer::new(64));
        outs.insert(PortName::intern("out2"), AudioBuffer::new(64));
        m.process(InputPorts::new(&[]), &mut outs, &ProcessContext::default());
        let out1 = &outs[&PortName::intern("out1")];
        assert!(
            (0..64).all(|i| (out1[i] - 0.7).abs() < 1e-6),
            "whole buffer filled"
        );
        // An unscripted slot stays at 0.
        let out2 = &outs[&PortName::intern("out2")];
        assert!((0..64).all(|i| out2[i] == 0.0));
    }

    #[test]
    fn clearing_a_slot_zeroes_its_cached_output() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(1, Some(const_script(0.9)));
        let ctx = EvalContext::new(750.0);
        let _ = m.eval_script_slot(1, &[], &ctx);
        // Clear the slot — cached output must drop to 0 so the port goes silent.
        assert!(m.set_script(1, None).is_some());
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::intern("out2"), AudioBuffer::new(8));
        m.process(InputPorts::new(&[]), &mut outs, &ProcessContext::default());
        assert!((0..8).all(|i| outs[&PortName::intern("out2")][i] == 0.0));
    }
}
