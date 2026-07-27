//! Script module — one YAMS program as a first-class graph node: "write your
//! own control module".
//!
//! A single compiled program per instance (not the former 8-slot rack) with a
//! small, fixed set of CV ports both ways: **4 inputs** `in1`..`in4` (cables
//! wired *into* the node, read as `in1`..`in4` in the script) and **4 outputs**
//! `out1`..`out4` (cables *out*). One program can compute a value once and feed
//! several outputs (shared locals), and its outputs are reusable — referenced as
//! a Mod-Matrix source (`scr-1.out1`), wired into another port, or read by
//! another script (`src x = scr-1.out1`). Need more, or a second independent
//! program? Add another `Script` module — extra instances are cheap.
//!
//! This is the control-rate twin of the audio-rate
//! [`AudioScript`](crate::AudioScript): both hold exactly one immutable
//! `Arc<BoundScript>` plus this voice's own [`RegisterFile`] (state cells + PRNG).
//! Evaluation is driven by the `Voice`: it resolves the program's address-based
//! block-constant sources **and** the `in1`..`in4` port values from the graph,
//! then calls [`PolyModule::eval_control_multi`], which this module overrides to
//! run the program once into 4 cached outputs. `process()` then broadcasts each
//! cached value across its output-port buffer over the *whole* block, so a port
//! consumer reads a clean steady signal, not just `buffer[0]`.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::script::{
    BoundScript, EvalContext, RegisterFile, SCRIPT_PRNG_SEED, ScriptParams, knob_descriptor,
};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleType, Param,
    PolyModule, PortDescriptor, PortName, ProcessContext, ScriptParam,
};
use synth_core::{MidiNote, SampleRate, Velocity};

/// Number of fixed CV ports the Script module exposes each way: `in1`..`in4`
/// (inputs) and `out1`..`out4` (outputs). Matches the VM's `OUT_SLOTS`, so the
/// output capture needs no widening.
pub const SCRIPT_MODULE_PORTS: usize = 4;

/// Canonical output-port name for a 0-based slot (`out1`..`out4`), the single
/// source of truth for the numbering (mirrored by the interned `out_names` in
/// [`ScriptModule::new`]). Returns `None` for an out-of-range slot.
#[must_use]
pub fn output_port_name(slot: usize) -> Option<String> {
    (slot < SCRIPT_MODULE_PORTS).then(|| format!("out{}", slot + 1))
}

/// Inverse of [`output_port_name`]: parse `"out3"` back to its 0-based slot, or
/// `None` for any other / out-of-range name. Kept beside the generator so the
/// two can't drift.
#[must_use]
pub fn output_port_slot(port: &str) -> Option<usize> {
    let n: usize = port.strip_prefix("out")?.parse().ok()?;
    (1..=SCRIPT_MODULE_PORTS).contains(&n).then(|| n - 1)
}

/// Canonical input-port name for a 0-based slot (`in1`..`in4`).
#[must_use]
pub fn input_port_name(slot: usize) -> Option<String> {
    (slot < SCRIPT_MODULE_PORTS).then(|| format!("in{}", slot + 1))
}

/// Inverse of [`input_port_name`]: parse `"in2"` back to its 0-based slot, or
/// `None` for any other / out-of-range name.
#[must_use]
pub fn input_port_slot(port: &str) -> Option<usize> {
    let n: usize = port.strip_prefix("in")?.parse().ok()?;
    (1..=SCRIPT_MODULE_PORTS).contains(&n).then(|| n - 1)
}

