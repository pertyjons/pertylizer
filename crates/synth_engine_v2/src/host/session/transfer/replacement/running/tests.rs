use super::super::{PlanReplacementOutcome, PlanReplacementRefusal};
use super::*;
use crate::{
    compile::{RenderConfig, compile},
    host::session::transfer::tests::{graph, limits, notes_plan, profile},
    quantities::ChannelLayout,
    render::AudioBlockMut,
    time::{PlanPosition, SampleTime},
};

#[test]
fn fresh_running_owner_is_adopted_without_advancing_dsp_or_clock() {
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let anchor = StreamAnchor::new(SampleTime::new(4096), PlanPosition::new(128));
    let (control, mut audio) = prepare_initial(plan, stream, profile(), limits(), anchor).unwrap();
    assert_eq!(
        control.snapshot(),
        SessionSnapshot {
            clock: anchor.time(),
            playback: PlaybackState::Playing(anchor)
        }
    );
    assert_eq!(audio.clock(), anchor.time());
    assert_eq!(audio.frames_until_plan_boundary().as_u64(), 0);
    assert!(!control.has_outstanding());
    assert!(!audio.has_retained_commands());
    assert!(audio.take_completed().is_none());
    let mut samples = [0.0; 64];
    assert_eq!(
        crate::render_allocation::count_allocs(|| audio
            .render(AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).unwrap())
            .unwrap()),
        0
    );
    assert!(
        samples.iter().all(|sample| *sample == 0.25),
        "there is no extra priming quantum"
    );
}

