//! Audio processing benchmark for profiling.
//!
//! Run with:
//!   cargo bench -p synth_engine --bench audio_profile
//!
//! For flamegraph profiling:
//!   cargo flamegraph --bench audio_profile -p synth_engine --root
//!
//! For perf:
//!   perf record -g cargo bench -p synth_engine --bench audio_profile
//!   perf report

use std::hint::black_box;
use std::time::Instant;

use synth_core::{
    AudioBuffer, Bpm, Cents, Gain, Hertz, MidiNote, NormalizedValue, Param, ProcessContext,
    SampleCount, SamplePosition, SampleRate, Seconds, Velocity, Waveform,
};
use synth_core::{EnvelopeParam, FilterParam, LfoParam, OscillatorParam};
use synth_engine::graph::ModuleGraph;
use synth_engine::voice::Voice;
use synth_modules::{
    AdditiveOsc, Amplifier, Envelope, Filter, FractalOscillator, GranularOsc, Lfo,
    MathOscillator, Oscillator, StereoOutput, WavetableOsc,
};

const SAMPLE_RATE: SampleRate = SampleRate::DVD_QUALITY; // 48 kHz
const BLOCK_SIZE: usize = 256;
const NUM_BLOCKS: usize = 10_000; // ~53 seconds of audio
const NUM_VOICES: usize = 8;

// ── Patch builders ─────────────────────────────────────────────────────────

