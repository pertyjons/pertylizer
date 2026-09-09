//! `P08-S002`, `SOUND-INV-032`: the balance stage, the trim, V1's soft clipper and V1's
//! output clamp — the four stereo stages a whole project's mix passes through beside the
//! channel and the sum of `mix_channel`.
//!
//! Every kernel here is tested at two channels, which is the obligation ADR-0041 clause 12
//! attaches to a port that admits more than one (`SOUND-INV-015`). The oracles are V1's
//! own laws spelled term for term from V1's source — `apply_track_control` and `soft_clip`
//! in `synth_engine::instrument` are private, so unlike the channel's `Gain::from_pan` they
//! cannot be called — and every comparison is of bits, not of values within a tolerance.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::controller::BipolarLevel;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, Frequency, ParameterValue};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const STAGE: NodeId = NodeId::new(10);
const SECOND: NodeId = NodeId::new(11);
const THIRD: NodeId = NodeId::new(12);
const FOURTH: NodeId = NodeId::new(13);
const Q: u64 = QUANTUM_FRAMES as u64;
const QUANTA: u64 = 8;
const FRAMES: u64 = QUANTA * Q;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

fn constant(level: f32) -> IrNodeKind {
    IrNodeKind::Constant {
        level: Amplitude::new(level).expect("finite"),
    }
}

fn balance(level: f32, pan: f32, muted: bool) -> IrNodeKind {
    IrNodeKind::Balance {
        level: Amplitude::new(level).expect("finite"),
        pan: BipolarLevel::new(pan).expect("in range"),
        muted,
    }
}

fn trim(level: f32) -> IrNodeKind {
    IrNodeKind::Trim {
        level: Amplitude::new(level).expect("finite"),
    }
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

fn frames(interleaved: &[f32]) -> impl Iterator<Item = (f32, f32)> + '_ {
    interleaved.as_chunks::<2>().0.iter().map(|[l, r]| (*l, *r))
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|s| s.to_bits()).collect()
}

/// V1's `apply_track_control`, term for term: the left gain is `(1 − pan).sqrt() × volume`
/// and the right `(1 + pan).sqrt() × volume`, each then multiplied onto the sample.
fn v1_balance_sides(volume: f32, pan: f32) -> (f32, f32) {
    let left_gain = (1.0 - pan).sqrt() * volume;
    let right_gain = (1.0 + pan).sqrt() * volume;
    (left_gain, right_gain)
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

#[test]
fn a_balance_scales_each_side_by_the_level_and_v1s_balance_law_bit_for_bit() {
    // A triple on which the multiplication order is visible in the bits: V1 forms the
    // per-side gain first and multiplies the sample by it, and a kernel that multiplied the
    // sample by the coefficient before the level would round the other way here. Searched
    // rather than assumed, so the fixture cannot silently lose the property.
    let mut chosen = None;
    'search: for input in [0.3_f32, 0.7, 0.9, 0.55, 0.61] {
        for pan in [-0.3_f32, 0.37, 0.71, -0.83] {
            for level in [0.6_f32, 0.77, 0.43, 0.91] {
                let (left, right) = v1_balance_sides(level, pan);
                let other_left = (input * (1.0 - pan).sqrt()) * level;
                let other_right = (input * (1.0 + pan).sqrt()) * level;
                if (input * left).to_bits() != other_left.to_bits()
                    || (input * right).to_bits() != other_right.to_bits()
                {
                    chosen = Some((input, pan, level));
                    break 'search;
                }
            }
        }
    }
    let (input, pan, level) = chosen.expect("some triple distinguishes the two orders");
    let plan = admit(&staged(
        constant(input),
        &[(STAGE, balance(level, pan, false), ExecutionScope::Channel)],
    ));
    let rendered = render(&plan, &[]);
    let (left, right) = v1_balance_sides(level, pan);
    for (l, r) in frames(&rendered) {
        assert_eq!(
            l.to_bits(),
            (input * left).to_bits(),
            "left {l} is not {}",
            input * left
        );
        assert_eq!(
            r.to_bits(),
            (input * right).to_bits(),
            "right {r} is not {}",
            input * right
        );
    }
    assert_ne!(left, right, "the pan places the signal off centre");
}

#[test]
fn a_balance_at_unity_and_centre_is_neutral_where_the_channel_is_not() {
    let input = 0.5_f32;
    let neutral = render(
        &admit(&staged(
            constant(input),
            &[(STAGE, balance(1.0, 0.0, false), ExecutionScope::Channel)],
        )),
        &[],
    );
    for (l, r) in frames(&neutral) {
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            (input.to_bits(), input.to_bits())
        );
    }
    // The control: the mix channel's constant-power law attenuates the same signal at unity
    // and centre, which is the difference between the two laws and the reason for two kinds.
    let channelled = render(
        &admit(&staged(
            constant(input),
            &[(
                STAGE,
                IrNodeKind::Channel {
                    fader: Amplitude::UNITY,
                    pan: BipolarLevel::ZERO,
                    muted: false,
                },
                ExecutionScope::Channel,
            )],
        )),
        &[],
    );
    let (l, _) = frames(&channelled).next().expect("a frame");
    assert!(
        l < input,
        "the channel at unity and centre is {l}, not {input}"
    );
}

