//! `P08-S004`, `SOUND-INV-034`: sends, buses and the bus graph.
//!
//! A bus is an entry sum, inserts, a strip and a clipper under one `Bus(tag)` scope; a send
//! is a node in the tagged scope of the channel or bus it belongs to, reading V1's tap point
//! for that scope and entering a bus's entry. The oracle for every gain is **V1's own law
//! spelled term for term** — `apply_send_tap` for a channel's two taps, `render_output` for
//! a bus's — so each comparison is bit for bit, including the one product order that
//! separates V1's post-fader tap from a channel followed by a send.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::controller::BipolarLevel;
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{
    BusTag, ChannelTag, ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain,
    parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::{CompiledPlan, SendSource, SendTap};
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, Frequency, ParameterValue};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::report::{ResourceAmount, ResourceField};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const STRIP: NodeId = NodeId::new(10);
const SEND: NodeId = NodeId::new(11);
const ENTRY: NodeId = NodeId::new(12);
const BUS_STRIP: NodeId = NodeId::new(13);
const BUS_CLIP: NodeId = NodeId::new(14);
const BUS_SEND: NodeId = NodeId::new(15);
const ENTRY_B: NodeId = NodeId::new(16);
const BUS_STRIP_B: NodeId = NodeId::new(17);
const BUS_SEND_B: NodeId = NodeId::new(18);
const ENTRY_C: NodeId = NodeId::new(19);
const BUS_STRIP_C: NodeId = NodeId::new(20);
const MASTER: NodeId = NodeId::new(21);
const SECOND_SEND: NodeId = NodeId::new(22);
const Q: u64 = QUANTUM_FRAMES as u64;
const QUANTA: u64 = 8;
const FRAMES: u64 = QUANTA * Q;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const CHANNEL_A: ExecutionScope = ExecutionScope::Channel(ChannelTag::FIRST);
const BUS_A: ExecutionScope = ExecutionScope::Bus(BusTag::FIRST);
const BUS_B: ExecutionScope = ExecutionScope::Bus(BusTag::new(1));
const BUS_C: ExecutionScope = ExecutionScope::Bus(BusTag::new(2));

fn level(value: f32) -> Amplitude {
    Amplitude::new(value).expect("finite")
}

fn constant(value: f32) -> IrNodeKind {
    IrNodeKind::Constant {
        level: level(value),
    }
}

fn sine(hz: f32) -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(hz).expect("finite"),
        amplitude: level(0.5),
    }
}

fn channel(fader: f32, pan: f32, muted: bool) -> IrNodeKind {
    IrNodeKind::Channel {
        fader: level(fader),
        pan: BipolarLevel::new(pan).expect("in range"),
        muted,
    }
}

fn post_fader_send(fader: f32, pan: f32, muted: bool, amount: f32) -> IrNodeKind {
    IrNodeKind::PostFaderSend {
        fader: level(fader),
        pan: BipolarLevel::new(pan).expect("in range"),
        muted,
        level: level(amount),
    }
}

fn send(amount: f32, muted: bool) -> IrNodeKind {
    IrNodeKind::Send {
        level: level(amount),
        muted,
    }
}

fn audio(from: NodeId, to: NodeId) -> ((NodeId, PortId), (NodeId, PortId)) {
    ((from, PortId::FIRST), (to, PortId::FIRST))
}

/// Nodes and cables into a plan, every strip into one master sum into the output.
fn plan_of(nodes: &[(NodeId, IrNodeKind, ExecutionScope)], cables: &[(NodeId, NodeId)]) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(MASTER, IrNodeKind::Mix, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (MASTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    for (id, kind, scope) in nodes {
        builder = builder.node(*id, *kind, *scope);
    }
    for (from, to) in cables {
        let (from, to) = audio(*from, *to);
        builder = builder.connect(from, to, SignalDomain::Audio);
    }
    builder.build().expect("a readable plan")
}

/// One source through a channel strip and one send into one bus, both strips into the master.
fn sent(source: IrNodeKind, strip: IrNodeKind, tap: IrNodeKind, bus_strip: IrNodeKind) -> GraphIr {
    plan_of(
        &[
            (SOURCE, source, ExecutionScope::Global),
            (STRIP, strip, CHANNEL_A),
            (SEND, tap, CHANNEL_A),
            (ENTRY, IrNodeKind::Mix, BUS_A),
            (BUS_STRIP, bus_strip, BUS_A),
        ],
        &[
            (SOURCE, STRIP),
            (SOURCE, SEND),
            (SEND, ENTRY),
            (ENTRY, BUS_STRIP),
            (STRIP, MASTER),
            (BUS_STRIP, MASTER),
        ],
    )
}

fn compile(ir: &GraphIr) -> synth_engine_v2::compile::CompileOutcome {
    synth_engine_v2::compile::compile(
        ir,
        &synth_engine_v2::compile::RenderConfig::new(profile(FRAMES, ChannelLayout::Stereo)),
    )
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    common::admit(ir, profile(FRAMES, ChannelLayout::Stereo))
}

fn refuse(ir: &GraphIr) -> CompileError {
    common::refuse(ir, profile(FRAMES, ChannelLayout::Stereo))
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

fn frames(interleaved: &[f32]) -> impl Iterator<Item = (f32, f32)> + '_ {
    interleaved
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| (frame[0], frame[1]))
}