/// A one-program control-rate scripted node with 4 CV inputs and 4 CV outputs.
#[derive(Clone)]
pub struct ScriptModule {
    /// The single compiled program (shared immutably). `None` until installed.
    script: Option<Arc<BoundScript>>,
    /// This voice's persistent state + PRNG for the program.
    regs: RegisterFile,
    /// Stable per-(voice, module) seed base, set at voice allocation and used to
    /// re-seed `regs` on note-on. The graph folds the module identity into it, so
    /// two Script modules in one voice decorrelate.
    voice_index: u32,
    /// Last evaluated outputs `out1`..`out4`, filled by
    /// [`eval_control_multi`](PolyModule::eval_control_multi) and broadcast across
    /// their port buffers in [`process`](PolyModule::process).
    outputs: [f32; SCRIPT_MODULE_PORTS],
    /// Interned `out1`..`out4` port names, computed once off the audio thread
    /// (`PortName::intern` locks, so it must not run in `process()`). The
    /// `in1`..`in4` ports need no cached names here — the voice matches them with
    /// the compile-time `PortName::IN1`..`IN4` constants when resolving cables.
    out_names: [PortName; SCRIPT_MODULE_PORTS],
    /// User-declared knob store (values + mod-offsets), remapped in place from the
    /// installed program's `param` decls on `set_script`.
    knobs: ScriptParams,
}

