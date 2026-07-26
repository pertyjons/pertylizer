//! Tests for `tests`.

use super::*;
use crate::note_processor::{
    Humanize, MAX_EXPANSION_EVENTS_PER_TICK, NoteProcessor, lookback_pool,
};

fn proc_node() -> NoteModuleConfig {
    NoteModuleConfig::Processor(NoteProcessor::Humanize(Humanize::default()))
}

fn id(n: u32) -> NoteModuleId {
    NoteModuleId::new(n)
}

/// Build a graph with `n` processor nodes (ids `1..=n`), unconnected.
fn graph_with_nodes(n: u32) -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "test");
    for i in 1..=n {
        g.try_insert_node(id(i), proc_node()).expect("under cap");
    }
    g.rebuild_derived().expect("valid");
    g
}

/// Every node config deserializes from an empty externally-tagged object
/// `{"<Kind>":{}}`, falling back to its `Default` — so the MCP
/// `add_note_graph_module` accepts a partial/empty config just like the GUI's
/// `::default()`. Regression for the missing `#[serde(default)]` on the seven
/// config structs (they had a `Default` impl but serde still required every
/// field, so `{}` failed with `missing field …`).
#[test]
fn node_configs_deserialize_from_empty_object() {
    let cases: &[(&str, NoteModuleConfig)] = &[
        (
            r#"{"Euclidean":{}}"#,
            NoteModuleConfig::Euclidean(EuclideanGenerator::default()),
        ),
        (
            r#"{"ProbabilityGate":{}}"#,
            NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
        ),
        (
            r#"{"NoteLfo":{}}"#,
            NoteModuleConfig::NoteLfo(NoteLfo::default()),
        ),
        (
            r#"{"StepLfo":{}}"#,
            NoteModuleConfig::StepLfo(StepLfo::default()),
        ),
        (
            r#"{"NoteEnvelope":{}}"#,
            NoteModuleConfig::NoteEnvelope(NoteEnvelope::default()),
        ),
        (
            r#"{"NoteDelay":{}}"#,
            NoteModuleConfig::NoteDelay(NoteDelay::default()),
        ),
        (
            r#"{"Ratchet":{}}"#,
            NoteModuleConfig::Ratchet(Ratchet::default()),
        ),
    ];
    for (json, expected) in cases {
        let parsed: NoteModuleConfig = serde_json::from_str(json)
            .unwrap_or_else(|e| panic!("`{json}` should deserialize to its default: {e}"));
        assert_eq!(
            &parsed, expected,
            "`{json}` should equal its default config"
        );
    }
}

/// Freeze's bake reports the events it had to drop: 130 notes all onsetting
/// at tick 0 overflow the 128-event expansion buffer, so `bake_counted`
/// returns a nonzero drop count (surfaced by freeze as a UI/MCP warning,
/// plan §7). Regression for the drop-reporting seam.
#[test]
fn bake_counted_reports_dropped_events_on_overflow() {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "overflow");
    g.try_insert_node(id(1), proc_node()).expect("under cap");
    g.rebuild_derived().expect("valid");
    let src: Vec<Note> = (0..130)
        .map(|n| Note::new(NoteId(n + 1), PatternTick(0), pitch(60), Velocity::MF))
        .collect();
    let (baked, dropped) = g.bake_counted(
        &src,
        PatternTick(1),
        HostKey::from(PatternId::new(1)),
        synth_core::Bpm::DEFAULT,
        None,
    );
    assert!(
        dropped >= 2,
        "expected the 130 - 128 overflow to be counted, got {dropped}"
    );
    assert_eq!(
        baked.len(),
        MAX_EXPANSION_EVENTS_PER_TICK,
        "the buffer caps the baked notes at the 128-event limit"
    );
}

#[test]
fn arpeggiator_node_arpeggiates_in_graph_not_passthrough() {
    use crate::note_processor::Arpeggiator;
    // A held C-major triad over one beat (960 ticks).
    let held = |n, p| {
        Note::new(NoteId(n), PatternTick(0), pitch(p), Velocity::MF).with_duration(Duration(960))
    };
    let src = vec![held(1, 60), held(2, 64), held(3, 67)];
    let mut g = NoteGraph::new(NoteGraphId::new(0), "arp");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::Arpeggiator(Arpeggiator::default())),
    )
    .expect("under cap");
    g.rebuild_derived().expect("valid");
    let host = HostKey::from(PatternId::new(1));
    // At tick 0, pass-through would emit the whole 3-note chord; the arp
    // replaces the stream with one step — its lowest tone (default Up mode).
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        &src,
        PatternTick(0),
        host,
        Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1, "arp emits one step, not the chord");
    assert_eq!(
        buf.notes()[0].pitch,
        pitch(60),
        "Up mode starts on the lowest tone"
    );
    // Over the beat it steps repeatedly — not a single pass-through onset.
    let baked = g.bake(&src, PatternTick(960), host, Bpm::DEFAULT, None);
    assert!(
        baked.len() > 1,
        "arp must step over the held chord, got {}",
        baked.len()
    );
}

#[test]
fn strummed_chord_node_strums_in_graph_not_passthrough() {
    use crate::note_processor::{Chord, StrumDirection};
    // One held source note; a strummed triad staggers its tones over ticks.
    let src = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MF).with_duration(Duration(960)),
    ];
    let chord = Chord::major().with_strum(Duration(30), StrumDirection::Up);
    let mut g = NoteGraph::new(NoteGraphId::new(0), "strum");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::Chord(chord)),
    )
    .expect("under cap");
    g.rebuild_derived().expect("valid");
    let host = HostKey::from(PatternId::new(1));
    let baked = g.bake(&src, PatternTick(960), host, Bpm::DEFAULT, None);
    // Three tones, each on its own strum tick (0, 30, 60) — not a block chord
    // at tick 0 (which pass-through would leave, un-staggered).
    let ticks: Vec<u32> = baked.iter().map(|(t, _)| t.0).collect();
    assert!(
        ticks.contains(&0) && ticks.contains(&30) && ticks.contains(&60),
        "strum tones must stagger across ticks, got {ticks:?}"
    );
}

#[test]
fn note_script_transform_rewrites_clamps_drops_and_passes_through() {
    use synth_script::{CompileOptions, compile};

    let compile_note = |src: &str| -> Arc<BoundScript> {
        let opts = CompileOptions {
            note_event: true,
            ..Default::default()
        };
        let (program, diags) = compile(src, &opts);
        let program = program.unwrap_or_else(|| panic!("compile failed: {diags:?}"));
        Arc::new(program.into_bound(src.to_owned()))
    };
    let run = |node: NoteScriptTransform| -> ExpansionBuffer {
        let mut g = NoteGraph::new(NoteGraphId::new(0), "s");
        g.try_insert_node(id(1), NoteModuleConfig::NoteScriptTransform(node))
            .expect("under cap");
        g.rebuild_derived().expect("valid");
        let src = vec![Note::new(
            NoteId(1),
            PatternTick(0),
            pitch(60),
            Velocity::new(0.8),
        )];
        let mut buf = ExpansionBuffer::new();
        g.expand_at_tick(
            &src,
            PatternTick(0),
            HostKey::from(PatternId::new(1)),
            Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        buf
    };

    // Transpose up an octave, halve velocity; dur/gate pass through.
    let mut xform = NoteScriptTransform::new("");
    xform.set_compiled(Some(compile_note(
        "out.pitch = note_pitch + 12\nout.vel = note_vel * 0.5",
    )));
    let buf = run(xform);
    assert_eq!(buf.notes().len(), 1);
    assert_eq!(buf.notes()[0].pitch, pitch(72));
    assert!((buf.notes()[0].velocity.as_f32() - 0.4).abs() < 1e-4);

    // Negative out.vel is the drop sentinel (checked before clamping).
    let mut dropper = NoteScriptTransform::new("");
    dropper.set_compiled(Some(compile_note("out.vel = 0 - 1")));
    assert_eq!(run(dropper).notes().len(), 0, "negative out.vel drops");

    // out.pitch clamps into MIDI range (200 → 127).
    let mut clamp = NoteScriptTransform::new("");
    clamp.set_compiled(Some(compile_note("out.pitch = 200")));
    assert_eq!(run(clamp).notes()[0].pitch, pitch(127));

    // An uncompiled node is pass-through (source retained but not run).
    let buf = run(NoteScriptTransform::new("out.pitch = note_pitch + 12"));
    assert_eq!(buf.notes().len(), 1);
    assert_eq!(buf.notes()[0].pitch, pitch(60), "uncompiled = pass-through");
}

#[test]
fn note_script_prng_varies_per_note() {
    use synth_script::{CompileOptions, compile};
    let opts = CompileOptions {
        note_event: true,
        ..Default::default()
    };
    let src = "out.vel = rand()";
    let (program, diags) = compile(src, &opts);
    let program = program.unwrap_or_else(|| panic!("compile: {diags:?}"));
    let mut node = NoteScriptTransform::new(src);
    node.set_compiled(Some(Arc::new(program.into_bound(src.to_owned()))));
    let mut g = NoteGraph::new(NoteGraphId::new(0), "r");
    g.try_insert_node(id(1), NoteModuleConfig::NoteScriptTransform(node))
        .expect("under cap");
    g.rebuild_derived().expect("valid");
    // Three notes at the same tick — the old code seeded every event identically,
    // so rand() returned one constant; the per-event seed must decorrelate them.
    let src_notes = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MF),
        Note::new(NoteId(2), PatternTick(0), pitch(64), Velocity::MF),
        Note::new(NoteId(3), PatternTick(0), pitch(67), Velocity::MF),
    ];
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        &src_notes,
        PatternTick(0),
        HostKey::from(PatternId::new(1)),
        Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 3);
    let vels: Vec<f32> = buf.notes().iter().map(|n| n.velocity.as_f32()).collect();
    assert!(
        !(vels[0] == vels[1] && vels[1] == vels[2]),
        "rand() must vary per note, got {vels:?}"
    );
}