/// V1's channel gain per side, as `mix_channel_busses` forms it: the pan coefficient from
/// V1's own function times the fader.
fn v1_sides(fader: f32, pan: f32) -> (f32, f32) {
    let (left, right) = synth_core::Gain::from_pan(synth_core::BipolarValue::new(pan));
    (left.as_f32() * fader, right.as_f32() * fader)
}

/// V1's post-fader tap per side, as `apply_send_tap` forms it: the side's gain times the
/// level, **then** the sample times that.
fn v1_post_fader_tap(sample: f32, side_gain: f32, amount: f32) -> f32 {
    let l = side_gain * amount;
    sample * l
}

/// V1's `soft_clip`, term for term, `signum` included.
fn v1_soft_clip(sample: f32) -> f32 {
    const SOFT_CLIP_THRESHOLD: f32 = 0.8;
    if sample.abs() <= SOFT_CLIP_THRESHOLD {
        sample
    } else {
        let sign = sample.signum();
        let abs_sample = sample.abs();
        let excess = abs_sample - SOFT_CLIP_THRESHOLD;
        let headroom = 1.0 - SOFT_CLIP_THRESHOLD;
        let compressed = SOFT_CLIP_THRESHOLD + headroom * (1.0 - (-excess / headroom).exp());
        sign * compressed
    }
}

/// V1's return output per sample, as `render_output` forms it: the sample times the side's
/// gain, soft-clipped.
fn v1_return(sample: f32, side_gain: f32) -> f32 {
    v1_soft_clip(sample * side_gain)
}

/// The mute slot of the node whose kind declares `parameter`.
fn mute_slot(
    plan: &CompiledPlan,
    node: NodeId,
    parameter: synth_engine_v2::ir::ParameterId,
) -> synth_engine_v2::plan::ParameterSlot {
    plan.parameter_addresses()
        .iter()
        .find(|address| address.node == node && address.parameter == parameter)
        .map(|address| address.slot)
        .expect("the node declares the control")
}

#[test]
fn a_post_fader_send_forms_v1s_gain_in_v1s_order_bit_for_bit() {
    // Codex's distinguishing sample: at this source, fader, pan and level V1's
    // `src × (gain × level)` and a channel followed by a send's `(src × gain) × level` differ
    // in the last bit, so the oracle's order is what the test holds and the other order is
    // the control that would fail it.
    let (source, fader, pan, amount) = (0.1_f32, 0.8_f32, 0.0_f32, 0.6_f32);
    let plan = admit(&sent(
        constant(source),
        channel(fader, pan, false),
        post_fader_send(fader, pan, false, amount),
        channel(1.0, 0.0, false),
    ));
    let (left, right) = v1_sides(fader, pan);
    let (bus_left, bus_right) = v1_sides(1.0, 0.0);
    // The master holds the dry channel and the bus; the bus is what the send fed, times the
    // unity-centre bus strip. Read the wet path alone by muting the dry strip.
    let dry_mute = plan.channels().first().expect("the channel").mute;
    let wet_alone = render(
        &plan,
        &[OfflineEvent::new(
            SampleTime::ZERO,
            CompiledPayload::SetParameter {
                slot: dry_mute,
                value: ParameterValue::ONE,
            },
        )],
    );
    let expected_left = v1_post_fader_tap(source, left, amount) * bus_left;
    let expected_right = v1_post_fader_tap(source, right, amount) * bus_right;
    let other_order = ((source * left) * amount) * bus_left;
    assert_ne!(
        expected_left.to_bits(),
        other_order.to_bits(),
        "the sample separates the two product orders; otherwise it holds nothing"
    );
    for (index, (l, r)) in frames(&wet_alone).enumerate() {
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            (expected_left.to_bits(), expected_right.to_bits()),
            "frame {index}: V1's post-fader tap, bit for bit"
        );
    }
    let record = plan.sends().first().expect("one send");
    assert_eq!(record.node, SEND);
    assert_eq!(record.tap, SendTap::PostFader);
    assert_eq!(
        record.from,
        SendSource::Channel(plan.channels().first().expect("the channel").id)
    );
    assert_eq!(record.into, plan.buses().first().expect("the bus").id);
}

