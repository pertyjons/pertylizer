//! EVD-0014's harness: what the minimal patch costs, V1 against V2, in one
//! binary.
//!
//! ```text
//! evd_0014_cost c3 <dir>              # the workload-equality control
//! evd_0014_cost null <dir> [sweeps]   # C1's null pass — all four slots one engine
//! evd_0014_cost sweeps <dir> [sweeps] # the comparison collection
//! ```
//!
//! `<dir>` is EVD-0013's output directory: this record shares that record's
//! `aligned` fixture, and control **C3** is what holds the two to it.
//!
//! # The four arm slots
//!
//! A slot is one measurement unit — an engine plus a repetition index — and
//! there are exactly four: `V1·a`, `V1·b`, `V2·a`, `V2·b`. A **sweep** is one
//! pass over all four in that sweep's permutation order, and every ratio is
//! formed inside one sweep. All 24 permutations are used, one per sweep, so
//! every ordered pair of slots occupies every separation equally often — the
//! gap EVD-0012 had to amend its noise floor around.
//!
//! # Why the two pairs need two different V2 amplitudes
//!
//! V1's chain applies an equal-power centre pan **three** times, and only two of
//! them are inside the voice's own module graph: the amplifier's and the stereo
//! output's. The third is the instrument fader, applied by `SynthEngine::process`
//! (`crates/synth_engine/src/synth_engine.rs:3835`) — outside `ModuleGraph`
//! entirely. So the `whole-render` pair has to cancel three and the `voice-dsp`
//! pair only two, and a single amplitude for both would put a 3 dB level
//! difference into one of them.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pertylizer::audio::arrangement_render::{BUFFER_SIZE, OfflineEngineSession};
use pertylizer::audio::wav_format::WavFormat;
use pertylizer::render::headless::load_project_file;
use pertylizer::render::tick_window_from_seconds;
use pertylizer::synth_core::audio::DeviceSampleRate;
use pertylizer::synth_core::{
    AnalysisScope, BipolarValue, Bpm, FilterMode, Gain, Hertz, MidiNote, NormalizedValue, Param,
    ProcessContext, SampleCount, SampleRate as V1SampleRate, Seconds, Velocity, Waveform,
};
use synth_core::module_traits::PolyModule;
use synth_core::params::{AmplifierParam, EnvelopeParam, FilterParam, MixerParam, OscillatorParam};
use synth_engine::graph::ModuleGraph;
use synth_modules::{Amplifier, Envelope, Filter, Oscillator, StereoOutput};

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, NormalizedLevel, Resonance,
    SampleRate as V2SampleRate, Seconds as V2Seconds,
};
use synth_engine_v2::render::{AudioBlockMut, PreparedRenderer, Renderer, TimedEvent, TimedEvents};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

// ---------------------------------------------------------------------------
// The fixture, shared with EVD-0013
// ---------------------------------------------------------------------------

const SAMPLE_RATE: u32 = 44_100;
const TOTAL_FRAMES: u64 = 132_300;
const NOTE_OFF_FRAME: u64 = 66_150;
const CORNER_HZ: f32 = 1_000.0;
const FREQUENCY_HZ: f32 = 440.0;
const MIDI: u8 = 69;

const ATTACK_S: f32 = 0.010;
const DECAY_S: f32 = 0.100;
const SUSTAIN: f32 = 0.700;
const RELEASE_S: f32 = 0.200;

/// V1's normalised resonance that reproduces `Resonance::BUTTERWORTH`'s damping
/// exactly in `f32`.
const RESONANCE: f32 = 0.292_893_23;

/// Three centre pans: what the whole render applies.
const V2_AMPLITUDE_WHOLE: f32 = 0.353_553_35;

/// Two centre pans: what the voice's own module graph applies, the instrument
/// fader being outside it.
const V2_AMPLITUDE_VOICE: f32 = 0.5;

/// Calls of this size, which is V1's `BUFFER_SIZE` — a compile-time constant, so
/// the only block size V1 can be measured at without a rebuild. V2 matches V1
/// rather than the reverse.
const BLOCK: usize = BUFFER_SIZE;