#[test]
fn source_context_ignores_upstream_detection() {
    use crate::note_processor::{Arpeggiator, ScaleQuantize};
    // ScaleQuantize → Arp: a pitch transform the arp consumes — not ignored.
    let mut ok = NoteGraph::new(NoteGraphId::new(0), "ok");
    ok.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize::default())),
    )
    .expect("under cap");
    ok.try_insert_node(
        id(2),
        NoteModuleConfig::Processor(NoteProcessor::Arpeggiator(Arpeggiator::default())),
    )
    .expect("under cap");
    ok.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("linear");
    ok.rebuild_derived().expect("valid");
    assert!(!ok.source_context_ignores_upstream(id(2)));
    assert!(
        !ok.source_context_ignores_upstream(id(1)),
        "quantize isn't source-context"
    );

    // Euclidean generator → Arp: the generator is ignored by the arp.
    let mut bad = NoteGraph::new(NoteGraphId::new(0), "bad");
    bad.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator::default()),
    )
    .expect("under cap");
    bad.try_insert_node(
        id(2),
        NoteModuleConfig::Processor(NoteProcessor::Arpeggiator(Arpeggiator::default())),
    )
    .expect("under cap");
    bad.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("linear");
    bad.rebuild_derived().expect("valid");
    assert!(bad.source_context_ignores_upstream(id(2)));
}

#[test]
fn expand_at_tick_tapped_stops_after_the_node() {
    use crate::note_processor::{Chord, ScaleQuantize};
    // ScaleQuantize (node 1) → block Chord (node 2).
    let mut g = NoteGraph::new(NoteGraphId::new(0), "t");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize::default())),
    )
    .expect("under cap");
    g.try_insert_node(
        id(2),
        NoteModuleConfig::Processor(NoteProcessor::Chord(Chord::major())),
    )
    .expect("under cap");
    g.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("linear");
    g.rebuild_derived().expect("valid");
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MF,
    )];
    let host = HostKey::from(PatternId::new(1));
    let mut buf = ExpansionBuffer::new();

    // Tap after node 1: just the (quantized) source note, no chord tones.
    g.expand_at_tick_tapped(
        &src,
        PatternTick(0),
        host,
        Bpm::DEFAULT,
        id(1),
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1);
    // Tap after node 2: the major triad expanded (3 tones).
    g.expand_at_tick_tapped(
        &src,
        PatternTick(0),
        host,
        Bpm::DEFAULT,
        id(2),
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 3);
    // Tapping the last spine node equals the full expansion.
    let mut full = ExpansionBuffer::new();
    g.expand_at_tick(
        &src,
        PatternTick(0),
        host,
        Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut full,
    );
    assert_eq!(full.notes().len(), 3);
}

#[test]
fn max_strum_tail_extends_freeze_walk_for_strummed_chord() {
    use crate::note_processor::{Chord, StrumDirection};
    let mut g = NoteGraph::new(NoteGraphId::new(0), "s");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(30), StrumDirection::Up),
        )),
    )
    .expect("under cap");
    // Major triad (3 tones) staggered by 30 ticks → (3-1)·30 = 60 tail.
    assert_eq!(g.max_strum_tail(), 60);
    // A block chord (no strum) extends the walk by nothing.
    let mut g2 = NoteGraph::new(NoteGraphId::new(0), "b");
    g2.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::Chord(Chord::major())),
    )
    .expect("under cap");
    assert_eq!(g2.max_strum_tail(), 0);
}

#[test]
fn empty_graph_is_valid_with_empty_order() {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "empty");
    assert!(g.rebuild_derived().is_ok());
    assert!(g.processing_order.is_empty());
    assert_eq!(g.stream_output_node(), None);
}

#[test]
fn linear_chain_yields_topological_order() {
    let mut g = graph_with_nodes(3);
    // Connect out of insertion order to prove the order is derived, not
    // insertion-based: 2->3 first, then 1->2.
    g.try_connect(NoteConnection::stream(id(2), id(3))).unwrap();
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    assert_eq!(g.processing_order, vec![id(1), id(2), id(3)]);
    assert_eq!(g.stream_output_node(), Some(id(3)));
}

#[test]
fn split_stream_output_rejected() {
    let mut g = graph_with_nodes(3);
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    let err = g
        .try_connect(NoteConnection::stream(id(1), id(3)))
        .unwrap_err();
    assert_eq!(err, NoteGraphError::DuplicateStreamOutput(id(1)));
    // Rolled back: only the first edge survives and the order stays valid.
    assert_eq!(g.connections.len(), 1);
    assert_eq!(g.processing_order.len(), 3);
}

#[test]
fn merge_stream_input_rejected() {
    let mut g = graph_with_nodes(3);
    g.try_connect(NoteConnection::stream(id(1), id(3))).unwrap();
    let err = g
        .try_connect(NoteConnection::stream(id(2), id(3)))
        .unwrap_err();
    assert_eq!(err, NoteGraphError::DuplicateStreamInput(id(3)));
    assert_eq!(g.connections.len(), 1);
}

#[test]
fn cycle_rejected() {
    let mut g = graph_with_nodes(2);
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    // 2->1 passes the in/out-degree check but closes a loop.
    let err = g
        .try_connect(NoteConnection::stream(id(2), id(1)))
        .unwrap_err();
    assert_eq!(err, NoteGraphError::Cycle);
    assert_eq!(g.connections.len(), 1);
}

#[test]
fn off_spine_stream_nodes_are_inert() {
    use crate::note_processor::{PitchClass, ScaleMask, ScaleQuantize};
    // A quantize spine plus an UNWIRED Euclidean generator: the generator
    // must neither emit nor steal the spine's source seeding.
    let mut g = NoteGraph::new(NoteGraphId::new(6), "spine+loose");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
            root: PitchClass::new(0),
            mask: ScaleMask::MAJOR,
        })),
    )
    .unwrap();
    g.try_insert_node(id(2), proc_node()).unwrap();
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    // The loose generator would fire at tick 0 if it were active.
    g.try_insert_node(
        id(3),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 4,
            pulses: 4,
            rotation: 0,
            step_len: Duration(100),
            pitch: pitch(72),
            velocity: Velocity::MF,
        }),
    )
    .unwrap();
    assert_eq!(g.stream_spine, vec![id(1), id(2)], "longest chain wins");
    assert_eq!(g.stream_output_node(), Some(id(2)));

    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(61),
        Velocity::new(0.8),
    )];
    let host = HostKey::from(PatternId::new(1));
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        &src,
        PatternTick(0),
        host,
        synth_core::Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut buf,
    );
    // One note: the quantized source (61→62). No generated 72, no concat.
    assert_eq!(buf.notes().len(), 1);
    assert_eq!(buf.notes()[0].pitch, pitch(62));

    // Wire the generator ahead of the spine and it takes over as head:
    // 3→1→2 is now the longest chain and the source is ignored.
    g.try_connect(NoteConnection::stream(id(3), id(1))).unwrap();
    assert_eq!(g.stream_spine, vec![id(3), id(1), id(2)]);
    g.expand_at_tick(
        &src,
        PatternTick(0),
        host,
        synth_core::Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1);
    assert_eq!(
        buf.notes()[0].pitch,
        pitch(72),
        "generator head replaces source"
    );
}

#[test]
fn sanitize_repairs_legacy_duplicate_and_double_fed_edges() {
    // Files saved before the duplicate/double-feed rules existed may carry
    // both shapes; sanitize must repair them (keeping the LAST value edge,
    // the old evaluator's winner) so the graph still validates and plays.
    let mut g = NoteGraph::new(NoteGraphId::new(5), "legacy");
    g.try_insert_node(id(1), NoteModuleConfig::NoteLfo(NoteLfo::default()))
        .unwrap();
    g.try_insert_node(id(2), NoteModuleConfig::StepLfo(StepLfo::default()))
        .unwrap();
    g.try_insert_node(
        id(3),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
    )
    .unwrap();
    g.try_insert_node(id(4), proc_node()).unwrap();
    // Bypass validation the way a legacy file does: raw field writes.
    g.connections = vec![
        NoteConnection::stream(id(3), id(4)),
        NoteConnection::stream(id(3), id(4)), // exact duplicate
        NoteConnection::value(id(1), id(3), 0),
        NoteConnection::value(id(2), id(3), 0), // double-fed input
    ];
    assert!(g.rebuild_derived().is_err(), "legacy shape must be invalid");

    g.sanitize_connections();
    assert!(g.rebuild_derived().is_ok(), "sanitized graph validates");
    assert_eq!(
        g.connections,
        vec![
            NoteConnection::stream(id(3), id(4)),
            // The LAST value edge survives (old last-edge-wins eval).
            NoteConnection::value(id(2), id(3), 0),
        ]
    );
}

#[test]
fn duplicate_connection_rejected() {
    let mut g = graph_with_nodes(2);
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    let err = g
        .try_connect(NoteConnection::stream(id(1), id(2)))
        .unwrap_err();
    assert_eq!(err, NoteGraphError::DuplicateConnection(id(1), id(2)));
    assert_eq!(g.connections.len(), 1);
}

