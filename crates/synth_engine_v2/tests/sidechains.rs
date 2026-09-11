//! Current-quantum sidechain timing and path compensation.
mod common;
use synth_core::{Decibels, Milliseconds, Ratio};
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::dynamics::{CompressorDetector, CompressorSettings};
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::offline::render_offline;
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, CutoffFrequency, NormalizedLevel};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor};

fn settings() -> CompressorSettings {
    CompressorSettings::new(
        Decibels::new(-20.0),
        Ratio::new(10.0),
        Milliseconds::new(0.1),
        Milliseconds::new(10.0),
        Decibels::new(0.0),
        NormalizedLevel::FULL,
        CompressorDetector::External {
            cutoff: CutoffFrequency::new(20.0).expect("cutoff"),
        },
    )
    .expect("settings")
}
fn graph(delay: u64, connected: bool, detector_level: f32) -> GraphIr {
    let mut b = GraphIr::builder();
    for (id, kind) in [
        (
            9,
            IrNodeKind::Constant {
                level: Amplitude::new(0.5).expect("level"),
            },
        ),
        (
            8,
            IrNodeKind::Constant {
                level: Amplitude::new(detector_level).expect("level"),
            },
        ),
        (
            7,
            IrNodeKind::Latency {
                frames: FrameCount::new(delay),
            },
        ),
        (
            2,
            IrNodeKind::Compressor {
                settings: settings(),
            },
        ),
        (1, IrNodeKind::Output),
    ] {
        b = b.node(NodeId::new(id), kind, ExecutionScope::Global);
    }
    for (from, to, port) in [(9, 2, 0), (8, 7, 0), (2, 1, 0)] {
        b = b.connect(
            (NodeId::new(from), PortId::FIRST),
            (NodeId::new(to), PortId::new(port)),
            SignalDomain::Audio,
        );
    }
    if connected {
        b = b.connect(
            (NodeId::new(7), PortId::FIRST),
            (NodeId::new(2), PortId::new(1)),
            SignalDomain::Audio,
        );
    }
    b.build().expect("graph")
}
fn plan(ir: &GraphIr) -> synth_engine_v2::plan::CompiledPlan {
    compile(
        ir,
        &RenderConfig::new(common::profile(4096, ChannelLayout::Stereo)),
    )
    .into_plan()
    .expect("plan")
}
fn samples(ir: &GraphIr) -> Vec<f32> {
    render_offline(plan(ir), FrameCount::new(2048), PlanPosition::ZERO, &[]).expect("render")
}

#[test]
fn detector_changes_gain_without_becoming_an_audible_input() {
    let loud = samples(&graph(0, true, 1.0));
    let silent = samples(&graph(0, true, 0.0));
    let absent = samples(&graph(0, false, 0.0));
    assert_ne!(loud, silent);
    assert_ne!(
        silent, absent,
        "a silent cable must not select the unpatched fallback"
    );
    assert!(loud[2048..].iter().all(|value| *value < 0.1));
    assert!(silent[2048..].iter().all(|value| *value == 0.5));
    let internal = CompressorSettings::default();
    assert_eq!(internal.detector(), CompressorDetector::Internal);
}

#[test]
fn detector_latency_aligns_main_audio_and_is_independent_of_callback_size() {
    let p = plan(&graph(137, true, 1.0));
    assert_eq!(p.added_latency(), FrameCount::new(201));
    assert!(
        p.path_latencies()
            .edges()
            .iter()
            .any(|edge| edge.compensation() == FrameCount::new(137))
    );
    let drive = |pattern: &[usize]| {
        let (_, mut renderer) = StreamControl::open(
            p.clone(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("open");
        let mut out = vec![0.0; 4096];
        let mut at = 0;
        let mut index = 0;
        while at < out.len() {
            let n = pattern[index % pattern.len()].min((out.len() - at) / 2);
            renderer
                .render(
                    AudioBlockMut::new(&mut out[at..at + n * 2], n, ChannelLayout::Stereo)
                        .expect("shape"),
                    TimedEvents::EMPTY,
                )
                .expect("render");
            at += n * 2;
            index += 1;
        }
        out
    };
    let expected = drive(&[2048]);
    assert!(expected[..402].iter().all(|s| *s == 0.0));
    assert!(expected[402] > 0.0);
    for pattern in [&[64][..], &[256], &[13, 257, 1, 63]] {
        assert_eq!(expected, drive(pattern));
    }
}

#[test]
fn invalid_dynamics_are_rejected_at_construction() {
    for threshold in [f32::NAN, f32::INFINITY, -61.0, 1.0] {
        assert!(
            CompressorSettings::new(
                Decibels::new(threshold),
                Ratio::new(4.0),
                Milliseconds::new(10.0),
                Milliseconds::new(100.0),
                Decibels::new(0.0),
                NormalizedLevel::FULL,
                CompressorDetector::Internal
            )
            .is_err()
        );
    }
    assert!(
        CompressorSettings::new(
            Decibels::new(-20.0),
            Ratio::new(4.0),
            Milliseconds::new(10.0),
            Milliseconds::new(100.0),
            Decibels::new(0.0),
            NormalizedLevel::FULL,
            CompressorDetector::External {
                cutoff: CutoffFrequency::new(501.0).expect("positive")
            }
        )
        .is_err()
    );
}
