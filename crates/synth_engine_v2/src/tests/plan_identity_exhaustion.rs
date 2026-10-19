//! Exhaustion must refuse a compile with a retained report, never reuse a plan ID.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn plan_identity_exhaustion_permanently_refuses_compilation_with_a_report() {
    let profile = HostProfile::harness(
        crate::quantities::SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(256),
        ChannelLayout::Mono,
    )
    .expect("valid profile");
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Silence,
            crate::ir::ExecutionScope::Global,
        )
        .node(
            NodeId::new(2),
            IrNodeKind::Output,
            crate::ir::ExecutionScope::Global,
        )
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(2), PortId::FIRST),
            crate::ir::SignalDomain::Audio,
        )
        .build()
        .expect("valid graph");
    let counter = AtomicU64::new(u64::MAX - 1);
    let last = compile_with_issuer(
        &graph,
        &RenderConfig::new(profile),
        ArenaPolicy::Reuse,
        || crate::plan::issue_plan_id_from(&counter),
    )
    .into_plan()
    .expect("last identity available");
    assert_eq!(last.id().as_u64(), u64::MAX);
    for _ in 0..4 {
        let outcome = compile_with_issuer(
            &graph,
            &RenderConfig::new(profile),
            ArenaPolicy::Reuse,
            || crate::plan::issue_plan_id_from(&counter),
        );
        assert_eq!(outcome.plan(), Err(&CompileError::PlanIdentityExhausted));
        assert!(
            !outcome.report().rows().is_empty(),
            "refusal retains its resource report"
        );
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
    assert_eq!(
        last.id().as_u64(),
        u64::MAX,
        "the final admitted plan is still distinct"
    );
}

#[test]
fn concurrent_plan_issuers_never_alias_at_the_ceiling() {
    let counter = std::sync::Arc::new(AtomicU64::new(u64::MAX - 32));
    let start_together = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let counter = std::sync::Arc::clone(&counter);
            let start_together = std::sync::Arc::clone(&start_together);
            std::thread::spawn(move || {
                start_together.wait();
                (0..8)
                    .filter_map(|_| crate::plan::issue_plan_id_from(&counter))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let mut ids = Vec::new();
    for worker in workers {
        ids.extend(worker.join().expect("issuer joined"));
    }
    assert_eq!(ids.len(), 32);
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 32);
    assert_eq!(ids.last().map(|id| id.as_u64()), Some(u64::MAX));
    assert!(crate::plan::issue_plan_id_from(&counter).is_none());
}

#[test]
fn exhausted_plan_identity_retains_every_advisory_warning_in_its_preflight_report() {
    let profile = HostProfile::harness(
        crate::quantities::SampleRate::new(44_100.0).expect("rate"),
        FrameCount::new(256),
        ChannelLayout::Mono,
    )
    .expect("profile");
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Output,
            crate::ir::ExecutionScope::Global,
        )
        .declaring(PlanDeclarations {
            note_producers: vec![crate::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(512),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(512),
            voices_per_instrument: crate::quantities::VoiceCount::measured(128),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("graph");
    let result = compile_with_issuer(
        &graph,
        &RenderConfig::new(profile),
        ArenaPolicy::Reuse,
        || None,
    );
    assert_eq!(result.plan(), Err(&CompileError::PlanIdentityExhausted));
    let exceeded: Vec<_> = result
        .report()
        .rows()
        .iter()
        .filter(|row| row.field().is_advisory() && row.fit() == Fit::Exceeds)
        .collect();
    assert!(
        !exceeded.is_empty(),
        "fixture must exceed an advisory budget"
    );
    for row in exceeded {
        assert!(result.warnings().iter().any(|warning| matches!(warning,
            CompileWarning::AdvisoryBudgetExceeded { field, predicted, permitted, .. }
                if *field == row.field() && *predicted == row.requested() && *permitted == row.available()
        )), "missing advisory warning: {row:?}");
    }
}
