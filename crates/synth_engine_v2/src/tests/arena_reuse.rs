//! ADR-0005 clause 8's two checks, which are in-crate because one of them needs the
//! arena policy that clause 9 keeps crate-private.
//!
//! The structural check reads the assignment and fails when two overlapping lives share
//! a slot. The behavioural one compiles the same plan with reuse *disabled* and requires
//! bit-identical audio — which is what makes a later change of strategy safe, because
//! the same test decides it.

use crate::arena::{ArenaPolicy, assign, overlapping_chains};
use crate::compile::{RenderConfig, compile, compile_with};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::offline::render_offline;
use crate::plan::PlanOp;
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, GainFactor, Resonance, SampleRate,
};
use crate::time::{FrameCount, PlanPosition};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);

fn profile(layout: ChannelLayout) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(256),
        layout,
    )
    .expect("the harness profile is valid")
}

fn gain(value: f32) -> IrNodeKind {
    IrNodeKind::Gain {
        factor: GainFactor::new(value).expect("finite"),
    }
}

/// A source through `links` gains into the output.
fn chain(links: u32, layout: ChannelLayout) -> (GraphIr, HostProfile) {
    let mut builder = GraphIr::builder().node(
        SOURCE,
        IrNodeKind::Sine {
            frequency: Frequency::new(440.0).expect("finite"),
            amplitude: Amplitude::new(0.5).expect("finite"),
        },
        ExecutionScope::Voice,
    );
    let mut previous = SOURCE;
    for step in 0..links {
        let id = NodeId::new(10 + step);
        builder = builder.node(id, gain(0.5), ExecutionScope::Global).connect(
            (previous, PortId::FIRST),
            (id, PortId::FIRST),
            SignalDomain::Audio,
        );
        previous = id;
    }
    let ir = builder
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (previous, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");
    (ir, profile(layout))
}

#[test]
fn no_two_overlapping_lives_share_a_slot() {
    // The check itself lives in `assign`, under `debug_assertions`, so **every** plan
    // any test compiles is checked rather than the shapes someone thought to list. This
    // test is the explicit half: it re-derives the assignment the compiler made and
    // reads it directly, over the shapes a linear chain does not cover.
    for (name, ir, host) in shapes() {
        for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
            let host = if layout == ChannelLayout::Stereo {
                profile(ChannelLayout::Stereo)
            } else {
                host
            };
            let plan = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::Reuse)
                .into_plan()
                .expect("admissible");
            let virtual_ops = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::NoReuse)
                .into_plan()
                .expect("admissible");

            // The **reuse** assignment, over the *virtual* operations. Compiling with
            // `NoReuse` produces exactly those, because its mapping is the identity —
            // so running `assign` over them with `Reuse` reproduces the assignment the
            // compiler made, which is the thing clause 8 asks to be checked. Running it
            // over already-rewritten operations would check a fresh identity mapping
            // against itself and could never fail.
            // Every region in the `NoReuse` plan is one signal's own, so its widths are
            // the widths the reuse assignment is over.
            let widths: Vec<usize> = virtual_ops
                .regions()
                .iter()
                .map(|region| region.length())
                .collect();
            let assignment = assign(virtual_ops.ops(), &widths, ArenaPolicy::Reuse, &[]);
            assert_eq!(
                assignment.regions.len(),
                plan.buffer_count(),
                "the re-derived assignment must be the one the compiler used for {name}"
            );
            assert_eq!(
                assignment
                    .regions
                    .iter()
                    .map(|region| region.end())
                    .max()
                    .unwrap_or(0),
                plan.arena_samples(),
                "and it must reach the same extent for {name}"
            );
            assert!(
                overlapping_chains(&assignment).is_none(),
                "the {name} plan in {layout} gave two live values regions that intersect"
            );
        }
    }
}