/// Classic subtractive: Saw → Filter → Amp → Output, with Envelope + LFO.
fn build_subtractive_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc = graph.add_module(Box::new(Oscillator::new()));
    let filter = graph.add_module(Box::new(Filter::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let lfo = graph.add_module(Box::new(Lfo::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    graph.connect(osc, "out", filter, "in").unwrap();
    graph.connect(filter, "out", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();
    graph.connect(lfo, "out", filter, "cutoff_cv").unwrap();

    // Configure
    graph.set_param(osc, Param::Oscillator(OscillatorParam::Waveform(Waveform::Sawtooth)));
    graph.set_param(
        filter,
        Param::Filter(FilterParam::Cutoff(Hertz::new(800.0))),
    );
    graph.set_param(
        filter,
        Param::Filter(FilterParam::Resonance(NormalizedValue::new(0.4))),
    );
    graph.set_param(
        env,
        Param::Envelope(EnvelopeParam::Attack(Seconds::new(0.01))),
    );
    graph.set_param(
        env,
        Param::Envelope(EnvelopeParam::Release(Seconds::new(0.5))),
    );
    graph.set_param(lfo, Param::Lfo(LfoParam::Rate(Hertz::new(0.5))));
    graph.set_param(
        lfo,
        Param::Lfo(LfoParam::Depth(NormalizedValue::new(0.3))),
    );

    Voice::from_graph(0, graph)
}

/// Heavy additive: 32-harmonic additive osc → Filter → Amp → Output.
fn build_additive_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc = graph.add_module(Box::new(AdditiveOsc::new()));
    let filter = graph.add_module(Box::new(Filter::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    graph.connect(osc, "out", filter, "in").unwrap();
    graph.connect(filter, "out", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();

    Voice::from_graph(0, graph)
}

/// Fractal oscillator: 64 partials → Amp → Output (stereo out_l/out_r).
fn build_fractal_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc = graph.add_module(Box::new(FractalOscillator::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    // Fractal osc has stereo output (out_l/out_r), use left channel
    graph.connect(osc, "out_l", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();

    Voice::from_graph(0, graph)
}

/// Wavetable oscillator → Filter → Amp → Output.
fn build_wavetable_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc = graph.add_module(Box::new(WavetableOsc::new()));
    let filter = graph.add_module(Box::new(Filter::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    graph.connect(osc, "out", filter, "in").unwrap();
    graph.connect(filter, "out", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();

    Voice::from_graph(0, graph)
}

/// Granular oscillator → Amp → Output.
fn build_granular_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc = graph.add_module(Box::new(GranularOsc::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    graph.connect(osc, "out", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();

    Voice::from_graph(0, graph)
}

/// Math oscillator → Filter → Amp → Output.
fn build_math_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc = graph.add_module(Box::new(MathOscillator::new()));
    let filter = graph.add_module(Box::new(Filter::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    graph.connect(osc, "out", filter, "in").unwrap();
    graph.connect(filter, "out", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();

    Voice::from_graph(0, graph)
}

/// Dual oscillator with unison: 2x Saw (7 unison) → Filter → Amp → Output.
fn build_unison_voice() -> Voice {
    let mut graph = ModuleGraph::new();

    let osc1 = graph.add_module(Box::new(Oscillator::new()));
    let osc2 = graph.add_module(Box::new(Oscillator::new()));
    let amp_mix = graph.add_module(Box::new(Amplifier::new()));
    let filter = graph.add_module(Box::new(Filter::new()));
    let amp = graph.add_module(Box::new(Amplifier::new()));
    let env = graph.add_module(Box::new(Envelope::new()));
    let output = graph.add_module(Box::new(StereoOutput::new()));

    graph.connect(osc1, "out", amp_mix, "in").unwrap();
    // osc2 also to amp_mix — the amplifier will sum
    graph.connect(amp_mix, "out", filter, "in").unwrap();
    graph.connect(filter, "out", amp, "in").unwrap();
    graph.connect(amp, "out", output, "in").unwrap();
    graph.connect(env, "out", amp, "cv").unwrap();

    // Configure unison
    graph.set_param(
        osc1,
        Param::Oscillator(OscillatorParam::Waveform(Waveform::Sawtooth)),
    );
    graph.set_param(
        osc1,
        Param::Oscillator(OscillatorParam::UnisonVoices(7)),
    );
    graph.set_param(
        osc1,
        Param::Oscillator(OscillatorParam::UnisonDetune(Cents::new(15.0))),
    );
    graph.set_param(
        osc2,
        Param::Oscillator(OscillatorParam::Waveform(Waveform::Square)),
    );
    graph.set_param(
        osc2,
        Param::Oscillator(OscillatorParam::UnisonVoices(5)),
    );
    graph.set_param(
        osc2,
        Param::Oscillator(OscillatorParam::Level(Gain::new(0.5))),
    );

    Voice::from_graph(0, graph)
}

// ── Benchmark runner ───────────────────────────────────────────────────────

fn make_context() -> ProcessContext {
    ProcessContext {
        sample_rate: SAMPLE_RATE,
        samples: SampleCount::new(BLOCK_SIZE),
        tempo: Bpm::new(120.0),
        is_playing: true,
        ..ProcessContext::default()
    }
}

fn bench_voice(name: &str, mut voice: Voice) {
    let context = make_context();
    let mut left = AudioBuffer::new(BLOCK_SIZE);
    let mut right = AudioBuffer::new(BLOCK_SIZE);

    // Trigger note
    voice.note_on(MidiNote::new(60), Velocity::new(0.8), SamplePosition::ZERO);

    // Warm up (1 block)
    voice.process_audio(&mut left, &mut right, &context);

    // Timed run
    let start = Instant::now();
    for _ in 0..NUM_BLOCKS {
        left.clear();
        right.clear();
        voice.process_audio(&mut left, &mut right, &context);
        black_box(&left);
        black_box(&right);
    }
    let elapsed = start.elapsed();

    let total_samples = NUM_BLOCKS * BLOCK_SIZE;
    let audio_seconds = total_samples as f64 / SAMPLE_RATE.as_f32() as f64;
    let wall_seconds = elapsed.as_secs_f64();
    let realtime_ratio = audio_seconds / wall_seconds;
    let ns_per_sample = elapsed.as_nanos() as f64 / total_samples as f64;

    eprintln!(
        "  {name:<25} {wall_seconds:>7.3}s  ({audio_seconds:.1}s audio)  {realtime_ratio:>6.1}x realtime  {ns_per_sample:>5.1} ns/sample"
    );
}

fn bench_polyphony(name: &str, build_fn: fn() -> Voice) {
    let context = make_context();
    let mut voices: Vec<Voice> = (0..NUM_VOICES).map(|_| build_fn()).collect();

    let mut left = AudioBuffer::new(BLOCK_SIZE);
    let mut right = AudioBuffer::new(BLOCK_SIZE);
    let mut mix_left = AudioBuffer::new(BLOCK_SIZE);
    let mut mix_right = AudioBuffer::new(BLOCK_SIZE);

    // Trigger different notes across voices (spread chord)
    let notes = [48, 52, 55, 60, 64, 67, 72, 76];
    for (i, voice) in voices.iter_mut().enumerate() {
        voice.note_on(
            MidiNote::new(notes[i % notes.len()]),
            Velocity::new(0.7),
            SamplePosition::ZERO,
        );
    }

    // Warm up
    for voice in &mut voices {
        voice.process_audio(&mut left, &mut right, &context);
    }

    // Timed run
    let start = Instant::now();
    for _ in 0..NUM_BLOCKS {
        mix_left.clear();
        mix_right.clear();
        for voice in &mut voices {
            left.clear();
            right.clear();
            voice.process_audio(&mut left, &mut right, &context);
            mix_left.add_from(&left);
            mix_right.add_from(&right);
        }
        black_box(&mix_left);
        black_box(&mix_right);
    }
    let elapsed = start.elapsed();

    let total_samples = NUM_BLOCKS * BLOCK_SIZE;
    let audio_seconds = total_samples as f64 / SAMPLE_RATE.as_f32() as f64;
    let wall_seconds = elapsed.as_secs_f64();
    let realtime_ratio = audio_seconds / wall_seconds;
    let ns_per_sample = elapsed.as_nanos() as f64 / total_samples as f64;

    eprintln!(
        "  {name:<25} {wall_seconds:>7.3}s  ({audio_seconds:.1}s audio)  {realtime_ratio:>6.1}x realtime  {ns_per_sample:>5.1} ns/sample  ({NUM_VOICES} voices)"
    );
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    eprintln!();
    eprintln!("=== Pertylizer Audio Profiling Benchmark ===");
    eprintln!("  Sample rate: {} Hz", SAMPLE_RATE.as_f32());
    eprintln!("  Block size:  {BLOCK_SIZE} samples");
    eprintln!("  Blocks:      {NUM_BLOCKS} ({:.1}s audio)", NUM_BLOCKS as f64 * BLOCK_SIZE as f64 / SAMPLE_RATE.as_f32() as f64);
    eprintln!();

    // ── Single voice benchmarks ────────────────────────────────────────
    eprintln!("--- Single voice ---");
    bench_voice("Subtractive (saw+filter)", build_subtractive_voice());
    bench_voice("Additive (32 harmonics)", build_additive_voice());
    bench_voice("Fractal (64 partials)", build_fractal_voice());
    bench_voice("Wavetable", build_wavetable_voice());
    bench_voice("Granular", build_granular_voice());
    bench_voice("Math oscillator", build_math_voice());
    bench_voice("Unison (7+5 voices)", build_unison_voice());
    eprintln!();

    // ── Polyphony benchmarks ───────────────────────────────────────────
    eprintln!("--- {NUM_VOICES}-voice polyphony ---");
    bench_polyphony("Subtractive x8", build_subtractive_voice);
    bench_polyphony("Additive x8", build_additive_voice);
    bench_polyphony("Fractal x8", build_fractal_voice);
    bench_polyphony("Wavetable x8", build_wavetable_voice);
    bench_polyphony("Unison x8", build_unison_voice);
    eprintln!();

    eprintln!("Done. Use flamegraph/perf for detailed profiling.");
    eprintln!();
}