#[test]
fn a_pre_fader_send_is_the_signal_times_the_level_and_its_mute_silences_it_from_its_sample() {
    let (source, amount) = (0.3_f32, 0.6_f32);
    let plan = admit(&sent(
        constant(source),
        channel(0.5, 0.7, false),
        send(amount, false),
        channel(1.0, 0.0, false),
    ));
    let dry_mute = plan.channels().first().expect("the channel").mute;
    let send_mute = plan.sends().first().expect("one send").mute;
    assert_eq!(send_mute, mute_slot(&plan, SEND, parameters::SEND_MUTE));
    let (on, off) = (37_u64, 100_u64);
    let rendered = render(
        &plan,
        &[
            OfflineEvent::new(
                SampleTime::ZERO,
                CompiledPayload::SetParameter {
                    slot: dry_mute,
                    value: ParameterValue::ONE,
                },
            ),
            OfflineEvent::new(
                SampleTime::new(on),
                CompiledPayload::SetParameter {
                    slot: send_mute,
                    value: ParameterValue::ONE,
                },
            ),
            OfflineEvent::new(
                SampleTime::new(off),
                CompiledPayload::SetParameter {
                    slot: send_mute,
                    value: ParameterValue::ZERO,
                },
            ),
        ],
    );
    let (bus_left, bus_right) = v1_sides(1.0, 0.0);
    // V1's pre-fader tap: the raw signal times the level on both sides, and nothing from a
    // channel that is not audible.
    let expected = ((source * amount) * bus_left, (source * amount) * bus_right);
    for (index, (l, r)) in frames(&rendered).enumerate() {
        let frame = index as u64;
        let want = if (on..off).contains(&frame) {
            (0.0, 0.0)
        } else {
            expected
        };
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            (want.0.to_bits(), want.1.to_bits()),
            "frame {frame}"
        );
    }
    assert_eq!(
        plan.sends().first().expect("one send").tap,
        SendTap::PreFader
    );
}

#[test]
fn the_master_holds_dry_plus_wet_exactly_and_each_alone_changes_the_render() {
    let plan = admit(&sent(
        sine(440.0),
        channel(0.8, -0.3, false),
        post_fader_send(0.8, -0.3, false, 0.6),
        channel(0.9, 0.2, false),
    ));
    let dry_mute = plan.channels().first().expect("the channel").mute;
    let bus_mute = plan.buses().first().expect("the bus").mute;
    let send_level = plan.sends().first().expect("the send").level;
    let mute = |slot| {
        OfflineEvent::new(
            SampleTime::ZERO,
            CompiledPayload::SetParameter {
                slot,
                value: ParameterValue::ONE,
            },
        )
    };
    let full = render(&plan, &[]);
    let dry_alone = render(&plan, &[mute(bus_mute)]);
    let wet_alone = render(&plan, &[mute(dry_mute)]);
    let sent_nothing = render(
        &plan,
        &[OfflineEvent::new(
            SampleTime::ZERO,
            CompiledPayload::SetParameter {
                slot: send_level,
                value: ParameterValue::ZERO,
            },
        )],
    );
    assert!(full.iter().any(|s| *s != 0.0));
    // The one sum: dry seeded, wet accumulated, so the master is exactly the float sum.
    for (index, ((f, d), w)) in full.iter().zip(&dry_alone).zip(&wet_alone).enumerate() {
        assert_eq!(f.to_bits(), (d + w).to_bits(), "sample {index}");
    }
    // CORPUS-0004-P1's shape: muting the return alone changes the render, and so does
    // zeroing the send.
    assert_ne!(full, dry_alone, "the bus contributes");
    assert_ne!(full, sent_nothing, "the send contributes");
    assert_eq!(
        dry_alone, sent_nothing,
        "a send at zero and a muted return are the same absence"
    );
}