#[test]
fn prepared_start_cuts_crossing_notes_and_plays_later_onsets() {
    let (plan, stream) = notes_plan();
    let anchor = StreamAnchor::new(SampleTime::new(4096), PlanPosition::new(64));
    let (_control, mut audio) = prepare_initial(plan, stream, profile(), limits(), anchor).unwrap();
    let mut first = [9.0; 64];
    audio
        .render(AudioBlockMut::new(&mut first, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert!(
        first.iter().all(|sample| *sample == 0.0),
        "the note crossing P=64 must be cut"
    );
    let mut later = [0.0; 256];
    audio
        .render(AudioBlockMut::new(&mut later, 256, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert!(later.iter().any(|sample| *sample != 0.0));
}

fn constant(level: crate::quantities::Amplitude) -> CompiledPlan {
    use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant { level },
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
    compile(&graph, &RenderConfig::new(profile()))
        .into_plan()
        .unwrap()
}
fn request(end: Option<PlanPosition>) -> RunningReplacementRequest {
    RunningReplacementRequest {
        profile: profile(),
        transfer: limits(),
        song_end: end,
        maximum_timeline_events: EventCount::limit(16_384).unwrap(),
        mode: RunningEditMode::Reset,
    }
}
fn render(audio: &mut SessionAudio, frames: usize) -> Vec<f32> {
    let mut output = vec![9.0; frames];
    assert_eq!(
        crate::render_allocation::count_allocs(|| audio
            .render(AudioBlockMut::new(&mut output, frames, ChannelLayout::Mono).unwrap())
            .unwrap()),
        0
    );
    output
}
fn collect(control: &mut SessionControl, audio: &mut SessionAudio) {
    while let Some(packet) = audio.take_completed() {
        assert!(matches!(
            control.collect(packet).unwrap().outcome,
            SessionOutcome::Applied { .. }
        ));
    }
}
fn playing() -> (SessionControl, SessionAudio) {
    let (mut control, mut audio) = crate::host::session::transfer::tests::setup();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    render(&mut audio, 256);
    collect(&mut control, &mut audio);
    (control, audio)
}
fn candidate(
    control: &mut SessionControl,
    at: SampleTime,
    end: Option<PlanPosition>,
) -> Box<PreparedPlanReplacement> {
    let plan = constant(crate::quantities::Amplitude::UNITY);
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    control
        .prepare_running_replacement(plan, stream, request(end), |_| Some(at))
        .unwrap()
}
fn begin(
    audio: &mut SessionAudio,
    candidate: Box<PreparedPlanReplacement>,
) -> Box<PreparedPlanReplacement> {
    let mut returned = None;
    assert_eq!(
        crate::render_allocation::count_allocs(
            || returned = Some(audio.begin_running_replacement(candidate).unwrap())
        ),
        0
    );
    returned.unwrap()
}
fn fade(
    audio: &mut SessionAudio,
    candidate: &mut PreparedPlanReplacement,
    frames: usize,
) -> Vec<f32> {
    let mut output = vec![9.0; frames];
    assert_eq!(
        crate::render_allocation::count_allocs(|| audio
            .render_running_replacement(
                candidate,
                AudioBlockMut::new(&mut output, frames, ChannelLayout::Mono).unwrap()
            )
            .unwrap()),
        0
    );
    output
}

#[test]
fn running_fade_has_the_same_endpoint_and_receipt_under_every_partition() {
    for partition in [&[1][..], &[64], &[256], &[7, 19, 3, 111]] {
        let (mut control, mut audio) = playing();
        let at = audio.clock();
        let old_epoch = audio.epoch();
        let old_generation = control.generation();
        let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
        assert!(candidate.outcome().is_none());
        assert_ne!(audio.epoch(), old_epoch);
        let mut output = Vec::new();
        let mut cursor = 0;
        while output.len() < 512 {
            let n = partition[cursor % partition.len()].min(512 - output.len());
            if candidate.outcome().is_none() {
                output.extend(fade(&mut audio, &mut candidate, n));
            } else {
                output.extend(render(&mut audio, n));
            }
            cursor += 1;
        }
        for (frame, sample) in output.into_iter().enumerate() {
            let expected = if frame >= 63 {
                1.0
            } else {
                let gain = f32::from(u16::try_from(frame + 1).unwrap()) / 64.0;
                0.25 * (1.0 - gain) + gain
            };
            assert_eq!(
                sample.to_bits(),
                expected.to_bits(),
                "{partition:?} frame {frame}"
            );
        }
        let receipt = control.collect_replacement(candidate).unwrap();
        let PlanReplacementOutcome::Installed(snapshot) = receipt.outcome else {
            panic!("{receipt:?}");
        };
        assert_eq!(snapshot.clock, at.checked_advance_quantum().unwrap());
        assert!(matches!(snapshot.playback, PlaybackState::Playing(_)));
        assert_ne!(control.generation(), old_generation);
        assert!(!control.has_outstanding());
        assert!(!control.has_replacements());
    }
}

#[test]
fn an_edit_past_its_new_end_fades_to_silence_and_keeps_the_unclamped_position() {
    let (mut control, mut audio) = playing();
    let at = audio.clock();
    let mut candidate = begin(
        &mut audio,
        candidate(&mut control, at, Some(PlanPosition::new(1))),
    );
    let output = fade(&mut audio, &mut candidate, 256);
    assert!(output[..63].iter().any(|sample| *sample != 0.0));
    assert!(output[63..].iter().all(|sample| *sample == 0.0));
    let receipt = control.collect_replacement(candidate).unwrap();
    let PlanReplacementOutcome::Installed(snapshot) = receipt.outcome else {
        panic!("{receipt:?}");
    };
    let PlaybackState::Stopped(position) = snapshot.playback else {
        panic!("{snapshot:?}");
    };
    assert!(position > PlanPosition::new(1));
}

#[test]
fn late_attempt_is_collected_without_changing_the_old_sound_or_issuer() {
    let (mut control, mut audio) = playing();
    let at = audio.clock();
    let plan = audio.plan_id();
    let generation = control.generation();
    let candidate = candidate(&mut control, at, None);
    assert!(render(&mut audio, 64).iter().all(|sample| *sample == 0.25));
    let returned = begin(&mut audio, candidate);
    assert!(matches!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::LateRunningBoundary { .. }
        ))
    ));
    let receipt = control.collect_replacement(returned).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Refused(_)
    ));
    assert_eq!(audio.plan_id(), plan);
    assert_eq!(control.generation(), generation);
    assert!(!control.has_replacements());
    assert!(render(&mut audio, 256).iter().all(|sample| *sample == 0.25));
}

#[test]
fn interrupted_fade_silences_the_outer_block_and_promotes_only_a_closed_origin() {
    let (mut control, mut audio) = playing();
    let at = audio.clock();
    let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
    assert!(
        fade(&mut audio, &mut candidate, 17)
            .iter()
            .all(|sample| *sample > 0.25)
    );
    candidate.audio.fault = Some(crate::schedule::ScheduledRenderError::RunningReplacementState);
    let mut output = [9.0; 256];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(
                audio
                    .render_running_replacement(
                        &mut candidate,
                        AudioBlockMut::new(&mut output, 256, ChannelLayout::Mono).unwrap()
                    )
                    .is_err()
            );
        }),
        0
    );
    assert!(output.iter().all(|sample| *sample == 0.0));
    assert!(matches!(
        candidate.outcome(),
        Some(PlanReplacementOutcome::Interrupted(_))
    ));
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Interrupted(_)
    ));
    assert_eq!(control.plan_id(), audio.plan_id());
    assert!(matches!(
        control.prepare_stop(audio.clock()),
        Err(SessionError::Closed)
    ));
}

