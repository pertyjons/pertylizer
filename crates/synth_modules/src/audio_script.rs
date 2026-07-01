//! Audio-rate scripted DSP module — a YAMS program evaluated **once per sample**.
//!
//! Unlike the control-rate [`ScriptModule`](crate::ScriptModule) (one eval per
//! block, output broadcast across the buffer), the `AudioScript` module runs its
//! program at the full sample rate inside [`process`](PolyModule::process), so it
//! can implement per-sample DSP: waveshaper/folder, bitcrusher, ring-mod, custom
//! IIR/feedback filters. It exposes stereo audio in (`in_l`/`in_r`) and out
//! (`out_l`/`out_r`) ports; the program reads `in` / `in_l` / `in_r` and writes
//! a mono `out` (duplicated) or `out.left` / `out.right`.
//!
//! The compiled program is installed via [`set_script`](PolyModule::set_script)
//! (compiled with `audio_rate` enabled). Its **block-constant** sources
//! (macros, context vars, module addresses) are resolved by the `Voice` once per
//! block and delivered via
//! [`set_audio_block_sources`](PolyModule::set_audio_block_sources); the
//! per-sample audio inputs and `first_sample` one-shot are injected by the VM's
//! `eval_block` from the [`AudioBindings`] computed at install time.

use std::collections::HashMap;
use std::sync::Arc;

use synth_core::script::{
    AudioBindings, AudioInputChannel, BoundScript, EvalContext, RegisterFile, SCRIPT_PRNG_SEED,
    ScriptContext, ScriptInput,
};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleType, Param,
    PolyModule, PortDescriptor, PortName, ProcessContext,
};
use synth_core::{MidiNote, Velocity};

/// A per-voice audio-rate scripted DSP node.
///
/// Holds exactly one program (unlike the 8-slot control
/// [`ScriptModule`](crate::ScriptModule)): an immutable `Arc<BoundScript>` shared
/// across voices, plus this voice's own mutable [`RegisterFile`] (state cells +
/// PRNG). The voice index — already folded with the module identity by the graph
/// — seeds the PRNG so simultaneous voices decorrelate and retriggers reproduce.
#[derive(Clone)]
pub struct AudioScript {
    /// The compiled program (shared immutably). `None` until one is installed.
    script: Option<Arc<BoundScript>>,
    /// This voice's persistent state + PRNG for the program.
    regs: RegisterFile,
    /// Stable per-(voice, module) seed base, set at voice allocation and used to
    /// re-seed `regs` on note-on.
    voice_index: u32,
    /// Resolved block-constant source values for the current block (filled by the
    /// voice via [`set_audio_block_sources`](PolyModule::set_audio_block_sources));
    /// the per-sample audio-in / `first_sample` registers are overwritten by
    /// `eval_block`. Sized to the installed script's source count.
    sources: Vec<f32>,
    /// Which source registers carry the per-sample audio inputs / `first_sample`,
    /// computed from the script's inputs at install time.
    bindings: AudioBindings,
    /// Per-channel output scratch, filled by `eval_block` then copied into the
    /// output port buffers (a `HashMap` cannot yield two `&mut` channel buffers at
    /// once). Pre-grown to the block size; reused every block.
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    /// `true` until the first block of the note has been processed — drives the
    /// `first_sample` one-shot. Set by `note_on`, cleared after the first block.
    first_block: bool,
}

impl AudioScript {
    #[must_use]
    pub fn new() -> Self {
        Self {
            script: None,
            regs: RegisterFile::new(0, SCRIPT_PRNG_SEED),
            voice_index: 0,
            sources: Vec::new(),
            bindings: AudioBindings::default(),
            out_l: Vec::new(),
            out_r: Vec::new(),
            first_block: true,
        }
    }

    /// Compute the per-sample [`AudioBindings`] from a script's input list: which
    /// source registers read the left/right audio input and the `first_sample`
    /// one-shot. Everything else is a block-constant the voice resolves.
    fn bindings_for(script: &BoundScript) -> AudioBindings {
        let mut b = AudioBindings::default();
        for (i, input) in script.inputs.iter().enumerate() {
            let reg = i as u16;
            match input {
                ScriptInput::AudioIn(AudioInputChannel::Left) => b.in_left = Some(reg),
                ScriptInput::AudioIn(AudioInputChannel::Right) => b.in_right = Some(reg),
                ScriptInput::Context(ScriptContext::FirstSample) => b.first_sample = Some(reg),
                _ => {}
            }
        }
        b
    }
}

