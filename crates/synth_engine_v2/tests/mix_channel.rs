//! `P08-S001`, `SOUND-INV-031`: the mix channel and explicit summing.
//!
//! A channel scales each side of one stereo signal by its fader and V1's constant-power pan
//! law and silences it from the sample its mute lands on; a sum adds every cable into its
//! one summed port linearly, in float, unclamped. The oracle for the pan law is **V1's own
//! function**, `synth_core::Gain::from_pan`, called from the test rather than from the
//! kernel (`SOUND-INV-013`), and the gain is formed as V1's channel stage forms it, so the
//! comparison is bit for bit. Both kinds declare stereo on every port, so both kernels are
//! tested at two channels here, which is the obligation ADR-0041 clause 12 attaches to a
//! port that admits more than one (`SOUND-INV-015`); `graph_validation` holds every other
//! kind to one.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::controller::BipolarLevel;
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, LfoPolarity, LfoWaveform, ModulationDepth, ModulationUnit,
    NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::{catalog, ports};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, Frequency, NormalizedLevel, ParameterValue, PhaseOffset,
};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};
use synth_engine_v2::validate::FanIn;

const CHANNEL: NodeId = NodeId::new(10);
const SECOND_SOURCE: NodeId = NodeId::new(11);
const MIX: NodeId = NodeId::new(12);
const LFO: NodeId = NodeId::new(13);
const SCRIPT: NodeId = NodeId::new(14);
const Q: u64 = QUANTUM_FRAMES as u64;
const QUANTA: u64 = 8;
const FRAMES: u64 = QUANTA * Q;
const LEVEL: f32 = 0.5;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

fn constant(level: f32) -> IrNodeKind {
    IrNodeKind::Constant {
        level: Amplitude::new(level).expect("finite"),
    }
}

fn sine(hz: f32) -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(hz).expect("finite"),
        amplitude: Amplitude::new(LEVEL).expect("finite"),
    }
}

fn channel(fader: f32, pan: f32, muted: bool) -> IrNodeKind {
    IrNodeKind::Channel {
        fader: Amplitude::new(fader).expect("finite"),
        pan: BipolarLevel::new(pan).expect("in range"),
        muted,
    }
}