#[test]
fn second_value_source_into_same_input_rejected() {
    // Two LFOs into the one ProbabilityGate threshold port: the second
    // edge would silently shadow the first at eval time, so reject it.
    let mut g = NoteGraph::new(NoteGraphId::new(4), "mods");
    g.try_insert_node(id(1), NoteModuleConfig::NoteLfo(NoteLfo::default()))
        .unwrap();
    g.try_insert_node(id(2), NoteModuleConfig::StepLfo(StepLfo::default()))
        .unwrap();
    g.try_insert_node(
        id(3),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
    )
    .unwrap();
    g.try_connect(NoteConnection::value(id(1), id(3), 0))
        .unwrap();
    let err = g
        .try_connect(NoteConnection::value(id(2), id(3), 0))
        .unwrap_err();
    assert_eq!(err, NoteGraphError::DuplicateValueInput(id(3), 0));
    assert_eq!(g.connections.len(), 1);
}

#[test]
fn unknown_node_rejected() {
    let mut g = graph_with_nodes(1);
    let err = g
        .try_connect(NoteConnection::stream(id(1), id(99)))
        .unwrap_err();
    assert_eq!(err, NoteGraphError::UnknownNode(id(99)));
    assert!(g.connections.is_empty());
}

// --- Evaluation core (the three gate properties) -----------------------

use crate::note::Note;
use crate::note_processor::{PitchClass, ScaleMask, ScaleQuantize};
use crate::pitch::{Pitch, Velocity};
use synth_core::NormalizedValue;

fn pitch(m: u8) -> Pitch {
    Pitch::new(m).expect("valid midi")
}

fn src_notes() -> Vec<Note> {
    vec![
        Note::new(NoteId(1), PatternTick(0), pitch(61), Velocity::new(0.8)),
        Note::new(NoteId(2), PatternTick(0), pitch(64), Velocity::new(0.5)),
        Note::new(NoteId(3), PatternTick(480), pitch(66), Velocity::new(0.7)),
    ]
}

/// A 2-node chain: snap to C major, then humanize velocity (seeded).
fn quantize_then_humanize() -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(1), "q+h");
    let sq = NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
        root: PitchClass::new(0),
        mask: ScaleMask::MAJOR,
    }));
    let hu = NoteModuleConfig::Processor(NoteProcessor::Humanize(Humanize {
        velocity: NormalizedValue::new(0.5),
        gate: NormalizedValue::new(0.0),
        seed: 0,
    }));
    g.try_insert_node(id(1), sq).expect("under cap");
    g.try_insert_node(id(2), hu).expect("under cap");
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    g
}

#[test]
fn seek_order_independence() {
    // Expanding a tick must not depend on what was expanded before it —
    // the property that makes seek / preview / random access correct.
    let g = quantize_then_humanize();
    let src = src_notes();
    let host = HostKey::from(PatternId::new(1));
    let mut buf = ExpansionBuffer::new();

    for &t in &[0u32, 5, 480, 481, 999] {
        g.expand_at_tick(
            &src,
            PatternTick(t),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        let direct = buf.notes().to_vec();
        // Expand an unrelated tick, then re-expand t: must match `direct`.
        g.expand_at_tick(
            &src,
            PatternTick(7777),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        g.expand_at_tick(
            &src,
            PatternTick(t),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        assert_eq!(buf.notes(), direct.as_slice(), "tick {t} depends on order");
    }
}

#[test]
fn bake_equals_playback() {
    // A full bake walk must reproduce exactly what independent per-tick
    // playback produces (purity → offline == live).
    let g = quantize_then_humanize();
    let src = src_notes();
    let host = HostKey::from(PatternId::new(1));
    let length = PatternTick(960);

    let baked = g.bake(&src, length, host, synth_core::Bpm::DEFAULT, None);

    let mut buf = ExpansionBuffer::new();
    let mut playback = Vec::new();
    for t in 0..length.0 {
        g.expand_at_tick(
            &src,
            PatternTick(t),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        for note in buf.notes() {
            playback.push((PatternTick(t), *note));
        }
    }
    assert_eq!(baked, playback);
    // Sanity: the three source onsets survived the bake.
    assert_eq!(baked.len(), 3);
}

#[test]
fn shared_graph_decorrelation() {
    // One pooled graph, two hosts: humanized velocities must differ
    // (decorrelated) while the deterministic quantized pitches match, and
    // each host is reproducible.
    let g = quantize_then_humanize();
    let src = src_notes();
    let host_a = HostKey::from(PatternId::new(1));
    let host_b = HostKey::from(PatternId::new(2));
    let length = PatternTick(960);

    let baked_a = g.bake(&src, length, host_a, synth_core::Bpm::DEFAULT, None);
    let baked_b = g.bake(&src, length, host_b, synth_core::Bpm::DEFAULT, None);

    assert_eq!(baked_a.len(), baked_b.len());
    let velocities_differ = baked_a
        .iter()
        .zip(&baked_b)
        .any(|((_, a), (_, b))| a.velocity != b.velocity);
    assert!(velocities_differ, "host key did not decorrelate humanize");
    for ((_, a), (_, b)) in baked_a.iter().zip(&baked_b) {
        assert_eq!(a.pitch, b.pitch, "quantize must be host-independent");
    }
    // Reproducible: same host again → identical bake.
    assert_eq!(
        baked_a,
        g.bake(&src, length, host_a, synth_core::Bpm::DEFAULT, None)
    );
}

#[test]
fn seed_helpers_are_deterministic_and_distinct() {
    // Distinct host ids fold to distinct salts, deterministically.
    let a = HostKey::from(PatternId::new(1));
    let b = HostKey::from(PatternId::new(2));
    assert_ne!(a.get(), b.get());
    assert_eq!(a.get(), HostKey::from(PatternId::new(1)).get());

    // NoteEventKey::seed is stable and decorrelates on each field.
    let k = NoteEventKey::new(NoteId(3), PatternTick(480), 1);
    assert_eq!(
        k.seed(),
        NoteEventKey::new(NoteId(3), PatternTick(480), 1).seed()
    );
    assert_ne!(
        k.seed(),
        NoteEventKey::new(NoteId(3), PatternTick(480), 2).seed()
    );
    assert_ne!(
        k.seed(),
        NoteEventKey::new(NoteId(4), PatternTick(480), 1).seed()
    );

    // Fallback key varies with tick, pitch, and slot.
    let base = fallback_note_seed(PatternTick(10), pitch(60), 0);
    assert_ne!(base, fallback_note_seed(PatternTick(11), pitch(60), 0));
    assert_ne!(base, fallback_note_seed(PatternTick(10), pitch(61), 0));
    assert_ne!(base, fallback_note_seed(PatternTick(10), pitch(60), 1));
}

#[test]
fn euclidean_generator_emits_onset_pattern() {
    let mut g = NoteGraph::new(NoteGraphId::new(2), "euclid");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 8,
            pulses: 4,
            rotation: 0,
            step_len: Duration(100),
            pitch: pitch(60),
            velocity: Velocity::MF,
        }),
    )
    .unwrap();
    // No source notes — the generator produces its own stream.
    let host = HostKey::from(PatternId::new(1));
    let baked = g.bake(&[], PatternTick(800), host, synth_core::Bpm::DEFAULT, None);
    // E(4,8) rotation 0 → onsets at steps 0,2,4,6 → ticks 0,200,400,600.
    let ticks: Vec<u32> = baked.iter().map(|(t, _)| t.0).collect();
    assert_eq!(ticks, vec![0, 200, 400, 600]);
    assert!(baked.iter().all(|(_, n)| n.pitch == pitch(60)));
}

#[test]
fn generator_headed_graph_ignores_source_notes() {
    // A Euclidean-headed graph bound to a pattern that already has notes must
    // NOT leak those source notes into the output — the generator defines the
    // stream (plan §5.A "source-independent").
    let mut g = NoteGraph::new(NoteGraphId::new(9), "gen");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 4,
            pulses: 1, // one hit at tick 0 per 400-tick cycle
            rotation: 0,
            step_len: Duration(100),
            pitch: pitch(72),
            velocity: Velocity::MF,
        }),
    )
    .unwrap();
    let host = HostKey::from(PatternId::new(1));
    let mut buf = ExpansionBuffer::new();
    // A source note also starts at tick 0 — it must be dropped, leaving only
    // the generated hit (pitch 72), not the source pitch 61.
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(61),
        Velocity::new(0.8),
    )];
    g.expand_at_tick(
        &src,
        PatternTick(0),
        host,
        synth_core::Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1);
    assert_eq!(buf.notes()[0].pitch, pitch(72));

    // A transform-headed graph, by contrast, still processes the source.
    let mut q = NoteGraph::new(NoteGraphId::new(10), "quant");
    q.try_insert_node(
        id(1),
        NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
            root: PitchClass::new(0),
            mask: ScaleMask::MAJOR,
        })),
    )
    .unwrap();
    q.expand_at_tick(
        &src,
        PatternTick(0),
        host,
        synth_core::Bpm::DEFAULT,
        |_| true,
        None,
        None,
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1, "transform head keeps the source note");
}