impl Default for AudioScript {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for AudioScript {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("audio_script", "Audio Script")
            .description(
                "Audio-rate scripted DSP (one YAMS eval per sample): waveshaper, bitcrusher, \
                 ring-mod, custom IIR. Stereo in/out.",
            )
            // A per-voice graph node (not an effect-chain insert), like its
            // control-rate sibling `ScriptModule` — must be wired by hand.
            .category(ModuleCategory::Utility)
            .tag("script")
            .tag("dsp")
            .tag("effect")
            .port(
                PortDescriptor::audio_input(PortName::IN_L, "In L").description("Left audio input"),
            )
            .port(
                PortDescriptor::audio_input(PortName::IN_R, "In R")
                    .description("Right audio input"),
            )
            .port(
                PortDescriptor::audio_output(PortName::OUT_L, "Out L")
                    .description("Left audio output"),
            )
            .port(
                PortDescriptor::audio_output(PortName::OUT_R, "Out R")
                    .description("Right audio output"),
            )
    }
}

impl PolyModule for AudioScript {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext<'_>,
    ) {
        let n = context.samples.as_usize();

        // Per-sample audio inputs. A mono signal patched to `in_l` feeds the
        // program's `in` (which aliases the left channel); `in_r` falls back to
        // the left channel when unconnected.
        let in_l = inputs
            .get(PortName::IN_L)
            .map_or(&[][..], |b| &b.as_slice()[..b.len().min(n)]);
        let in_r = inputs
            .get(PortName::IN_R)
            .map_or(in_l, |b| &b.as_slice()[..b.len().min(n)]);

        // Reused per-channel scratch (eval_block needs two disjoint &mut slices,
        // which a HashMap cannot hand out simultaneously). Amortized no-alloc.
        if self.out_l.len() < n {
            self.out_l.resize(n, 0.0);
            self.out_r.resize(n, 0.0);
        }
        // Build the eval context from the live per-call sample rate, never a
        // cached field: the offline render loader (`arrangement_render.rs`) never
        // calls `set_sample_rate`, so a stale 48 kHz default would detune every
        // audio-rate `phasor`/time op whenever the render rate differs (44.1 kHz).
        let ctx = EvalContext::audio(context.sample_rate.as_f32());
        // Cheap Arc clone (atomic bump, no alloc) so the immutable script borrow
        // does not conflict with the mutable `sources`/`regs`/output borrows.
        let script = self.script.clone();
        let ran = if let Some(s) = &script {
            s.script.eval_block(
                &mut self.sources,
                &self.bindings,
                in_l,
                in_r,
                &mut self.out_l[..n],
                &mut self.out_r[..n],
                &mut self.regs,
                &ctx,
                self.first_block,
            );
            true
        } else {
            false
        };
        // Clear the one-shot only once a non-empty block actually ran with a
        // program, so `first_sample` cannot be lost on a zero-sample block or
        // before a program is installed mid-note.
        if ran && n > 0 {
            self.first_block = false;
        }

        // Copy the scratch channels into the output ports (or silence with no
        // script). Done channel-by-channel since `outputs` is a `HashMap`.
        if let Some(buf) = outputs.get_mut(&PortName::OUT_L) {
            write_channel(buf, &self.out_l[..n], ran);
        }
        if let Some(buf) = outputs.get_mut(&PortName::OUT_R) {
            write_channel(buf, &self.out_r[..n], ran);
        }
    }

    fn set_param(&mut self, _param: Param) {
        // No numeric parameters — the program is installed via `set_script`.
    }

    fn get_param(&self, _param: &Param) -> Option<f32> {
        None
    }

    fn get_params(&self) -> Vec<Param> {
        Vec::new()
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::AudioScript
    }

    fn reset(&mut self) {
        self.first_block = true;
        self.out_l.fill(0.0);
        self.out_r.fill(0.0);
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Re-seed the program state (zero cells, re-seed PRNG from the voice index
        // for a deterministic retrigger) and arm the `first_sample` one-shot.
        self.regs.reset(self.voice_index, SCRIPT_PRNG_SEED);
        self.first_block = true;
    }

    fn note_off(&mut self) {}

    fn scripts(&self) -> Option<&[Option<Arc<BoundScript>>]> {
        // One program; expose it as a 1-element slice so the voice's slot-0
        // source-resolution path works uniformly with the control Script module.
        Some(std::slice::from_ref(&self.script))
    }

    fn set_script(
        &mut self,
        _slot: usize,
        script: Option<Arc<BoundScript>>,
    ) -> Option<Arc<BoundScript>> {
        // Recompute the per-sample bindings and size the block-constant source
        // buffer from the new program. Done here (the install runs on the audio
        // thread via the command drain), but it is only a small scan + a buffer
        // resize that stabilizes to the script's source count. Returns the
        // replaced `Arc` so the engine drops it off the audio thread.
        match &script {
            Some(s) => {
                self.bindings = Self::bindings_for(s);
                self.sources.resize(s.script.source_count() as usize, 0.0);
            }
            None => {
                self.bindings = AudioBindings::default();
                self.sources.clear();
            }
        }
        std::mem::replace(&mut self.script, script)
    }

    fn set_audio_block_sources(&mut self, sources: &[f32]) {
        // Copy the voice-resolved block constants into our buffer, preserving the
        // per-sample audio-in / `first_sample` registers (overwritten in `process`).
        let n = sources.len().min(self.sources.len());
        self.sources[..n].copy_from_slice(&sources[..n]);
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

/// Copy one scratch channel into an output buffer, or silence it when no script
/// ran (`ran == false`), so a port consumer never reads a stale block.
fn write_channel(buf: &mut AudioBuffer, scratch: &[f32], ran: bool) {
    let dst = buf.as_mut_slice();
    if ran {
        let k = dst.len().min(scratch.len());
        dst[..k].copy_from_slice(&scratch[..k]);
        dst[k..].fill(0.0);
    } else {
        dst.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::script::{Builtin, CompiledScript, Op};
    use synth_core::{SampleCount, SampleRate};

    const N: usize = 16;

    /// Build a `BoundScript` from raw parts (no compiler dependency).
    fn bound(code: Vec<Op>, constants: Vec<f32>, inputs: Vec<ScriptInput>) -> Arc<BoundScript> {
        let src_count = inputs.len() as u16;
        Arc::new(BoundScript::new(
            CompiledScript::new(code, constants, src_count, 0),
            inputs,
            String::new(),
        ))
    }

    fn run(m: &mut AudioScript, input: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut in_buf = AudioBuffer::new(input.len());
        in_buf.as_mut_slice().copy_from_slice(input);
        let in_ports = [(PortName::IN_L, &in_buf)];
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::OUT_L, AudioBuffer::new(input.len()));
        outs.insert(PortName::OUT_R, AudioBuffer::new(input.len()));
        let ctx = ProcessContext {
            samples: SampleCount::new(input.len()),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        m.process(InputPorts::new(&in_ports), &mut outs, &ctx);
        (
            outs[&PortName::OUT_L].as_slice().to_vec(),
            outs[&PortName::OUT_R].as_slice().to_vec(),
        )
    }

    #[test]
    fn bindings_map_audio_in_and_first_sample_registers() {
        let s = bound(
            vec![Op::PushSource(0)],
            vec![],
            vec![
                ScriptInput::AudioIn(AudioInputChannel::Right),
                ScriptInput::Context(ScriptContext::FirstSample),
                ScriptInput::AudioIn(AudioInputChannel::Left),
            ],
        );
        let b = AudioScript::bindings_for(&s);
        assert_eq!(b.in_right, Some(0));
        assert_eq!(b.first_sample, Some(1));
        assert_eq!(b.in_left, Some(2));
    }

    #[test]
    fn waveshaper_runs_per_sample_and_duplicates_to_both_channels() {
        // out = tanh(in_l * 4)
        let mut m = AudioScript::new();
        let _ = m.set_script(
            0,
            Some(bound(
                vec![
                    Op::PushSource(0),
                    Op::PushConst(0),
                    Op::Mul,
                    Op::Call(Builtin::Tanh),
                ],
                vec![4.0],
                vec![ScriptInput::AudioIn(AudioInputChannel::Left)],
            )),
        );
        let input: Vec<f32> = (0..N).map(|i| (i as f32 / N as f32) - 0.5).collect();
        let (l, r) = run(&mut m, &input);
        for i in 0..N {
            let expected = (input[i] * 4.0).tanh();
            assert!((l[i] - expected).abs() < 1e-6, "L sample {i}");
            assert!((r[i] - expected).abs() < 1e-6, "R duplicated");
        }
    }

    #[test]
    fn multi_out_writes_independent_channels() {
        // out.left = in_l;  out.right = -in_l
        let mut m = AudioScript::new();
        let _ = m.set_script(
            0,
            Some(bound(
                vec![
                    Op::PushSource(0),
                    Op::StoreAudioOut(0),
                    Op::PushSource(0),
                    Op::Neg,
                    Op::StoreAudioOut(1),
                ],
                vec![],
                vec![ScriptInput::AudioIn(AudioInputChannel::Left)],
            )),
        );
        let input: Vec<f32> = vec![0.25; N];
        let (l, r) = run(&mut m, &input);
        assert!(l.iter().all(|&v| (v - 0.25).abs() < 1e-6));
        assert!(r.iter().all(|&v| (v + 0.25).abs() < 1e-6));
    }

    #[test]
    fn unscripted_module_is_silent() {
        let mut m = AudioScript::new();
        let (l, r) = run(&mut m, &[0.5; N]);
        assert!(l.iter().all(|&v| v == 0.0), "no script → silence");
        assert!(r.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn block_constant_source_is_applied() {
        // out = in_l * k, where k is a block-constant source (register 1) the voice
        // fills via set_audio_block_sources.
        let mut m = AudioScript::new();
        let _ = m.set_script(
            0,
            Some(bound(
                vec![Op::PushSource(0), Op::PushSource(1), Op::Mul],
                vec![],
                vec![
                    ScriptInput::AudioIn(AudioInputChannel::Left),
                    ScriptInput::Zero, // a stand-in block-constant (e.g. a macro)
                ],
            )),
        );
        m.set_audio_block_sources(&[0.0, 0.5]); // register 0 (audio-in) ignored, reg 1 = 0.5
        let (l, _r) = run(&mut m, &[1.0; N]);
        assert!(
            l.iter().all(|&v| (v - 0.5).abs() < 1e-6),
            "gain from block source"
        );
    }

    #[test]
    fn first_sample_survives_a_zero_sample_block() {
        // `out = first_sample`. A zero-sample block (n == 0) must NOT consume the
        // one-shot — the next real block's sample 0 still reads 1.
        let mut m = AudioScript::new();
        let _ = m.set_script(
            0,
            Some(bound(
                vec![Op::PushSource(0)],
                vec![],
                vec![ScriptInput::Context(ScriptContext::FirstSample)],
            )),
        );
        m.note_on(MidiNote::A4, Velocity::MAX);

        // A zero-sample block first (nothing ran).
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::OUT_L, AudioBuffer::new(0));
        outs.insert(PortName::OUT_R, AudioBuffer::new(0));
        let zero_ctx = ProcessContext {
            samples: SampleCount::new(0),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        m.process(InputPorts::empty(), &mut outs, &zero_ctx);

        // Then a real block — first_sample must still fire at sample 0.
        let (l, _r) = run(&mut m, &[0.0; N]);
        assert!(
            (l[0] - 1.0).abs() < 1e-6,
            "first_sample fires on the first real sample"
        );
        assert!(l[1..].iter().all(|&v| v == 0.0), "and only there");
    }

    /// Run an audio-rate `out = phasor(freq)` block at `sr` and return its
    /// left-channel phase ramp. No audio inputs — a pure scripted oscillator.
    fn phasor_ramp(freq: f32, sr: SampleRate, n: usize) -> Vec<f32> {
        let mut m = AudioScript::new();
        let _ = m.set_script(
            0,
            Some(bound(
                vec![Op::PushConst(0), Op::Phasor(0)],
                vec![freq],
                vec![],
            )),
        );
        let mut outs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outs.insert(PortName::OUT_L, AudioBuffer::new(n));
        outs.insert(PortName::OUT_R, AudioBuffer::new(n));
        let ctx = ProcessContext {
            samples: SampleCount::new(n),
            sample_rate: sr,
            ..ProcessContext::default()
        };
        m.process(InputPorts::empty(), &mut outs, &ctx);
        outs[&PortName::OUT_L].as_slice().to_vec()
    }

    #[test]
    fn phasor_tracks_live_sample_rate_not_a_stale_default() {
        // Regression for the offline-render detune bug: `process` must build the
        // eval context from `ProcessContext.sample_rate`, not a cached field that
        // defaulted to 48 kHz. A `phasor(freq)` ramps by `freq / sample_rate` per
        // sample; the slope must match whatever rate the caller passes (the offline
        // render runs at 44.1 kHz, the device at 48 kHz).
        let freq = 220.0;
        for sr in [SampleRate::CD_QUALITY, SampleRate::DVD_QUALITY] {
            let n = 32;
            let l = phasor_ramp(freq, sr, n);
            let expected = freq / sr.as_f32(); // per-sample phase increment
            for i in 1..n {
                let delta = l[i] - l[i - 1];
                assert!(
                    (delta - expected).abs() < 1e-6,
                    "sr {}: sample {i} slope {delta} != {expected}",
                    sr.as_f32()
                );
            }
        }
    }

    #[test]
    fn clearing_the_script_silences_and_resets_bindings() {
        let mut m = AudioScript::new();
        let _ = m.set_script(
            0,
            Some(bound(
                vec![Op::PushSource(0)],
                vec![],
                vec![ScriptInput::AudioIn(AudioInputChannel::Left)],
            )),
        );
        assert_eq!(m.bindings.in_left, Some(0));
        let replaced = m.set_script(0, None);
        assert!(
            replaced.is_some(),
            "returns the old script for deferred drop"
        );
        assert_eq!(m.bindings.in_left, None);
        assert!(m.sources.is_empty());
        let (l, _r) = run(&mut m, &[0.7; N]);
        assert!(l.iter().all(|&v| v == 0.0));
    }
}