/// A three-bus chain: the channel sends into A, A sends into B after its clipper, B sends
/// into C; the three returns and the dry channel reach the master. The oracle applies V1's
/// laws in that order to the source alone.
fn chained(clipped: bool) -> GraphIr {
    let mut nodes = vec![
        (SOURCE, constant(0.95), ExecutionScope::Global),
        (STRIP, channel(1.0, 0.0, false), CHANNEL_A),
        (SEND, post_fader_send(1.0, 0.0, false, 1.0), CHANNEL_A),
        (ENTRY, IrNodeKind::Mix, BUS_A),
        (BUS_STRIP, channel(2.0, 0.1, false), BUS_A),
        (BUS_SEND, send(0.8, false), BUS_A),
        (ENTRY_B, IrNodeKind::Mix, BUS_B),
        (BUS_STRIP_B, channel(1.5, -0.2, false), BUS_B),
        (BUS_SEND_B, send(0.7, false), BUS_B),
        (ENTRY_C, IrNodeKind::Mix, BUS_C),
        (BUS_STRIP_C, channel(0.5, 0.0, false), BUS_C),
    ];
    let mut cables = vec![
        (SOURCE, STRIP),
        (SOURCE, SEND),
        (SEND, ENTRY),
        (ENTRY, BUS_STRIP),
        (ENTRY_B, BUS_STRIP_B),
        (ENTRY_C, BUS_STRIP_C),
        (BUS_SEND, ENTRY_B),
        (BUS_SEND_B, ENTRY_C),
        (STRIP, MASTER),
        (BUS_STRIP_C, MASTER),
    ];
    if clipped {
        nodes.push((BUS_CLIP, IrNodeKind::SoftClip, BUS_A));
        cables.extend([
            (BUS_STRIP, BUS_CLIP),
            (BUS_CLIP, BUS_SEND),
            (BUS_CLIP, MASTER),
        ]);
        // B's send taps B's strip directly: a bus without a clipper.
        cables.extend([(BUS_STRIP_B, BUS_SEND_B), (BUS_STRIP_B, MASTER)]);
    } else {
        cables.extend([(BUS_STRIP, BUS_SEND), (BUS_STRIP, MASTER)]);
        cables.extend([(BUS_STRIP_B, BUS_SEND_B), (BUS_STRIP_B, MASTER)]);
    }
    plan_of(&nodes, &cables)
}

#[test]
fn a_bus_send_taps_the_clipped_output_and_a_chain_renders_v1s_laws_in_order() {
    let plan = admit(&chained(true));
    assert_eq!(plan.buses().len(), 3);
    assert_eq!(plan.sends().len(), 3);
    let rendered = render(&plan, &[]);
    // The oracle, per side, V1's laws in order over the constant source: the dry channel,
    // then A (post-fader tap, strip, clip), then A's send into B (clipped output times
    // level), B's strip, B's send into C, C's strip; the master sums in ascending source
    // identity — the dry strip (10), then A's clipper (14), B's strip (17), C's strip (20).
    let source = 0.95_f32;
    let (dry_l, dry_r) = v1_sides(1.0, 0.0);
    let (a_l, a_r) = v1_sides(2.0, 0.1);
    let (b_l, b_r) = v1_sides(1.5, -0.2);
    let (c_l, c_r) = v1_sides(0.5, 0.0);
    let side = |dry: f32, a: f32, b: f32, c: f32| {
        let dry_out = source * dry;
        let tapped = v1_post_fader_tap(source, dry, 1.0);
        let a_out = v1_return(tapped, a);
        let a_sent = a_out * 0.8;
        let b_out = a_sent * b;
        let b_sent = b_out * 0.7;
        let c_out = b_sent * c;
        ((dry_out + a_out) + b_out) + c_out
    };
    let expected = (side(dry_l, a_l, b_l, c_l), side(dry_r, a_r, b_r, c_r));
    // The clipper is in the oracle rather than identity only where A's **panned** output
    // passes the knee on both sides — the post-fader tap attenuates by V1's centre
    // coefficient and A's strip by its own pan law, which the raw product ignores; an
    // independent read found the earlier check computed the product and the signal
    // sat under the knee.
    for (side, dry, a) in [("left", dry_l, a_l), ("right", dry_r, a_r)] {
        let into_clip = v1_post_fader_tap(source, dry, 1.0) * a;
        assert!(
            into_clip > 0.8,
            "{side}: {into_clip} enters A's clipper under the knee, so the tap point would \
             be indistinguishable"
        );
        assert_ne!(v1_soft_clip(into_clip).to_bits(), into_clip.to_bits());
    }
    for (index, (l, r)) in frames(&rendered).enumerate() {
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            (expected.0.to_bits(), expected.1.to_bits()),
            "frame {index}"
        );
    }
    // The chain's dependency order is compiled once: the same bits under every partition.
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
}

