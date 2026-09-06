//! ADR-0041 clause 16's acceptance check: the same audio, before and after the arena
//! becomes interleaved.
//!
//! The conversion moves where samples live and changes no arithmetic, so every
//! difference it produces is a defect rather than a renegotiation. This file is the
//! instrument that says so: five fixtures, one digest per rendered quantum, compared
//! against baselines generated from the **planar** build in the commit before the
//! conversion. Generating them afterwards would be writing the answer down after seeing
//! it, which is why the baselines land first, in their own commit.
//!
//! Every value below was read off the planar build rather than reasoned out — the clause
//! is explicit that a fixture whose parameters are left to its implementer is a different
//! fixture in every hand.
//!
//! # Regenerating
//!
//! ```text
//! cargo test -p synth_engine_v2 --test layout_baseline -- --ignored regenerate
//! ```
//!
//! Only ever from the planar build, and only when this file's fixtures change. A
//! regeneration after the conversion would silently retire the check.
//!
//! # When it fails
//!
//! The failure names the fixture, the first differing quantum, both digests, and the path
//! of a dump holding that quantum's rendered samples. The other half of the comparison is
//! one command away, on the baseline commit:
//!
//! ```text
//! cargo test -p synth_engine_v2 --test layout_baseline -- --ignored dump_fixture_samples
//! ```
//!
//! which writes every quantum of every fixture in the same format, so the two `# quantum
//! N` blocks diff directly.
//!
//! # What the digest cannot see
//!
//! Storage. Three fixtures exist for an alias — an in-place merge, or a region a dead
//! value frees and a later chain takes — and every one of them renders the same samples
//! if the alias goes away. So each fixture also states the aliases its compiled plan must
//! have, checked before the render; see [`check_structure`].
//!
//! `sha2` is a **dev**-dependency. It does not widen the crate's own surface, which
//! `crate_boundary` reads from `[dependencies]`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use synth_engine_v2::compile::{RenderConfig, compile};

use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::AMPLIFIER_CONTROL;
use synth_engine_v2::plan::{CompiledPlan, PlanOp};
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, GainFactor, NormalizedLevel,
    ParameterValue, Resonance, SampleRate, Seconds,
};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, Renderer, TimedEvent, TimedEvents,
};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor, TimeSource};

/// The quantum, and the harness profile's maximum block, so one call is one quantum.
const QUANTUM: usize = 64;
/// Digested quanta. The 257th call is the first, and it renders none of them.
const QUANTA: usize = 256;
/// The quantum the gate falls in, which is baseline line 192.
const GATE_OFF_QUANTUM: usize = 192;

/// The node identities the fixtures use. All five end in `OUTPUT`.
const ENVELOPE: NodeId = NodeId::new(1);
const SINE: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const FIRST_GAIN: NodeId = NodeId::new(5);
const SECOND_GAIN: NodeId = NodeId::new(6);
const OUTPUT: NodeId = NodeId::new(9);
const CONSTANT: NodeId = NodeId::new(40);

fn profile(layout: ChannelLayout) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("48 kHz is a valid rate"),
        FrameCount::new(QUANTUM as u64),
        layout,
    )
    .expect("the harness profile is valid")
}

fn envelope() -> IrNodeKind {
    IrNodeKind::Envelope {
        attack: Seconds::new(0.010).expect("finite"),
        decay: Seconds::new(0.100).expect("finite"),
        sustain: NormalizedLevel::new(0.700).expect("in range"),
        release: Seconds::new(0.200).expect("finite"),
        velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
    }
}

fn sine() -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    }
}

fn filter() -> IrNodeKind {
    IrNodeKind::Filter {
        cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
        resonance: Resonance::BUTTERWORTH,
    }
}

fn gain(factor: f32) -> IrNodeKind {
    IrNodeKind::Gain {
        factor: GainFactor::new(factor).expect("finite"),
    }
}