#[test]
fn an_in_flight_owner_cannot_be_cancelled_or_collected_before_its_fade_finishes() {
    let (mut control, mut audio) = playing();
    let at = audio.clock();
    let candidate = begin(&mut audio, candidate(&mut control, at, None));
    let (candidate, error) = control.cancel_replacement(candidate).unwrap_err();
    assert!(matches!(error, PlanReplacementError::State));
    let (mut candidate, error) = control.collect_replacement(candidate).unwrap_err();
    assert!(matches!(error, PlanReplacementError::State));
    assert!(control.has_replacements());
    fade(&mut audio, &mut candidate, 64);
    assert!(matches!(
        control.collect_replacement(candidate).unwrap().outcome,
        PlanReplacementOutcome::Installed(_)
    ));
}

#[test]
fn fresh_counters_accept_transport_and_the_next_native_replacement() {
    let (mut control, mut audio) = crate::host::session::transfer::tests::setup();
    render(&mut audio, 64); // Drain only the initial priming carry.
    for _ in 0..2 {
        let plan = constant(crate::quantities::Amplitude::new(0.25).unwrap());
        let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
        let candidate = control
            .prepare_replacement(plan, stream, profile(), limits())
            .unwrap();
        let returned = audio.install_replacement(candidate).unwrap();
        assert!(matches!(
            control.collect_replacement(returned).unwrap().outcome,
            PlanReplacementOutcome::Installed(_)
        ));
    }
    for index in 0..3 {
        audio
            .enqueue(control.prepare_play(audio.clock()).unwrap())
            .unwrap();
        render(&mut audio, 64);
        collect(&mut control, &mut audio);
        if index < 2 {
            audio
                .enqueue(control.prepare_stop(audio.clock()).unwrap())
                .unwrap();
            render(&mut audio, 64);
            collect(&mut control, &mut audio);
        }
    }
    assert!(audio.runtime.serial > 1);
    assert!(audio.publication_serial > 1);
    let at = audio.clock();
    let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
    fade(&mut audio, &mut candidate, 64);
    assert!(matches!(
        control.collect_replacement(candidate).unwrap().outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    assert_eq!(audio.runtime.serial, 1);
    assert_eq!(audio.publication_serial, 0);
    audio
        .enqueue(control.prepare_stop(audio.clock()).unwrap())
        .unwrap();
    render(&mut audio, 64);
    collect(&mut control, &mut audio);
    let plan = constant(crate::quantities::Amplitude::new(0.5).unwrap());
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let candidate = control
        .prepare_replacement(plan, stream, profile(), limits())
        .unwrap();
    let returned = audio.install_replacement(candidate).unwrap();
    assert!(matches!(
        control.collect_replacement(returned).unwrap().outcome,
        PlanReplacementOutcome::Installed(_)
    ));
}

fn oscillator(profile: HostProfile, hz: f32) -> CompiledPlan {
    use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: crate::quantities::Frequency::new(hz).unwrap(),
                amplitude: crate::quantities::Amplitude::new(0.5).unwrap(),
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
    compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap()
}

fn ordinary_oscillator(profile: HostProfile, hz: f32) -> (SessionControl, SessionAudio) {
    let plan = oscillator(profile, hz);
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (mut control, mut audio) =
        SessionControl::prepare(plan, stream, profile, limits()).unwrap();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    render(&mut audio, 64); // Ordinary constructor's priming Q, with no oscillator advance.
    collect(&mut control, &mut audio);
    (control, audio)
}

#[test]
fn running_oscillators_match_independently_started_ordinary_players_at_four_rates() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = HostProfile::harness(
            crate::quantities::SampleRate::new(rate).unwrap(),
            FrameCount::new(512),
            ChannelLayout::Mono,
        )
        .unwrap();
        for partition in [&[1][..], &[64], &[256], &[7, 19, 3, 111]] {
            let (mut control, mut audio) = ordinary_oscillator(profile, 440.0);
            let (_, mut old_reference) = ordinary_oscillator(profile, 440.0);
            let (_, mut new_reference) = ordinary_oscillator(profile, 660.0);
            render(&mut audio, 256);
            collect(&mut control, &mut audio);
            render(&mut old_reference, 256);
            let old = render(&mut old_reference, 64);
            let new = render(&mut new_reference, 512);
            let plan = oscillator(profile, 660.0);
            let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
            let mut request = request(None);
            request.profile = profile;
            let at = audio.clock();
            let prepared = control
                .prepare_running_replacement(plan, stream, request, |_| Some(at))
                .unwrap();
            let mut candidate = begin(&mut audio, prepared);
            let mut actual = Vec::new();
            let mut cursor = 0;
            while actual.len() < 512 {
                let n = partition[cursor % partition.len()].min(512 - actual.len());
                actual.extend(if candidate.outcome().is_none() {
                    fade(&mut audio, &mut candidate, n)
                } else {
                    render(&mut audio, n)
                });
                cursor += 1;
            }
            for frame in 0..512 {
                let expected = if frame >= 63 {
                    new[frame]
                } else {
                    let gain = f32::from(u16::try_from(frame + 1).unwrap()) / 64.0;
                    old[frame] * (1.0 - gain) + new[frame] * gain
                };
                assert_eq!(
                    actual[frame].to_bits(),
                    expected.to_bits(),
                    "rate={rate}, partition={partition:?}, frame={frame}"
                );
            }
            let _receipt = control.collect_replacement(candidate).unwrap();
        }
    }
}