/// The stereo harness profile with eight times the default event partition, for a plan
/// whose catch-up addresses outnumber the default session share.
fn roomy_profile() -> synth_engine_v2::profile::HostProfile {
    use synth_engine_v2::profile::{EventLimits, HostProfile, ProducerShares, RenderLimits};
    use synth_engine_v2::quantities::EventCount;
    let base = profile(FRAMES, ChannelLayout::Stereo);
    let defaults = RenderLimits::engine_defaults(base.capabilities()).expect("defaults");
    let events = defaults.events();
    let shares = events.shares();
    let scale = |count: EventCount| EventCount::limit(count.get() * 8).expect("a capacity");
    let shares = ProducerShares::new(
        scale(shares.compiled_event_share()),
        scale(shares.authored_runtime_event_share()),
        scale(shares.live_event_share()),
        scale(shares.session_event_share()),
        scale(shares.internal_event_share()),
        scale(shares.release_event_share()),
        scale(shares.release_hold_capacity()),
    )
    .expect("scaled shares");
    let events = EventLimits::new(
        scale(events.max_events_per_quantum()),
        events.max_note_expansion_per_tick(),
        scale(events.max_scheduled_events_in_flight()),
        events.forward_event_horizon(),
        events.queues(),
        shares,
    )
    .expect("scaled events");
    let limits = RenderLimits::new(
        defaults.stream(),
        defaults.graph(),
        defaults.voices(),
        events,
        defaults.observation(),
        defaults.mixing(),
        defaults.memory(),
        defaults.script(),
        defaults.recording(),
        defaults.cost(),
    )
    .expect("consistent limits");
    HostProfile::new(base.capabilities(), limits).expect("a consistent profile")
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

#[test]
fn two_buses_sending_into_each_other_are_refused_naming_a_closing_cable() {
    // A sends into B and B into A: a cycle through two bus graphs, refused at compilation
    // by the cable that closes it — any cable of the cycle is a correct name, since which
    // one the walk reaches last depends on identities rather than on intent.
    let ir = plan_of(
        &[
            (SOURCE, constant(0.5), ExecutionScope::Global),
            (STRIP, channel(1.0, 0.0, false), CHANNEL_A),
            (SEND, send(0.5, false), CHANNEL_A),
            (ENTRY, IrNodeKind::Mix, BUS_A),
            (BUS_STRIP, channel(1.0, 0.0, false), BUS_A),
            (BUS_SEND, send(0.5, false), BUS_A),
            (ENTRY_B, IrNodeKind::Mix, BUS_B),
            (BUS_STRIP_B, channel(1.0, 0.0, false), BUS_B),
            (BUS_SEND_B, send(0.5, false), BUS_B),
        ],
        &[
            (SOURCE, STRIP),
            (SOURCE, SEND),
            (SEND, ENTRY),
            (ENTRY, BUS_STRIP),
            (BUS_STRIP, BUS_SEND),
            (BUS_SEND, ENTRY_B),
            (ENTRY_B, BUS_STRIP_B),
            (BUS_STRIP_B, BUS_SEND_B),
            (BUS_SEND_B, ENTRY),
            (STRIP, MASTER),
            (BUS_STRIP, MASTER),
            (BUS_STRIP_B, MASTER),
        ],
    );
    let cycle: Vec<(NodeId, NodeId)> = vec![
        (ENTRY, BUS_STRIP),
        (BUS_STRIP, BUS_SEND),
        (BUS_SEND, ENTRY_B),
        (ENTRY_B, BUS_STRIP_B),
        (BUS_STRIP_B, BUS_SEND_B),
        (BUS_SEND_B, ENTRY),
    ];
    match refuse(&ir) {
        CompileError::Cycle { edge, node, .. } => {
            let named = ir
                .edges()
                .iter()
                .find(|candidate| candidate.id() == edge)
                .expect("the named edge exists");
            assert!(
                cycle.contains(&(named.from().0, named.to().0)),
                "{edge} ({} -> {}) is not a cable of the bus cycle",
                named.from().0,
                named.to().0
            );
            assert_eq!(named.to().0, node, "the edge re-enters the node it names");
        }
        other => panic!("expected the cycle refused, got {other:?}"),
    }
}

#[test]
fn the_mixer_scopes_are_held_to_their_shape_by_name() {
    let strip = channel(1.0, 0.0, false);
    let base = |extra_nodes: &[(NodeId, IrNodeKind, ExecutionScope)],
                cables: &[(NodeId, NodeId)]| {
        let mut nodes = vec![
            (SOURCE, constant(0.5), ExecutionScope::Global),
            (STRIP, strip, CHANNEL_A),
            (ENTRY, IrNodeKind::Mix, BUS_A),
            (BUS_STRIP, strip, BUS_A),
        ];
        nodes.extend_from_slice(extra_nodes);
        let mut all = vec![
            (SOURCE, STRIP),
            (ENTRY, BUS_STRIP),
            (STRIP, MASTER),
            (BUS_STRIP, MASTER),
        ];
        all.extend_from_slice(cables);
        plan_of(&nodes, &all)
    };
    // A send in the global scope says whose send it is with nothing.
    assert!(matches!(
        refuse(&base(
            &[(SEND, send(0.5, false), ExecutionScope::Global)],
            &[(SOURCE, SEND), (SEND, ENTRY)]
        )),
        CompileError::SendOutsideMixerScope { node: SEND, .. }
    ));
    // A channel's send reads what its strip reads, not the strip's output.
    assert!(matches!(
        refuse(&base(
            &[(SEND, send(0.5, false), CHANNEL_A)],
            &[(STRIP, SEND), (SEND, ENTRY)]
        )),
        CompileError::SendTapMismatch {
            node: SEND,
            strip: STRIP
        }
    ));
    // A send goes somewhere: one cable, into a bus's entry.
    assert!(matches!(
        refuse(&base(
            &[(SEND, send(0.5, false), CHANNEL_A)],
            &[(SOURCE, SEND)]
        )),
        CompileError::SendNotIntoBus { node: SEND, .. }
    ));
    assert!(matches!(
        refuse(&base(
            &[(SEND, send(0.5, false), CHANNEL_A)],
            &[(SOURCE, SEND), (SEND, MASTER)]
        )),
        CompileError::SendNotIntoBus { node: SEND, .. }
    ));
    // A bus's send reads its strip or the clipper the strip feeds, not the entry.
    assert!(matches!(
        refuse(&base(
            &[
                (BUS_SEND, send(0.5, false), BUS_A),
                (ENTRY_B, IrNodeKind::Mix, BUS_B),
                (BUS_STRIP_B, strip, BUS_B),
            ],
            &[
                (ENTRY, BUS_SEND),
                (BUS_SEND, ENTRY_B),
                (ENTRY_B, BUS_STRIP_B),
                (BUS_STRIP_B, MASTER)
            ]
        )),
        CompileError::SendTapMismatch {
            node: BUS_SEND,
            strip: BUS_STRIP
        }
    ));
    // A post-fader send forms a channel's gain; a bus's tap is its clipped output.
    assert!(matches!(
        refuse(&base(
            &[
                (BUS_SEND, post_fader_send(1.0, 0.0, false, 0.5), BUS_A),
                (ENTRY_B, IrNodeKind::Mix, BUS_B),
                (BUS_STRIP_B, strip, BUS_B),
            ],
            &[
                (ENTRY, BUS_SEND),
                (BUS_SEND, ENTRY_B),
                (ENTRY_B, BUS_STRIP_B),
                (BUS_STRIP_B, MASTER)
            ]
        )),
        CompileError::PostFaderSendOnBus { node: BUS_SEND }
    ));
    // A strip is a channel's or a bus's.
    assert!(matches!(
        refuse(&plan_of(
            &[
                (SOURCE, constant(0.5), ExecutionScope::Global),
                (STRIP, strip, ExecutionScope::Global)
            ],
            &[(SOURCE, STRIP), (STRIP, MASTER)]
        )),
        CompileError::ChannelOutsideChannelScope { node: STRIP, .. }
    ));
    // One strip per tagged scope.
    assert!(matches!(
        refuse(&base(
            &[(SECOND_SEND, strip, CHANNEL_A)],
            &[(SOURCE, SECOND_SEND), (SECOND_SEND, MASTER)]
        )),
        CompileError::ScopeWithTwoStrips { .. }
    ));
    // A bus has an entry, and one.
    assert!(matches!(
        refuse(&plan_of(
            &[
                (SOURCE, constant(0.5), ExecutionScope::Global),
                (BUS_STRIP, strip, BUS_A)
            ],
            &[(SOURCE, BUS_STRIP), (BUS_STRIP, MASTER)]
        )),
        CompileError::BusWithoutEntry {
            strip: BUS_STRIP,
            ..
        }
    ));
    assert!(matches!(
        refuse(&base(
            &[(ENTRY_B, IrNodeKind::Mix, BUS_A)],
            &[(SOURCE, ENTRY_B), (ENTRY_B, MASTER)]
        )),
        CompileError::BusWithTwoEntries { .. }
    ));
    // An entry with no strip is a bus that goes nowhere.
    assert!(matches!(
        refuse(&plan_of(
            &[
                (SOURCE, constant(0.5), ExecutionScope::Global),
                (ENTRY, IrNodeKind::Mix, BUS_A)
            ],
            &[(SOURCE, ENTRY), (ENTRY, MASTER)]
        )),
        CompileError::ScopeWithoutStrip { node: ENTRY, .. }
    ));
    // A send in the voice scope would be instantiated per voice.
    assert!(matches!(
        refuse(&base(
            &[(SEND, send(0.5, false), ExecutionScope::Voice)],
            &[(SOURCE, SEND), (SEND, ENTRY)]
        )),
        CompileError::MixerNodeInVoiceScope { node: SEND }
    ));
}

#[test]
fn admission_counts_buses_and_the_busiest_channels_sends_from_the_plan() {
    // One channel with a pre-fader and a post-fader send into two buses: two buses, and two
    // sends on the one channel — pre and post together, as V1's per-channel list holds both.
    let ir = plan_of(
        &[
            (SOURCE, constant(0.5), ExecutionScope::Global),
            (STRIP, channel(1.0, 0.0, false), CHANNEL_A),
            (SEND, post_fader_send(1.0, 0.0, false, 0.5), CHANNEL_A),
            (SECOND_SEND, send(0.5, false), CHANNEL_A),
            (ENTRY, IrNodeKind::Mix, BUS_A),
            (BUS_STRIP, channel(1.0, 0.0, false), BUS_A),
            (ENTRY_B, IrNodeKind::Mix, BUS_B),
            (BUS_STRIP_B, channel(1.0, 0.0, false), BUS_B),
        ],
        &[
            (SOURCE, STRIP),
            (SOURCE, SEND),
            (SOURCE, SECOND_SEND),
            (SEND, ENTRY),
            (SECOND_SEND, ENTRY_B),
            (ENTRY, BUS_STRIP),
            (ENTRY_B, BUS_STRIP_B),
            (STRIP, MASTER),
            (BUS_STRIP, MASTER),
            (BUS_STRIP_B, MASTER),
        ],
    );
    let outcome = compile(&ir);
    let requested = |field| {
        outcome
            .report()
            .row(field)
            .expect("the row exists")
            .requested()
    };
    assert!(matches!(
        requested(ResourceField::MaxBuses),
        ResourceAmount::Buses(count) if count.get() == 2
    ));
    assert!(matches!(
        requested(ResourceField::MaxSendsPerChannel),
        ResourceAmount::Sends(count) if count.get() == 2
    ));
    assert!(matches!(
        requested(ResourceField::MaxMixChannels),
        ResourceAmount::MixChannels(count) if count.get() == 1
    ));
    let plan = outcome.into_plan().expect("admitted");
    let buses = plan.buses();
    assert_eq!(buses.len(), 2);
    assert_eq!((buses[0].strip, buses[0].entry), (BUS_STRIP, ENTRY));
    assert_eq!((buses[1].strip, buses[1].entry), (BUS_STRIP_B, ENTRY_B));
    assert_eq!(buses[0].id.index(), 0);
    assert_eq!(buses[1].id.index(), 1);
    let sends = plan.sends();
    assert_eq!(sends.len(), 2);
    assert_eq!(sends[0].node, SEND);
    assert_eq!(sends[0].tap, SendTap::PostFader);
    assert_eq!(sends[0].into, buses[0].id);
    assert_eq!(sends[1].node, SECOND_SEND);
    assert_eq!(sends[1].tap, SendTap::PreFader);
    assert_eq!(sends[1].into, buses[1].id);
    assert_eq!(plan.channels().len(), 1, "a bus strip is not a mix channel");
}

#[test]
fn seventeen_sends_on_one_channel_are_refused_by_name_where_v1_dropped_the_seventeenth() {
    // Sixteen sends carry thirty-two writable controls, past the harness profile's session
    // share (ADR-0051's catch-up charges one row per address), so the refusal under test
    // is read against a profile whose event partition is eight times the default and whose
    // send cap is the default's.
    let host = roomy_profile();
    let limit = host.limits().mixing().max_sends_per_channel().get();
    assert_eq!(limit, 16, "V1's carry-over, `LIMIT-0024`");
    let build = |count: u32| {
        let mut nodes = vec![
            (SOURCE, constant(0.1), ExecutionScope::Global),
            (STRIP, channel(1.0, 0.0, false), CHANNEL_A),
            (ENTRY, IrNodeKind::Mix, BUS_A),
            (BUS_STRIP, channel(1.0, 0.0, false), BUS_A),
        ];
        let mut cables = vec![
            (SOURCE, STRIP),
            (ENTRY, BUS_STRIP),
            (STRIP, MASTER),
            (BUS_STRIP, MASTER),
        ];
        for k in 0..count {
            let id = NodeId::new(100 + k);
            nodes.push((id, send(0.5, false), CHANNEL_A));
            cables.extend([(SOURCE, id), (id, ENTRY)]);
        }
        plan_of(&nodes, &cables)
    };
    assert_eq!(common::admit(&build(limit), host).sends().len(), 16);
    match common::refuse(&build(limit + 1), host) {
        CompileError::LimitExceeded {
            field,
            requested,
            available,
            ..
        } => {
            assert_eq!(field, ResourceField::MaxSendsPerChannel);
            assert!(matches!(requested, ResourceAmount::Sends(count) if count.get() == 17));
            assert!(matches!(available, ResourceAmount::Sends(count) if count.get() == 16));
        }
        other => panic!("expected the send count refused, got {other:?}"),
    }
}

#[test]
fn two_channels_on_one_source_keep_independent_sends() {
    // The first exit bullet's send clause: two channels, two sends into two buses; moving
    // one channel's send level leaves the other bus's contribution bit for bit.
    const STRIP_2: NodeId = NodeId::new(40);
    const SEND_2: NodeId = NodeId::new(41);
    let channel_b = ExecutionScope::Channel(ChannelTag::new(1));
    let ir = plan_of(
        &[
            (SOURCE, sine(220.0), ExecutionScope::Global),
            (STRIP, channel(0.7, 0.0, false), CHANNEL_A),
            (SEND, post_fader_send(0.7, 0.0, false, 0.5), CHANNEL_A),
            (STRIP_2, channel(0.7, 0.0, false), channel_b),
            (SEND_2, post_fader_send(0.7, 0.0, false, 0.5), channel_b),
            (ENTRY, IrNodeKind::Mix, BUS_A),
            (BUS_STRIP, channel(1.0, 0.0, false), BUS_A),
            (ENTRY_B, IrNodeKind::Mix, BUS_B),
            (BUS_STRIP_B, channel(1.0, 0.0, false), BUS_B),
        ],
        &[
            (SOURCE, STRIP),
            (SOURCE, SEND),
            (SOURCE, STRIP_2),
            (SOURCE, SEND_2),
            (SEND, ENTRY),
            (SEND_2, ENTRY_B),
            (ENTRY, BUS_STRIP),
            (ENTRY_B, BUS_STRIP_B),
            (STRIP, MASTER),
            (STRIP_2, MASTER),
            (BUS_STRIP, MASTER),
            (BUS_STRIP_B, MASTER),
        ],
    );
    let outcome = compile(&ir);
    assert!(
        matches!(
            outcome
                .report()
                .row(ResourceField::MaxSendsPerChannel)
                .expect("the row exists")
                .requested(),
            ResourceAmount::Sends(count) if count.get() == 1
        ),
        "one send per channel, not two in the plan"
    );
    let plan = outcome.into_plan().expect("admitted");
    assert_eq!(plan.channels().len(), 2);
    assert_eq!(plan.sends().len(), 2);
    let mute_all_but = |keep: NodeId| -> Vec<OfflineEvent> {
        let mut events = Vec::new();
        for record in plan.channels() {
            events.push(OfflineEvent::new(
                SampleTime::ZERO,
                CompiledPayload::SetParameter {
                    slot: record.mute,
                    value: ParameterValue::ONE,
                },
            ));
        }
        for record in plan.buses() {
            if record.strip != keep {
                events.push(OfflineEvent::new(
                    SampleTime::ZERO,
                    CompiledPayload::SetParameter {
                        slot: record.mute,
                        value: ParameterValue::ONE,
                    },
                ));
            }
        }
        events
    };
    let bus_b_before = render(&plan, &mute_all_but(BUS_STRIP_B));
    let send_a = plan
        .sends()
        .iter()
        .find(|record| record.node == SEND)
        .expect("channel A's send")
        .level;
    let mut moved = mute_all_but(BUS_STRIP_B);
    moved.push(OfflineEvent::new(
        SampleTime::ZERO,
        CompiledPayload::SetParameter {
            slot: send_a,
            value: ParameterValue::ZERO,
        },
    ));
    let bus_b_after = render(&plan, &moved);
    assert!(bus_b_before.iter().any(|s| *s != 0.0));
    assert_eq!(
        bus_b_before, bus_b_after,
        "B's route does not read A's send"
    );
    // And the intervention was real: A's bus does change.
    let mut moved_a = mute_all_but(BUS_STRIP);
    let bus_a_before = render(&plan, &moved_a);
    moved_a.push(OfflineEvent::new(
        SampleTime::ZERO,
        CompiledPayload::SetParameter {
            slot: send_a,
            value: ParameterValue::ZERO,
        },
    ));
    assert_ne!(bus_a_before, render(&plan, &moved_a));
}