/// Calls per timed batch on the `voice-dsp` pair: 512 000 frames, about 11.6 s
/// of audio, so `Instant::now()` is read twice per round rather than twice per
/// call.
const VOICE_CALLS: u32 = 2_000;

/// Renders per timed batch on the `whole-render` pair: 12 s of audio, matching
/// the other pair's batch closely enough that neither is dominated by setup.
const WHOLE_RENDERS: u32 = 4;

/// Rounds per measurement. The minimum over them is that slot's figure for that
/// sweep — EVD-0012's estimator, unchanged.
const ROUNDS: u32 = 3;

const ENVELOPE: NodeId = NodeId::new(1);
const SINE: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(9);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let dir = PathBuf::from(args.next().unwrap_or_else(|| ".".to_string()));
    let sweeps: u32 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(24);

    match command.as_str() {
        "c3" => control_c3(&dir),
        "null" => collect(&dir, sweeps, Mode::Null),
        "sweeps" => collect(&dir, sweeps, Mode::Comparison),
        other => Err(format!("unknown subcommand {other:?}; expected c3, null or sweeps").into()),
    }
}

// ---------------------------------------------------------------------------
// The V1 arms
// ---------------------------------------------------------------------------

/// The fixture's five modules as a `ModuleGraph`, exactly as the patch wires
/// them.
///
/// Control **C3** is what checks this reconstruction against V2's arm rather
/// than trusting it: a graph built wrong would still time perfectly well.
fn v1_voice_graph() -> ModuleGraph {
    let mut graph = ModuleGraph::new();

    let mut oscillator = Oscillator::new();
    oscillator.set_param(Param::Oscillator(OscillatorParam::Waveform(Waveform::Sine)));
    oscillator.set_param(Param::Oscillator(OscillatorParam::Level(Gain::UNITY)));
    // Asymmetry 3: the shipped default randomises the start phase from the voice
    // index, which would make the audio depend on allocation order.
    oscillator.set_param(Param::Oscillator(OscillatorParam::UnisonPhaseRandom(
        NormalizedValue::MIN,
    )));

    let mut filter = Filter::new();
    filter.set_param(Param::Filter(FilterParam::Mode(FilterMode::Lowpass)));
    filter.set_param(Param::Filter(FilterParam::Cutoff(Hertz::new(CORNER_HZ))));
    filter.set_param(Param::Filter(FilterParam::Resonance(NormalizedValue::new(
        RESONANCE,
    ))));
    filter.set_param(Param::Filter(FilterParam::EnvAmount(BipolarValue::CENTER)));

    let mut envelope = Envelope::new();
    envelope.set_param(Param::Envelope(EnvelopeParam::Attack(Seconds::new(
        ATTACK_S,
    ))));
    envelope.set_param(Param::Envelope(EnvelopeParam::Decay(Seconds::new(DECAY_S))));
    envelope.set_param(Param::Envelope(EnvelopeParam::Sustain(
        NormalizedValue::new(SUSTAIN),
    )));
    envelope.set_param(Param::Envelope(EnvelopeParam::Release(Seconds::new(
        RELEASE_S,
    ))));

    let mut amplifier = Amplifier::new();
    amplifier.set_param(Param::Amplifier(AmplifierParam::Level(Gain::UNITY)));

    let mut output = StereoOutput::new();
    // The shipped default is 0.8, which would be a silent 2 dB in a comparison.
    output.set_param(Param::Mixer(MixerParam::Master(Gain::UNITY)));

    let oscillator_id = graph.add_module(Box::new(oscillator));
    let filter_id = graph.add_module(Box::new(filter));
    let envelope_id = graph.add_module(Box::new(envelope));
    let amplifier_id = graph.add_module(Box::new(amplifier));
    let output_id = graph.add_module(Box::new(output));

    // Not discarded. A failed connection renders a *quieter* graph, which in a
    // cost comparison is a cheaper arm — the exact failure C3 exists to catch,
    // and one no timing figure would report.
    for (from, from_port, to, to_port) in [
        (oscillator_id, "out", filter_id, "in"),
        (filter_id, "out", amplifier_id, "in"),
        (envelope_id, "out", amplifier_id, "cv"),
        (amplifier_id, "out", output_id, "in"),
    ] {
        graph
            .connect(from, from_port, to, to_port)
            .unwrap_or_else(|error| panic!("the fixture's patch is a valid graph: {error:?}"));
    }
    graph.set_voice_index(0);
    graph
}

