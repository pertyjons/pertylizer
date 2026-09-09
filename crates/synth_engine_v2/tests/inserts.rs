//! `P08-S003`, `SOUND-INV-033`: the first two insert kinds — V1's distortion in its soft-clip
//! mode and V1's delay in its mono mode — and the declaration's latency, tail and history.
//!
//! The oracles are V1's own laws spelled term for term from V1's source (`distortion.rs`,
//! `delay.rs`, `synth_core`'s `BufferIndex`, `FilterState` and `StereoSample`), run over the
//! same widened input the V2 node receives, and every comparison is of bits. The tail rule
//! is V2's own and is **checked** here at named points rather than proved tight.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::ir::{
    ChannelTag, ExecutionScope, GraphIr, IrNodeKind, NodeId, PlanDeclarations, PortId,
    SignalDomain, StealingPolicy, parameters,
};
use synth_engine_v2::node::AMPLIFIER_CONTROL;
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, DelayFeedback, DelayTime, EventCount, Frequency, HeldNoteCount,
    NormalizedLevel, ParameterValue, SampleRate, Seconds,
};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const STAGE: NodeId = NodeId::new(10);
const SECOND: NodeId = NodeId::new(11);
const Q: u64 = QUANTUM_FRAMES as u64;
const FRAMES: u64 = 512 * Q;
const RATE: f32 = 48_000.0;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

fn level(value: f32) -> NormalizedLevel {
    NormalizedLevel::new(value).expect("a level")
}

fn sine(amplitude: f32) -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(amplitude).expect("finite"),
    }
}

fn distortion(drive: f32, tone: f32, mix: f32) -> IrNodeKind {
    IrNodeKind::Distortion {
        drive: level(drive),
        tone: level(tone),
        mix: level(mix),
    }
}

fn delay(time_left: f32, time_right: f32, feedback: f32, mix: f32, tone: f32) -> IrNodeKind {
    IrNodeKind::Delay {
        time_left: DelayTime::new(time_left).expect("in range"),
        time_right: DelayTime::new(time_right).expect("in range"),
        feedback: DelayFeedback::new(feedback).expect("in range"),
        mix: level(mix),
        tone: level(tone),
    }
}

/// The corpus's delay: `CORPUS-0005`'s `dly-1`.
fn corpus_delay() -> IrNodeKind {
    delay(0.25, 0.25, 0.45, 0.5, 0.4)
}

/// One source through the given stages, in order, into the output.
fn staged(source: IrNodeKind, stages: &[(NodeId, IrNodeKind, ExecutionScope)]) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(SOURCE, source, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);
    let mut previous = SOURCE;
    for (id, kind, scope) in stages {
        builder = builder.node(*id, *kind, *scope).connect(
            (previous, PortId::FIRST),
            (*id, PortId::FIRST),
            SignalDomain::Audio,
        );
        previous = *id;
    }
    builder
        .connect(
            (previous, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a staged source is a readable plan")
}

/// The stereo harness profile with a 512-quantum block and twice the default scratch budget.
///
/// The block is the harness's, so a whole render can be one call; its event scratch is the
/// per-quantum cap times the quanta a call spans, and at the cap EVD-0021 reselected
/// (`P08-S004`) 513 quanta of it pass the 16 MiB default a 4 096-frame host sits well under.
/// The budget is raised for this block alone rather than the block shrunk, so every render
/// below stays a single call as the digests in the slice's commit were measured.
fn harness() -> HostProfile {
    use synth_engine_v2::profile::{MemoryLimits, RenderLimits};
    use synth_engine_v2::quantities::PreparedBytes;
    let base = profile(FRAMES, ChannelLayout::Stereo);
    let defaults = RenderLimits::engine_defaults(base.capabilities()).expect("defaults");
    let memory = defaults.memory();
    let memory = MemoryLimits::new(
        memory.prepared_immutable_bytes(),
        memory.mutable_state_bytes(),
        PreparedBytes::limit(32 * 1024 * 1024).expect("positive"),
    )
    .expect("a larger scratch budget");
    let limits = RenderLimits::new(
        defaults.stream(),
        defaults.graph(),
        defaults.voices(),
        defaults.events(),
        defaults.observation(),
        defaults.mixing(),
        memory,
        defaults.script(),
        defaults.recording(),
        defaults.cost(),
    )
    .expect("consistent limits");
    HostProfile::new(base.capabilities(), limits).expect("a consistent profile")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    common::admit(ir, harness())
}

fn admit_at(ir: &GraphIr, host: HostProfile) -> CompiledPlan {
    common::admit(ir, host)
}

fn render_frames(plan: &CompiledPlan, frames: u64, events: &[OfflineEvent]) -> Vec<f32> {
    render_offline(
        plan.clone(),
        FrameCount::new(frames),
        PlanPosition::ZERO,
        events,
    )
    .expect("renders")
}

fn render(plan: &CompiledPlan, events: &[OfflineEvent]) -> Vec<f32> {
    render_frames(plan, FRAMES, events)
}

fn frames(interleaved: &[f32]) -> impl Iterator<Item = (f32, f32)> + '_ {
    interleaved.as_chunks::<2>().0.iter().map(|[l, r]| (*l, *r))
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|s| s.to_bits()).collect()
}