#[test]
fn a_balance_mute_silences_from_its_sample_writes_positive_zero_and_releases_from_its_own() {
    // A negative source: a kernel that muted by multiplying with zero would write −0.0,
    // where V1 fills a muted voice with 0.0. Bits, so the sign of zero is checked.
    let input = -0.5_f32;
    let plan = admit(&staged(
        constant(input),
        &[(STAGE, balance(1.0, 0.0, false), ExecutionScope::Channel)],
    ));
    let mute = plan
        .resolve_parameter(STAGE, parameters::BALANCE_MUTE)
        .expect("the mute is declared");
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
    for (index, (l, r)) in frames(&rendered).enumerate() {
        let frame = index as u64;
        let expected = if (on..off).contains(&frame) {
            0.0_f32
        } else {
            input
        };
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            (expected.to_bits(), expected.to_bits()),
            "frame {frame}: mute in force from {on} to {off}, got ({l}, {r})"
        );
    }
    // And a stage that starts muted renders positive zeros until told otherwise.
    let muted = render(
        &admit(&staged(
            constant(input),
            &[(STAGE, balance(1.0, 0.0, true), ExecutionScope::Channel)],
        )),
        &[],
    );
    assert!(muted.iter().all(|s| s.to_bits() == 0.0_f32.to_bits()));
}

#[test]
fn a_trim_scales_every_sample_by_its_level_and_a_write_moves_it_at_the_next_boundary() {
    let input = 0.5_f32;
    let plan = admit(&staged(
        constant(input),
        &[(STAGE, trim(0.25), ExecutionScope::Global)],
    ));
    let authored = render(&plan, &[]);
    for (l, r) in frames(&authored) {
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            ((input * 0.25).to_bits(), (input * 0.25).to_bits())
        );
    }
    // A write inside quantum 3 is quantum-rate: in force from the boundary that follows it,
    // and not before (`SOUND-INV-016`).
    let level = plan
        .resolve_parameter(STAGE, parameters::TRIM_LEVEL)
        .expect("the level is declared");
    let at = 3 * Q + 5;
    let written = render(
        &plan,
        &[OfflineEvent::new(
            SampleTime::new(at),
            CompiledPayload::SetParameter {
                slot: level,
                value: ParameterValue::new(0.75).expect("finite"),
            },
        )],
    );
    for (index, (l, r)) in frames(&written).enumerate() {
        let frame = index as u64;
        let expected = if frame < 4 * Q {
            input * 0.25
        } else {
            input * 0.75
        };
        assert_eq!(
            (l.to_bits(), r.to_bits()),
            (expected.to_bits(), expected.to_bits()),
            "frame {frame}: got ({l}, {r}), expected {expected}"
        );
    }
}

#[test]
fn a_soft_clip_passes_the_knee_unchanged_and_shapes_above_it_by_v1s_law_bit_for_bit() {
    for input in [
        0.0_f32, 0.5, -0.5, 0.8, -0.8, 0.8000001, 0.9, 1.0, 1.5, -1.5, 3.0, -7.25,
    ] {
        let rendered = render(
            &admit(&staged(
                constant(input),
                &[(STAGE, IrNodeKind::SoftClip, ExecutionScope::Global)],
            )),
            &[],
        );
        let expected = v1_soft_clip(input);
        for (l, r) in frames(&rendered) {
            assert_eq!(
                (l.to_bits(), r.to_bits()),
                (expected.to_bits(), expected.to_bits()),
                "{input} clipped to ({l}, {r}), V1 clips it to {expected}"
            );
        }
        if input.abs() <= 0.8 {
            assert_eq!(
                expected.to_bits(),
                input.to_bits(),
                "{input} is within the knee"
            );
        } else if input.abs() >= 0.9 {
            // Just past the knee the curve is within a bit of the identity; further out it
            // is visibly below the input and no more than full scale, with the input's sign.
            assert!(
                expected.abs() < input.abs() && expected.abs() <= 1.0,
                "{input} → {expected}"
            );
            assert_eq!(expected.is_sign_negative(), input.is_sign_negative());
        }
    }
}

