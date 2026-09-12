//! Same-epoch test seam: public stream construction still issues a fresh epoch.

use super::*;
use crate::compile::{RenderConfig, compile};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{Amplitude, ChannelLayout, SampleRate};
use crate::render::AudioBlockMut;
use crate::schedule::{CompiledEventScheduler, ScheduledRenderError};
use crate::time::FrameCount;
use crate::transport::ActivationRefused;

struct Pair {
    control: StreamControl,
    renderer: PreparedRenderer,
    scheduler: CompiledEventScheduler,
    stream: AdmittedCompiledStream,
    arbiter: PublicationArbiter,
}

fn crossed_tables() -> (Pair, Pair) {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant {
                level: Amplitude::new(0.25).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(2), PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    let plan = compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    let anchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
    let (first, first_renderer) = StreamControl::open(plan.clone(), anchor).unwrap();
    let (mut second, discarded_renderer) = StreamControl::open(plan, anchor).unwrap();
    drop(discarded_renderer);
    // Model a future same-device readmission, including cloning the same PlanId.
    second.epoch = first.epoch;
    let second_renderer = PreparedRenderer::prepare(
        Arc::clone(second.plan_arc()),
        anchor,
        second.epoch(),
        second.table_id(),
    )
    .unwrap();
    assert_eq!(first.plan_id(), second.plan_id());
    assert_eq!(first.epoch(), second.epoch());
    assert_ne!(first.table_id(), second.table_id());
    let prepare = |mut control: StreamControl, renderer: PreparedRenderer| {
        let stream = AdmittedCompiledStream::admit(control.plan(), &[]).unwrap();
        let scheduler = CompiledEventScheduler::prepare(&mut control, &stream).unwrap();
        Pair {
            control,
            renderer,
            scheduler,
            stream,
            arbiter: PublicationArbiter::prepare(&profile).unwrap(),
        }
    };
    (
        prepare(first, first_renderer),
        prepare(second, second_renderer),
    )
}

fn candidate(pair: &mut Pair) -> Box<TransportActivation> {
    pair.control
        .plan_activation(
            &pair.stream,
            ActivationRequest {
                at: SampleTime::ZERO,
                position: PlanPosition::ZERO,
                loop_interval: None,
            },
        )
        .unwrap()
}

#[test]
fn a_crossed_same_epoch_renderer_refuses_before_render_or_adoption() {
    let (mut first, mut second) = crossed_tables();
    let activation = candidate(&mut first);
    let before = second.renderer.diagnostics().refused_activations();
    let mut rejected = None;
    let mut samples = [9.0; 128];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            rejected = Some(
                first
                    .scheduler
                    .offer(&mut second.renderer, activation)
                    .unwrap_err(),
            );
            let error = first
                .scheduler
                .render(
                    &mut second.renderer,
                    &mut first.arbiter,
                    AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap(),
                )
                .unwrap_err();
            assert!(matches!(error, ScheduledRenderError::TableMismatch { .. }));
        }),
        0
    );
    let (activation, refusal) = rejected.unwrap();
    assert!(matches!(
        refusal,
        ActivationRefused::ForeignRendererTable { .. }
    ));
    assert_eq!(samples, [9.0; 128]);
    assert_eq!(second.renderer.clock(), SampleTime::ZERO);
    assert_eq!(second.renderer.diagnostics().refused_activations(), before);
    assert!(first.scheduler.collect().is_none());
    first.control.withdraw(activation).unwrap();
    // The correct pair remains usable after both preflight refusals.
    let activation = candidate(&mut first);
    first
        .scheduler
        .offer(&mut first.renderer, activation)
        .unwrap();
    first
        .scheduler
        .render(
            &mut first.renderer,
            &mut first.arbiter,
            AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap(),
        )
        .unwrap();
    first
        .control
        .adopted(first.scheduler.collect().unwrap())
        .unwrap();
    assert_eq!(&samples[..64], &[0.0; 64]);
    assert_eq!(&samples[64..], &[0.25; 64]);
}

#[test]
fn a_foreign_same_epoch_candidate_returns_to_its_original_control() {
    let (mut first, mut second) = crossed_tables();
    let activation = candidate(&mut second);
    let mut rejected = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            rejected = Some(
                first
                    .scheduler
                    .offer(&mut first.renderer, activation)
                    .unwrap_err(),
            );
        }),
        0
    );
    let (activation, refusal) = rejected.unwrap();
    assert!(matches!(
        refusal,
        ActivationRefused::ForeignCandidateTable { .. }
    ));
    assert_eq!(first.renderer.diagnostics().refused_activations(), 1);
    assert!(first.scheduler.collect().is_none());
    let (activation, error) = first.control.withdraw(activation).unwrap_err();
    assert!(matches!(error, ActivationCollectError::ForeignTable { .. }));
    assert_eq!(second.control.outstanding_candidates(), 1);
    second.control.withdraw(activation).unwrap();
    assert_eq!(second.control.outstanding_candidates(), 0);
}

#[test]
fn an_adopted_same_epoch_activation_cannot_replace_another_controls_minter() {
    let (mut first, mut second) = crossed_tables();
    let activation = candidate(&mut second);
    second
        .scheduler
        .offer(&mut second.renderer, activation)
        .unwrap();
    second
        .scheduler
        .render(
            &mut second.renderer,
            &mut second.arbiter,
            AudioBlockMut::new(&mut [0.0; 128], 128, ChannelLayout::Mono).unwrap(),
        )
        .unwrap();
    let activation = second.scheduler.collect().unwrap();
    let first_table = first.control.table_id();
    let first_anchor = first.control.anchor();
    let first_sequence = first.control.in_force();
    let (activation, error) = first.control.adopted(activation).unwrap_err();
    assert!(matches!(error, ActivationCollectError::ForeignTable { .. }));
    assert_eq!(first.control.table_id(), first_table);
    assert_eq!(first.control.anchor(), first_anchor);
    assert_eq!(first.control.in_force(), first_sequence);
    assert_eq!(second.control.outstanding_candidates(), 1);
    second.control.adopted(activation).unwrap();
    assert_eq!(second.control.outstanding_candidates(), 0);
}