/// One source through one channel into the output.
fn channelled(source: IrNodeKind, strip: IrNodeKind) -> GraphIr {
    GraphIr::builder()
        .node(SOURCE, source, ExecutionScope::Voice)
        .node(CHANNEL, strip, ExecutionScope::Channel)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (CHANNEL, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a channelled source is a readable plan")
}

/// The given sources, every one into one sum, into the output.
fn summed(sources: &[(NodeId, IrNodeKind)]) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(MIX, IrNodeKind::Mix, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (MIX, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    for (id, kind) in sources {
        builder = builder.node(*id, *kind, ExecutionScope::Global).connect(
            (*id, PortId::FIRST),
            (MIX, PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    builder
        .build()
        .expect("sources into a sum is a readable plan")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    common::admit(ir, profile(FRAMES, ChannelLayout::Stereo))
}

fn render(plan: &CompiledPlan, events: &[OfflineEvent]) -> Vec<f32> {
    render_offline(
        plan.clone(),
        FrameCount::new(FRAMES),
        PlanPosition::ZERO,
        events,
    )
    .expect("renders")
}

/// V1's channel stage, as `mix_channel_busses` forms it: the pan coefficient from V1's own
/// function times the fader, then the sample times that. The oracle every pan check reads.
fn v1_sides(fader: f32, pan: f32) -> (f32, f32) {
    let (left, right) = synth_core::Gain::from_pan(synth_core::BipolarValue::new(pan));
    (left.as_f32() * fader, right.as_f32() * fader)
}

fn frames(interleaved: &[f32]) -> impl Iterator<Item = (f32, f32)> + '_ {
    interleaved
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| (frame[0], frame[1]))
}

#[test]
fn a_channel_scales_each_side_by_the_fader_and_v1s_pan_law_bit_for_bit() {
    let rendered = render(
        &admit(&channelled(constant(LEVEL), channel(0.75, -0.3, false))),
        &[],
    );
    let (left, right) = v1_sides(0.75, -0.3);
    assert_eq!(rendered.len(), (FRAMES * 2) as usize, "stereo frames");
    for (index, (l, r)) in frames(&rendered).enumerate() {
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            ((LEVEL * left).to_bits(), (LEVEL * right).to_bits()),
            "frame {index}: ({l}, {r}) is not V1's ({}, {})",
            LEVEL * left,
            LEVEL * right
        );
    }
    // And the pan moved the signal: the two sides differ, and both differ from the input.
    let (l, r) = frames(&rendered).next().expect("a frame");
    assert_ne!(l, r);
    assert_ne!(l, LEVEL);
}

#[test]
fn the_per_side_gain_is_formed_in_v1s_order() {
    // `(sample × coefficient) × fader` and `sample × (coefficient × fader)` round differently
    // for some triples, and V1's channel stage forms the second. A mutation to the first
    // survived the fixture above — its constants round the same either way — so this one
    // finds triples where the two orders differ and holds the render to V1's on each. The
    // control is that at least one triple differs at all.
    let candidates = [(0.3_f32, 0.6_f32, 1.7_f32), (0.7_f32, -0.9_f32, 1.3_f32)];
    let mut distinguished = 0;
    for (input, pan, fader) in candidates {
        let (left, right) = v1_sides(fader, pan);
        let (coefficient, _) = synth_core::Gain::from_pan(synth_core::BipolarValue::new(pan));
        if ((input * coefficient.as_f32()) * fader).to_bits() != (input * left).to_bits() {
            distinguished += 1;
        }
        let rendered = render(
            &admit(&channelled(constant(input), channel(fader, pan, false))),
            &[],
        );
        for (index, (l, r)) in frames(&rendered).enumerate() {
            assert_eq!(
                (l.to_bits(), r.to_bits()),
                ((input * left).to_bits(), (input * right).to_bits()),
                "frame {index} at ({input}, {pan}, {fader}): the gain was not formed in V1's order"
            );
        }
    }
    assert!(
        distinguished > 0,
        "no candidate distinguishes the two orders"
    );
}

#[test]
fn a_channel_at_unity_and_centre_is_v1s_centre_coefficient_and_not_neutral() {
    // The reviewed premise of `P08-S001`: neutral is *no channel*; a channel at unity and
    // centre attenuates each side by `cos(π/4)`, which is V1's law and what EVD-0013
    // measured three of in V1's chain.
    let rendered = render(
        &admit(&channelled(constant(LEVEL), channel(1.0, 0.0, false))),
        &[],
    );
    let (left, right) = v1_sides(1.0, 0.0);
    assert_eq!(left, right, "centre is symmetric");
    assert!(
        (left - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6,
        "centre is cos(π/4), got {left}"
    );
    for (l, r) in frames(&rendered) {
        assert_eq!(l.to_bits(), (LEVEL * left).to_bits());
        assert_eq!(r.to_bits(), (LEVEL * right).to_bits());
        assert_ne!(l, LEVEL, "a neutral channel is not a pass-through");
    }
}

#[test]
fn a_plan_with_no_channel_widens_a_mono_source_as_before() {
    // The bit-identity claim belongs to a plan that declares no channel: `layout_baseline`
    // pins the digests, and this holds the shape — a mono source into a stereo output is
    // duplicated, unattenuated, exactly as it was.
    let plan = common::admit(
        &common::source_plan(constant(LEVEL)),
        profile(FRAMES, ChannelLayout::Stereo),
    );
    assert!(plan.channels().is_empty());
    for (l, r) in frames(&render(&plan, &[])) {
        assert_eq!((l, r), (LEVEL, LEVEL));
    }
}

#[test]
fn two_sources_through_one_sum_render_the_sum_of_each_alone_exactly() {
    let one = render(&admit(&summed(&[(SOURCE, sine(440.0))])), &[]);
    let other = render(&admit(&summed(&[(SECOND_SOURCE, sine(660.0))])), &[]);
    let both = render(
        &admit(&summed(&[
            (SOURCE, sine(440.0)),
            (SECOND_SOURCE, sine(660.0)),
        ])),
        &[],
    );
    assert_eq!(both.len(), one.len());
    for (index, ((sum, a), b)) in both.iter().zip(&one).zip(&other).enumerate() {
        assert_eq!(
            sum.to_bits(),
            (a + b).to_bits(),
            "sample {index}: {sum} is not {a} + {b}"
        );
    }
    // A source alone through a sum is the source, widened: the sum added nothing to it.
    let direct = render(
        &common::admit(
            &common::source_plan(sine(440.0)),
            profile(FRAMES, ChannelLayout::Stereo),
        ),
        &[],
    );
    assert_eq!(one, direct, "one cable into a sum is the cable");
    // And the sum is a sum: the two-source render is not either alone.
    assert_ne!(both, one);
}

#[test]
fn a_sum_above_full_scale_is_preserved_in_float() {
    let both = render(
        &admit(&summed(&[
            (SOURCE, constant(0.8)),
            (SECOND_SOURCE, constant(0.7)),
        ])),
        &[],
    );
    for (l, r) in frames(&both) {
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            ((0.8_f32 + 0.7).to_bits(), (0.8_f32 + 0.7).to_bits())
        );
        assert!(l > 1.0, "the headroom is preserved, not clamped: {l}");
    }
}

#[test]
fn a_summed_port_adds_its_cables_in_ascending_source_identity() {
    // Three sources whose float sum depends on the order: the plan adds them in ascending
    // node identity, whichever order the cables were connected in (`SOUND-INV-008`).
    let a = 1.0e8_f32;
    let b = -1.0e8_f32;
    let c = 1.0_f32;
    let forward = summed(&[
        (NodeId::new(20), constant(a)),
        (NodeId::new(21), constant(b)),
        (NodeId::new(22), constant(c)),
    ]);
    let backward = summed(&[
        (NodeId::new(22), constant(c)),
        (NodeId::new(21), constant(b)),
        (NodeId::new(20), constant(a)),
    ]);
    let expected = (a + b) + c;
    assert_ne!(
        expected,
        a + (b + c),
        "the fixture's order matters in float"
    );
    for ir in [forward, backward] {
        for (l, r) in frames(&render(&admit(&ir), &[])) {
            assert_eq!((l, r), (expected, expected));
        }
    }
}

#[test]
fn a_mute_silences_the_channel_from_the_sample_it_lands_on_and_releases_from_its_own() {
    let plan = admit(&channelled(constant(LEVEL), channel(1.0, 0.0, false)));
    let mute = plan.channels().first().expect("one channel").mute;
    let (on, off) = (37_u64, 100_u64);
    let events = [
        OfflineEvent::new(
            SampleTime::new(on),
            CompiledPayload::SetParameter {
                slot: mute,
                value: ParameterValue::ONE,
            },
        ),
        OfflineEvent::new(
            SampleTime::new(off),
            CompiledPayload::SetParameter {
                slot: mute,
                value: ParameterValue::ZERO,
            },
        ),
    ];
    let rendered = render(&plan, &events);
    let (left, _) = v1_sides(1.0, 0.0);
    for (index, (l, r)) in frames(&rendered).enumerate() {
        let frame = index as u64;
        let expected = if (on..off).contains(&frame) {
            0.0
        } else {
            LEVEL * left
        };
        assert_eq!(
            (l, r),
            (expected, expected),
            "frame {frame}: mute in force from {on} to {off}"
        );
    }
}

#[test]
fn a_channel_that_starts_muted_renders_zeros() {
    let rendered = render(
        &admit(&channelled(constant(LEVEL), channel(1.0, 0.0, true))),
        &[],
    );
    assert!(rendered.iter().all(|sample| *sample == 0.0));
}

/// A control program whose output is the constant `level`.
fn control_script(level: f32) -> synth_engine_v2::script::ScriptProgram {
    use synth_engine_v2::script::{ProjectSeed, ScriptIdentity, ScriptStateId};
    let mut identity = ScriptIdentity::new(SCRIPT, ScriptStateId::new(1), ProjectSeed::new(1));
    identity
        .compile_control(&format!("out = {level}"), common::rate(48_000.0), &[])
        .expect("a constant program compiles")
}

#[test]
fn a_fader_write_an_edge_and_a_script_compose_in_the_one_slot_under_the_decibel_law() {
    // `SOUND-INV-023`'s order over the channel's fader: the override write is the base the
    // modulation sum composes on, and the sum gathers an LFO edge and a script edge in
    // decibels. The LFO is held at one — rate zero, a quarter turn in — so the oracle is a
    // closed form: `override × 10^((1 × 6 + 0.5 × 4) / 20)`, then V1's centre coefficient.
    const OVERRIDE: f32 = 0.5;
    let held_lfo = IrNodeKind::Lfo {
        waveform: LfoWaveform::Sine,
        rate: Frequency::ZERO,
        depth: NormalizedLevel::FULL,
        phase_offset: PhaseOffset::new(0.25).expect("a quarter turn"),
        polarity: LfoPolarity::Bipolar,
    };
    let modulated = GraphIr::builder()
        .node(SOURCE, constant(LEVEL), ExecutionScope::Global)
        .node(CHANNEL, channel(1.0, 0.0, false), ExecutionScope::Channel)
        .node(LFO, held_lfo, ExecutionScope::Global)
        .script(control_script(0.5), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (CHANNEL, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (CHANNEL, parameters::CHANNEL_FADER),
            ModulationDepth::new(ModulationUnit::Decibels, 6.0).expect("finite"),
        )
        .modulate(
            (SCRIPT, PortId::FIRST),
            (CHANNEL, parameters::CHANNEL_FADER),
            ModulationDepth::new(ModulationUnit::Decibels, 4.0).expect("finite"),
        )
        .build()
        .expect("a modulated channel is a readable plan");
    let plan = admit(&modulated);
    let fader = plan.channels().first().expect("one channel").fader;
    let write = [OfflineEvent::new(
        SampleTime::ZERO,
        CompiledPayload::SetParameter {
            slot: fader,
            value: ParameterValue::new(OVERRIDE).expect("finite"),
        },
    )];
    let rendered = render(&plan, &write);
    let (centre, _) = v1_sides(1.0, 0.0);
    let expected = LEVEL * centre * OVERRIDE * 10.0_f32.powf((6.0 + 0.5 * 4.0) / 20.0);
    // The last quantum, after every layer is in force.
    let last = &rendered[((FRAMES - Q) * 2) as usize..];
    for (l, r) in frames(last) {
        assert!(
            (l - expected).abs() < 1e-5 && (r - expected).abs() < 1e-5,
            "({l}, {r}) is not the composed {expected}"
        );
    }
    // The control: the write alone, without the edges, is a different render. The slot is
    // the plain plan's own — one from another plan is refused as foreign, and an earlier
    // draft of this control wrote through a second admission's slot and measured nothing.
    let plain = admit(&channelled(constant(LEVEL), channel(1.0, 0.0, false)));
    let unmodulated = render(
        &plain,
        &[OfflineEvent::new(
            SampleTime::ZERO,
            CompiledPayload::SetParameter {
                slot: plain.channels().first().expect("one channel").fader,
                value: ParameterValue::new(OVERRIDE).expect("finite"),
            },
        )],
    );
    let (l, _) = frames(&unmodulated[((FRAMES - Q) * 2) as usize..])
        .next()
        .expect("a frame");
    assert!(
        (l - LEVEL * centre * OVERRIDE).abs() < 1e-6,
        "the write alone: {l}"
    );
    assert_ne!(l.to_bits(), rendered[((FRAMES - Q) * 2) as usize].to_bits());
}

#[test]
fn a_channel_count_over_the_profile_is_refused_by_name_and_counted_from_the_plan() {
    // The plan's channels are its `Channel` nodes: two here, each with its own identity in
    // ascending node order and its three slots resolving on that node.
    const SECOND_CHANNEL: NodeId = NodeId::new(15);
    let two = GraphIr::builder()
        .node(SOURCE, constant(LEVEL), ExecutionScope::Global)
        .node(CHANNEL, channel(1.0, 0.0, false), ExecutionScope::Channel)
        .node(
            SECOND_CHANNEL,
            channel(0.5, 0.0, false),
            ExecutionScope::Channel,
        )
        .node(MIX, IrNodeKind::Mix, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (SECOND_CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (CHANNEL, PortId::FIRST),
            (MIX, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_CHANNEL, PortId::FIRST),
            (MIX, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MIX, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("two channels into a sum is a readable plan");
    let plan = admit(&two);
    let channels = plan.channels();
    assert_eq!(channels.len(), 2);
    assert_eq!(channels[0].node, CHANNEL);
    assert_eq!(channels[1].node, SECOND_CHANNEL);
    assert_ne!(channels[0].id, channels[1].id);
    assert_eq!((channels[0].id.index(), channels[1].id.index()), (0, 1));
    for record in channels {
        assert_eq!(record.id.plan(), plan.id());
        assert_eq!(
            plan.resolve_parameter(record.node, parameters::CHANNEL_FADER),
            Some(record.fader)
        );
        assert_eq!(
            plan.resolve_parameter(record.node, parameters::CHANNEL_PAN),
            Some(record.pan)
        );
        assert_eq!(
            plan.resolve_parameter(record.node, parameters::CHANNEL_MUTE),
            Some(record.mute)
        );
    }
    // And the two channels' faders sum: unity plus a half, each at centre.
    let (centre, _) = v1_sides(1.0, 0.0);
    for (l, r) in frames(&render(&plan, &[])) {
        let expected = LEVEL * centre + LEVEL * (centre * 0.5);
        assert!(
            (l - expected).abs() < 1e-6 && (r - expected).abs() < 1e-6,
            "({l}, {r})"
        );
    }
    // The refusal by count is `admission`'s table case for `max_mix_channels`; here the
    // report row reads the compiled count.
    let outcome = synth_engine_v2::compile::compile(
        &two,
        &synth_engine_v2::compile::RenderConfig::new(profile(FRAMES, ChannelLayout::Stereo)),
    );
    let row = outcome
        .report()
        .rows()
        .iter()
        .find(|row| row.field() == synth_engine_v2::report::ResourceField::MaxMixChannels)
        .expect("the mix-channel row");
    assert_eq!(
        row.requested(),
        synth_engine_v2::report::ResourceAmount::MixChannels(
            synth_engine_v2::quantities::MixChannelCount::measured(2)
        )
    );
}

#[test]
fn fan_in_into_a_port_that_does_not_declare_it_is_still_refused() {
    let two_into_a_channel = GraphIr::builder()
        .node(SOURCE, constant(LEVEL), ExecutionScope::Global)
        .node(SECOND_SOURCE, constant(LEVEL), ExecutionScope::Global)
        .node(CHANNEL, channel(1.0, 0.0, false), ExecutionScope::Channel)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_SOURCE, PortId::FIRST),
            (CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (CHANNEL, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable, and refused at compilation");
    let error = common::refuse(&two_into_a_channel, profile(FRAMES, ChannelLayout::Stereo));
    assert!(
        matches!(
            error,
            CompileError::UnsupportedFanIn {
                node: CHANNEL,
                edges: 2,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_channel_in_the_voice_scope_is_refused_by_name() {
    // An independent read built a two-voice channel and found the second voice's controls
    // landing on inserted steps. The voice sum's seed once also read a stereo instance
    // output as mono frames; `P08-S002` corrected the seed, so what keeps these two kinds
    // out of the voice scope is what they are — a channel is per instrument and a sum runs
    // once. Refused at validation, naming the node, rather than instantiated per voice.
    let voiced = GraphIr::builder()
        .node(SOURCE, constant(LEVEL), ExecutionScope::Voice)
        .node(CHANNEL, channel(1.0, 0.0, false), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (CHANNEL, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (CHANNEL, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(2))
        .build()
        .expect("readable, and refused at compilation");
    let error = common::refuse(&voiced, profile(FRAMES, ChannelLayout::Stereo));
    assert!(
        matches!(error, CompileError::MixerNodeInVoiceScope { node: CHANNEL }),
        "{error:?}"
    );
    // And a sum in the voice scope is refused the same way; the two kinds run once, outside
    // it, which is where the lowerer places the channel (`ExecutionScope::Channel`).
    let voiced_sum = GraphIr::builder()
        .node(SOURCE, constant(LEVEL), ExecutionScope::Voice)
        .node(MIX, IrNodeKind::Mix, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (MIX, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MIX, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable, and refused at compilation");
    assert!(matches!(
        common::refuse(&voiced_sum, profile(FRAMES, ChannelLayout::Stereo)),
        CompileError::MixerNodeInVoiceScope { node: MIX }
    ));
}

#[test]
fn a_channel_and_a_sum_admit_exactly_two_channels_on_every_port() {
    // ADR-0041 clause 12, in the direction `graph_validation` does not take: the kinds
    // whose kernels this file and `mix_stages` test at two channels declare exactly two on
    // every port, so the exemption every other kind takes cannot apply to them by mistake.
    use synth_engine_v2::node::NodeKindId;
    for kind in catalog() {
        let two = matches!(
            kind.id,
            NodeKindId::Channel
                | NodeKindId::Mix
                | NodeKindId::Balance
                | NodeKindId::Trim
                | NodeKindId::SoftClip
                | NodeKindId::HardClamp
        );
        if !two {
            continue;
        }
        for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
            let sample = match kind.id {
                NodeKindId::Channel => channel(1.0, 0.0, false),
                NodeKindId::Balance => IrNodeKind::Balance {
                    level: Amplitude::UNITY,
                    pan: BipolarLevel::ZERO,
                    muted: false,
                },
                NodeKindId::Trim => IrNodeKind::Trim {
                    level: Amplitude::UNITY,
                },
                NodeKindId::SoftClip => IrNodeKind::SoftClip,
                NodeKindId::HardClamp => IrNodeKind::HardClamp,
                _ => IrNodeKind::Mix,
            };
            for port in ports(sample, layout) {
                assert_eq!(port.layout(), ChannelLayout::Stereo, "{}", kind.name);
            }
        }
        // And the one port that sums is the sum's input, and only it.
        for port in &kind.ports {
            let sums = port.fan_in == FanIn::Summed;
            assert_eq!(
                sums,
                kind.id == synth_engine_v2::node::NodeKindId::Mix
                    && port.direction == synth_engine_v2::validate::PortDirection::Input,
                "{} port {:?}",
                kind.name,
                port.id
            );
        }
    }
}

#[test]
fn a_channelled_render_is_the_same_bits_under_every_host_partition() {
    let plan = admit(&channelled(sine(440.0), channel(0.8, 0.4, false)));
    let whole: Vec<usize> = vec![FRAMES as usize];
    let blocks_256: Vec<usize> = vec![256; (FRAMES / 256) as usize];
    let blocks_64: Vec<usize> = vec![64; (FRAMES / 64) as usize];
    let mut irregular = vec![37, 91, 5, 128, 1];
    let taken: usize = irregular.iter().sum();
    irregular.push(FRAMES as usize - taken);
    let reference = render_partitioned(&plan, &whole);
    let bits = |samples: &[f32]| samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
    for partition in [&blocks_256[..], &blocks_64[..], &irregular[..]] {
        assert_eq!(
            bits(&render_partitioned(&plan, partition)),
            bits(&reference),
            "partition {partition:?} rendered other bits"
        );
    }
    assert!(reference.iter().any(|s| s.abs() > 0.1), "and it sounds");
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
    assert_eq!(
        out.len(),
        (FRAMES * 2) as usize,
        "the partition covers the render"
    );
    out
}
