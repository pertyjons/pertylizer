use super::*;
use crate::compile::{RenderConfig, compile};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::profile::HostProfile;
use crate::quantities::{Amplitude, Frequency, SampleRate};
use crate::stream::StreamControl;
use crate::time::PlanPosition;

fn renderer(layout: ChannelLayout) -> PreparedRenderer {
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).unwrap(),
                amplitude: Amplitude::new(0.5).unwrap(),
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
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        layout,
    )
    .unwrap();
    let plan = compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    StreamControl::open(
        plan,
        crate::time::StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap()
    .1
}

fn block(samples: &mut [f32], layout: ChannelLayout) -> AudioBlockMut<'_> {
    AudioBlockMut::new(samples, samples.len() / layout.channels(), layout).unwrap()
}

#[test]
fn idle_preserves_rendered_carry_and_freezes_oscillator_state() {
    for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
        let channels = layout.channels();
        let mut reference = renderer(layout);
        let mut expected = vec![0.0; 192 * channels];
        reference
            .render(block(&mut expected, layout), TimedEvents::EMPTY)
            .unwrap();
        let mut subject = renderer(layout);
        let original_anchor = subject.anchor;
        let mut first = vec![0.0; 100 * channels];
        subject
            .render(block(&mut first, layout), TimedEvents::EMPTY)
            .unwrap();
        assert_eq!(first, expected[..100 * channels]);
        assert_eq!(subject.clock(), SampleTime::new(64));

        let mut idle = vec![9.0; 67 * channels];
        subject.render_idle(block(&mut idle, layout)).unwrap();
        assert_eq!(
            idle[..28 * channels],
            expected[100 * channels..128 * channels]
        );
        assert!(idle[28 * channels..].iter().all(|sample| *sample == 0.0));
        assert_eq!(subject.clock(), SampleTime::new(128));
        assert_eq!(subject.anchor, original_anchor);

        // This isolated oscillator has no musical schedule. Rendering it directly
        // exposes its saved phase; a session must activate its schedule before resume.
        let mut resumed = vec![9.0; 89 * channels];
        subject
            .render(block(&mut resumed, layout), TimedEvents::EMPTY)
            .unwrap();
        assert!(resumed[..25 * channels].iter().all(|sample| *sample == 0.0));
        assert_eq!(resumed[25 * channels..], expected[128 * channels..]);
    }
}

#[test]
fn idle_output_and_clock_are_independent_of_callback_partition() {
    for partition in [512, 256, 64, 37, 1] {
        let mut subject = renderer(ChannelLayout::Mono);
        let mut output = [9.0; 1024];
        for chunk in output.chunks_mut(partition) {
            subject
                .render_idle(block(chunk, ChannelLayout::Mono))
                .unwrap();
        }
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(subject.clock(), SampleTime::new(960));
        assert_eq!(subject.carry_frames(), 0);
    }
}

#[test]
fn idle_refuses_wrong_layout_and_faults_on_oversized_callback() {
    let mut subject = renderer(ChannelLayout::Mono);
    let mut wrong = [1.0; 128];
    assert!(matches!(
        subject.render_idle(block(&mut wrong, ChannelLayout::Stereo)),
        Err(crate::diagnostics::RenderError::OutputBufferShape { .. })
    ));
    assert_eq!(subject.clock(), SampleTime::ZERO);
    assert_eq!(subject.carry_frames(), 64);
    let mut oversized = [1.0; 513];
    assert!(matches!(
        subject.render_idle(block(&mut oversized, ChannelLayout::Mono)),
        Err(crate::diagnostics::RenderError::OversizedCallback { .. })
    ));
    assert!(oversized.iter().all(|sample| *sample == 0.0));
    assert_eq!(subject.carry_frames(), 0);
    assert!(subject.diagnostics().needs_reprepare());
    let mut next = [1.0; 64];
    assert_eq!(
        subject.render_idle(block(&mut next, ChannelLayout::Mono)),
        Err(crate::diagnostics::RenderError::NeedsReprepare)
    );
    assert_eq!(next, [0.0; 64]);
}

#[test]
fn idle_clock_exhaustion_is_terminal_without_allocating_or_reclaiming() {
    let mut subject = renderer(ChannelLayout::Mono);
    subject.clock = SampleTime::new(u64::MAX - 63);
    subject.carry_frames = 0;
    let mut output = [1.0; 64];
    let allocations = crate::render_allocation::count_allocs(|| {
        assert!(matches!(
            subject.render_idle(block(&mut output, ChannelLayout::Mono)),
            Err(crate::diagnostics::RenderError::ClockExhausted(_))
        ));
    });
    assert_eq!(allocations, 0);
    assert!(subject.diagnostics().needs_reprepare());
    assert_eq!(subject.clock(), SampleTime::new(u64::MAX - 63));
    assert_eq!(output, [0.0; 64]);
}

#[test]
fn idle_first_call_and_empty_calls_neither_allocate_nor_advance_dsp() {
    let mut subject = renderer(ChannelLayout::Stereo);
    let mut output = [1.0; 1024];
    let allocations = crate::render_allocation::count_allocs(|| {
        subject
            .render_idle(block(&mut [], ChannelLayout::Stereo))
            .unwrap();
        assert_eq!(subject.clock(), SampleTime::ZERO);
        subject
            .render_idle(block(&mut output, ChannelLayout::Stereo))
            .unwrap();
    });
    assert_eq!(allocations, 0);
    assert_eq!(output, [0.0; 1024]);
    assert_eq!(subject.clock(), SampleTime::new(448));
}
