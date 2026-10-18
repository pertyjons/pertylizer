//! EVD-0026: the saved `instrument-inserts` project through the private mixed lowering.
//!
//! `plans/v2/evidence/phase-09/EVD-0026-combined-host-capacity-selection.md` names this
//! project as the common host's compiled load. The mixed rehearsal cannot be enabled from
//! this crate (ADR-0078 clause 1). These tests compare its saved lowering with a
//! transcription in `synth_engine_v2/tests/support/instrument_inserts.rs`: the pinned
//! source, the ordinary and mixed lowerings equal to it exactly, the mixed lowering
//! differing only by its appended live range, and the actual lowered mixed plan
//! admitted with disjoint compiled and live targets. The two-source host uses this
//! transcription for its saved-project functional controls.
use super::*;
use crate::corpus::{CORPUS_DIR, CorpusManifest};
use sha2::{Digest, Sha256};
use synth_engine_v2::host::mixed_targets::MixedTargetAdmission;
use synth_engine_v2::ir::GraphIr;
use synth_engine_v2::quantities::{ChannelLayout, HeldNoteCount, SampleRate};
use synth_engine_v2::report::{ResourceAmount, ResourceField};
use synth_engine_v2::schedule::{AdmittedCompiledStream, PlanEvent};
use synth_engine_v2::time::PlanPosition;

#[path = "../../../../synth_engine_v2/tests/support/instrument_inserts.rs"]
mod instrument_inserts;

fn repository() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The transcription's 48 kHz; the block does not reach the graph.
fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(1_024),
        ChannelLayout::Stereo,
    )
    .expect("profile")
}

fn live() -> LiveNoteRange {
    let declared = instrument_inserts::live();
    LiveNoteRange::new(declared.simultaneous_notes, declared.simultaneous_holds)
        .expect("a nonempty live range")
}

fn lower(live: Option<LiveNoteRange>) -> (LoweredProject, Option<GraphIr>) {
    lower_with_profile(live, profile())
}

fn lower_with_profile(
    live: Option<LiveNoteRange>,
    profile: HostProfile,
) -> (LoweredProject, Option<GraphIr>) {
    let project = crate::project::ProjectFile::load(repository().join(instrument_inserts::SOURCE))
        .expect("the pinned project loads");
    let mut built = None;
    let lowered = lower_project_with(
        &project.instruments,
        &project.song,
        &project.global,
        profile,
        OutputPolicy::Parity,
        live,
        &mut built,
    );
    (lowered, built)
}

#[test]
fn the_transcription_matches_lowering_and_idle_live_audio_at_every_candidate_rate() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = HostProfile::harness(
            SampleRate::new(hz).expect("candidate rate"),
            FrameCount::new(64),
            ChannelLayout::Stereo,
        )
        .expect("candidate profile");
        let mut audio = Vec::new();
        for live in [None, Some(live())] {
            let (lowered, ir) = lower_with_profile(live, profile);
            assert_eq!(
                ir.expect("lowered graph"),
                instrument_inserts::ir(live.map(|_| instrument_inserts::live()))
            );
            let plan = lowered.plan.expect("saved project admits");
            let slot = plan
                .resolve_note(instrument_inserts::ENVELOPE)
                .expect("saved envelope");
            let expected =
                instrument_inserts::events_at(slot, profile.capabilities().sample_rate());
            let actual: Vec<_> = lowered
                .events
                .iter()
                .map(|event| (event.time().as_u64(), event.payload()))
                .collect();
            assert_eq!(actual, expected, "rate {hz}");
            let frames = expected.last().expect("last release").0 + 1_024;
            let rendered = render_offline(
                plan,
                FrameCount::new(frames),
                PlanPosition::ZERO,
                &lowered.events,
            )
            .expect("saved audio");
            assert!(rendered.iter().any(|sample| sample.abs() > 0.001));
            audio.push(rendered.into_iter().map(f32::to_bits).collect::<Vec<_>>());
        }
        assert_eq!(
            audio[0], audio[1],
            "idle live range changes compiled audio at {hz}"
        );
    }
}

/// The lowered events as `(frame, payload)`, the transcription's shape.
fn timeline(lowered: &LoweredProject) -> Vec<(u64, synth_engine_v2::schedule::CompiledPayload)> {
    lowered
        .events
        .iter()
        .map(|event| (event.time().as_u64(), event.payload()))
        .collect()
}

fn requested(lowered: &LoweredProject, field: ResourceField) -> u32 {
    match lowered
        .report
        .as_ref()
        .and_then(|report| report.row(field))
        .map(synth_engine_v2::report::ResourceRow::requested)
    {
        Some(ResourceAmount::Events(count)) => count.get(),
        other => panic!("{field:?} is not an event request: {other:?}"),
    }
}