#[test]
fn euclidean_generator_is_seek_order_independent() {
    let mut g = NoteGraph::new(NoteGraphId::new(2), "euclid");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator::default()),
    )
    .unwrap();
    let host = HostKey::from(PatternId::new(1));
    let mut buf = ExpansionBuffer::new();
    for &t in &[0u32, 240, 480, 7777] {
        g.expand_at_tick(
            &[],
            PatternTick(t),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        let direct = buf.notes().to_vec();
        g.expand_at_tick(
            &[],
            PatternTick(9999),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        g.expand_at_tick(
            &[],
            PatternTick(t),
            host,
            synth_core::Bpm::DEFAULT,
            |_| true,
            None,
            None,
            &mut buf,
        );
        assert_eq!(
            buf.notes(),
            direct.as_slice(),
            "generator tick {t} order-dependent"
        );
    }
}

/// A saturated Euclidean generator (`pulses == steps`) feeding a gate — a
/// stream of `steps` hits to filter.
fn saturated_euclid_then_gate(prob: f32, seed: u64, steps: u8) -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(3), "prob");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps,
            pulses: steps,
            rotation: 0,
            step_len: Duration(100),
            pitch: pitch(60),
            velocity: Velocity::MF,
        }),
    )
    .unwrap();
    g.try_insert_node(
        id(2),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate {
            probability: NormalizedValue::new(prob),
            seed,
        }),
    )
    .unwrap();
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    g
}

#[test]
fn probability_gate_blocks_deterministically_and_decorrelates() {
    let g = saturated_euclid_then_gate(0.5, 7, 16);
    let host_a = HostKey::from(PatternId::new(1));
    let host_b = HostKey::from(PatternId::new(2));
    let len = PatternTick(1600); // 16 steps * 100 ticks

    let a1 = g.bake(&[], len, host_a, synth_core::Bpm::DEFAULT, None);
    // Some notes pass, some are blocked (not all 16, not zero).
    assert!(!a1.is_empty() && a1.len() < 16, "gate kept {}", a1.len());
    // Reproducible for the same host.
    assert_eq!(a1, g.bake(&[], len, host_a, synth_core::Bpm::DEFAULT, None));
    // Decorrelated: a different host keeps a different subset.
    let b1 = g.bake(&[], len, host_b, synth_core::Bpm::DEFAULT, None);
    let a_ticks: Vec<u32> = a1.iter().map(|(t, _)| t.0).collect();
    let b_ticks: Vec<u32> = b1.iter().map(|(t, _)| t.0).collect();
    assert_ne!(a_ticks, b_ticks, "hosts should keep different notes");
}

#[test]
fn probability_gate_extremes_pass_all_or_block_all() {
    let host = HostKey::from(PatternId::new(1));
    assert_eq!(
        saturated_euclid_then_gate(1.0, 0, 8)
            .bake(&[], PatternTick(800), host, synth_core::Bpm::DEFAULT, None)
            .len(),
        8
    );
    assert_eq!(
        saturated_euclid_then_gate(0.0, 0, 8)
            .bake(&[], PatternTick(800), host, synth_core::Bpm::DEFAULT, None)
            .len(),
        0
    );
}

#[test]
fn note_lfo_is_a_pure_tick_function() {
    let lfo = NoteLfo {
        shape: LfoShape::Saw,
        period: Duration(200),
        phase: NormalizedValue::new(0.0),
        depth: NormalizedValue::new(1.0),
    };
    // Saw ramps 0→1 over the period and wraps.
    assert!((lfo.value(PatternTick(0)) - 0.0).abs() < 1e-6);
    assert!((lfo.value(PatternTick(100)) - 0.5).abs() < 1e-6);
    // Same phase one period later → identical value (no cross-tick state).
    assert_eq!(lfo.value(PatternTick(100)), lfo.value(PatternTick(300)));
    // Depth scales the output.
    let half = NoteLfo {
        depth: NormalizedValue::new(0.5),
        ..lfo
    };
    assert!((half.value(PatternTick(100)) - 0.25).abs() < 1e-6);
    // Zero period freezes at the phase offset.
    let frozen = NoteLfo {
        period: Duration(0),
        phase: NormalizedValue::new(0.3),
        ..lfo
    };
    assert_eq!(
        frozen.value(PatternTick(0)),
        frozen.value(PatternTick(9999))
    );
}

#[test]
fn step_lfo_cycles_and_is_pure() {
    let s = StepLfo {
        steps: vec![
            NormalizedValue::new(0.0),
            NormalizedValue::new(1.0),
            NormalizedValue::new(0.5),
        ],
        step_len: Duration(100),
    };
    assert_eq!(s.value(PatternTick(0)), 0.0);
    assert_eq!(s.value(PatternTick(50)), 0.0); // still step 0
    assert_eq!(s.value(PatternTick(100)), 1.0); // step 1
    assert_eq!(s.value(PatternTick(200)), 0.5); // step 2
    assert_eq!(s.value(PatternTick(300)), 0.0); // wraps to step 0
    // Empty table → silent.
    let empty = StepLfo {
        steps: vec![],
        step_len: Duration(100),
    };
    assert_eq!(empty.value(PatternTick(100)), 0.0);
    // Zero step length → frozen on the first step.
    let frozen = StepLfo {
        steps: s.steps.clone(),
        step_len: Duration(0),
    };
    assert_eq!(frozen.value(PatternTick(999)), 0.0);
}

#[test]
fn note_envelope_tracks_source_onsets() {
    let env = NoteEnvelope {
        attack: Duration(100),
        decay: Duration(100),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::SourceOnset,
    };
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(200),
        pitch(60),
        Velocity::MF,
    )];
    // Silent before the onset.
    assert_eq!(env.value(PatternTick(199), &src), 0.0);
    // Attack ramp: 0 at onset, 0.5 halfway, 1.0 at the peak.
    assert!(env.value(PatternTick(200), &src).abs() < 1e-6);
    assert!((env.value(PatternTick(250), &src) - 0.5).abs() < 1e-6);
    assert!((env.value(PatternTick(300), &src) - 1.0).abs() < 1e-6);
    // Decay ramp back to zero, then silent past the window.
    assert!((env.value(PatternTick(350), &src) - 0.5).abs() < 1e-6);
    assert_eq!(env.value(PatternTick(400), &src), 0.0);
    assert_eq!(env.value(PatternTick(9999), &src), 0.0);
    // A later onset retriggers the envelope from zero.
    let retrig = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MF),
        Note::new(NoteId(2), PatternTick(300), pitch(62), Velocity::MF),
    ];
    assert!((env.value(PatternTick(350), &retrig) - 0.5).abs() < 1e-6);

    // Zero-length decay: the attack ramp still reaches `peak` at its apex,
    // then snaps to silence.
    let spike = NoteEnvelope {
        attack: Duration(100),
        decay: Duration(0),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::SourceOnset,
    };
    let at0 = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MF,
    )];
    assert!((spike.value(PatternTick(50), &at0) - 0.5).abs() < 1e-6);
    assert!((spike.value(PatternTick(100), &at0) - 1.0).abs() < 1e-6);
    assert_eq!(spike.value(PatternTick(101), &at0), 0.0);
}

#[test]
fn envelope_modulates_gate_from_source_onsets() {
    // Envelope (source-triggered) → gate threshold. Notes near a source
    // onset (high envelope → high threshold) pass more than notes in the
    // envelope's silent tail — proving `source` reaches the Value node.
    let mut g = NoteGraph::new(NoteGraphId::new(7), "env-gate");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 32,
            pulses: 32,
            rotation: 0,
            step_len: Duration(100),
            pitch: pitch(60),
            velocity: Velocity::MF,
        }),
    )
    .unwrap();
    g.try_insert_node(
        id(2),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate {
            probability: NormalizedValue::new(0.5),
            seed: 3,
        }),
    )
    .unwrap();
    g.try_insert_node(
        id(3),
        NoteModuleConfig::NoteEnvelope(NoteEnvelope {
            attack: Duration(0),
            decay: Duration(400),
            peak: NormalizedValue::new(1.0),
            trigger: EnvelopeTrigger::SourceOnset,
        }),
    )
    .unwrap();
    g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
    g.try_connect(NoteConnection::value(id(3), id(2), 0))
        .unwrap();

    let host = HostKey::from(PatternId::new(1));
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(48),
        Velocity::MF,
    )];
    // With a source onset at 0, the first steps see a high threshold; the
    // envelope decays to zero by tick 400, blocking the later steps.
    let with_src = g.bake(
        &src,
        PatternTick(3200),
        host,
        synth_core::Bpm::DEFAULT,
        None,
    );
    let without_src = g.bake(&[], PatternTick(3200), host, synth_core::Bpm::DEFAULT, None);
    // Source-driven envelope keeps strictly more early notes than the dry
    // (threshold-0 → all blocked) run.
    assert!(
        with_src.len() > without_src.len(),
        "envelope did not open the gate from source onsets"
    );
    assert!(
        without_src.is_empty(),
        "no envelope → threshold 0 → all blocked"
    );
}

#[test]
fn value_edge_endpoint_validation() {
    let mut g = NoteGraph::new(NoteGraphId::new(4), "mod");
    g.try_insert_node(id(1), NoteModuleConfig::NoteLfo(NoteLfo::default()))
        .unwrap();
    g.try_insert_node(
        id(2),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
    )
    .unwrap();
    g.try_insert_node(id(3), proc_node()).unwrap();

    // LFO → gate threshold (port 0) is valid.
    g.try_connect(NoteConnection::value(id(1), id(2), 0))
        .unwrap();
    // A stream node (Humanize) is not a Value source.
    assert_eq!(
        g.try_connect(NoteConnection::value(id(3), id(2), 0)),
        Err(NoteGraphError::NotValueSource(id(3)))
    );
    // The gate exposes exactly one Value input; port 1 is out of range.
    assert_eq!(
        g.try_connect(NoteConnection::value(id(1), id(2), 1)),
        Err(NoteGraphError::InvalidValueInput(id(2), 1))
    );
    // A Humanize node takes no Value input at all.
    assert_eq!(
        g.try_connect(NoteConnection::value(id(1), id(3), 0)),
        Err(NoteGraphError::InvalidValueInput(id(3), 0))
    );
}