fn v1_context(frames: usize) -> ProcessContext<'static> {
    ProcessContext {
        sample_rate: V1SampleRate::new(SAMPLE_RATE as f32),
        samples: SampleCount::new(frames),
        tempo: Bpm::new(120.0),
        is_playing: true,
        ..ProcessContext::default()
    }
}

/// A V1 voice graph with its note played and its envelope settled into sustain,
/// so the timed loop is not timing a different segment mixture than V2's.
fn v1_settled() -> (ModuleGraph, pertylizer::synth_core::AudioBuffer) {
    let mut graph = v1_voice_graph();
    let mut output = pertylizer::synth_core::AudioBuffer::new(BLOCK);
    graph.note_on(MidiNote::new(MIDI), Velocity::FFF);
    // Past the 10 ms attack and the 100 ms decay, and then some.
    let settle_calls = (0.25 * SAMPLE_RATE as f32 / BLOCK as f32).ceil() as u32;
    for _ in 0..settle_calls {
        graph.process(&mut output, &v1_context(BLOCK));
    }
    (graph, output)
}

/// The V1 `voice-dsp` arm: `ModuleGraph::process` over the fixture's graph.
fn v1_voice_dsp(calls: u32) -> f64 {
    let (mut graph, mut output) = v1_settled();
    let context = v1_context(BLOCK);
    let start = Instant::now();
    for _ in 0..calls {
        graph.process(&mut output, &context);
        black_box(&output);
    }
    let elapsed = start.elapsed().as_secs_f64();
    ms_per_second(elapsed, u64::from(calls) * BLOCK as u64)
}

/// The V1 `whole-render` arm: the path `pertylizer render` uses.
///
/// A fresh session per render, exactly as `render_cost.rs` does it and for the
/// reason recorded there: a reused session skips the warm-up block and clears
/// the previous render's voices first, so its second render is not the same
/// work as its first. Session construction is outside the timed region.
fn v1_whole_render(project_path: &Path, renders: u32) -> Result<f64, Box<dyn std::error::Error>> {
    let project = load_project_file(project_path)?;
    let window = tick_window_from_seconds(&project.song, None, Seconds::new(2.0))?;
    let scope = AnalysisScope {
        master_effects: true,
        return_effects: true,
        render_sample_rate: DeviceSampleRate::new(SAMPLE_RATE),
    };

    let mut elapsed = 0.0_f64;
    let mut frames = 0_u64;
    for _ in 0..renders {
        let (mut engine, _) =
            OfflineEngineSession::new_with_scope(&project.session, &project.sample_library, scope)?;
        let start = Instant::now();
        let rendered = engine.render_range_with_tail(
            &project.song,
            window.start().0,
            window.end().0,
            Seconds::new(1.0),
        )?;
        elapsed += start.elapsed().as_secs_f64();
        frames += (rendered.samples.len() / usize::from(rendered.channels.max(1))) as u64;
        black_box(&rendered.samples);
    }
    Ok(ms_per_second(elapsed, frames))
}

// ---------------------------------------------------------------------------
// The V2 arms
// ---------------------------------------------------------------------------

fn v2_graph(amplitude: f32) -> GraphIr {
    GraphIr::builder()
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: V2Seconds::new(ATTACK_S).expect("finite"),
                decay: V2Seconds::new(DECAY_S).expect("finite"),
                sustain: NormalizedLevel::new(SUSTAIN).expect("in range"),
                release: V2Seconds::new(RELEASE_S).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            SINE,
            IrNodeKind::Sine {
                frequency: Frequency::new(FREQUENCY_HZ).expect("finite"),
                amplitude: Amplitude::new(amplitude).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(CORNER_HZ).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SINE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(synth_engine_v2::ir::PlanDeclarations {
            // A plan that starts notes must say who starts them: the identity range a
            // compiled note-on mints from is partitioned across declared producers.
            note_producers: vec![synth_engine_v2::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(1),
                simultaneous_holds: synth_engine_v2::quantities::EventCount::NONE,
            }],
            ..synth_engine_v2::ir::PlanDeclarations::default()
        })
        .build()
        .expect("the voice path is a readable plan")
}