#[test]
fn the_transcribed_source_is_corpus_0005_as_the_manifest_pins_it() {
    let bytes = std::fs::read(repository().join(instrument_inserts::SOURCE)).expect("source");
    assert_eq!(
        hex(&Sha256::digest(&bytes)),
        instrument_inserts::SOURCE_SHA256
    );
    let corpus = repository().join(CORPUS_DIR);
    let manifest = CorpusManifest::load(&corpus.join("manifest.json")).expect("manifest");
    let case = manifest
        .cases
        .iter()
        .find(|case| case.id.as_str() == "CORPUS-0005")
        .expect("the insert-chain case");
    assert_eq!(case.sha256, instrument_inserts::SOURCE_SHA256);
    assert_eq!(
        case.input_path(&corpus),
        repository().join(instrument_inserts::SOURCE)
    );
}

#[test]
fn the_mixed_lowering_differs_from_the_saved_graph_only_by_its_appended_live_range() {
    let (ordinary, ordinary_ir) = lower(None);
    let (mixed, mixed_ir) = lower(Some(live()));
    let ordinary_ir = ordinary_ir.expect("the ordinary graph builds");
    let mixed_ir = mixed_ir.expect("the mixed graph builds");
    assert_eq!(ordinary_ir, instrument_inserts::ir(None));
    assert_eq!(
        mixed_ir,
        instrument_inserts::ir(Some(instrument_inserts::live()))
    );
    assert_eq!(mixed.diagnostics, ordinary.diagnostics);
    assert!(
        mixed
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity() != Severity::Refused),
        "{:?}",
        mixed.diagnostics
    );

    let ordinary_plan = ordinary.plan.as_ref().expect("the ordinary plan admits");
    let mixed_plan = mixed.plan.as_ref().expect("the mixed plan admits");
    let notes = |count| HeldNoteCount::measured(count);
    assert_eq!(ordinary_plan.note_producer_ranges(), [notes(2)]);
    assert_eq!(mixed_plan.note_producer_ranges(), [notes(2), notes(4)]);
    for plan in [ordinary_plan, mixed_plan] {
        assert_eq!(
            plan.compiled_note_producer().map(|id| id.as_u16()),
            Some(0),
            "the compiled producer keeps the first range"
        );
    }
    assert_eq!(ordinary_plan.voice_instances().get(), 2);
    assert_eq!(mixed_plan.voice_instances().get(), 6);
    assert_eq!(mixed_plan.node_timings(), ordinary_plan.node_timings());
    assert_eq!(
        mixed_plan.instance_groups().len(),
        ordinary_plan.instance_groups().len()
    );
    assert_eq!(
        mixed_plan.sum_groups().len(),
        ordinary_plan.sum_groups().len()
    );

    // The same timeline, each on its own plan's one envelope.
    for (lowered, plan) in [(&ordinary, ordinary_plan), (&mixed, mixed_plan)] {
        let slot = plan
            .resolve_note(instrument_inserts::ENVELOPE)
            .expect("the envelope plays notes");
        assert_eq!(
            timeline(lowered),
            instrument_inserts::events_at(slot, profile().capabilities().sample_rate())
        );
    }
    assert_eq!(mixed.lowered_events, ordinary.lowered_events);
    assert_eq!(mixed.lowered_events.get(), 6);
    assert_eq!(mixed.lowered_frames, ordinary.lowered_frames);

    // Admission charges the live range and nothing the arrangement asked for.
    for field in [
        ResourceField::CompiledEventShare,
        ResourceField::SessionEventShare,
        ResourceField::LiveEventShare,
        ResourceField::ReleaseEventShare,
    ] {
        assert_eq!(
            requested(&mixed, field),
            requested(&ordinary, field),
            "{field:?}"
        );
    }
    assert_eq!(requested(&ordinary, ResourceField::ReleaseHoldCapacity), 0);
    assert_eq!(requested(&mixed, ResourceField::ReleaseHoldCapacity), 4);
}