/// The disconnected constant. Its value is written and never read, so its region is free
/// at the operation that writes it — which is what fixtures 4 and 5 are built on.
fn constant() -> IrNodeKind {
    IrNodeKind::Constant {
        level: Amplitude::new(0.5).expect("finite"),
    }
}

/// The minimal voice path: envelope into the amplifier's control, sine into filter into
/// amplifier into output.
/// The tuning the gated fixtures' scope resolves keys through.
fn twelve_tet() -> synth_engine_v2::tuning::PreparedTuning {
    synth_engine_v2::tuning::PreparedTuning::equal_temperament()
        .expect("twelve-tone equal temperament prepares")
}

fn voice_graph() -> GraphIr {
    GraphIr::builder()
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(SINE, sine(), ExecutionScope::Voice)
        .node(FILTER, filter(), ExecutionScope::Voice)
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
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        // The sine is a pitch destination in the played node's scope, so the scope has to
        // say what a key resolves to. Nothing here plays one — the baselines are unchanged
        // by it — but a plan that could not answer is refused at admission.
        .tuning(ExecutionScope::Voice, twelve_tet())
        .build()
        .expect("the minimal voice path is a readable plan")
}

/// The voice path plus a constant connected to nothing: one region assigned twice.
fn reuse_graph() -> GraphIr {
    GraphIr::builder()
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(SINE, sine(), ExecutionScope::Voice)
        .node(FILTER, filter(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(CONSTANT, constant(), ExecutionScope::Global)
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
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .tuning(ExecutionScope::Voice, twelve_tet())
        .build()
        .expect("a disconnected constant beside the voice path is a readable plan")
}

/// Sine into two gains into the output: ADR-0005 clause 5's in-place path, taken twice.
fn merged_graph() -> GraphIr {
    GraphIr::builder()
        .node(SINE, sine(), ExecutionScope::Voice)
        .node(FIRST_GAIN, gain(0.5), ExecutionScope::Global)
        .node(SECOND_GAIN, gain(0.25), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SINE, PortId::FIRST),
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
        .expect("a merged chain is a readable plan")
}

/// A filter whose input is unconnected, over the region a constant filled with 0.5.
///
/// The constant is what gives the fixture its power: the kernel must write silence over a
/// region holding something else, so a conversion that widens a region without widening
/// the write renders 0.5 where the baseline holds zeros.
fn unpatched_graph() -> GraphIr {
    GraphIr::builder()
        .node(CONSTANT, constant(), ExecutionScope::Global)
        .node(FILTER, filter(), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (FILTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("an unpatched filter beside a constant is a readable plan")
}

/// One fixture: what it is called, what it renders, and what its plan must look like.
///
/// The digest alone cannot see the thing three of these fixtures exist for. Fixture 5's
/// committed output is entirely zero, so an allocator that stopped handing the filter the
/// constant's storage would render zeros into a fresh region and pass; the merged and
/// reuse fixtures could likewise stop reusing storage without changing a sample. So each
/// fixture states the aliases its plan must still have, and they are checked **before**
/// the render — the gate is structural and behavioural, and a fixture that stops
/// exercising reuse fails rather than passing quietly.
struct Fixture {
    name: &'static str,
    graph: fn() -> GraphIr,
    layout: ChannelLayout,
    gated: bool,
    /// How many node operations the plan must schedule.
    node_ops: usize,
    /// Pairs of node operations that must write the **same** storage.
    ///
    /// Indices into the plan's node operations, in schedule order. Every pair here is one
    /// the layout conversion must preserve: an in-place merge, or a region freed by a
    /// dead value and handed to a later chain. Both sides are mono and therefore the same
    /// width on either build, which is why the expectation survives the conversion — the
    /// mono-to-stereo widening's target does not, since it needs a wider region than the
    /// one it took when every slot was `Q`.
    aliases: &'static [(usize, usize)],
    /// How many distinct storages the plan must use, where that is layout-independent.
    ///
    /// `None` for the stereo fixture: its widening occupies `c * Q` after the conversion
    /// and `Q` before it, so the count is a property of the layout rather than of the
    /// reuse this fixture protects.
    distinct_storages: Option<usize>,
}

const FIXTURES: [Fixture; 5] = [
    // Sine, filter, envelope, amplifier. The filter and the amplifier merge in place, so
    // the sine's storage carries the whole audio chain and the envelope holds the other.
    Fixture {
        name: "voice-mono",
        graph: voice_graph,
        layout: ChannelLayout::Mono,
        gated: true,
        node_ops: 4,
        aliases: &[(0, 1), (0, 3)],
        distinct_storages: Some(2),
    },
    // The same graph where the stream is stereo: the same two in-place merges, plus the
    // widening the output needs.
    Fixture {
        name: "voice-stereo",
        graph: voice_graph,
        layout: ChannelLayout::Stereo,
        gated: true,
        node_ops: 5,
        aliases: &[(0, 1), (0, 3)],
        distinct_storages: None,
    },
    // ADR-0005 clause 5's in-place path, taken twice: one storage for all three.
    Fixture {
        name: "merged-chain",
        graph: merged_graph,
        layout: ChannelLayout::Mono,
        gated: false,
        node_ops: 3,
        aliases: &[(0, 1), (0, 2)],
        distinct_storages: Some(1),
    },
    // The constant is written and never read, so its storage is free at the operation
    // that writes it and the sine chain is handed it. That alias is the fixture.
    Fixture {
        name: "reuse",
        graph: reuse_graph,
        layout: ChannelLayout::Mono,
        gated: true,
        node_ops: 5,
        aliases: &[(0, 1), (1, 2), (1, 4)],
        distinct_storages: Some(2),
    },
    // The filter writes silence over the region the constant filled with 0.5. Without the
    // alias the fixture proves nothing: a fresh region is already zero.
    Fixture {
        name: "unpatched-input",
        graph: unpatched_graph,
        layout: ChannelLayout::Mono,
        gated: false,
        node_ops: 2,
        aliases: &[(0, 1)],
        distinct_storages: Some(1),
    },
];

fn baseline_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("baselines")
}

fn baseline_path(fixture: &Fixture) -> PathBuf {
    baseline_dir().join(format!("layout-{}.txt", fixture.name))
}

/// Which storage a node operation writes, as an identity that survives the conversion.
///
/// Today a physical slot index; once the arena holds variable-width regions it is the
/// region's own identity. Either way, two operations that return the same value write the
/// same samples, which is the only property the fixtures assert.
fn output_storage(plan: &CompiledPlan, op: usize) -> usize {
    let mut nodes = plan.ops().iter().filter_map(|op| match op {
        PlanOp::Node(step) => Some(step.out().index()),
        PlanOp::Output { .. } => None,
    });
    nodes
        .nth(op)
        .unwrap_or_else(|| panic!("the plan has no node operation {op}"))
}

/// The aliases the fixture exists to protect, checked before a sample is rendered.
///
/// A digest cannot see storage. Fixture 5 renders zeros whether the filter writes silence
/// over the constant's region or into a fresh one, so without this the acceptance gate
/// would keep passing after the fixture stopped testing anything.
fn check_structure(fixture: &Fixture, plan: &CompiledPlan) {
    let node_ops = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, PlanOp::Node(_)))
        .count();
    assert_eq!(
        node_ops, fixture.node_ops,
        "fixture {} schedules {node_ops} node operations rather than {}",
        fixture.name, fixture.node_ops
    );

    for (left, right) in fixture.aliases {
        assert_eq!(
            output_storage(plan, *left),
            output_storage(plan, *right),
            "fixture {} needs node operations {left} and {right} to write one storage; without              that alias the fixture renders the same samples while testing nothing",
            fixture.name
        );
    }

    if let Some(expected) = fixture.distinct_storages {
        let mut storages: Vec<usize> = (0..node_ops).map(|op| output_storage(plan, op)).collect();
        storages.sort_unstable();
        storages.dedup();
        assert_eq!(
            storages.len(),
            expected,
            "fixture {} uses {} distinct storages rather than {expected}",
            fixture.name,
            storages.len()
        );
    }
}

/// SHA-256 over one call's block: every sample's `f32` bit pattern as four little-endian
/// bytes, in frame order with the frame's channels adjacent.
fn digest(block: &[f32]) -> String {
    let mut hasher = Sha256::new();
    for sample in block {
        hasher.update(sample.to_bits().to_le_bytes());
    }
    let mut rendered = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(rendered, "{byte:02x}").expect("writing to a String cannot fail");
    }
    rendered
}