#[test]
fn retirement_counts_an_old_compiled_onset_during_the_fade() {
    let (mut control, mut audio) = crate::host::session::transfer::tests::setup_notes();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    render(&mut audio, 256); // Clock 192; the old onset at 192 is still ahead.
    collect(&mut control, &mut audio);
    let at = audio.clock();
    let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
    fade(&mut audio, &mut candidate, 64);
    let receipt = control.collect_replacement(candidate).unwrap();
    assert_eq!(receipt.retired_compiled_notes.get(), 1);
}

#[test]
fn maximum_timeline_late_publication_is_refused_once_and_returns_its_credit() {
    use crate::schedule::{CompiledPayload, PlanEvent};
    let (mut control, mut audio) = playing();
    let (plan, initial) = notes_plan();
    let on = initial.events()[0].payload();
    let off = initial.events()[1].payload();
    assert!(matches!(on, CompiledPayload::NoteOn { .. }));
    let events: Vec<_> = (0_u64..16_384)
        .map(|index| {
            PlanEvent::new(
                PlanPosition::new(index * 64),
                if index.is_multiple_of(2) { on } else { off },
            )
        })
        .collect();
    let stream = AdmittedCompiledStream::admit(&plan, &events).unwrap();
    let at = audio.clock();
    let prepared = control
        .prepare_running_replacement(plan, stream, request(None), |_| Some(at))
        .unwrap();
    render(&mut audio, 64); // Concurrent audio can pass B even at the admitted cold limit.
    let returned = begin(&mut audio, prepared);
    assert!(matches!(
        returned.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::LateRunningBoundary { .. }
        ))
    ));
    let _receipt = control.collect_replacement(returned).unwrap();
    assert!(!control.has_replacements());
    assert!(render(&mut audio, 64).iter().all(|value| *value == 0.25));
}

#[test]
fn ordinary_render_or_closed_successor_interrupts_instead_of_bypassing_the_fade() {
    for close in [false, true] {
        let (mut control, mut audio) = playing();
        let at = audio.clock();
        let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
        if close {
            audio.close_after_quiescence().unwrap();
        } else {
            let mut output = [9.0; 64];
            assert!(
                audio
                    .render(AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap())
                    .is_err()
            );
            assert!(output.iter().all(|value| *value == 0.0));
            assert_eq!(
                audio.clock(),
                at,
                "bypass cannot spend the successor's bootstrap debt"
            );
        }
        let mut output = [9.0; 256];
        assert!(
            audio
                .render_running_replacement(
                    &mut candidate,
                    AudioBlockMut::new(&mut output, 256, ChannelLayout::Mono).unwrap()
                )
                .is_err()
        );
        assert!(output.iter().all(|value| *value == 0.0));
        let receipt = control.collect_replacement(candidate).unwrap();
        assert!(matches!(
            receipt.outcome,
            PlanReplacementOutcome::Interrupted(_)
        ));
        assert!(
            control
                .prepare_stop(at.checked_advance_quantum().unwrap())
                .is_err()
        );
    }
}

#[test]
fn repeating_a_completed_fade_latches_a_terminal_interrupted_outcome() {
    let (mut control, mut audio) = playing();
    let at = audio.clock();
    let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
    fade(&mut audio, &mut candidate, 64);
    let mut output = [9.0; 64];
    assert!(
        audio
            .render_running_replacement(
                &mut candidate,
                AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap()
            )
            .is_err()
    );
    assert!(output.iter().all(|value| *value == 0.0));
    assert!(
        audio
            .render(AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap())
            .is_err()
    );
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Interrupted(_)
    ));
}