/// V1's `fast_tanh`, term for term.
fn v1_fast_tanh(x: f32) -> f32 {
    if x < -3.0 {
        return -1.0;
    }
    if x > 3.0 {
        return 1.0;
    }
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// V1's `Hertz::to_exp_coeff`, term for term.
fn v1_exp_coeff(hertz: f32, rate: f32) -> f32 {
    (-core::f32::consts::TAU * hertz / rate).exp()
}

/// V1's `Distortion::process` in its soft-clip mode over an interleaved stereo buffer, term
/// for term: `drive_gain(drive, 50)`, `fast_tanh`, `apply_tone` per channel and
/// `NormalizedValue::blend`.
fn v1_distortion(input: &[f32], drive: f32, tone: f32, mix: f32, rate: f32) -> Vec<f32> {
    let coef = v1_exp_coeff(200.0 + tone * tone * 15000.0, rate);
    let gain = 1.0 + drive * drive * 50.0;
    let mut state = [0.0_f32; 2];
    let mut output = vec![0.0_f32; input.len()];
    for i in 0..input.len() {
        let channel = i % 2;
        let dry = input[i];
        let driven = dry * gain;
        let distorted = v1_fast_tanh(driven);
        state[channel] = distorted * (1.0 - coef) + state[channel] * coef;
        let filtered = state[channel];
        output[i] = dry * (1.0 - mix) + filtered * mix;
    }
    output
}

/// V1's `BufferIndex::read_interpolated`, term for term.
fn v1_read_interpolated(buffer: &[f32], write: usize, delay_samples: f32) -> f32 {
    if buffer.is_empty() {
        return 0.0;
    }
    let len = buffer.len();
    let read_pos = (write as f32 - delay_samples).rem_euclid(len as f32);
    let idx0 = (read_pos as usize) % len;
    let idx1 = (idx0 + 1) % len;
    let frac = read_pos - read_pos.floor();
    buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
}

/// V1's `Delay::process` in its mono mode over an interleaved stereo buffer, term for term,
/// with V1's own line length `(2.0 × rate) as usize` per side, its `FilterState::one_pole`,
/// `StereoSample::to_mono`, `soft_clip` (`f32::tanh`) and `blend`.
struct V1Delay {
    left: Vec<f32>,
    right: Vec<f32>,
    write: usize,
    filter_left: f32,
    filter_right: f32,
}

impl V1Delay {
    fn new(rate: f32) -> Self {
        let size = (2.0 * rate) as usize;
        Self {
            left: vec![0.0; size],
            right: vec![0.0; size],
            write: 0,
            filter_left: 0.0,
            filter_right: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments, reason = "V1's parameters, one each")]
    fn process(
        &mut self,
        input: &[f32],
        output: &mut [f32],
        rate: f32,
        time_left: f32,
        time_right: f32,
        feedback: f32,
        mix: f32,
        tone: f32,
    ) {
        let high_cut = 200.0 + tone * (20000.0 - 200.0);
        let delay_samples_left = (time_left * rate).min((self.left.len() - 1) as f32);
        let delay_samples_right = (time_right * rate).min((self.right.len() - 1) as f32);
        let len = self.left.len();
        for frame in 0..input.len() / 2 {
            let (dry_left, dry_right) = (input[frame * 2], input[frame * 2 + 1]);
            let delayed_left = v1_read_interpolated(&self.left, self.write, delay_samples_left);
            let delayed_right = v1_read_interpolated(&self.right, self.write, delay_samples_right);
            let coef = v1_exp_coeff(high_cut, rate);
            self.filter_left = delayed_left + (self.filter_left - delayed_left) * coef;
            let fb_left = self.filter_left;
            self.filter_right = delayed_right + (self.filter_right - delayed_right) * coef;
            let fb_right = self.filter_right;
            let mono_in = (dry_left + dry_right) * 0.5;
            let mono_fb = (fb_left + fb_right) * 0.5;
            let write = (mono_in + mono_fb * feedback).tanh();
            self.left[self.write] = write;
            self.right[self.write] = write;
            self.write = (self.write + 1) % len;
            let dry_amt = 1.0 - mix;
            output[frame * 2] = dry_left * dry_amt + delayed_left * mix;
            output[frame * 2 + 1] = dry_right * dry_amt + delayed_right * mix;
        }
    }
}

/// The source alone, widened by the output: what the insert receives, frame for frame.
fn widened(source: IrNodeKind) -> Vec<f32> {
    render(&admit(&staged(source, &[])), &[])
}

#[test]
fn a_distortion_shapes_each_sample_by_v1s_law_bit_for_bit() {
    for (drive, tone, mix) in [
        (0.7_f32, 0.8_f32, 1.0_f32),
        (0.3, 0.2, 0.5),
        (0.0, 1.0, 1.0),
    ] {
        let source = sine(0.9);
        let rendered = render(
            &admit(&staged(
                source,
                &[(STAGE, distortion(drive, tone, mix), ExecutionScope::Global)],
            )),
            &[],
        );
        let input = widened(source);
        let expected = v1_distortion(&input, drive, tone, mix, RATE);
        assert_eq!(
            bits(&rendered),
            bits(&expected),
            "drive {drive} tone {tone} mix {mix}"
        );
        // The shaping is real where the drive is: a clean pass would satisfy a broken
        // oracle that returned its input.
        if drive > 0.0 {
            assert_ne!(
                bits(&rendered),
                bits(&input),
                "drive {drive} shapes nothing"
            );
        }
    }
}

#[test]
fn a_delay_reads_writes_and_blends_by_v1s_law_bit_for_bit() {
    // The corpus's values; unequal sides; and a time whose frame count is fractional at
    // this rate, so the two-tap interpolation weighs both taps.
    for (time_left, time_right, feedback, mix, tone) in [
        (0.25_f32, 0.25_f32, 0.45_f32, 0.5_f32, 0.4_f32),
        (0.125, 0.25, 0.6, 0.7, 0.0),
        (0.123_4, 0.056_7, 0.95, 1.0, 1.0),
    ] {
        let source = sine(0.5);
        let rendered = render(
            &admit(&staged(
                source,
                &[(
                    STAGE,
                    delay(time_left, time_right, feedback, mix, tone),
                    ExecutionScope::Global,
                )],
            )),
            &[],
        );
        let input = widened(source);
        let mut oracle = V1Delay::new(RATE);
        let mut expected = vec![0.0_f32; input.len()];
        oracle.process(
            &input,
            &mut expected,
            RATE,
            time_left,
            time_right,
            feedback,
            mix,
            tone,
        );
        assert_eq!(
            bits(&rendered),
            bits(&expected),
            "times {time_left}/{time_right} feedback {feedback} mix {mix} tone {tone}"
        );
        // The repeats are real: past the shorter time the wet side differs from the dry.
        let first_repeat = (time_left.min(time_right) * RATE) as usize + 2;
        assert!(
            frames(&rendered)
                .zip(frames(&input))
                .skip(first_repeat)
                .any(|((l, _), (dl, _))| l != dl),
            "the delay repeats nothing"
        );
    }
}

/// The two inserts in the corpus's order over a driven source render the same bits under
/// every host partition, and the reversed order is a different signal (`CORPUS-0005-P4`).
#[test]
fn an_insert_chain_is_the_same_bits_under_every_partition_and_its_order_is_audible() {
    let ordered = admit(&staged(
        sine(0.9),
        &[
            (STAGE, distortion(0.7, 0.8, 1.0), ExecutionScope::Global),
            (SECOND, corpus_delay(), ExecutionScope::Global),
        ],
    ));
    let reversed = admit(&staged(
        sine(0.9),
        &[
            (STAGE, corpus_delay(), ExecutionScope::Global),
            (SECOND, distortion(0.7, 0.8, 1.0), ExecutionScope::Global),
        ],
    ));
    let whole: Vec<usize> = vec![FRAMES as usize];
    let reference = render_partitioned(&ordered, &whole);
    assert_ne!(
        bits(&reference),
        bits(&render_partitioned(&reversed, &whole)),
        "the chain's order is authored data"
    );
    let blocks_256: Vec<usize> = vec![256; (FRAMES / 256) as usize];
    let blocks_64: Vec<usize> = vec![64; (FRAMES / 64) as usize];
    let mut irregular = vec![37, 91, 5, 128, 1];
    let taken: usize = irregular.iter().sum();
    irregular.push(FRAMES as usize - taken);
    for partition in [&blocks_256[..], &blocks_64[..], &irregular[..]] {
        assert_eq!(
            bits(&render_partitioned(&ordered, partition)),
            bits(&reference),
            "partition {partition:?} rendered other bits"
        );
    }
}

fn render_partitioned(plan: &CompiledPlan, partition: &[usize]) -> Vec<f32> {
    let (_control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut out = Vec::with_capacity((FRAMES * 2) as usize);
    for block in partition.iter().copied() {
        let mut samples = vec![0.0_f32; block * 2];
        let output =
            AudioBlockMut::new(&mut samples, block, ChannelLayout::Stereo).expect("a shaped block");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("the block renders");
        out.extend_from_slice(&samples);
    }
    assert_eq!(out.len(), (FRAMES * 2) as usize);
    out
}

/// A delay fed a sustained input, then silence, at the given rate: the rendered frames and
/// the frame the input fell silent at. The input is a constant through a mix channel whose
/// mute is written at `silent_from`, so the insert sees V1's own kind of stop — a signal
/// that ends mid-stream — rather than a plan that never sounded.
fn ring_out(
    rate: f32,
    insert: IrNodeKind,
    input: f32,
    silent_from: u64,
    frames_total: u64,
) -> Vec<f32> {
    let host = HostProfile::harness(
        SampleRate::new(rate).expect("a rate"),
        FrameCount::new(256),
        ChannelLayout::Stereo,
    )
    .expect("a valid profile");
    let ir = staged(
        IrNodeKind::Constant {
            level: Amplitude::new(input).expect("finite"),
        },
        &[
            (
                STAGE,
                IrNodeKind::Channel {
                    fader: Amplitude::UNITY,
                    pan: synth_engine_v2::controller::BipolarLevel::ZERO,
                    muted: false,
                },
                ExecutionScope::Channel(ChannelTag::FIRST),
            ),
            (SECOND, insert, ExecutionScope::Global),
        ],
    );
    let plan = admit_at(&ir, host);
    let mute = plan.channels().first().expect("one channel").mute;
    render_frames(
        &plan,
        frames_total,
        &[OfflineEvent::new(
            SampleTime::new(silent_from),
            CompiledPayload::SetParameter {
                slot: mute,
                value: ParameterValue::ONE,
            },
        )],
    )
}

/// The declared tail bounds the ring-out: past `silent_from + tail`, every sample is at or
/// below −60 dB of the loudest sample after the input stopped — and the loop did ring, so
/// the bound is doing work.
fn assert_decays_within_tail(rendered: &[f32], silent_from: usize, tail: usize, what: &str) {
    let after: Vec<f32> = rendered[silent_from * 2..].to_vec();
    let peak = after.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    assert!(peak > 0.0, "{what}: nothing rang after the input stopped");
    let threshold = peak * 1e-3;
    let boundary = tail * 2;
    assert!(
        boundary < after.len(),
        "{what}: the render must reach past the declared tail of {tail} frames"
    );
    let (first, offender) = after[boundary..]
        .iter()
        .enumerate()
        .find(|(_, s)| s.abs() > threshold)
        .map_or((usize::MAX, 0.0), |(i, s)| (i, *s));
    assert!(
        first == usize::MAX,
        "{what}: {offender} at {} frames past the declared tail of {tail} exceeds −60 dB of the \
         ring-out's peak {peak}",
        first / 2
    );
}

/// The tail rule at four named points: the corpus's delay from an impulse and from sustained
/// input; the short dark loop where the feedback filter's memory outlasts a traversal, which
/// falsified the naive repeat count; V1's feedback ceiling on a short loop; and no feedback,
/// where the tail is one repeat.
#[test]
fn a_delay_decays_below_60_db_within_its_declared_tail() {
    let tail_of = |kind: IrNodeKind, rate: f32| {
        synth_engine_v2::node::timing_of(kind, SampleRate::new(rate).expect("a rate"))
            .tail
            .expect("a delay states its tail")
            .as_u64() as usize
    };

    // The corpus's delay: 0.25 s, feedback 0.45, tone 0.4, at 48 kHz, sustained then silent.
    let corpus = corpus_delay();
    let tail = tail_of(corpus, RATE);
    assert!(
        (100_000..140_000).contains(&tail),
        "the corpus delay's tail is a few repeats past nine quarter-seconds: {tail}"
    );
    let silent_from = 20_000_usize;
    let rendered = ring_out(
        RATE,
        corpus,
        0.5,
        silent_from as u64,
        (silent_from + tail + 20_000) as u64,
    );
    assert_decays_within_tail(&rendered, silent_from, tail, "corpus, sustained");

    // The same delay from one impulse: `mix 1` so the output is the repeats alone.
    let impulse = staged(
        IrNodeKind::Impulse {
            position: PlanPosition::ZERO,
        },
        &[(
            STAGE,
            delay(0.25, 0.25, 0.45, 1.0, 0.4),
            ExecutionScope::Global,
        )],
    );
    let tail = tail_of(delay(0.25, 0.25, 0.45, 1.0, 0.4), RATE);
    let rendered = render_frames(&admit(&impulse), (tail + 30_000) as u64, &[]);
    assert_decays_within_tail(&rendered, 1, tail, "corpus, impulse");

    // The short dark loop: 1/512 s at 32 768 Hz is 64 frames, feedback 0.5, tone 0 — a
    // 200 Hz high cut whose state decays over 26 frames, longer than a traversal is short.
    let short = delay(1.0 / 512.0, 1.0 / 512.0, 0.5, 1.0, 0.0);
    let tail = tail_of(short, 32_768.0);
    assert!(
        tail > 937,
        "the naive repeat count declared 640 frames here and −60 dB arrived at 937: {tail}"
    );
    let silent_from = 10_000_usize;
    let rendered = ring_out(
        32_768.0,
        short,
        0.0001,
        silent_from as u64,
        (silent_from + tail + 4_000) as u64,
    );
    assert_decays_within_tail(&rendered, silent_from, tail, "short dark loop");

    // V1's ceiling: feedback 0.95 on a 10 ms dark loop.
    let hot = delay(0.01, 0.01, 0.95, 1.0, 0.0);
    let tail = tail_of(hot, RATE);
    let silent_from = 5_000_usize;
    let rendered = ring_out(
        RATE,
        hot,
        0.5,
        silent_from as u64,
        (silent_from + tail + 5_000) as u64,
    );
    assert_decays_within_tail(&rendered, silent_from, tail, "feedback ceiling");

    // No feedback: one repeat, and the tail is that repeat's time.
    let single = delay(0.05, 0.05, 0.0, 1.0, 0.5);
    let tail = tail_of(single, RATE);
    assert_eq!(tail, 2_400, "one repeat of 2 400 frames");
    let silent_from = 5_000_usize;
    let rendered = ring_out(
        RATE,
        single,
        0.5,
        silent_from as u64,
        (silent_from + tail + 5_000) as u64,
    );
    assert_decays_within_tail(&rendered, silent_from, tail, "no feedback");
}

/// The declaration's timing reaches the plan per node and the report as the longest stated
/// tail; a kind that keeps signal without a stated rule makes the plan's tail unknown rather
/// than a figure that omits it.
#[test]
fn declared_timing_is_visible_per_node_and_the_plans_tail_is_the_longest_stated_one() {
    let rate = SampleRate::new(RATE).expect("a rate");
    let ir = staged(
        sine(0.5),
        &[
            (STAGE, distortion(0.7, 0.8, 1.0), ExecutionScope::Global),
            (SECOND, corpus_delay(), ExecutionScope::Global),
        ],
    );
    let outcome = synth_engine_v2::compile::compile(
        &ir,
        &synth_engine_v2::compile::RenderConfig::new(harness()),
    );
    let reported = outcome.report().reported().declared_tail();
    let plan = outcome.into_plan().expect("admits");
    let delay_timing = plan
        .timing_of(SECOND)
        .expect("the delay is a node of the plan");
    assert_eq!(delay_timing.latency, FrameCount::ZERO);
    assert_eq!(
        delay_timing.history,
        FrameCount::new(96_000),
        "one line of 2 s at 48 kHz, at the output's width"
    );
    assert_eq!(
        delay_timing.tail,
        synth_engine_v2::node::timing_of(corpus_delay(), rate).tail
    );
    let distortion_timing = plan
        .timing_of(STAGE)
        .expect("the distortion is a node of the plan");
    assert_eq!(distortion_timing.history, FrameCount::ZERO);
    let tone_frames = distortion_timing
        .tail
        .expect("a distortion states its tail");
    // `200 + 0.64 × 15000 = 9800 Hz` at 48 kHz: a coefficient near 0.28, gone in six frames.
    assert!((1..=8).contains(&tone_frames.as_u64()), "{tone_frames}");
    assert_eq!(
        plan.timing_of(SOURCE)
            .expect("the source is a node of the plan")
            .tail,
        Some(FrameCount::ZERO),
        "a source has no input to outlast"
    );
    assert_eq!(plan.declared_tail(), delay_timing.tail);
    assert_eq!(
        reported, delay_timing.tail,
        "the report carries the same figure"
    );

    // A filter keeps signal and has not stated a rule: the plan's tail is unknown.
    let filtered = staged(
        sine(0.5),
        &[
            (
                STAGE,
                IrNodeKind::Filter {
                    cutoff: synth_engine_v2::quantities::CutoffFrequency::new(1_000.0)
                        .expect("positive"),
                    resonance: synth_engine_v2::quantities::Resonance::BUTTERWORTH,
                },
                ExecutionScope::Global,
            ),
            (SECOND, corpus_delay(), ExecutionScope::Global),
        ],
    );
    let plan = admit(&filtered);
    assert_eq!(plan.timing_of(STAGE).expect("present").tail, None);
    assert_eq!(plan.declared_tail(), None);
}

/// A written feedback past V1's ceiling and a written time below V1's floor are held to them
/// by the slot, so the render is the one authored at the bound (`SOUND-INV-033`).
#[test]
fn a_written_feedback_and_time_are_held_to_v1s_own_domain() {
    let authored_at_bounds = admit(&staged(
        sine(0.5),
        &[(
            STAGE,
            delay(0.001, 0.25, 0.95, 1.0, 0.5),
            ExecutionScope::Global,
        )],
    ));
    let plain = admit(&staged(
        sine(0.5),
        &[(
            STAGE,
            delay(0.25, 0.25, 0.2, 1.0, 0.5),
            ExecutionScope::Global,
        )],
    ));
    let feedback = plain
        .resolve_parameter(STAGE, parameters::DELAY_FEEDBACK)
        .expect("declared");
    let time_left = plain
        .resolve_parameter(STAGE, parameters::DELAY_TIME_LEFT)
        .expect("declared");
    let written = render(
        &plain,
        &[
            OfflineEvent::new(
                SampleTime::ZERO,
                CompiledPayload::SetParameter {
                    slot: feedback,
                    value: ParameterValue::new(5.0).expect("finite"),
                },
            ),
            OfflineEvent::new(
                SampleTime::ZERO,
                CompiledPayload::SetParameter {
                    slot: time_left,
                    value: ParameterValue::new(-1.0).expect("finite"),
                },
            ),
        ],
    );
    let expected = render(&authored_at_bounds, &[]);
    assert_eq!(bits(&written), bits(&expected));
    // The control: the plain plan without the writes is another render.
    assert_ne!(bits(&written), bits(&render(&plain, &[])));
}

/// A delay in the instrument scope keeps its line while a voice under it is stolen: the
/// first note's repeat still arrives after the steal, because a steal's reset addresses the
/// taken voice's instance group and the shared insert is in none.
#[test]
fn an_instrument_scope_delays_line_survives_a_stolen_voice() {
    const ENVELOPE: NodeId = NodeId::new(20);
    const AMPLIFIER: NodeId = NodeId::new(21);
    const DELAY: NodeId = NodeId::new(22);
    let ir = GraphIr::builder()
        .node(SOURCE, sine(0.5), ExecutionScope::Voice)
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            DELAY,
            delay(0.05, 0.05, 0.0, 1.0, 1.0),
            ExecutionScope::InstrumentInstance,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (DELAY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (DELAY, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, common::twelve_tet())
        .declaring(PlanDeclarations {
            note_producers: vec![synth_engine_v2::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(1),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(1),
            stealing: StealingPolicy::Oldest {
                fade: FrameCount::new(64),
            },
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan");
    let plan = admit(&ir);
    let slot = plan.resolve_note(ENVELOPE).expect("the envelope is played");
    // The first note from frame 0, stolen by the second at frame 1 000; the delay is wet
    // only, so the output before 2 400 is silence and 2 400..3 400 is the first note's
    // repeat alone — the second note's begins at 3 400.
    let rendered = render_frames(
        &plan,
        6 * Q * 16,
        &[
            OfflineEvent::new(SampleTime::ZERO, common::note_on(slot)),
            OfflineEvent::new(SampleTime::new(1_000), common::note_on(slot)),
        ],
    );
    let repeat = &rendered[2_400 * 2..3_400 * 2];
    assert!(
        repeat.iter().any(|s| s.abs() > 1e-3),
        "the stolen voice's repeat was erased with the steal"
    );
    assert!(
        rendered[..2_300 * 2].iter().all(|s| *s == 0.0),
        "wet only: nothing before the first repeat"
    );
}