#[test]
fn the_lowered_mixed_plan_admits_disjoint_compiled_and_live_targets() {
    let (ordinary, _) = lower(None);
    let (mixed, _) = lower(Some(live()));
    let ordinary_ops = ordinary.plan.as_ref().expect("ordinary plan").ops().len();
    let plan = mixed.plan.expect("mixed plan");
    let mixed_ops = plan.ops().len();
    let slot = plan
        .resolve_note(instrument_inserts::ENVELOPE)
        .expect("the envelope plays notes");
    let events: Vec<PlanEvent> = mixed
        .events
        .iter()
        .map(|event| PlanEvent::new(PlanPosition::new(event.time().as_u64()), event.payload()))
        .collect();
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the compiled stream fits");
    let binding = MixedTargetAdmission::admit(plan, stream, slot)
        .unwrap_or_else(|failure| panic!("the saved graph cannot partition: {}", failure.reason()));
    assert_eq!(binding.compiled_slots(), [slot]);
    assert_eq!(binding.live_slot(), slot);

    // Two compiled and four live instances of each local node, one shared voice sum per
    // instance, and the inserts, channel and master global. The ordinary plan is the same
    // shape with two instances, so the mixed plan adds four instances of each local node
    // and sum step and nothing else.
    let partition = binding.instance_partition();
    let local = partition.compiled_nodes().len() / 2;
    let sums = partition.shared_sum_nodes().len() / 6;
    assert!(local > 0 && sums > 0);
    assert_eq!(partition.compiled_nodes().len(), 2 * local);
    assert_eq!(partition.live_nodes().len(), 4 * local);
    assert_eq!(partition.shared_sum_nodes().len(), 6 * sums);
    assert_eq!(mixed_ops - ordinary_ops, 4 * (local + sums));
    assert!(partition.shared_sum_rows().is_empty());
    assert_eq!(
        partition.live_rows().len(),
        2 * partition.compiled_rows().len()
    );
    // Every compiled row is restored over the compiled producer's two instances.
    assert!(!partition.restoration_groups().is_empty());
    for group in partition.restoration_groups() {
        assert_eq!(
            (group.instances().first(), group.instances().count()),
            (0, 2),
            "{group:?}"
        );
    }
}

#[test]
fn an_empty_live_range_is_refused_and_an_unrepresentable_one_refuses_the_project() {
    assert_eq!(
        LiveNoteRange::new(HeldNoteCount::measured(0), EventCount::measured(4)),
        None
    );
    assert_eq!(
        LiveNoteRange::new(HeldNoteCount::measured(2), EventCount::measured(3)),
        None
    );
    let huge = LiveNoteRange::new(HeldNoteCount::measured(u32::MAX), EventCount::NONE)
        .expect("a nonempty range");
    let (lowered, built) = lower(Some(huge));
    assert!(lowered.plan.is_none());
    assert!(
        built.is_none(),
        "no graph is built for an unrepresentable plan"
    );
    assert!(lowered.diagnostics.iter().any(|diagnostic| {
        diagnostic.severity() == Severity::Refused
            && *diagnostic.subject() == ProjectSubject::Project
    }));
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    reason = "a mean of finite f32 values fits f32; round once"
)]
fn mono_saved_lowering_keeps_the_stereo_graph_and_marks_the_explicit_terminal_fold() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for live in [None, Some(live())] {
            let profile = |layout| {
                HostProfile::harness(
                    SampleRate::new(hz).expect("rate"),
                    FrameCount::new(64),
                    layout,
                )
                .expect("profile")
            };
            let (stereo, stereo_ir) = lower_with_profile(live, profile(ChannelLayout::Stereo));
            let (mono, mono_ir) = lower_with_profile(live, profile(ChannelLayout::Mono));
            assert_eq!(
                stereo_ir.expect("stereo graph"),
                instrument_inserts::ir(live.map(|_| instrument_inserts::live()))
            );
            assert_eq!(
                mono_ir.expect("mono graph"),
                instrument_inserts::ir_with_layout(
                    live.map(|_| instrument_inserts::live()),
                    ChannelLayout::Mono
                )
            );
            assert!(
                mono.diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.reason(),
                    LoweringReason::MonoOutputFold
                ))
            );
            assert!(!Fidelity::of(&mono.diagnostics).admits_parity_comparison());
            assert!(
                !stereo.diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.reason(),
                    LoweringReason::MonoOutputFold
                ))
            );
            for lowered in [&mono, &stereo] {
                let slot = lowered
                    .plan
                    .as_ref()
                    .expect("admitted plan")
                    .resolve_note(instrument_inserts::ENVELOPE)
                    .expect("saved envelope");
                assert_eq!(
                    timeline(lowered),
                    instrument_inserts::events_at(
                        slot,
                        profile(ChannelLayout::Mono).capabilities().sample_rate()
                    )
                );
            }
            let frames = stereo
                .events
                .last()
                .expect("compiled release")
                .time()
                .as_u64()
                + 1_024;
            let stereo = render_offline(
                stereo.plan.expect("stereo plan"),
                FrameCount::new(frames),
                PlanPosition::ZERO,
                &stereo.events,
            )
            .expect("stereo render");
            let mono = render_offline(
                mono.plan.expect("mono plan"),
                FrameCount::new(frames),
                PlanPosition::ZERO,
                &mono.events,
            )
            .expect("mono render");
            let expected: Vec<_> = stereo
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[left, right]| {
                    (((f64::from(*left) + f64::from(*right)) * 0.5) as f32).to_bits()
                })
                .collect();
            assert_eq!(
                mono.into_iter().map(f32::to_bits).collect::<Vec<_>>(),
                expected,
                "rate {hz}, live {live:?}"
            );
        }
    }
}