fn v2_plan(amplitude: f32) -> CompiledPlan {
    let profile = HostProfile::harness(
        V2SampleRate::new(SAMPLE_RATE as f32).expect("a valid rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Stereo,
    )
    .expect("the harness profile is valid");
    compile(&v2_graph(amplitude), &RenderConfig::new(profile))
        .into_plan()
        .expect("the fixture is admissible")
}

/// Clause 5's input carry, as a cost rather than as state.
///
/// EVD-0012's `InputCarry`, unchanged in shape: `PreparedRenderer::input_carry`
/// is private, so this is its own buffer of exactly clause 5's size, appending
/// `N` frames per call and consuming `Q` per quantum rendered.
struct InputCarry {
    buffer: Vec<f32>,
    source: Vec<f32>,
    channels: usize,
    held: usize,
}

impl InputCarry {
    fn new(channels: usize) -> Self {
        let frames = BLOCK + QUANTUM_FRAMES as usize;
        Self {
            buffer: vec![0.0; frames * channels],
            source: vec![0.5; BLOCK * channels],
            channels,
            held: 0,
        }
    }

    fn call(&mut self, frames: usize, quanta: usize) {
        let channels = self.channels;
        let start = self.held * channels;
        let end = start + frames * channels;
        if let (Some(into), Some(from)) = (
            self.buffer.get_mut(start..end),
            self.source.get(..frames * channels),
        ) {
            into.copy_from_slice(from);
        }
        self.held += frames;

        let consumed = quanta * QUANTUM_FRAMES as usize;
        if consumed <= self.held {
            self.held -= consumed;
            let live = (consumed + self.held) * channels;
            self.buffer.copy_within(consumed * channels..live, 0);
        }
        black_box(&self.buffer);
    }
}

/// A V2 renderer with its note played and its envelope settled into sustain.
fn v2_settled(carry: Option<&mut InputCarry>) -> (PreparedRenderer, Vec<f32>) {
    let plan = v2_plan(V2_AMPLITUDE_VOICE);
    let channels = ChannelLayout::Stereo.channels();
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let mut block = vec![0.0_f32; BLOCK * channels];
    let mut carry = carry;

    let quantum = QUANTUM_FRAMES as usize;
    let mut call = |renderer: &mut PreparedRenderer,
                    block: &mut Vec<f32>,
                    frames: usize,
                    events: &[TimedEvent]| {
        let quanta = renderer.quanta_needed_for(frames);
        let region = block
            .get_mut(..frames * channels)
            .expect("the block holds a call");
        let output =
            AudioBlockMut::new(region, frames, ChannelLayout::Stereo).expect("a well-shaped block");
        renderer
            .render(output, TimedEvents::new(events))
            .expect("the fixture renders");
        if let Some(carry) = carry.as_deref_mut() {
            carry.call(frames, quanta);
        }
    };

    // The output carry is primed with `Q` frames of silence, so this call
    // renders no quantum and no event may be presented with it.
    call(&mut renderer, &mut block, quantum, &[]);
    // Stamped rather than hand-built: a note event carries an occurrence, and only the
    // plan's own admitted partition can mint one.
    let _epoch = renderer.epoch();
    let played = synth_engine_v2::schedule::stamp_compiled(
        &mut renderer,
        &[synth_engine_v2::schedule::CompiledEvent::new(
            SampleTime::ZERO,
            CompiledPayload::NoteOn { slot },
        )],
    )
    .expect("the fixture declares a compiled note producer");
    call(&mut renderer, &mut block, quantum, &played);
    while renderer.clock().as_u64() < (0.25 * SAMPLE_RATE as f64) as u64 {
        call(&mut renderer, &mut block, quantum, &[]);
    }
    (renderer, block)
}

fn v2_voice_dsp(calls: u32, clause_five: bool) -> f64 {
    let channels = ChannelLayout::Stereo.channels();
    let mut carry = clause_five.then(|| InputCarry::new(channels));
    let (mut renderer, mut block) = v2_settled(carry.as_mut());

    let start = Instant::now();
    for _ in 0..calls {
        let quanta = renderer.quanta_needed_for(BLOCK);
        let region = block
            .get_mut(..BLOCK * channels)
            .expect("the block holds a call");
        let output =
            AudioBlockMut::new(region, BLOCK, ChannelLayout::Stereo).expect("a well-shaped block");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("renders a block");
        if let Some(carry) = carry.as_mut() {
            carry.call(BLOCK, quanta);
        }
        black_box(&block);
    }
    let elapsed = start.elapsed().as_secs_f64();
    ms_per_second(elapsed, u64::from(calls) * BLOCK as u64)
}

fn v2_offline_events(plan: &CompiledPlan) -> Vec<OfflineEvent> {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    vec![
        OfflineEvent::new(SampleTime::ZERO, CompiledPayload::NoteOn { slot }),
        OfflineEvent::new(
            SampleTime::new(NOTE_OFF_FRAME),
            CompiledPayload::NoteOff { slot },
        ),
    ]
}

fn v2_whole_render(renders: u32) -> f64 {
    let mut elapsed = 0.0_f64;
    for _ in 0..renders {
        // Compiled outside the timed region, as V1's session construction is.
        let plan = v2_plan(V2_AMPLITUDE_WHOLE);
        let events = v2_offline_events(&plan);
        let start = Instant::now();
        let rendered = render_offline(
            plan,
            FrameCount::new(TOTAL_FRAMES),
            PlanPosition::ZERO,
            &events,
        )
        .expect("the fixture renders");
        elapsed += start.elapsed().as_secs_f64();
        black_box(&rendered);
    }
    ms_per_second(elapsed, u64::from(renders) * TOTAL_FRAMES)
}

/// Elapsed seconds over rendered frames, as milliseconds per second of audio.
fn ms_per_second(elapsed: f64, frames: u64) -> f64 {
    let seconds = frames as f64 / f64::from(SAMPLE_RATE);
    elapsed * 1000.0 / seconds
}

// ---------------------------------------------------------------------------
// C3 — the arms render the same music
// ---------------------------------------------------------------------------

/// Write both `voice-dsp` arms' output so they can be compared against each
/// other, and check the `whole-render` arms against EVD-0013's renders.
///
/// A stability check would pass an arm that rendered silence every sweep, which
/// is the cheapest possible way to win a timing comparison. So this compares
/// across arms, and asserts non-silence explicitly.
fn control_c3(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let wav = dir.join("wav");
    std::fs::create_dir_all(&wav)?;

    println!("# EVD-0014 control C3: the arms render the same music");

    // --- voice-dsp: render the fixture's whole window through both arms ---
    let frames = TOTAL_FRAMES as usize;
    let v1 = v1_voice_dsp_capture(frames);
    let v2 = v2_voice_dsp_capture(frames);

    let v1_peak = v1.iter().fold(0.0_f32, |peak, x| peak.max(x.abs()));
    let v2_peak = v2.iter().fold(0.0_f32, |peak, x| peak.max(x.abs()));
    println!("control,arm,frames,peak,non_silent");
    println!(
        "C3,voice-dsp-v1,{},{v1_peak:.6},{}",
        v1.len() / 2,
        v1_peak > 0.0
    );
    println!(
        "C3,voice-dsp-v2,{},{v2_peak:.6},{}",
        v2.len() / 2,
        v2_peak > 0.0
    );

    let v1_path = wav.join("c3-voice-dsp-v1.wav");
    let v2_path = wav.join("c3-voice-dsp-v2.wav");
    write_wav(&v1_path, &v1)?;
    write_wav(&v2_path, &v2)?;
    println!("# compare these two through EVD-0013's thresholds:");
    println!(
        "#   pertylizer compare --reference {} --candidate {}",
        v1_path.display(),
        v2_path.display()
    );

    // --- whole-render: against EVD-0013's own renders ---
    println!();
    println!("control,arm,matches_evd_0013");
    let reference_v2 = pertylizer::compare::Signal::load(&wav.join("v2-aligned.wav"))?;
    let plan = v2_plan(V2_AMPLITUDE_WHOLE);
    let events = v2_offline_events(&plan);
    let rendered = render_offline(
        plan,
        FrameCount::new(TOTAL_FRAMES),
        PlanPosition::ZERO,
        &events,
    )
    .expect("the fixture renders");
    let same = reference_v2.interleaved.len() == rendered.len()
        && reference_v2
            .interleaved
            .iter()
            .zip(&rendered)
            .all(|(a, b)| a.to_bits() == b.to_bits());
    println!("C3,whole-render-v2,{same}");

    // The V1 whole-render arm against the same record's V1 render. Both go
    // through `OfflineEngineSession`, so this checks that the arm's window,
    // rate and scope are the ones EVD-0013 compared rather than assuming it.
    let reference_v1 = pertylizer::compare::Signal::load(&wav.join("v1-aligned.wav"))?;
    let project_path = dir.join("v1").join("aligned.ptz");
    let project = load_project_file(&project_path)?;
    let window = tick_window_from_seconds(&project.song, None, Seconds::new(2.0))?;
    let scope = AnalysisScope {
        master_effects: true,
        return_effects: true,
        render_sample_rate: DeviceSampleRate::new(SAMPLE_RATE),
    };
    let (mut engine, _) =
        OfflineEngineSession::new_with_scope(&project.session, &project.sample_library, scope)?;
    let rendered_v1 = engine.render_range_with_tail(
        &project.song,
        window.start().0,
        window.end().0,
        Seconds::new(1.0),
    )?;
    let same_v1 = reference_v1.interleaved.len() == rendered_v1.samples.len()
        && reference_v1
            .interleaved
            .iter()
            .zip(&rendered_v1.samples)
            .all(|(a, b)| a.to_bits() == b.to_bits());
    println!("C3,whole-render-v1,{same_v1}");
    Ok(())
}

/// The V1 `voice-dsp` arm's output over the fixture's window, gate and all.
fn v1_voice_dsp_capture(frames: usize) -> Vec<f32> {
    let mut graph = v1_voice_graph();
    let mut block = pertylizer::synth_core::AudioBuffer::new(BLOCK);
    let mut out = Vec::with_capacity(frames * 2);
    graph.note_on(MidiNote::new(MIDI), Velocity::FFF);
    let mut released = false;
    let mut rendered = 0_u64;
    while (rendered as usize) < frames {
        if !released && rendered >= NOTE_OFF_FRAME {
            graph.note_off();
            released = true;
        }
        graph.process(&mut block, &v1_context(BLOCK));
        for index in 0..BLOCK {
            let sample = block[index];
            // `ModuleGraph::process` yields the output module's mono `OUT`; the
            // fixture's stereo channels are identical, so duplicating it is the
            // same signal in the shape the comparison reads.
            out.push(sample);
            out.push(sample);
        }
        rendered += BLOCK as u64;
    }
    out.truncate(frames * 2);
    out
}

/// The V2 `voice-dsp` arm's output over the same window.
fn v2_voice_dsp_capture(frames: usize) -> Vec<f32> {
    let plan = v2_plan(V2_AMPLITUDE_VOICE);
    let events = v2_offline_events(&plan);
    render_offline(
        plan,
        FrameCount::new(frames as u64),
        PlanPosition::ZERO,
        &events,
    )
    .expect("the fixture renders")
}

fn write_wav(path: &Path, interleaved: &[f32]) -> Result<(), Box<dyn std::error::Error>> {
    let spec = WavFormat::Float32.spec(2, SAMPLE_RATE);
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &sample in interleaved {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The sweep machinery
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every slot holds the same engine, so every ratio has a true value of
    /// zero. This is what `N` is taken from.
    Null,
    Comparison,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Engine {
    V1,
    V2,
}

/// One measurement unit: an engine and a repetition index.
#[derive(Clone, Copy)]
struct Slot {
    engine: Engine,
    rep: char,
}

impl Slot {
    /// The slot's identity, which the analysis pairs on.
    ///
    /// The repetition letter is carried through rather than collapsed: the null
    /// pass holds four slots of one engine, and two of them sharing a label
    /// would make its ratios pair the wrong measurements.
    fn label(self) -> String {
        let engine = match self.engine {
            Engine::V1 => "V1",
            Engine::V2 => "V2",
        };
        format!("{engine}{}", self.rep)
    }
}

fn slots(mode: Mode) -> [Slot; 4] {
    match mode {
        // The null pass holds V2 in all four, so every reported ratio is
        // between two measurements of one program.
        Mode::Null => [
            Slot {
                engine: Engine::V2,
                rep: 'a',
            },
            Slot {
                engine: Engine::V2,
                rep: 'b',
            },
            Slot {
                engine: Engine::V2,
                rep: 'c',
            },
            Slot {
                engine: Engine::V2,
                rep: 'd',
            },
        ],
        Mode::Comparison => [
            Slot {
                engine: Engine::V1,
                rep: 'a',
            },
            Slot {
                engine: Engine::V1,
                rep: 'b',
            },
            Slot {
                engine: Engine::V2,
                rep: 'a',
            },
            Slot {
                engine: Engine::V2,
                rep: 'b',
            },
        ],
    }
}

/// All 24 permutations of four positions, in lexicographic order.
fn permutations() -> Vec<[usize; 4]> {
    let mut out = Vec::with_capacity(24);
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let candidate = [a, b, c, d];
                    let mut seen = [false; 4];
                    if candidate.iter().all(|&index| {
                        let fresh = !seen[index];
                        seen[index] = true;
                        fresh
                    }) {
                        out.push(candidate);
                    }
                }
            }
        }
    }
    out
}