#[test]
fn next_module_id_fills_gaps_and_avoids_collisions() {
    let mut g = NoteGraph::new(NoteGraphId::new(8), "alloc");
    assert_eq!(g.next_module_id(), id(0));
    g.try_insert_node(id(0), proc_node()).unwrap();
    g.try_insert_node(id(1), proc_node()).unwrap();
    assert_eq!(g.next_module_id(), id(2));
    // Removing a middle id frees it; the allocator reuses the gap.
    g.remove_node(id(0));
    assert_eq!(g.next_module_id(), id(0));
    // A saturated max id must not saturate into a collision.
    g.try_insert_node(NoteModuleId::new(u32::MAX), proc_node())
        .unwrap();
    assert_eq!(g.next_module_id(), id(0));
}

#[test]
fn stream_edge_requires_stream_ports() {
    // A `NoteLfo` is a Value source with neither stream port, so it can
    // never sit on the linear spine.
    let mut g = NoteGraph::new(NoteGraphId::new(4), "spine");
    g.try_insert_node(id(1), proc_node()).unwrap();
    g.try_insert_node(id(2), NoteModuleConfig::NoteLfo(NoteLfo::default()))
        .unwrap();
    assert_eq!(
        g.try_connect(NoteConnection::stream(id(1), id(2))),
        Err(NoteGraphError::InvalidStreamEndpoint(id(2)))
    );
    assert_eq!(
        g.try_connect(NoteConnection::stream(id(2), id(1))),
        Err(NoteGraphError::InvalidStreamEndpoint(id(2)))
    );
    assert!(g.connections.is_empty());
}

#[test]
fn replacing_node_rolls_back_when_it_invalidates_an_edge() {
    // LFO(1) → gate(2) threshold. Replacing the LFO with a stream processor
    // would orphan the Value edge; the failed insert must leave the graph
    // exactly as it was (atomicity).
    let mut g = NoteGraph::new(NoteGraphId::new(6), "replace");
    g.try_insert_node(id(1), NoteModuleConfig::NoteLfo(NoteLfo::default()))
        .unwrap();
    g.try_insert_node(
        id(2),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
    )
    .unwrap();
    g.try_connect(NoteConnection::value(id(1), id(2), 0))
        .unwrap();
    let before = g.clone();

    assert_eq!(
        g.try_insert_node(id(1), proc_node()),
        Err(NoteGraphError::NotValueSource(id(1)))
    );
    // Rolled back: the LFO config survived and the order is still valid.
    assert_eq!(g.nodes, before.nodes);
    assert_eq!(g.processing_order, before.processing_order);
    assert!(matches!(g.nodes[&id(1)], NoteModuleConfig::NoteLfo(_)));
}

#[test]
fn lfo_modulates_probability_threshold() {
    // A saturated Euclid feeds a gate whose threshold is driven by an LFO.
    // The modulated keep-set must differ from the static-threshold set, and
    // must stay reproducible and host-decorrelated.
    let build = |with_lfo: bool| {
        let mut g = NoteGraph::new(NoteGraphId::new(5), "lfo-gate");
        g.try_insert_node(
            id(1),
            NoteModuleConfig::Euclidean(EuclideanGenerator {
                steps: 16,
                pulses: 16,
                rotation: 0,
                step_len: Duration(100),
                pitch: pitch(60),
                velocity: Velocity::MF,
            }),
        )
        .unwrap();
        g.try_insert_node(
            id(2),
            NoteModuleConfig::ProbabilityGate(ProbabilityGate {
                probability: NormalizedValue::new(0.5),
                seed: 7,
            }),
        )
        .unwrap();
        g.try_connect(NoteConnection::stream(id(1), id(2))).unwrap();
        if with_lfo {
            g.try_insert_node(
                id(3),
                NoteModuleConfig::NoteLfo(NoteLfo {
                    shape: LfoShape::Saw,
                    period: Duration(700),
                    phase: NormalizedValue::new(0.0),
                    depth: NormalizedValue::new(1.0),
                }),
            )
            .unwrap();
            g.try_connect(NoteConnection::value(id(3), id(2), 0))
                .unwrap();
        }
        g
    };
    let host = HostKey::from(PatternId::new(1));
    let len = PatternTick(1600);
    let static_ticks: Vec<u32> = build(false)
        .bake(&[], len, host, synth_core::Bpm::DEFAULT, None)
        .iter()
        .map(|(t, _)| t.0)
        .collect();
    let modulated = build(true);
    let mod_ticks: Vec<u32> = modulated
        .bake(&[], len, host, synth_core::Bpm::DEFAULT, None)
        .iter()
        .map(|(t, _)| t.0)
        .collect();
    assert_ne!(static_ticks, mod_ticks, "LFO did not affect the threshold");
    // Reproducible for the same host.
    let mod_ticks2: Vec<u32> = modulated
        .bake(&[], len, host, synth_core::Bpm::DEFAULT, None)
        .iter()
        .map(|(t, _)| t.0)
        .collect();
    assert_eq!(mod_ticks, mod_ticks2);
    // Decorrelated across hosts.
    let host_b = HostKey::from(PatternId::new(2));
    let mod_ticks_b: Vec<u32> = modulated
        .bake(&[], len, host_b, synth_core::Bpm::DEFAULT, None)
        .iter()
        .map(|(t, _)| t.0)
        .collect();
    assert_ne!(mod_ticks, mod_ticks_b);
}

#[test]
fn node_cap_enforced_but_replacement_allowed() {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "cap");
    for i in 0..MAX_NOTE_GRID_NODES as u32 {
        g.try_insert_node(id(i), proc_node()).expect("under cap");
    }
    // One past the cap fails.
    assert_eq!(
        g.try_insert_node(id(MAX_NOTE_GRID_NODES as u32), proc_node()),
        Err(NoteGraphError::NodeCapExceeded)
    );
    // Replacing an existing id at the cap is fine (no growth).
    assert!(g.try_insert_node(id(0), proc_node()).is_ok());
}

// ========================================================================
// Note scope (plan §2.1): `Note::note_graph` runs during source seeding.
// ========================================================================

/// Build a single-node note-scope graph wrapping `proc`, pooled at `gid`.
fn note_scope_graph(gid: u32, proc: NoteProcessor) -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(gid), "note-scope");
    g.try_insert_node(id(1), NoteModuleConfig::Processor(proc))
        .expect("under cap");
    g.rebuild_derived().expect("valid");
    g
}

#[test]
fn note_scope_expands_a_single_note_through_its_graph() {
    use crate::note_processor::Chord;
    // A note bound to a note-scope major-triad graph articulates one C into
    // a three-tone chord during seeding — not a single pass-through onset.
    let pool = vec![note_scope_graph(7, NoteProcessor::Chord(Chord::major()))];
    let mut note = Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MF);
    note.note_graph = Some(NoteGraphId::new(7));
    let src = vec![note];

    let mut scratch = ExpansionBuffer::new();
    let mut buf = ExpansionBuffer::new();
    let mut ctx = NoteScopeCtx {
        pool: &pool,
        scratch: &mut scratch,
    };
    seed_source_at_tick(
        &src,
        PatternTick(0),
        Bpm::DEFAULT,
        &|_| true,
        Some(&mut ctx),
        &mut buf,
    );
    let mut pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
    pitches.sort_unstable();
    assert_eq!(pitches, vec![60, 64, 67], "C major triad articulated");
}

#[test]
fn note_scope_output_chains_into_pattern_processing() {
    use crate::note_processor::{Chord, PitchClass, ScaleMask, ScaleQuantize};
    use crate::pattern::Pattern;
    // Note scope articulates C into a C-E-G triad; the pattern rack then
    // scale-quantizes to a scale WITHOUT the major third (interval 4), so the
    // generated E must be snapped — proving the pattern stage saw and
    // processed the note-scope-generated tones (the note → pattern pipeline).
    let pool = vec![note_scope_graph(7, NoteProcessor::Chord(Chord::major()))];
    let mut pattern = Pattern::new(PatternId::new(1), Duration(960));
    let nid = pattern.add_note(PatternTick(0), pitch(60), Velocity::MF);
    pattern.note_mut(nid).expect("note").note_graph = Some(NoteGraphId::new(7));
    pattern.add_processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
        root: PitchClass::new(0),
        mask: ScaleMask::from_intervals(&[0, 2, 3, 5, 7, 9, 10]), // no interval 4
    }));

    let mut scratch = ExpansionBuffer::new();
    let mut buf = ExpansionBuffer::new();
    let mut ctx = NoteScopeCtx {
        pool: &pool,
        scratch: &mut scratch,
    };
    pattern.expand_at_tick(
        PatternTick(0),
        |_| true,
        Bpm::DEFAULT,
        Some(&mut ctx),
        &mut buf,
    );
    let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
    assert_eq!(buf.notes().len(), 3, "three tones survive: {pitches:?}");
    assert!(pitches.contains(&60), "root C passes through: {pitches:?}");
    assert!(pitches.contains(&67), "fifth G passes through: {pitches:?}");
    assert!(
        !pitches.contains(&64),
        "E must be snapped out of the no-third scale: {pitches:?}"
    );
}