#[test]
fn early_begin_keeps_the_candidate_ready_and_a_second_preparation_refuses() {
    let (mut control, mut audio) = playing();
    let at = audio.clock().checked_advance_quantum().unwrap();
    let prepared = candidate(&mut control, at, None);
    let plan = constant(crate::quantities::Amplitude::UNITY);
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        control.prepare_running_replacement(plan, stream, request(None), |_| Some(at)),
        Err(PlanReplacementError::RunningTransport)
    ));
    let fence = audio.publication_serial;
    let (prepared, error) = audio.begin_running_replacement(prepared).unwrap_err();
    assert!(matches!(error, PlanReplacementError::State));
    assert!(prepared.outcome().is_none());
    assert_eq!(audio.publication_serial, fence);
    render(&mut audio, 64);
    let mut prepared = begin(&mut audio, prepared);
    fade(&mut audio, &mut prepared, 64);
    let _receipt = control.collect_replacement(prepared).unwrap();
}

#[test]
fn a_post_fade_clock_failure_replaces_installed_with_interrupted_and_silences_the_outer_block() {
    let plan = constant(crate::quantities::Amplitude::new(0.25).unwrap());
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let at = SampleTime::new(u64::MAX - 127);
    let (mut control, mut audio) = prepare_initial(
        plan,
        stream,
        profile(),
        limits(),
        StreamAnchor::new(at, PlanPosition::ZERO),
    )
    .unwrap();
    let mut candidate = begin(&mut audio, candidate(&mut control, at, None));
    let mut output = [9.0; 256];
    assert!(
        audio
            .render_running_replacement(
                &mut candidate,
                AudioBlockMut::new(&mut output, 256, ChannelLayout::Mono).unwrap()
            )
            .is_err()
    );
    assert!(output.iter().all(|value| *value == 0.0));
    let receipt = control.collect_replacement(candidate).unwrap();
    assert!(matches!(
        receipt.outcome,
        PlanReplacementOutcome::Interrupted(_)
    ));
    assert!(control.closed);
}

#[test]
fn a_timeline_over_the_cold_limit_refuses_before_reserving_a_credit() {
    let (mut control, _audio) = playing();
    let (plan, stream) = notes_plan();
    let mut request = request(None);
    request.maximum_timeline_events = EventCount::limit(3).unwrap();
    let serial = control.publication_serial;
    assert!(matches!(
        control.prepare_running_replacement(plan, stream, request, |_| panic!(
            "admission must precede the boundary chooser"
        )),
        Err(PlanReplacementError::RunningTimeline { .. })
    ));
    assert_eq!(control.publication_serial, serial);
    assert!(!control.has_replacements());
}

#[test]
fn cold_bootstrap_matches_normal_boundary_adoption_with_nonzero_position_automation() {
    use crate::publish::ProducerClass;
    use crate::schedule::{CompiledPayload, PlanEvent};
    let plan = oscillator(profile(), 440.0);
    let slot = plan
        .resolve_parameter(
            crate::ir::NodeId::new(1),
            crate::ir::parameters::SINE_FREQUENCY,
        )
        .unwrap();
    let events = [PlanEvent::new(
        PlanPosition::new(32),
        CompiledPayload::SetParameter {
            slot,
            value: crate::quantities::ParameterValue::new(220.0).unwrap(),
        },
    )];
    let stream = AdmittedCompiledStream::admit(&plan, &events).unwrap();
    let anchor = StreamAnchor::new(SampleTime::new(4096), PlanPosition::new(128));
    let (_, mut subject) =
        prepare_initial(plan.clone(), stream.clone(), profile(), limits(), anchor).unwrap();
    let (mut control, mut reference) =
        SessionControl::prepare(plan, stream, profile(), limits()).unwrap();
    // Only seed the stopped position/clock. Normal rendering must perform its own
    // boundary offer and scheduler adoption; the subject already adopted off-thread.
    control.acknowledged = SessionSnapshot {
        clock: anchor.time(),
        playback: PlaybackState::Stopped(anchor.position()),
    };
    reference.runtime.state = control.acknowledged.playback;
    reference.renderer.install_stopped_clock(anchor);
    reference
        .enqueue(control.prepare_play(anchor.time()).unwrap())
        .unwrap();
    assert_eq!(render(&mut subject, 256), render(&mut reference, 256));
    collect(&mut control, &mut reference);
    assert_eq!(
        subject.high_water(ProducerClass::Session),
        reference.high_water(ProducerClass::Session)
    );
    assert!(
        subject.high_water(ProducerClass::Session).get() > 0,
        "catch-up is charged in the first actual quantum"
    );
    assert_eq!(subject.publication_faults(), 0);
    assert_eq!(reference.publication_faults(), 0);
}
