use super::*;
use crate::compile::{RenderConfig, compile};
use crate::ir::GraphIr;
use crate::quantities::{ChannelLayout, SampleRate};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::time::{FrameCount, PlanPosition, StreamAnchor};

fn fixture() -> (StreamControl, PreparedRenderer, SessionRuntime) {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let plan = compile(
        &GraphIr::builder().build().unwrap(),
        &RenderConfig::new(profile),
    )
    .into_plan()
    .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (mut control, renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap();
    let session = SessionRuntime::prepare(
        ConnectionGeneration(1),
        &mut control,
        stream,
        &profile,
        SessionLimits {
            commands: SessionCommandCapacity::new(4).unwrap(),
            command_bytes: PreparedBytes::limit(8192).unwrap(),
        },
    )
    .unwrap();
    (control, renderer, session)
}

#[test]
fn first_adoption_idle_cancellation_and_resume_neither_allocate_nor_reclaim() {
    for cancelled in [false, true] {
        let (mut control, mut renderer, mut session) = fixture();
        let _play = session
            .offer_play(&mut control, renderer.clock(), SampleTime::ZERO, None)
            .unwrap();
        let at = SampleTime::new(if cancelled { 0 } else { 64 });
        let serial = session
            .validate_offer(renderer.clock(), at, SessionCommand::Stop)
            .unwrap();
        let _stop = session.insert(serial, at, SessionCommand::Stop, None, None);
        let mut output = [1.0; 320];
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                for chunk in output.chunks_mut(37) {
                    let frames = chunk.len();
                    session
                        .render(
                            &mut renderer,
                            AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap(),
                            None,
                        )
                        .unwrap();
                }
            }),
            0
        );
        let _play = session.collect(&mut control).unwrap().unwrap();
        let _stop = session.collect(&mut control).unwrap().unwrap();
        let _resume = session
            .offer_play(&mut control, renderer.clock(), renderer.clock(), None)
            .unwrap();
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                session
                    .render(
                        &mut renderer,
                        AudioBlockMut::new(&mut output, 320, ChannelLayout::Mono).unwrap(),
                        None,
                    )
                    .unwrap();
            }),
            0
        );
        let _resume = session.collect(&mut control).unwrap().unwrap();
    }
}

#[test]
fn serial_exhaustion_and_session_share_refuse_before_minting() {
    let (mut control, renderer, mut session) = fixture();
    session.serial = u64::MAX;
    assert!(matches!(
        session.offer_play(&mut control, renderer.clock(), SampleTime::ZERO, None),
        Err(SessionError::IdentityExhausted)
    ));
    assert_eq!(session.held, 0);
    session.serial = 0;
    session.session_share = EventCount::measured(1);
    let _play = session
        .offer_play(&mut control, renderer.clock(), SampleTime::ZERO, None)
        .unwrap();
    assert!(matches!(
        session.validate_offer(renderer.clock(), SampleTime::ZERO, SessionCommand::Stop),
        Err(SessionError::SessionShare)
    ));
    // The Stop entitlement remains usable at a later quantum.
    assert_eq!(
        session
            .validate_offer(renderer.clock(), SampleTime::new(64), SessionCommand::Stop)
            .unwrap(),
        2
    );
}

#[test]
fn rejected_byte_budget_does_not_latch_the_control() {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let plan = compile(
        &GraphIr::builder().build().unwrap(),
        &RenderConfig::new(profile),
    )
    .into_plan()
    .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (mut control, _renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap();
    assert!(matches!(
        SessionRuntime::prepare(
            ConnectionGeneration(1),
            &mut control,
            stream,
            &profile,
            SessionLimits {
                commands: SessionCommandCapacity::new(2).unwrap(),
                command_bytes: PreparedBytes::limit(1).unwrap()
            }
        ),
        Err(SessionError::ByteBudget { .. })
    ));
    let stream = AdmittedCompiledStream::admit(control.plan(), &[]).unwrap();
    assert!(CompiledEventScheduler::prepare(&mut control, &stream).is_ok());
}

#[test]
fn unrepresentable_final_position_reports_once_and_cannot_trap_owned_outcomes() {
    let (mut control, renderer, mut session) = fixture();
    let _play = session
        .offer_play(&mut control, renderer.clock(), SampleTime::ZERO, None)
        .unwrap();
    session.state = PlaybackState::Playing(StreamAnchor::new(
        SampleTime::ZERO,
        PlanPosition::new(u64::MAX),
    ));
    assert!(matches!(
        session.close(SampleTime::new(64)),
        Err(SessionError::Timeline(_))
    ));
    assert_eq!(session.state, PlaybackState::Unavailable);
    // A second coordinator call can collect and withdraw the retained candidate.
    session.close(SampleTime::new(64)).unwrap();
    assert_eq!(
        session.collect(&mut control).unwrap().unwrap().outcome,
        SessionOutcome::Cancelled
    );
    assert_eq!(session.held, 0);
    assert!(!session.play_outstanding);
}