#[test]
fn note_scope_decorrelates_by_note_id() {
    use crate::note_processor::Humanize;
    // One shared, seeded Humanize graph on two otherwise-identical notes must
    // perturb them *differently* — the host key is each note's id (plan §1.2).
    let pool = vec![note_scope_graph(
        7,
        NoteProcessor::Humanize(Humanize::default()),
    )];
    let mk = |nid: u64| {
        let mut n = Note::new(NoteId(nid), PatternTick(0), pitch(60), Velocity::new(0.5));
        n.note_graph = Some(NoteGraphId::new(7));
        n
    };
    let src = vec![mk(1), mk(2)];

    let mut scratch = ExpansionBuffer::new();
    let mut buf = ExpansionBuffer::new();
    let mut ctx = NoteScopeCtx {
        pool: &pool,
        scratch: &mut scratch,
    };
    seed_source_at_tick(
        &src,
        PatternTick(0),
        Bpm::DEFAULT,
        &|_| true,
        Some(&mut ctx),
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 2);
    assert_ne!(
        buf.notes()[0].velocity.as_f32(),
        buf.notes()[1].velocity.as_f32(),
        "same graph, different note id → different humanization"
    );
}

#[test]
fn dangling_or_absent_note_scope_passes_through_plain() {
    // A dangling id (not in the pool) and a `None` context both fall back to
    // the plain onset — never a panic or silence.
    let pool: Vec<NoteGraph> = Vec::new();
    let mut note = Note::new(NoteId(1), PatternTick(0), pitch(62), Velocity::MF);
    note.note_graph = Some(NoteGraphId::new(999)); // not in the (empty) pool
    let src = vec![note];

    let mut scratch = ExpansionBuffer::new();
    let mut buf = ExpansionBuffer::new();
    let mut ctx = NoteScopeCtx {
        pool: &pool,
        scratch: &mut scratch,
    };
    seed_source_at_tick(
        &src,
        PatternTick(0),
        Bpm::DEFAULT,
        &|_| true,
        Some(&mut ctx),
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1, "dangling id → plain onset");
    assert_eq!(buf.notes()[0].pitch, pitch(62), "pitch unchanged");

    // `None` context ignores the binding entirely, same plain result.
    let mut buf2 = ExpansionBuffer::new();
    seed_source_at_tick(
        &src,
        PatternTick(0),
        Bpm::DEFAULT,
        &|_| true,
        None,
        &mut buf2,
    );
    assert_eq!(buf2.notes().len(), 1);
    assert_eq!(buf2.notes()[0].pitch, pitch(62));
}

#[test]
fn note_scope_is_seek_order_independent() {
    use crate::note_processor::{Chord, StrumDirection};
    // A note-scope strummed chord staggers tones across ticks 0/30/60. Seek
    // independence (purity): expanding the ticks forward and backward yields
    // the same tone set — the bake-equals-playback property for note scope.
    let chord = Chord::major().with_strum(Duration(30), StrumDirection::Up);
    let pool = vec![note_scope_graph(7, NoteProcessor::Chord(chord))];
    let mut note =
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MF).with_duration(Duration(960));
    note.note_graph = Some(NoteGraphId::new(7));
    let src = vec![note];

    let collect = |ticks: &mut dyn Iterator<Item = u32>| -> Vec<(u32, u8)> {
        let mut scratch = ExpansionBuffer::new();
        let mut buf = ExpansionBuffer::new();
        let mut out = Vec::new();
        for t in ticks {
            let mut ctx = NoteScopeCtx {
                pool: &pool,
                scratch: &mut scratch,
            };
            seed_source_at_tick(
                &src,
                PatternTick(t),
                Bpm::DEFAULT,
                &|_| true,
                Some(&mut ctx),
                &mut buf,
            );
            for n in buf.notes() {
                out.push((t, n.pitch.as_midi()));
            }
        }
        out.sort_unstable();
        out
    };
    let forward = collect(&mut (0u32..90));
    let backward = collect(&mut (0u32..90).rev());
    assert_eq!(
        forward, backward,
        "note-scope expansion is seek-independent"
    );
    assert!(
        forward.len() >= 3,
        "the strum staggers at least three tones, got {forward:?}"
    );
}

// ── Timing modules: NoteDelay / Echo (plan §5.A, look-back spec) ──

/// Pitches sounding at `tick`, expanded through `g` with a real look-back pool.
fn pitches_at(g: &NoteGraph, src: &[Note], tick: u32) -> Vec<u8> {
    let mut pool = lookback_pool();
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        src,
        PatternTick(tick),
        HostKey::from(PatternId::new(1)),
        Bpm::DEFAULT,
        |_| true,
        None,
        Some(&mut pool),
        &mut buf,
    );
    buf.notes().iter().map(|n| n.pitch.as_midi()).collect()
}

fn delay_graph(delay: NoteDelay, upstream: Option<NoteProcessor>) -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "delay");
    if let Some(up) = upstream {
        g.try_insert_node(id(1), NoteModuleConfig::Processor(up))
            .expect("under cap");
        g.try_insert_node(id(2), NoteModuleConfig::NoteDelay(delay))
            .expect("under cap");
        g.try_connect(NoteConnection::stream(id(1), id(2)))
            .expect("valid");
    } else {
        g.try_insert_node(id(1), NoteModuleConfig::NoteDelay(delay))
            .expect("under cap");
    }
    g.rebuild_derived().expect("valid");
    g
}

#[test]
fn note_delay_source_direct_echoes_with_decay() {
    // One source note; delay = 100 ticks, 2 echoes, feedback 0.5.
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MAX,
    )];
    let delay = NoteDelay {
        delay_ticks: Duration(100),
        repeats: 2,
        feedback: NormalizedValue::new(0.5),
    };
    let g = delay_graph(delay, None);
    // Dry note at 0, echoes at 100 and 200, silence elsewhere.
    assert_eq!(pitches_at(&g, &src, 0), vec![60], "dry onset");
    assert_eq!(pitches_at(&g, &src, 100), vec![60], "first echo");
    assert_eq!(pitches_at(&g, &src, 200), vec![60], "second echo");
    assert!(pitches_at(&g, &src, 300).is_empty(), "no third echo");
    assert!(pitches_at(&g, &src, 50).is_empty(), "nothing off-grid");

    // Velocity decays by feedback^k.
    let mut pool = lookback_pool();
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        &src,
        PatternTick(100),
        HostKey::from(PatternId::new(1)),
        Bpm::DEFAULT,
        |_| true,
        None,
        Some(&mut pool),
        &mut buf,
    );
    assert!(
        (buf.notes()[0].velocity.as_f32() - 0.5).abs() < 1e-4,
        "first echo is 0.5× the source velocity"
    );
}

#[test]
fn note_delay_downstream_of_transform_echoes_transformed_onsets() {
    // Quantize (C-major) → Delay. A source F#4 (66) snaps to G4 (67); its
    // echo must be the *quantized* pitch, proving the delay re-runs the
    // upstream prefix at the earlier onset tick (eval_prefix_at_tick).
    use crate::note_processor::{PitchClass, ScaleMask, ScaleQuantize};
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(66),
        Velocity::MAX,
    )];
    let quant = ScaleQuantize {
        root: PitchClass::new(0),
        mask: ScaleMask::MAJOR,
    };
    let delay = NoteDelay {
        delay_ticks: Duration(100),
        repeats: 1,
        feedback: NormalizedValue::new(0.8),
    };
    let g = delay_graph(delay, Some(NoteProcessor::ScaleQuantize(quant)));
    assert_eq!(
        pitches_at(&g, &src, 0),
        vec![67],
        "dry note quantized to G4"
    );
    assert_eq!(
        pitches_at(&g, &src, 100),
        vec![67],
        "echo carries the quantized pitch, not the raw F#4"
    );
}

#[test]
fn note_delay_is_seek_order_independent() {
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MAX,
    )];
    let delay = NoteDelay {
        delay_ticks: Duration(48),
        repeats: 3,
        feedback: NormalizedValue::new(0.7),
    };
    let g = delay_graph(delay, None);
    let ticks = [0u32, 48, 96, 144, 200];
    let forward: Vec<_> = ticks.iter().map(|&t| pitches_at(&g, &src, t)).collect();
    let backward: Vec<_> = ticks
        .iter()
        .rev()
        .map(|&t| pitches_at(&g, &src, t))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert_eq!(forward, backward, "per-tick output is independent of order");
}

#[test]
fn note_delay_behind_a_delay_terminates_and_is_bounded() {
    // Delay → Delay: the inner look-back re-runs the upstream delay, which
    // itself looks back. The scratch-pool cap must make this terminate.
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MAX,
    )];
    let inner = NoteDelay {
        delay_ticks: Duration(50),
        repeats: 2,
        feedback: NormalizedValue::new(0.8),
    };
    let outer = NoteDelay {
        delay_ticks: Duration(50),
        repeats: 2,
        feedback: NormalizedValue::new(0.8),
    };
    let mut g = NoteGraph::new(NoteGraphId::new(0), "dd");
    g.try_insert_node(id(1), NoteModuleConfig::NoteDelay(inner))
        .expect("under cap");
    g.try_insert_node(id(2), NoteModuleConfig::NoteDelay(outer))
        .expect("under cap");
    g.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("valid");
    g.rebuild_derived().expect("valid");
    // Just reaching here without hanging proves termination; check a few ticks
    // stay within the buffer cap.
    for t in [0u32, 50, 100, 150, 200] {
        assert!(
            pitches_at(&g, &src, t).len() <= MAX_EXPANSION_EVENTS_PER_TICK,
            "output stays within the buffer cap at tick {t}"
        );
    }
}