#[test]
fn a_hard_clamp_holds_full_scale_and_passes_bits_within_it() {
    for (input, expected) in [
        (0.5_f32, 0.5_f32),
        (-0.25, -0.25),
        (1.0, 1.0),
        (-1.0, -1.0),
        (1.5, 1.0),
        (-1.5, -1.0),
        (40.0, 1.0),
    ] {
        let rendered = render(
            &admit(&staged(
                constant(input),
                &[(STAGE, IrNodeKind::HardClamp, ExecutionScope::Global)],
            )),
            &[],
        );
        for (l, r) in frames(&rendered) {
            assert_eq!(
                (l.to_bits(), r.to_bits()),
                (expected.to_bits(), expected.to_bits()),
                "{input} clamped to ({l}, {r})"
            );
        }
    }
}

/// A stereo stage in the voice scope is instantiated per voice and its instances are summed
/// **verbatim**: the voice sum's seed copies a stereo instance output as it is, where before
/// `P08-S002` it read the region as twice as many mono frames. The balance is V1's per-voice
/// track stage, so this is the placement the lowerer uses.
#[test]
fn a_stereo_stage_in_the_voice_scope_is_summed_per_voice_verbatim() {
    let input = 0.5_f32;
    let (level, pan) = (0.75_f32, 0.2_f32);
    let voiced = |voices: u32| {
        GraphIr::builder()
            .node(SOURCE, constant(input), ExecutionScope::Voice)
            .node(STAGE, balance(level, pan, false), ExecutionScope::Voice)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (STAGE, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (STAGE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .declaring(common::compiled_notes(voices))
            .build()
            .expect("a voiced stage is a readable plan")
    };
    let one = render(&admit(&voiced(1)), &[]);
    let two = render(&admit(&voiced(2)), &[]);
    let (left, right) = v1_balance_sides(level, pan);
    for (((l1, r1), (l2, r2)), frame) in frames(&one).zip(frames(&two)).zip(0..) {
        assert_eq!(
            (l1.to_bits(), r1.to_bits()),
            ((input * left).to_bits(), (input * right).to_bits()),
            "frame {frame}: one voice"
        );
        // Two identical instances: the seed copies the first and the accumulate adds the
        // second, so each side is exactly twice one voice and the sides stay distinct.
        assert_eq!(
            (l2.to_bits(), r2.to_bits()),
            ((l1 + l1).to_bits(), (r1 + r1).to_bits()),
            "frame {frame}: two voices"
        );
    }
    assert_ne!(left, right);
}

/// The four stages in V1's order after a channel — balance, then the channel's clipper, then
/// the master's trim and clamp — render the composition of the four laws over the source,
/// bit for bit, and the same bits under every host partition.
#[test]
fn a_staged_render_composes_v1s_laws_in_order_and_is_the_same_bits_under_every_partition() {
    let sine = IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.95).expect("finite"),
    };
    let (level, pan, master) = (1.0_f32, 0.2_f32, 1.5_f32);
    let plan = admit(&staged(
        sine,
        &[
            (STAGE, balance(level, pan, false), ExecutionScope::Channel),
            (SECOND, IrNodeKind::SoftClip, ExecutionScope::Global),
            (THIRD, trim(master), ExecutionScope::Global),
            (FOURTH, IrNodeKind::HardClamp, ExecutionScope::Global),
        ],
    ));
    let whole: Vec<usize> = vec![FRAMES as usize];
    let reference = render_partitioned(&plan, &whole);

    // The oracle: the source alone, widened by the output, through each law in order.
    let alone = render_partitioned(&admit(&staged(sine, &[])), &whole);
    let (left, right) = v1_balance_sides(level, pan);
    let mut shaped = 0_usize;
    for ((l, r), (sl, sr)) in frames(&reference).zip(frames(&alone)) {
        assert_eq!(
            sl.to_bits(),
            sr.to_bits(),
            "the widened source is the same on both sides"
        );
        let expect = |s: f32, gain: f32| (v1_soft_clip(s * gain) * master).clamp(-1.0, 1.0);
        assert_eq!(l.to_bits(), expect(sl, left).to_bits(), "left {l}");
        assert_eq!(r.to_bits(), expect(sr, right).to_bits(), "right {r}");
        if (sl * left).abs() > 0.8 && (sr * right).abs() > 0.8 {
            shaped += 1;
        }
    }
    assert!(
        shaped > 0,
        "the fixture drives the clipper past its knee on both sides"
    );
    assert!(reference.contains(&1.0), "and the clamp past full scale");

    let blocks_256: Vec<usize> = vec![256; (FRAMES / 256) as usize];
    let blocks_64: Vec<usize> = vec![64; (FRAMES / 64) as usize];
    let mut irregular = vec![37, 91, 5, 128, 1];
    let taken: usize = irregular.iter().sum();
    irregular.push(FRAMES as usize - taken);
    for partition in [&blocks_256[..], &blocks_64[..], &irregular[..]] {
        assert_eq!(
            bits(&render_partitioned(&plan, partition)),
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
