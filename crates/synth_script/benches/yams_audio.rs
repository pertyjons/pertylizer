//! Audio-rate YAMS benchmark — the scalar `eval_block` cost that Phase 4's
//! perf model hypothesizes (the order-of-magnitude lever vs. control rate).
//!
//! Run with:
//!   cargo bench -p synth_script --bench yams_audio
//!
//! Reports ns/sample for a few representative programs and the implied
//! core-fraction at 48 kHz × 16 voices (one `AudioScript` instance per voice).
//! The vectorized fast-path stays deferred until a bench shows scalar dispatch is
//! the bottleneck (plan §Phase 4); this measures the scalar baseline.

use std::hint::black_box;
use std::time::Instant;

use synth_core::script::{
    AudioBindings, AudioBlockInputs, AudioBlockOutputs, AudioInputChannel, BoundScript,
    EvalContext, NoteFrequencyRange, RegisterFile, ScriptContext, ScriptInput,
};
use synth_script::{CompileOptions, compile};

const SAMPLE_RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 20_000; // ~107 s of audio per program
const VOICES: usize = 16;

/// Compile an audio-rate program and derive its per-sample bindings (mirrors what
/// the `AudioScript` module does at install time).
fn build(src: &str) -> (BoundScript, AudioBindings, usize) {
    let opts = CompileOptions {
        audio_rate: true,
        control_rate: SAMPLE_RATE,
        ..CompileOptions::default()
    };
    let (prog, diags) = compile(src, &opts);
    let prog = prog.unwrap_or_else(|| panic!("compile failed for `{src}`: {diags:?}"));
    let n_sources = prog.inputs.len();
    let bound = prog.into_bound(src.to_string());
    let mut b = AudioBindings::default();
    for (i, input) in bound.inputs.iter().enumerate() {
        let reg = synth_core::script::ScriptRegister::new(i as u16);
        match input {
            ScriptInput::AudioIn(AudioInputChannel::Left) => b.in_left = Some(reg),
            ScriptInput::AudioIn(AudioInputChannel::Right) => b.in_right = Some(reg),
            ScriptInput::Context(ScriptContext::NoteHz) => b.note_hz = Some(reg),
            ScriptInput::Context(ScriptContext::FirstSample) => b.first_sample = Some(reg),
            _ => {}
        }
    }
    (bound, b, n_sources)
}

fn bench(label: &str, src: &str) {
    let (bound, bindings, n_sources) = build(src);
    let mut regs = RegisterFile::new(0, 0x5EED);
    let mut sources = vec![0.0f32; n_sources];
    let input: Vec<f32> = (0..BLOCK)
        .map(|i| ((i as f32 / BLOCK as f32) * 2.0 - 1.0) * 0.7)
        .collect();
    let mut out_l = vec![0.0f32; BLOCK];
    let mut out_r = vec![0.0f32; BLOCK];
    let ctx = EvalContext::audio(SAMPLE_RATE);
    let inputs = AudioBlockInputs {
        in_left: &input,
        in_right: &input,
        note_hz: NoteFrequencyRange::default(),
        first_block: false,
    };

    // Warm up.
    for _ in 0..64 {
        let mut outputs = AudioBlockOutputs {
            left: &mut out_l,
            right: &mut out_r,
        };
        bound.script.eval_block(
            &mut sources,
            &bindings,
            &inputs,
            &mut outputs,
            &mut regs,
            &ctx,
        );
    }

    let start = Instant::now();
    for _ in 0..BLOCKS {
        let mut outputs = AudioBlockOutputs {
            left: black_box(&mut out_l),
            right: black_box(&mut out_r),
        };
        bound.script.eval_block(
            black_box(&mut sources),
            black_box(&bindings),
            black_box(&inputs),
            black_box(&mut outputs),
            black_box(&mut regs),
            black_box(&ctx),
        );
    }
    let elapsed = start.elapsed();
    black_box(&out_l);

    let total_samples = (BLOCKS * BLOCK) as f64;
    let ns_per_sample = elapsed.as_nanos() as f64 / total_samples;
    // Fraction of one core at the audio rate, ×VOICES instances.
    let core_fraction_one = ns_per_sample * SAMPLE_RATE as f64 / 1e9;
    let ops = bound.script.code().len();
    println!(
        "{label:<28} {ops:>3} ops   {ns_per_sample:>6.2} ns/sample   \
         {:>5.1}% core (1 voice)   {:>5.1}% core ({VOICES} voices)",
        core_fraction_one * 100.0,
        core_fraction_one * VOICES as f64 * 100.0,
    );
}

fn main() {
    println!(
        "YAMS audio-rate eval_block — {SAMPLE_RATE:.0} Hz, {BLOCK}-sample blocks, \
         {BLOCKS} blocks/program\n"
    );
    bench("passthrough", "out = in");
    bench("gain", "out = in * 0.5");
    bench("tanh waveshaper", "out = tanh(in * 4)");
    bench("hard-clip fold", "out = clamp(in * 3, -1, 1)");
    bench("ring-mod (phasor)", "out = in * sin(phasor(110) * tau)");
    bench(
        "one-pole lowpass (IIR)",
        "state y = 0\ny = y + 0.1 * (in - y)\nout = y",
    );
    bench(
        "biquad-ish (2 states)",
        "state z1 = 0\nstate z2 = 0\nlet y = 0.2 * in + z1\nz1 = 0.1 * in - 0.3 * y + z2\nz2 = 0.05 * in - 0.15 * y\nout = y",
    );
    bench(
        "stereo widener (multi-out)",
        "let m = (in_l + in_r) * 0.5\nlet s = (in_l - in_r) * 0.5\nout.left = m + s * 1.5\nout.right = m - s * 1.5",
    );
}