#[test]
fn note_delay_freeze_equals_playback_including_tail() {
    // An echo whose repeats land *past* the note — freeze must walk far
    // enough (max_delay_tail) to bake them, and match playback tick-for-tick.
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(900),
        pitch(60),
        Velocity::MAX,
    )];
    let delay = NoteDelay {
        delay_ticks: Duration(200),
        repeats: 3,
        feedback: NormalizedValue::new(0.6),
    };
    let g = delay_graph(delay, None);
    let host = HostKey::from(PatternId::new(1));
    // Freeze walk-end mirrors Song::freeze_pattern_note_graph.
    let walk_end = 960u32 + g.max_strum_tail() + g.max_delay_tail();
    assert!(g.max_delay_tail() >= 600, "tail covers 3×200-tick echoes");
    let baked = g.bake(&src, PatternTick(walk_end), host, Bpm::DEFAULT, None);
    // Playback reference over the same range.
    let mut playback = Vec::new();
    for t in 0..walk_end {
        for p in pitches_at(&g, &src, t) {
            playback.push((t, p));
        }
    }
    let baked_pairs: Vec<(u32, u8)> = baked
        .iter()
        .map(|(t, n)| (t.0, n.pitch.as_midi()))
        .collect();
    assert_eq!(baked_pairs, playback, "freeze bakes exactly what plays");
    // The last echo lands at 900 + 3×200 = 1500, past the 960 length.
    assert!(
        baked_pairs.iter().any(|(t, _)| *t == 1500),
        "the tail echo past the pattern length is baked, got {baked_pairs:?}"
    );
}

#[test]
fn note_delay_echoes_respect_the_source_gate() {
    // Wet follows dry: a source note gated out at its onset does not echo.
    let src = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MAX),
        Note::new(NoteId(2), PatternTick(0), pitch(64), Velocity::MAX),
    ];
    let delay = NoteDelay {
        delay_ticks: Duration(100),
        repeats: 2,
        feedback: NormalizedValue::new(0.5),
    };
    let g = delay_graph(delay, None);
    // Gate that keeps note 1 and drops note 2 — the engine's probability roll
    // is exactly such a per-note gate.
    let gate = |n: &Note| n.id == NoteId(1);
    let expand = |tick: u32| {
        let mut pool = lookback_pool();
        let mut buf = ExpansionBuffer::new();
        g.expand_at_tick(
            &src,
            PatternTick(tick),
            HostKey::from(PatternId::new(1)),
            Bpm::DEFAULT,
            gate,
            None,
            Some(&mut pool),
            &mut buf,
        );
        buf.notes()
            .iter()
            .map(|n| n.pitch.as_midi())
            .collect::<Vec<_>>()
    };
    assert_eq!(expand(0), vec![60], "only the un-gated note plays dry");
    assert_eq!(expand(100), vec![60], "only the un-gated note echoes");
    assert_eq!(expand(200), vec![60], "…and on the second echo");
}

// ── Timing modules: Ratchet ──

fn ratchet_graph(ratchet: Ratchet, upstream: Option<NoteProcessor>) -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "ratchet");
    if let Some(up) = upstream {
        g.try_insert_node(id(1), NoteModuleConfig::Processor(up))
            .expect("under cap");
        g.try_insert_node(id(2), NoteModuleConfig::Ratchet(ratchet))
            .expect("under cap");
        g.try_connect(NoteConnection::stream(id(1), id(2)))
            .expect("valid");
    } else {
        g.try_insert_node(id(1), NoteModuleConfig::Ratchet(ratchet))
            .expect("under cap");
    }
    g.rebuild_derived().expect("valid");
    g
}

#[test]
fn ratchet_subdivides_a_sounding_note() {
    // A 480-tick note, subdivided into 4 hits 120 ticks apart.
    let src = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MAX).with_duration(Duration(480)),
    ];
    let ratchet = Ratchet {
        sub_ticks: Duration(120),
        count: 4,
        decay: NormalizedValue::new(1.0),
    };
    let g = ratchet_graph(ratchet, None);
    // Hits at 0, 120, 240, 360 (all within the 480-tick note); none at 480.
    for t in [0, 120, 240, 360] {
        assert_eq!(pitches_at(&g, &src, t), vec![60], "retrigger at {t}");
    }
    assert!(
        pitches_at(&g, &src, 480).is_empty(),
        "no retrigger once the note has ended"
    );
    assert!(pitches_at(&g, &src, 60).is_empty(), "nothing off the grid");

    // The dry onset is shortened to one subdivision (replaced, not layered).
    let mut pool = lookback_pool();
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        &src,
        PatternTick(0),
        HostKey::from(PatternId::new(1)),
        Bpm::DEFAULT,
        |_| true,
        None,
        Some(&mut pool),
        &mut buf,
    );
    assert_eq!(
        buf.notes()[0].duration,
        Some(Duration(120)),
        "dry onset shortened to the subdivision length"
    );
}

#[test]
fn ratchet_skips_notes_shorter_than_the_subdivision() {
    // A 100-tick note with a 120-tick subdivision never retriggers.
    let src = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MAX).with_duration(Duration(100)),
    ];
    let ratchet = Ratchet {
        sub_ticks: Duration(120),
        count: 4,
        decay: NormalizedValue::new(1.0),
    };
    let g = ratchet_graph(ratchet, None);
    assert_eq!(pitches_at(&g, &src, 0), vec![60], "the note plays once");
    assert!(pitches_at(&g, &src, 120).is_empty(), "no retrigger");
}

#[test]
fn ratchet_downstream_of_transform_retriggers_transformed_onsets() {
    // Quantize → Ratchet: an F#4 (66) snaps to G4 (67); every retrigger must
    // carry the quantized pitch (proves eval_prefix_at_tick under ratchet).
    use crate::note_processor::{PitchClass, ScaleMask, ScaleQuantize};
    let src = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(66), Velocity::MAX).with_duration(Duration(480)),
    ];
    let quant = ScaleQuantize {
        root: PitchClass::new(0),
        mask: ScaleMask::MAJOR,
    };
    let ratchet = Ratchet {
        sub_ticks: Duration(120),
        count: 3,
        decay: NormalizedValue::new(1.0),
    };
    let g = ratchet_graph(ratchet, Some(NoteProcessor::ScaleQuantize(quant)));
    assert_eq!(
        pitches_at(&g, &src, 120),
        vec![67],
        "retrigger is quantized"
    );
    assert_eq!(pitches_at(&g, &src, 240), vec![67], "and again");
}

#[test]
fn ratchet_freeze_equals_playback() {
    let src = vec![
        Note::new(NoteId(1), PatternTick(600), pitch(60), Velocity::MAX)
            .with_duration(Duration(480)),
    ];
    let ratchet = Ratchet {
        sub_ticks: Duration(120),
        count: 4,
        decay: NormalizedValue::new(0.8),
    };
    let g = ratchet_graph(ratchet, None);
    let host = HostKey::from(PatternId::new(1));
    let walk_end = 960u32 + g.max_strum_tail() + g.max_delay_tail();
    let baked: Vec<(u32, u8)> = g
        .bake(&src, PatternTick(walk_end), host, Bpm::DEFAULT, None)
        .iter()
        .map(|(t, n)| (t.0, n.pitch.as_midi()))
        .collect();
    let mut playback = Vec::new();
    for t in 0..walk_end {
        for p in pitches_at(&g, &src, t) {
            playback.push((t, p));
        }
    }
    assert_eq!(baked, playback, "ratchet freeze bakes exactly what plays");
}

// ── Transformed-stream NoteEnvelope (StreamOnset trigger) ──

/// A graph whose spine is a Euclidean generator emitting onsets every
/// `step_len` ticks — a transformed stream with known onset ticks.
fn euclid_spine(step_len: u32) -> NoteGraph {
    let mut g = NoteGraph::new(NoteGraphId::new(0), "euclid");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 4,
            pulses: 4,
            rotation: 0,
            step_len: Duration(step_len),
            pitch: pitch(60),
            velocity: Velocity::MAX,
        }),
    )
    .expect("under cap");
    g.rebuild_derived().expect("valid");
    g
}

#[test]
fn stream_envelope_retriggers_per_transformed_onset() {
    // Euclidean onsets at 0, 240, 480, … ; a StreamOnset envelope restarts on
    // each and decays over 120 ticks between them.
    let g = euclid_spine(240);
    let env = NoteEnvelope {
        attack: Duration(0),
        decay: Duration(120),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::StreamOnset,
    };
    let host = HostKey::from(PatternId::new(1));
    let val = |tick: u32| {
        let mut pool = lookback_pool();
        g.envelope_stream_value(
            &env,
            PatternTick(tick),
            &[],
            host,
            Bpm::DEFAULT,
            &|_| true,
            None,
            &mut pool,
        )
    };
    assert!((val(240) - 1.0).abs() < 1e-4, "peak at the onset");
    assert!((val(300) - 0.5).abs() < 1e-4, "half-decayed 60 ticks in");
    assert!(val(360).abs() < 1e-4, "fully decayed before the next onset");
    assert!(
        (val(480) - 1.0).abs() < 1e-4,
        "retriggered on the next onset"
    );
}