impl ScriptModule {
    #[must_use]
    pub fn new() -> Self {
        Self {
            script: None,
            regs: RegisterFile::new(0, SCRIPT_PRNG_SEED),
            voice_index: 0,
            outputs: [0.0; SCRIPT_MODULE_PORTS],
            out_names: std::array::from_fn(|i| {
                // `output_port_name(i)` is `Some` for every i < SCRIPT_MODULE_PORTS.
                PortName::intern(&output_port_name(i).unwrap_or_default())
            }),
            knobs: ScriptParams::new(),
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
                "Write your own control module in YAMS: one program with 4 CV inputs \
                 (in1..in4) and 4 CV outputs (out1..out4). Wire cables into in1..in4 and \
                 out of out1..out4; read/write them by name in the script.",
            )
            .category(ModuleCategory::Utility)
            .tag("modulation")
            .tag("script");
        // 4 CV input ports (in1..in4), then 4 CV output ports (out1..out4). Use the
        // canonical name generators so the advertised names can't drift from the
        // ones `process()`/the voice fill.
        for i in 0..SCRIPT_MODULE_PORTS {
            let name = input_port_name(i).unwrap_or_default();
            desc = desc.port(
                PortDescriptor::control_input(name, format!("In {}", i + 1))
                    .description(format!("CV input read as `in{}` in the script", i + 1)),
            );
        }
        for i in 0..SCRIPT_MODULE_PORTS {
            let name = output_port_name(i).unwrap_or_default();
            desc = desc.port(
                PortDescriptor::control_output(name, format!("Out {}", i + 1))
                    .description(format!("CV output written as `out{}` in the script", i + 1)),
            );
        }
        // User-declared knobs from the installed program (per-instance), at their
        // current stored value. Modulatable and automatable like any real param.
        if let Some(script) = &self.script {
            for decl in &script.params {
                let value = self.knobs.get(decl.name).unwrap_or(decl.default);
                desc = desc.parameter(knob_descriptor(decl, value));
            }
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
        // The program was already evaluated for this block by the Voice (which
        // resolved its block-constant sources and the in1..in4 port values from
        // the graph); broadcast each cached output across the whole port buffer so
        // a consumer sees a steady signal, not just buffer[0].
        for slot in 0..SCRIPT_MODULE_PORTS {
            if let Some(buf) = outputs.get_mut(&self.out_names[slot]) {
                buf.as_mut_slice().fill(self.outputs[slot]);
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        // Only the script-declared knobs are settable; the program itself is
        // installed via `set_script`. A knob whose name is not currently declared
        // is silently ignored (disable-and-keep).
        if let Param::Script(ScriptParam::Knob(name, value)) = param {
            self.knobs.set(name, value);
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Script(ScriptParam::Knob(name, _)) = param {
            self.knobs.get(*name)
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        self.knobs.as_params()
    }

    fn set_mod_offset(&mut self, target: &str, value: f32) {
        // Route mod-matrix offsets to the matching declared knob (lock-free `str`
        // match against the cached knob names). A target that names no knob is a
        // silent no-op, like every other module.
        self.knobs.add_offset(target, value);
    }

    fn clear_mod_offsets(&mut self) {
        self.knobs.clear_offsets();
    }

    fn set_param_override(&mut self, param: Param) {
        // Sequencer automation of a declared knob: a transient override on top of
        // the stored base (cleared on transport stop).
        if let Param::Script(ScriptParam::Knob(name, value)) = param {
            self.knobs.set_override(name, value);
        }
    }

    fn clear_param_overrides(&mut self) {
        self.knobs.clear_overrides();
    }

    fn effective_param(&self, name: PortName) -> Option<f32> {
        self.knobs.effective(name)
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Script
    }

    fn reset(&mut self) {
        self.outputs = [0.0; SCRIPT_MODULE_PORTS];
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Re-seed the program state (zero cells, re-seed PRNG from the voice index
        // for a deterministic retrigger).
        self.regs.reset(self.voice_index, SCRIPT_PRNG_SEED);
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, _sample_rate: SampleRate) {}

    fn scripts(&self) -> Option<&[Option<Arc<BoundScript>>]> {
        // One program; expose it as a 1-element slice so the voice's slot-0
        // source-resolution path works uniformly with the AudioScript module.
        Some(std::slice::from_ref(&self.script))
    }

    fn set_script(
        &mut self,
        slot: usize,
        script: Option<Arc<BoundScript>>,
    ) -> Option<Arc<BoundScript>> {
        // Only slot 0 is meaningful — this is one program, not a rack. Installing a
        // higher slot (e.g. loading a legacy 8-slot patch) is a graceful no-op that
        // hands the script straight back for a deferred drop off the audio thread;
        // the loader warns off-thread (see the patch-apply path). No `tracing`/log
        // here — this runs on the audio thread via the command drain.
        if slot != 0 {
            return script;
        }
        // Clearing the program must also drop the cached outputs, otherwise the
        // ports would keep emitting the last evaluated values.
        if script.is_none() {
            self.outputs = [0.0; SCRIPT_MODULE_PORTS];
        }
        let replaced = std::mem::replace(&mut self.script, script);
        // Remap the knob store from the new program's declared params — in place,
        // RT-safe — keeping a surviving knob's value across an edit.
        match &self.script {
            Some(s) => self.knobs.remap(&s.params),
            None => self.knobs.clear(),
        }
        replaced
    }

    fn eval_control_multi(&mut self, sources: &[f32], ctx: &EvalContext) {
        // Cheap Arc clone (atomic bump, no alloc) so the immutable script borrow
        // does not conflict with the mutable `regs` borrow.
        let script = self.script.clone();
        if let Some(s) = &script {
            let outs = s.script.eval_multi(sources, &mut self.regs, ctx);
            for (slot, cell) in self.outputs.iter_mut().enumerate() {
                *cell = outs[slot].unwrap_or(0.0);
            }
        } else {
            self.outputs = [0.0; SCRIPT_MODULE_PORTS];
        }
    }

    fn set_voice_index(&mut self, voice_index: u32) {
        // The graph has already folded the module identity into `voice_index`, so
        // two script modules in one voice decorrelate. Re-seed this voice's stream.
        self.voice_index = voice_index;
        self.regs.reset(voice_index, SCRIPT_PRNG_SEED);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::script::{BoundScript, CompiledScript, Op, ScriptInput, ScriptParamDecl};

    /// A program whose bare `out` (slot-0 fallback) is the constant `v`.
    fn const_script(v: f32) -> Arc<BoundScript> {
        Arc::new(BoundScript::new(
            CompiledScript::new(vec![Op::PushConst(0)], vec![v], 0, 0),
            Vec::new(),
            format!("out = {v}"),
        ))
    }

    #[test]
    fn port_name_and_slot_round_trip() {
        for slot in 0..SCRIPT_MODULE_PORTS {
            let out = output_port_name(slot).expect("in-range slot");
            assert_eq!(output_port_slot(&out), Some(slot));
            let inp = input_port_name(slot).expect("in-range slot");
            assert_eq!(input_port_slot(&inp), Some(slot));
        }
        assert_eq!(output_port_name(SCRIPT_MODULE_PORTS), None);
        assert_eq!(output_port_slot("out0"), None);
        assert_eq!(output_port_slot("out5"), None);
        assert_eq!(output_port_slot("in1"), None);
        assert_eq!(input_port_slot("in5"), None);
        assert_eq!(input_port_slot("out1"), None);
        assert_eq!(input_port_slot("in_l"), None);
    }

    #[test]
    fn descriptor_declares_four_in_and_four_out_ports() {
        let m = ScriptModule::new();
        let desc = m.descriptor();
        let ins = desc
            .ports
            .iter()
            .filter(|p| p.label.starts_with("In "))
            .count();
        let outs = desc
            .ports
            .iter()
            .filter(|p| p.label.starts_with("Out "))
            .count();
        assert_eq!(ins, SCRIPT_MODULE_PORTS);
        assert_eq!(outs, SCRIPT_MODULE_PORTS);
    }

    #[test]
    fn eval_caches_and_process_broadcasts_over_whole_buffer() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(0, Some(const_script(0.7)));
        let ctx = EvalContext::new(750.0);

        // Voice-driven eval caches the outputs (out1 = 0.7 via slot-0 fallback).
        m.eval_control_multi(&[], &ctx);

        // process() fills the entire out1 buffer with the cached value.
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::intern("out1"), AudioBuffer::new(64));
        outs.insert(PortName::intern("out2"), AudioBuffer::new(64));
        m.process(InputPorts::new(&[]), &mut outs, &ProcessContext::default());
        let out1 = &outs[&PortName::intern("out1")];
        assert!(
            (0..64).all(|i| (out1[i] - 0.7).abs() < 1e-6),
            "whole out1 buffer filled"
        );
        // An unwritten output stays at 0.
        let out2 = &outs[&PortName::intern("out2")];
        assert!((0..64).all(|i| out2[i] == 0.0));
    }

    #[test]
    fn in_ports_feed_outputs() {
        // out2 = in1 * in2; sources fed as ControlIn(0)=0.5, ControlIn(1)=0.4.
        let mut m = ScriptModule::new();
        let _ = m.set_script(
            0,
            Some(Arc::new(BoundScript::new(
                CompiledScript::new(
                    vec![
                        Op::PushSource(0),
                        Op::PushSource(1),
                        Op::Mul,
                        Op::StoreOut(1),
                    ],
                    vec![],
                    2,
                    0,
                ),
                vec![ScriptInput::ControlIn(0), ScriptInput::ControlIn(1)],
                "out2 = in1 * in2".to_string(),
            ))),
        );
        let ctx = EvalContext::new(750.0);
        m.eval_control_multi(&[0.5, 0.4], &ctx);

        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::intern("out1"), AudioBuffer::new(8));
        outs.insert(PortName::intern("out2"), AudioBuffer::new(8));
        m.process(InputPorts::new(&[]), &mut outs, &ProcessContext::default());
        // out2 = 0.5 * 0.4 = 0.2; out1 unwritten = 0.
        assert!((0..8).all(|i| (outs[&PortName::intern("out2")][i] - 0.2).abs() < 1e-6));
        assert!((0..8).all(|i| outs[&PortName::intern("out1")][i] == 0.0));
    }

    #[test]
    fn higher_slots_are_a_graceful_no_op() {
        // Only slot 0 installs a program; a legacy higher slot hands the script
        // straight back (for a deferred drop) and installs nothing.
        let mut m = ScriptModule::new();
        let handed_back = m.set_script(3, Some(const_script(0.9)));
        assert!(
            handed_back.is_some(),
            "slot 3 install is a no-op passthrough"
        );
        assert!(m.script.is_none(), "no program installed for slot 3");
    }

    /// A program `out1 = drive` where `drive` is a declared knob (source
    /// register 0 = `LocalParam`), with the decl carried on the BoundScript.
    fn param_script(name: &str, default: f32) -> Arc<BoundScript> {
        let pn = PortName::intern(name);
        let cs = CompiledScript::new(vec![Op::PushSource(0), Op::StoreOut(0)], vec![], 1, 0);
        Arc::new(
            BoundScript::new(
                cs,
                vec![ScriptInput::LocalParam(pn)],
                format!("param {name} = {default}\nout1 = {name}"),
            )
            .with_params(vec![ScriptParamDecl {
                name: pn,
                name_str: pn.as_str(),
                default,
                min: 0.0,
                max: 1.0,
                label: Some("Drive".to_string()),
                tooltip: None,
                unit: synth_core::ParameterUnit::None,
            }]),
        )
    }

    #[test]
    fn knob_shows_in_descriptor_and_drives_the_output() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(0, Some(param_script("drive", 0.5)));
        let drive = PortName::intern("drive");

        // The knob appears as a descriptor param (label "Drive") at its default.
        let desc = m.descriptor();
        let p = desc
            .parameters
            .iter()
            .find(|p| p.type_id == "drive")
            .expect("knob descriptor");
        assert_eq!(p.name, "Drive");
        assert_eq!(
            m.get_param(&Param::Script(ScriptParam::Knob(drive, 0.0))),
            Some(0.5)
        );

        // Turn the knob; the voice fills the LocalParam register from the store.
        m.set_param(Param::Script(ScriptParam::Knob(drive, 0.8)));
        let eff = m.effective_param(drive).expect("effective knob");
        m.eval_control_multi(&[eff], &EvalContext::new(750.0));
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::intern("out1"), AudioBuffer::new(8));
        m.process(InputPorts::new(&[]), &mut outs, &ProcessContext::default());
        assert!((0..8).all(|i| (outs[&PortName::intern("out1")][i] - 0.8).abs() < 1e-6));
    }

    #[test]
    fn mod_offset_adds_into_the_effective_knob_and_clears() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(0, Some(param_script("drive", 0.5)));
        let drive = PortName::intern("drive");
        m.set_mod_offset("drive", 0.2);
        assert!((m.effective_param(drive).unwrap() - 0.7).abs() < 1e-6);
        // An unknown target is a silent no-op.
        m.set_mod_offset("nope", 9.0);
        m.clear_mod_offsets();
        assert_eq!(m.effective_param(drive), Some(0.5));
    }

    #[test]
    fn editing_the_script_keeps_a_surviving_knob_value() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(0, Some(param_script("drive", 0.5)));
        let drive = PortName::intern("drive");
        m.set_param(Param::Script(ScriptParam::Knob(drive, 0.9)));
        // Re-install a program that still declares `drive`: its value survives.
        let _ = m.set_script(0, Some(param_script("drive", 0.5)));
        assert_eq!(
            m.get_param(&Param::Script(ScriptParam::Knob(drive, 0.0))),
            Some(0.9)
        );
        // Clearing the program drops the knobs.
        let _ = m.set_script(0, None);
        assert!(m.get_params().is_empty());
    }

    #[test]
    fn clearing_the_program_zeroes_cached_outputs() {
        let mut m = ScriptModule::new();
        let _ = m.set_script(0, Some(const_script(0.9)));
        let ctx = EvalContext::new(750.0);
        m.eval_control_multi(&[], &ctx);
        // Clear the program — cached outputs must drop to 0 so the ports go silent.
        assert!(m.set_script(0, None).is_some());
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::intern("out1"), AudioBuffer::new(8));
        m.process(InputPorts::new(&[]), &mut outs, &ProcessContext::default());
        assert!((0..8).all(|i| outs[&PortName::intern("out1")][i] == 0.0));
    }
}
