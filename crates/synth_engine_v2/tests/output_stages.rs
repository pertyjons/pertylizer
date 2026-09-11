//! Frame positioning and unpatched/negative CV at the terminating stages.
mod common;
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::controller::BipolarLevel;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, NormalizedLevel, ParameterValue, SampleRate,
};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime};

#[test]
fn stereo_velocity_controls_land_on_the_same_frame_on_both_channels() {
    let source = NodeId::new(1);
    let stage = NodeId::new(2);
    let output = NodeId::new(3);
    let ir = GraphIr::builder()
        .node(
            source,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(
            stage,
            IrNodeKind::StereoVelocityScaler {
                sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Global,
        )
        .node(output, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (source, PortId::FIRST),
            (stage, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (stage, PortId::FIRST),
            (output, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("graph");
    let plan = compile(
        &ir,
        &RenderConfig::new(common::profile(256, ChannelLayout::Stereo)),
    )
    .into_plan()
    .expect("plan");
    let slot = plan
        .resolve_parameter(stage, parameters::VELOCITY_SCALER_VELOCITY)
        .expect("velocity");
    let events = [OfflineEvent::new(
        SampleTime::new(19),
        CompiledPayload::SetParameter {
            slot,
            value: ParameterValue::new(0.25).expect("value"),
        },
    )];
    let samples =
        render_offline(plan, FrameCount::new(256), PlanPosition::ZERO, &events).expect("render");
    for (frame, pair) in samples.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(
            *pair,
            if frame < 19 { [1.0; 2] } else { [0.25; 2] },
            "frame {frame}"
        );
    }
}

#[test]
fn voice_amplifier_uses_unity_for_unpatched_cv_and_clamps_negative_cv() {
    use synth_engine_v2::script::{ProjectSeed, ScriptIdentity, ScriptStateId};
    let source = NodeId::new(1);
    let stage = NodeId::new(2);
    let output = NodeId::new(3);
    let control = NodeId::new(4);
    for cv in [None, Some(-0.7), Some(0.0), Some(0.3), Some(1.0)] {
        let mut builder = GraphIr::builder()
            .node(
                source,
                IrNodeKind::Constant {
                    level: Amplitude::new(0.7).expect("level"),
                },
                ExecutionScope::Global,
            )
            .node(
                stage,
                IrNodeKind::VoiceAmplifier {
                    pan: BipolarLevel::new(0.37).expect("pan"),
                },
                ExecutionScope::Global,
            )
            .node(output, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (source, PortId::FIRST),
                (stage, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (stage, PortId::FIRST),
                (output, PortId::FIRST),
                SignalDomain::Audio,
            );
        if let Some(cv) = cv {
            let program = ScriptIdentity::new(control, ScriptStateId::new(1), ProjectSeed::new(1))
                .compile_control(
                    &format!("out = {cv}"),
                    SampleRate::new(48_000.0).expect("rate"),
                    &[],
                )
                .expect("script");
            builder = builder.script(program, ExecutionScope::Global).connect(
                (control, PortId::FIRST),
                (stage, PortId::new(1)),
                SignalDomain::Control,
            );
        }
        let plan = compile(
            &builder.build().expect("graph"),
            &RenderConfig::new(common::profile(256, ChannelLayout::Stereo)),
        )
        .into_plan()
        .expect("plan");
        let samples =
            render_offline(plan, FrameCount::new(256), PlanPosition::ZERO, &[]).expect("render");
        let (l, r) = synth_core::Gain::from_pan(synth_core::BipolarValue::new(0.37));
        let gain = cv.unwrap_or(1.0_f32).max(0.0);
        let expected = (0.7 * gain * l.as_f32() + 0.7 * gain * r.as_f32()) * 0.5;
        assert!(
            samples
                .iter()
                .all(|sample| sample.to_bits() == expected.to_bits()),
            "CV {cv:?}"
        );
    }
}