#[test]
fn stream_envelope_is_seek_order_independent() {
    let g = euclid_spine(240);
    let env = NoteEnvelope {
        attack: Duration(0),
        decay: Duration(120),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::StreamOnset,
    };
    let host = HostKey::from(PatternId::new(1));
    let val = |tick: u32| {
        let mut pool = lookback_pool();
        g.envelope_stream_value(
            &env,
            PatternTick(tick),
            &[],
            host,
            Bpm::DEFAULT,
            &|_| true,
            None,
            &mut pool,
        )
    };
    let ticks = [240u32, 300, 360, 480, 540];
    let fwd: Vec<f32> = ticks.iter().map(|&t| val(t)).collect();
    let bwd: Vec<f32> = ticks.iter().rev().map(|&t| val(t)).collect();
    let bwd_reordered: Vec<f32> = bwd.into_iter().rev().collect();
    assert_eq!(fwd, bwd_reordered, "value is independent of probe order");
}

#[test]
fn source_onset_envelope_reads_source_not_stream() {
    // Default trigger still reads raw source: with an empty source it stays 0
    // even though the Euclidean stream is emitting onsets.
    let g = euclid_spine(240);
    let env = NoteEnvelope {
        attack: Duration(0),
        decay: Duration(120),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::SourceOnset,
    };
    let host = HostKey::from(PatternId::new(1));
    let mut pool = lookback_pool();
    // `SourceOnset` never enters the stream backward-scan; it reads `source`.
    let stream_val = g.envelope_stream_value(
        &env,
        PatternTick(240),
        &[],
        host,
        Bpm::DEFAULT,
        &|_| true,
        None,
        &mut pool,
    );
    // The stream method itself still finds the Euclidean onset (it is the
    // trigger-agnostic scan); the point is that `expand_impl` only routes a
    // `StreamOnset` env here — a `SourceOnset` env uses `value(tick, source)`.
    assert!((stream_val - 1.0).abs() < 1e-4);
    assert!(
        env.value(PatternTick(240), &[]).abs() < 1e-4,
        "source-onset value is 0 with an empty source"
    );
}

#[test]
fn stream_envelope_wired_into_its_own_spine_terminates() {
    // A StreamOnset envelope modulating a spine node it also reads through the
    // terminal is the recursion hazard the `in_lookback` guard defuses. Build
    // Euclidean → ProbabilityGate, with the envelope driving the gate
    // threshold, and confirm a full expansion just returns (no hang).
    let mut g = NoteGraph::new(NoteGraphId::new(0), "recur");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 4,
            pulses: 4,
            rotation: 0,
            step_len: Duration(240),
            pitch: pitch(60),
            velocity: Velocity::MAX,
        }),
    )
    .expect("under cap");
    g.try_insert_node(
        id(2),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
    )
    .expect("under cap");
    g.try_insert_node(
        id(3),
        NoteModuleConfig::NoteEnvelope(NoteEnvelope {
            attack: Duration(0),
            decay: Duration(120),
            peak: NormalizedValue::new(1.0),
            trigger: EnvelopeTrigger::StreamOnset,
        }),
    )
    .expect("under cap");
    g.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("valid");
    g.try_connect(NoteConnection::value(id(3), id(2), 0))
        .expect("valid");
    g.rebuild_derived().expect("valid");
    // If the guard failed this would recurse forever; reaching the assert is
    // the test. Output stays within the buffer cap.
    for t in [0u32, 240, 300, 480] {
        let pitches = pitches_at(&g, &[], t);
        assert!(pitches.len() <= MAX_EXPANSION_EVENTS_PER_TICK);
    }
}

#[test]
fn stream_envelope_retriggers_on_dry_onsets_not_echoes() {
    // Gate → Delay spine with a dry onset at 0 and an echo at 480. The
    // envelope's probes run with an empty look-back pool (the anti-stall
    // rule), so at tick 500 it keys on the dry onset (elapsed 500), NOT the
    // echo at 480 (elapsed 20). Measured rationale in the method doc.
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MAX,
    )];
    let mut g = NoteGraph::new(NoteGraphId::new(0), "echoed");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::ProbabilityGate(ProbabilityGate::default()),
    )
    .expect("under cap");
    g.try_insert_node(
        id(2),
        NoteModuleConfig::NoteDelay(NoteDelay {
            delay_ticks: Duration(480),
            repeats: 1,
            feedback: NormalizedValue::new(1.0),
        }),
    )
    .expect("under cap");
    g.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("valid");
    g.rebuild_derived().expect("valid");
    let env = NoteEnvelope {
        attack: Duration(0),
        decay: Duration(960),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::StreamOnset,
    };
    let mut pool = lookback_pool();
    let v = g.envelope_stream_value(
        &env,
        PatternTick(500),
        &src,
        HostKey::from(PatternId::new(1)),
        Bpm::DEFAULT,
        &|_| true,
        None,
        &mut pool,
    );
    let dry = 1.0 - 500.0 / 960.0;
    assert!(
        (v - dry).abs() < 1e-4,
        "keys on the dry onset (expected {dry}), got {v} — an echo retrigger \
         would read {}",
        1.0 - 20.0 / 960.0
    );
}

// ── Regression locks for the phase-11 review fixes ──

#[test]
fn source_onset_envelope_window_is_uncapped() {
    // A long pad envelope keeps its full ramp on the cheap SourceOnset path
    // (the MAX_NOTE_DELAY_TICKS cap belongs to the StreamOnset scan only).
    let env = NoteEnvelope {
        attack: Duration(10_000),
        decay: Duration(0),
        peak: NormalizedValue::new(1.0),
        trigger: EnvelopeTrigger::SourceOnset,
    };
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MAX,
    )];
    let v = env.value(PatternTick(8000), &src);
    assert!((v - 0.8).abs() < 1e-4, "mid-attack level survives, got {v}");
}

#[test]
fn ratchet_last_retrigger_clips_to_the_note_end() {
    // Note dur 130, sub 120: the retrigger at +120 has 10 ticks left, not a
    // full subdivision spilling past the note's end.
    let src = vec![
        Note::new(NoteId(1), PatternTick(0), pitch(60), Velocity::MAX).with_duration(Duration(130)),
    ];
    let ratchet = Ratchet {
        sub_ticks: Duration(120),
        count: 4,
        decay: NormalizedValue::new(1.0),
    };
    let g = ratchet_graph(ratchet, None);
    let mut pool = lookback_pool();
    let mut buf = ExpansionBuffer::new();
    g.expand_at_tick(
        &src,
        PatternTick(120),
        HostKey::from(PatternId::new(1)),
        Bpm::DEFAULT,
        |_| true,
        None,
        Some(&mut pool),
        &mut buf,
    );
    assert_eq!(buf.notes().len(), 1);
    assert_eq!(
        buf.notes()[0].duration,
        Some(Duration(10)),
        "the last retrigger is clipped to the remaining note span"
    );
}

#[test]
fn delay_tail_matches_what_the_emit_loop_can_reach() {
    // A step beyond the look-back cap emits nothing — its tail is 0, so the
    // freeze walk is not extended for echoes that cannot fire.
    let dead = delay_graph(
        NoteDelay {
            delay_ticks: Duration(8000),
            repeats: 3,
            feedback: NormalizedValue::new(0.8),
        },
        None,
    );
    assert_eq!(dead.max_delay_tail(), 0, "step beyond cap reaches nothing");
    let src = vec![Note::new(
        NoteId(1),
        PatternTick(0),
        pitch(60),
        Velocity::MAX,
    )];
    assert!(pitches_at(&dead, &src, 8000).is_empty(), "and never echoes");

    // A step that crosses the cap mid-series reaches only the k·step ≤ cap
    // probes: 2 × 3000 = 6000, not min(3 × 3000, 7680) = 7680.
    let partial = delay_graph(
        NoteDelay {
            delay_ticks: Duration(3000),
            repeats: 3,
            feedback: NormalizedValue::new(0.8),
        },
        None,
    );
    assert_eq!(partial.max_delay_tail(), 6000);
}

#[test]
fn generator_headed_spine_gets_no_freeze_tail() {
    // Euclid → Delay: the generator emits fresh onsets at every walked tick,
    // so a tail-extended freeze would bake phantom hits past the pattern end.
    // The walk tail is therefore 0, even though the delay itself has reach.
    let mut g = NoteGraph::new(NoteGraphId::new(0), "euclid-delay");
    g.try_insert_node(
        id(1),
        NoteModuleConfig::Euclidean(EuclideanGenerator {
            steps: 4,
            pulses: 4,
            rotation: 0,
            step_len: Duration(240),
            pitch: pitch(60),
            velocity: Velocity::MAX,
        }),
    )
    .expect("under cap");
    g.try_insert_node(
        id(2),
        NoteModuleConfig::NoteDelay(NoteDelay {
            delay_ticks: Duration(480),
            repeats: 3,
            feedback: NormalizedValue::new(0.6),
        }),
    )
    .expect("under cap");
    g.try_connect(NoteConnection::stream(id(1), id(2)))
        .expect("valid");
    g.rebuild_derived().expect("valid");
    assert_eq!(g.max_delay_tail(), 1440, "the delay itself has reach");
    assert_eq!(
        g.max_walk_tail(),
        0,
        "but a generator-headed spine bakes no tail"
    );
    // A source-headed graph keeps its tail (the policy is head-specific).
    let source_headed = delay_graph(
        NoteDelay {
            delay_ticks: Duration(480),
            repeats: 3,
            feedback: NormalizedValue::new(0.6),
        },
        None,
    );
    assert_eq!(source_headed.max_walk_tail(), 1440);
}
