use crate::quantities::EventCount;
use crate::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    profile::HostProfile,
    quantities::{Amplitude, ChannelLayout, Frequency, ParameterValue, SampleRate},
    stream::StreamControl,
    time::{FrameCount, PlanPosition, QuantumOffset, SampleTime, StreamAnchor},
};

#[test]
fn undersized_timed_scratch_preserves_required_and_rejected_writes() {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(64),
        ChannelLayout::Mono,
    )
    .expect("profile");
    let ir = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::A4,
                amplitude: Amplitude::UNITY,
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
        .expect("graph");
    let plan = compile(&ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("plan");
    let (_control, mut renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("renderer");
    let target = *renderer
        .plan
        .parameter_targets()
        .first()
        .expect("sample control");
    let node = target.node.index();
    renderer.timed_controls.truncate(1);
    renderer.control_starts.fill(0);
    renderer.control_fill.fill(0);
    renderer.rehearsal_usage.begin_controls();
    renderer.push_node_control(
        node,
        QuantumOffset::ZERO,
        target.control,
        ParameterValue::ZERO,
    );
    renderer.push_node_control(
        node,
        QuantumOffset::ZERO,
        target.control,
        ParameterValue::ONE,
    );
    let usage = renderer.rehearsal_usage().expect("observations");
    assert_eq!(usage.timed_controls_required, EventCount::measured(2));
    assert_eq!(usage.timed_controls_written, EventCount::measured(1));
    assert_eq!(usage.rejected_control_writes, EventCount::measured(1));
    assert_eq!(renderer.control_fill.get(node).copied(), Some(1));
    assert_eq!(renderer.timed_controls[0].value, ParameterValue::ZERO);
    renderer.rehearsal_usage.begin_controls();
    assert_eq!(renderer.rehearsal_usage().expect("retained peaks"), usage);
    renderer
        .rehearsal_usage
        .require_representable_add(u32::MAX, 1);
    assert!(matches!(
        renderer.rehearsal_usage(),
        Err(crate::render::RehearsalUsageError::Counter(
            "renderer internal counter"
        ))
    ));
}