/// The plan shapes the arena has to get right, beyond a straight chain.
fn shapes() -> Vec<(&'static str, GraphIr, HostProfile)> {
    const LOOSE: NodeId = NodeId::new(40);
    const FIRST: NodeId = NodeId::new(41);
    const SECOND: NodeId = NodeId::new(42);

    let mut shapes: Vec<(&'static str, GraphIr, HostProfile)> = (0..6)
        .map(|links| {
            let (ir, host) = chain(links, ChannelLayout::Mono);
            ("chain", ir, host)
        })
        .collect();

    // Fan-out: one value read by two consumers, so its life outlives the first of them
    // and in-place has to be declined.
    shapes.push((
        "fan-out",
        GraphIr::builder()
            .node(
                SOURCE,
                IrNodeKind::Sine {
                    frequency: Frequency::new(440.0).expect("finite"),
                    amplitude: Amplitude::new(0.5).expect("finite"),
                },
                ExecutionScope::Voice,
            )
            .node(FIRST, gain(0.5), ExecutionScope::Global)
            .node(SECOND, gain(0.25), ExecutionScope::Global)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (FIRST, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (SOURCE, PortId::FIRST),
                (SECOND, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (FIRST, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .expect("readable plan"),
        profile(ChannelLayout::Mono),
    ));

    // A disconnected branch beside a live one, which is where a freed slot is handed to
    // the next value — the case an unpatched input would otherwise inherit.
    shapes.push((
        "disconnected branch",
        GraphIr::builder()
            .node(
                LOOSE,
                IrNodeKind::Constant {
                    level: Amplitude::new(0.5).expect("finite"),
                },
                ExecutionScope::Global,
            )
            .node(
                SOURCE,
                IrNodeKind::Sine {
                    frequency: Frequency::new(440.0).expect("finite"),
                    amplitude: Amplitude::new(0.5).expect("finite"),
                },
                ExecutionScope::Voice,
            )
            .node(FIRST, gain(0.5), ExecutionScope::Global)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (FIRST, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (FIRST, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .expect("readable plan"),
        profile(ChannelLayout::Mono),
    ));

    // An unpatched input, whose silence has to be written rather than assumed.
    shapes.push((
        "unpatched input",
        GraphIr::builder()
            .node(FIRST, gain(1.0), ExecutionScope::Global)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (FIRST, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .expect("readable plan"),
        profile(ChannelLayout::Mono),
    ));

    // A stateful node merged in place. A filter keeps its history in its state rather
    // than in the buffer, so reuse must not change a sample of it — and a filter is the
    // node where an aliasing mistake would be least audible, because a wrong history
    // still produces a plausible sweep.
    shapes.push((
        "filter in place",
        GraphIr::builder()
            .node(
                SOURCE,
                IrNodeKind::Sine {
                    frequency: Frequency::new(440.0).expect("finite"),
                    amplitude: Amplitude::new(0.5).expect("finite"),
                },
                ExecutionScope::Voice,
            )
            .node(
                FIRST,
                IrNodeKind::Filter {
                    cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
                    resonance: Resonance::BUTTERWORTH,
                },
                ExecutionScope::Voice,
            )
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (FIRST, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (FIRST, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .expect("readable plan"),
        profile(ChannelLayout::Mono),
    ));

    shapes.push(("empty", GraphIr::empty(), profile(ChannelLayout::Mono)));
    shapes
}

#[test]
fn reuse_renders_bit_identically_to_no_reuse() {
    // Clause 8's behavioural half, and the reason a later strategy change is safe: the
    // same test decides it. Over every shape rather than the chains alone — a chain
    // merges into one in-place value and never recycles a freed slot, so it would not
    // exercise the free list at all, and a liveness error in the shapes that do would
    // agree with the structural check and pass both.
    for (name, ir, host) in shapes() {
        for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
            let host = if layout == ChannelLayout::Stereo {
                profile(ChannelLayout::Stereo)
            } else {
                host
            };
            let render = |policy| {
                let plan = compile_with(&ir, &RenderConfig::new(host), policy)
                    .into_plan()
                    .expect("admissible");
                render_offline(plan, FrameCount::new(256), PlanPosition::ZERO, &[])
                    .expect("renders")
            };

            assert_eq!(
                render(ArenaPolicy::Reuse),
                render(ArenaPolicy::NoReuse),
                "reuse changed the audio of the {name} plan in {layout}"
            );
        }
    }
}

#[test]
fn a_chain_of_gains_reuses_one_slot_rather_than_growing_the_arena() {
    // Every gain's input dies at the gain, so the whole chain is one value chain and one
    // buffer — which is the difference between an arena that grows with the size of a
    // plan and one that grows with its depth.
    let (ir, host) = chain(4, ChannelLayout::Mono);
    let reused = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::Reuse)
        .into_plan()
        .expect("admissible");
    let separate = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::NoReuse)
        .into_plan()
        .expect("admissible");

    assert_eq!(separate.buffer_count(), 5, "a sine and four gains");
    assert_eq!(
        reused.buffer_count(),
        1,
        "all five are one value passing through, so one buffer holds it"
    );
}

#[test]
fn in_place_is_declined_where_the_input_is_read_again() {
    // Clause 5's second condition. The source feeds both a gain and a second gain, so
    // its live range does not end at the first one — writing over it there would corrupt
    // what the second still has to read.
    const FIRST: NodeId = NodeId::new(10);
    const SECOND: NodeId = NodeId::new(11);
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(FIRST, gain(0.5), ExecutionScope::Global)
        .node(SECOND, gain(0.25), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FIRST, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (SECOND, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FIRST, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    let plan = compile_with(
        &ir,
        &RenderConfig::new(profile(ChannelLayout::Mono)),
        ArenaPolicy::Reuse,
    )
    .into_plan()
    .expect("admissible");

    let first_gain = plan
        .ops()
        .iter()
        .find_map(|op| match op {
            PlanOp::Node(step) => step
                .inputs()
                .first()
                .copied()
                .flatten()
                .map(|source| (step.out().index(), source.index())),
            PlanOp::Output { .. } => None,
        })
        .expect("the plan has a gain");
    assert_ne!(
        first_gain.0, first_gain.1,
        "the source is read again by the second gain, so the first may not write over it"
    );
}

#[test]
fn the_assignment_is_the_same_every_time_it_is_computed() {
    // Clause 3. A digest comparison across two compilations has to be measuring audio
    // rather than allocation order.
    let (ir, host) = chain(3, ChannelLayout::Stereo);
    let once = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::Reuse)
        .into_plan()
        .expect("admissible");
    let twice = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::Reuse)
        .into_plan()
        .expect("admissible");

    assert_eq!(once.ops(), twice.ops());
    assert_eq!(once.buffer_count(), twice.buffer_count());
}

#[test]
fn the_structural_check_can_fail() {
    // The control. An assertion that never fires reads exactly like one that always
    // passes, and this one guards a defect that produces plausible audio rather than a
    // crash — so it has to be shown catching the thing it is looking for.
    let quantum = crate::time::QUANTUM_FRAMES as usize;
    let whole = crate::plan::BufferRegion::raw(0, quantum);
    let overlapping = crate::arena::Assignment {
        regions: vec![whole],
        mapping: vec![
            crate::plan::BufferSlot::new(0),
            crate::plan::BufferSlot::new(0),
        ],
        chains: vec![
            crate::arena::Chain {
                first: 0,
                last: 5,
                slot: crate::plan::BufferSlot::new(0),
                region: whole,
                members: vec![0],
            },
            crate::arena::Chain {
                first: 3,
                last: 7,
                slot: crate::plan::BufferSlot::new(0),
                region: whole,
                members: vec![1],
            },
        ],
    };
    assert!(
        overlapping_chains(&overlapping).is_some(),
        "two lives sharing one region while both are live is what this check exists for"
    );

    // ADR-0041 clause 14: identity is no longer the question. Two **distinct** regions
    // that overlap in samples are the defect variable widths make possible, and equal
    // slots made unrepresentable — so the check has to catch a partial overlap too.
    let low = crate::plan::BufferRegion::raw(0, quantum * 2);
    let high = crate::plan::BufferRegion::raw(quantum, quantum * 2);
    let partial = crate::arena::Assignment {
        regions: vec![low, high],
        mapping: vec![
            crate::plan::BufferSlot::new(0),
            crate::plan::BufferSlot::new(1),
        ],
        chains: vec![
            crate::arena::Chain {
                first: 0,
                last: 5,
                slot: crate::plan::BufferSlot::new(0),
                region: low,
                members: vec![0],
            },
            crate::arena::Chain {
                first: 3,
                last: 7,
                slot: crate::plan::BufferSlot::new(1),
                region: high,
                members: vec![1],
            },
        ],
    };
    assert!(
        overlapping_chains(&partial).is_some(),
        "two live values in different slots whose samples overlap is the defect mixed \
         widths introduce"
    );
}

#[test]
fn an_unpatched_input_reads_silence_and_not_the_arena_s_last_tenant() {
    // With reuse, "the buffer it would otherwise read" is storage some earlier value
    // left behind. A disconnected constant scheduled before the gain is the case: its
    // slot is free by the time the gain runs, so the gain would inherit 0.5 and the plan
    // would render a signal nothing is patched into.
    const LOOSE: NodeId = NodeId::new(30);
    const GAIN: NodeId = NodeId::new(31);
    let ir = GraphIr::builder()
        .node(
            LOOSE,
            IrNodeKind::Constant {
                level: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Global,
        )
        .node(GAIN, gain(1.0), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    let plan = compile_with(
        &ir,
        &RenderConfig::new(profile(ChannelLayout::Mono)),
        ArenaPolicy::Reuse,
    )
    .into_plan()
    .expect("admissible");
    let rendered =
        render_offline(plan, FrameCount::new(128), PlanPosition::ZERO, &[]).expect("renders");

    assert!(
        rendered.iter().all(|sample| *sample == 0.0),
        "an unpatched input must read silence, whatever the arena last held there"
    );
}

#[test]
fn a_report_says_when_its_arena_row_is_an_upper_bound() {
    // A plan refused before lowering carries a scratch row computed as one buffer per
    // signal, which a chain of gains collapses to one. The number is not wrong for what
    // it is — an upper bound — but a reader has no way to tell unless the report says so.
    let (ir, host) = chain(4, ChannelLayout::Mono);

    let admitted = compile_with(&ir, &RenderConfig::new(host), ArenaPolicy::Reuse);
    assert!(
        !admitted.report().arena_is_estimated(),
        "an admitted plan's report describes the arena it was assigned"
    );
    assert!(admitted.plan().is_ok());

    // Refused on a field that has nothing to do with the arena: a plan declaring more
    // voices than any profile admits.
    // `P06-S001`: voices are derived from the producers' identity ranges, and a plan requests
    // them only where it has a voice-scope node to instantiate — so this one has both. The
    // preflight refuses it before lowering, which is what keeps a four-billion-instance
    // lowering from ever running.
    let greedy = GraphIr::builder()
        .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(crate::ir::PlanDeclarations {
            note_producers: vec![crate::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: crate::quantities::HeldNoteCount::measured(u32::MAX),
                simultaneous_holds: crate::quantities::EventCount::NONE,
            }],
            ..crate::ir::PlanDeclarations::default()
        })
        .build()
        .expect("a source into an output is a readable plan");

    let refused = compile_with(&greedy, &RenderConfig::new(host), ArenaPolicy::Reuse);
    assert!(
        refused.plan().is_err(),
        "no profile admits four billion voices"
    );
    assert!(
        refused.report().arena_is_estimated(),
        "a report produced before lowering must say that its arena row is a bound"
    );
}

/// ADR-0005 clause 6, now with a reader to extend a live range through.
///
/// The clause makes an observation tap a **reader**: a signal whose only remaining reader
/// is a tap is still live, and its region may not be handed to a later chain. Until
/// `P05-S008` the IR had no way to express one, and this test asserted the premise — a
/// declared tap changed neither schedule nor assignment. A monitor's declared tap
/// (`SOUND-INV-022`) is the reader now, and the case the premise stood in for is written:
/// the tapped region is not written by any later operation, and the control shows the
/// arena *would* have reused it without the pin.
#[test]
fn a_tapped_signal_stays_live_to_the_end_of_the_quantum() {
    use crate::report::ResourceField;

    const MONITOR: NodeId = NodeId::new(3);
    const FIRST_GAIN: NodeId = NodeId::new(4);
    const SECOND_GAIN: NodeId = NodeId::new(5);
    // Source, monitor, then two in-place gains: each gain would take over its input's
    // region, so without the pin the monitor's output — the tapped signal — is the region
    // the gains write.
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(MONITOR, IrNodeKind::Monitor, ExecutionScope::Voice)
        .node(FIRST_GAIN, gain(0.5), ExecutionScope::Voice)
        .node(SECOND_GAIN, gain(0.5), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (MONITOR, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MONITOR, PortId::FIRST),
            (FIRST_GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FIRST_GAIN, PortId::FIRST),
            (SECOND_GAIN, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_GAIN, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a monitored chain is a readable plan");
    let outcome = compile(&ir, &RenderConfig::new(profile(ChannelLayout::Mono)));
    assert_eq!(
        outcome
            .report()
            .row(ResourceField::MaxObservationTaps)
            .map(|row| row.requested()),
        Some(crate::report::ResourceAmount::Taps(
            crate::quantities::TapCount::measured(1)
        )),
        "the declared tap reaches the resource report"
    );
    let plan = outcome.into_plan().expect("admissible");
    let tap = plan.taps()[0];

    // No operation after the monitor's writes the tapped region.
    let monitor_at = plan
        .ops()
        .iter()
        .position(|op| matches!(op, PlanOp::Node(step) if step.node() == tap.node))
        .expect("the monitor is scheduled");
    for op in &plan.ops()[monitor_at + 1..] {
        if let PlanOp::Node(step) = op {
            assert_ne!(
                step.out(),
                tap.region,
                "a later operation writes the tapped region, so a subscriber would read \
                 the gain's output rather than the monitor's"
            );
        }
    }

    // The control: over the same virtual operations, an assignment that pins nothing
    // hands the monitor's region to a later gain — so the pin is what keeps it live.
    let virtual_ops = compile_with(
        &ir,
        &RenderConfig::new(profile(ChannelLayout::Mono)),
        ArenaPolicy::NoReuse,
    )
    .into_plan()
    .expect("admissible");
    let widths: Vec<usize> = virtual_ops
        .regions()
        .iter()
        .map(|region| region.length())
        .collect();
    let monitor_virtual = virtual_ops
        .ops()
        .iter()
        .find_map(|op| match op {
            PlanOp::Node(step) if step.node() == tap.node => Some(step.out()),
            _ => None,
        })
        .expect("the monitor is scheduled");
    let unpinned = assign(virtual_ops.ops(), &widths, ArenaPolicy::Reuse, &[]);
    let monitor_physical = unpinned.mapping[monitor_virtual.index()];
    let reused_later = virtual_ops.ops()[monitor_at + 1..]
        .iter()
        .any(|op| match op {
            PlanOp::Node(step) => unpinned.mapping[step.out().index()] == monitor_physical,
            PlanOp::Output { .. } => false,
        });
    assert!(
        reused_later,
        "without the pin nothing would have reused the monitor's region, so this test \
         cannot tell a pin from an accident of the schedule"
    );
}
