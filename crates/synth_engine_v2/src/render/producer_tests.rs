//! The merge must preserve external-first ties without moving a later external
//! control ahead of an earlier internal one.
use super::*;
#[test]
fn internal_controls_merge_by_offset_with_external_first_on_equal_offsets() {
    use crate::{
        compile::{RenderConfig, compile},
        ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
        node::kernels::ControlIndex,
        plan::NodeSlot,
        producer_programs::InternalEvent,
        profile::HostProfile,
        quantities::{Amplitude, Frequency, SampleRate},
        time::{PlanPosition, QuantumOffset},
    };
    let host = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(64),
        ChannelLayout::Mono,
    )
    .unwrap();
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).unwrap(),
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
        .unwrap();
    let plan = compile(&graph, &RenderConfig::new(host))
        .into_plan()
        .unwrap();
    let (_control, mut renderer) = crate::stream::StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap();
    // A merge fixture, not an admitted external writer to the producer-owned Delay.
    renderer.control_starts[0] = 0;
    renderer.control_starts[1] = 3;
    renderer.control_fill[0] = 2;
    for (index, offset) in [5, 10].into_iter().enumerate() {
        renderer.timed_controls[index] = TimedControl {
            offset: QuantumOffset::new(offset).unwrap(),
            control: ControlIndex::RESET,
            value: ParameterValue::ZERO,
        };
    }
    renderer.producer_runtime.arena = vec![Some(InternalEvent {
        node: NodeSlot::new(0),
        offset: QuantumOffset::new(5).unwrap(),
    })]
    .into_boxed_slice();
    renderer.producer_runtime.length = 1;
    assert_eq!(
        crate::render_allocation::count_allocs(|| renderer.merge_internal_controls().unwrap()),
        0
    );
    let controls = &renderer.timed_controls[..3];
    assert_eq!(
        controls
            .iter()
            .map(|control| control.offset.as_u16())
            .collect::<Vec<_>>(),
        [5, 5, 10]
    );
    assert_eq!(
        controls
            .iter()
            .map(|control| control.value)
            .collect::<Vec<_>>(),
        [
            ParameterValue::ZERO,
            ParameterValue::ONE,
            ParameterValue::ZERO
        ]
    );
}