/// Render one fixture and return `(quantum digest, quantum samples)` per digested call.
///
/// The **first** `render` call returns the carry `prepare` primed with `Q` frames of
/// silence, renders no quantum, and refuses any event presented with it. So the harness
/// makes 257 calls and digests calls 1 to 256 — index 0 is the primed one — and baseline
/// line `k` is the quantum call `k + 1` rendered.
fn render_fixture(fixture: &Fixture) -> Vec<(String, Vec<f32>)> {
    let ir = (fixture.graph)();
    let plan = compile(&ir, &RenderConfig::new(profile(fixture.layout)))
        .into_plan()
        .unwrap_or_else(|error| panic!("fixture {} must compile: {error:?}", fixture.name));
    check_structure(fixture, &plan);

    let gate = fixture.gated.then(|| {
        plan.resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
            .unwrap_or_else(|| panic!("fixture {} declares an envelope gate", fixture.name))
    });

    let (_control, mut renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap_or_else(|error| panic!("fixture {} must prepare: {error:?}", fixture.name));
    let epoch = renderer.epoch();
    let samples_per_call = QUANTUM * fixture.layout.channels();

    let mut digested = Vec::with_capacity(QUANTA);
    let mut block = vec![0.0_f32; samples_per_call];
    for call in 0..=QUANTA {
        block.iter_mut().for_each(|sample| *sample = 0.0);
        let events: Vec<TimedEvent> = match (gate, call) {
            (Some(slot), 1) => vec![TimedEvent::new(
                EventEnvelope::new(epoch, SampleTime::new(0), TimeSource::Compiled),
                EventPayload::SetParameter {
                    slot,
                    value: ParameterValue::new(1.0).expect("a raised gate is a valid value"),
                },
            )],
            (Some(slot), call) if call == GATE_OFF_QUANTUM + 1 => vec![TimedEvent::new(
                EventEnvelope::new(
                    epoch,
                    SampleTime::new((GATE_OFF_QUANTUM * QUANTUM) as u64),
                    TimeSource::Compiled,
                ),
                EventPayload::SetParameter {
                    slot,
                    value: ParameterValue::ZERO,
                },
            )],
            _ => Vec::new(),
        };
        let output = AudioBlockMut::new(&mut block, QUANTUM, fixture.layout)
            .expect("the block is one quantum of the fixture's layout");
        renderer
            .render(output, TimedEvents::new(&events))
            .unwrap_or_else(|error| {
                panic!("fixture {} failed on call {call}: {error:?}", fixture.name)
            });
        if call > 0 {
            digested.push((digest(&block), block.clone()));
        }
    }
    digested
}

fn baseline_text(rendered: &[(String, Vec<f32>)]) -> String {
    let mut text = String::with_capacity(rendered.len() * 70);
    for (quantum, (hash, _)) in rendered.iter().enumerate() {
        writeln!(text, "{quantum},{hash}").expect("writing to a String cannot fail");
    }
    text
}

#[test]
fn every_fixture_matches_its_committed_baseline() {
    for fixture in &FIXTURES {
        let path = baseline_path(fixture);
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "fixture {} has no baseline at {}: {error}. A missing baseline fails; it \
                 does not skip, because a check that skips is a check that passed",
                fixture.name,
                path.display()
            )
        });
        let rendered = render_fixture(fixture);
        assert_eq!(
            rendered.len(),
            QUANTA,
            "fixture {} digested {} quanta rather than {QUANTA}",
            fixture.name,
            rendered.len()
        );

        for (quantum, line) in expected.lines().enumerate() {
            let (index, hash) = line.split_once(',').unwrap_or_else(|| {
                panic!("baseline line {quantum} of {} is malformed", fixture.name)
            });
            assert_eq!(
                index.parse::<usize>().ok(),
                Some(quantum),
                "baseline line {quantum} of {} names quantum {index}",
                fixture.name
            );
            let Some((rendered_hash, samples)) = rendered.get(quantum) else {
                panic!(
                    "the baseline for {} has more lines than the render has quanta",
                    fixture.name
                );
            };
            if rendered_hash != hash {
                let dump = dump_quantum(fixture, quantum, samples);
                panic!(
                    "fixture {} differs first at quantum {quantum}: baseline {hash}, rendered \
                     {rendered_hash}. This build's samples for that quantum are in {}. For the \
                     other half: check out the baseline commit and run `cargo test -p \
                     synth_engine_v2 --test layout_baseline -- --ignored dump_fixture_samples`, \
                     which writes every quantum of every fixture in this same format, then diff \
                     the `# quantum {quantum}` block against this file",
                    fixture.name,
                    dump.display()
                );
            }
        }
        assert_eq!(
            expected.lines().count(),
            QUANTA,
            "the baseline for {} does not hold {QUANTA} lines",
            fixture.name
        );
    }
}