/// One (pair, variant) a slot measures.
const MEASUREMENTS: [(&str, &str); 3] = [
    ("voice-dsp", "as-built"),
    ("voice-dsp", "clause-5"),
    ("whole-render", "as-built"),
];

fn measure_one(
    engine: Engine,
    pair: &str,
    variant: &str,
    project: &Path,
) -> Result<Option<f64>, Box<dyn std::error::Error>> {
    // V1 has no counterpart to clause 5's input carry, so it has one variant.
    if engine == Engine::V1 && variant == "clause-5" {
        return Ok(None);
    }
    let mut best = f64::INFINITY;
    for _ in 0..ROUNDS {
        let cost = match (engine, pair) {
            (Engine::V1, "voice-dsp") => v1_voice_dsp(VOICE_CALLS),
            (Engine::V1, _) => v1_whole_render(project, WHOLE_RENDERS)?,
            (Engine::V2, "voice-dsp") => v2_voice_dsp(VOICE_CALLS, variant == "clause-5"),
            (Engine::V2, _) => v2_whole_render(WHOLE_RENDERS),
        };
        best = best.min(cost);
    }
    Ok(Some(best))
}

fn collect(dir: &Path, sweeps: u32, mode: Mode) -> Result<(), Box<dyn std::error::Error>> {
    let project = dir.join("v1").join("aligned.ptz");
    let slots = slots(mode);
    let permutations = permutations();

    println!("mode,sweep,position,slot,engine,pair,variant,cost_ms_per_s,spread_percent");
    for sweep in 0..sweeps {
        let order = permutations[(sweep as usize) % permutations.len()];
        for (position, &index) in order.iter().enumerate() {
            let slot = slots[index];
            for (pair, variant) in MEASUREMENTS {
                let Some(first) = measure_one(slot.engine, pair, variant, &project)? else {
                    continue;
                };
                // Control C2: the in-process spread, a diagnostic rather than a
                // threshold. It is what this machine does to two measurements
                // inside one process; every ratio below spans slots.
                let second = measure_one(slot.engine, pair, variant, &project)?.unwrap_or(first);
                let spread = (second - first).abs() / first.min(second) * 100.0;
                let cost = first.min(second);
                let engine = if slot.engine == Engine::V1 {
                    "V1"
                } else {
                    "V2"
                };
                let label = slot.label();
                let mode_name = if mode == Mode::Null {
                    "null"
                } else {
                    "comparison"
                };
                println!(
                    "{mode_name},{sweep},{position},{label},{engine},{pair},{variant},{cost:.6},{spread:.4}"
                );
            }
        }
    }
    Ok(())
}