/// One quantum's samples, in the format both halves of the comparison are written in.
fn quantum_block(quantum: usize, samples: &[f32]) -> String {
    let mut text = String::with_capacity(samples.len() * 24 + 16);
    writeln!(text, "# quantum {quantum}").expect("writing to a String cannot fail");
    for (index, sample) in samples.iter().enumerate() {
        writeln!(text, "{index},{sample:?},{:#010x}", sample.to_bits())
            .expect("writing to a String cannot fail");
    }
    text
}

/// Write one quantum's rendered samples where the failure message can name them.
fn dump_quantum(fixture: &Fixture, quantum: usize, samples: &[f32]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "synth_engine_v2-layout-{}-quantum-{quantum}.txt",
        fixture.name
    ));
    match std::fs::write(&path, quantum_block(quantum, samples)) {
        Ok(()) => path,
        Err(error) => panic!(
            "cannot write the failing quantum to {}: {error}",
            path.display()
        ),
    }
}

/// The other half of the comparison: every fixture's every quantum, from *this* build.
///
/// Ignored, because it exists to be run deliberately on the baseline commit — a failing
/// comparison names one quantum in this build, and the baseline's samples for it are not
/// otherwise recoverable from a digest. Without this the failure message would advertise
/// a workflow nobody can follow.
#[test]
#[ignore = "writes this build's samples for every fixture and quantum; run to compare against a failure"]
fn dump_fixture_samples() {
    for fixture in &FIXTURES {
        let rendered = render_fixture(fixture);
        let mut text = String::new();
        for (quantum, (_, samples)) in rendered.iter().enumerate() {
            text.push_str(&quantum_block(quantum, samples));
        }
        let path = std::env::temp_dir().join(format!(
            "synth_engine_v2-layout-{}-all-quanta.txt",
            fixture.name
        ));
        std::fs::write(&path, text)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        println!("wrote {} quanta to {}", rendered.len(), path.display());
    }
}

/// Regenerate the baselines. Ignored, and only ever run on the planar build.
#[test]
#[ignore = "writes the committed baselines; run deliberately, only from the planar build"]
fn regenerate_baselines() {
    std::fs::create_dir_all(baseline_dir()).expect("the baseline directory is writable");
    for fixture in &FIXTURES {
        let rendered = render_fixture(fixture);
        let path = baseline_path(fixture);
        std::fs::write(&path, baseline_text(&rendered))
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        println!("wrote {} lines to {}", rendered.len(), path.display());
    }
}
