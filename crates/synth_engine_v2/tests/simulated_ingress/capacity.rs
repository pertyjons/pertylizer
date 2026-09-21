//! EVD-0024: real producer occupancy, with controls before each measurement.
//! This does not qualify a production host or add independent owners' maxima.
use super::*;
use synth_engine_v2::{
    admit::AdmissionError,
    compile::{RenderConfig, compile},
    profile::HostProfile,
    quantities::{KeyIdentity, NoteVelocity, ParameterValue, SampleRate},
    schedule::CompiledStreamError,
    time::FrameCount,
};

const CALLBACKS: u64 = 64;
const NOTES: HeldNoteCount = HeldNoteCount::measured(8);

struct Fixture {
    profile: HostProfile,
    plan: CompiledPlan,
    control: StreamControl,
    renderer: PreparedRenderer,
    scheduler: CompiledEventScheduler,
    ingress: PerformanceIngress,
    arbiter: PublicationArbiter,
    samples: Vec<f32>,
}
impl Fixture {
    fn new(profile: HostProfile, loaded: bool) -> Self {
        let plan = compile(
            &gated_constant(live_only(NOTES.get(), NOTES.get())),
            &RenderConfig::new(profile),
        )
        .into_plan()
        .unwrap();
        let slot = plan
            .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
            .unwrap();
        let block = profile.capabilities().maximum_block_size().as_u64();
        let compiled = ProducerClass::Compiled.share_of(&profile).get();
        let event = |at| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::SetParameter {
                    slot,
                    value: ParameterValue::ONE,
                },
            )
        };
        // Admission control runs before the measured schedule is constructed.
        assert!(AdmittedCompiledStream::admit(&plan, &vec![event(0); compiled as usize]).is_ok());
        assert_eq!(
            AdmittedCompiledStream::admit(&plan, &vec![event(0); compiled as usize + 1]),
            Err(CompiledStreamError::Window(
                AdmissionError::WindowOverShare {
                    window_start: PlanPosition::ZERO,
                    requested: EventCount::measured(compiled + 1),
                    share: EventCount::measured(compiled),
                    quantum: QUANTUM_FRAMES,
                }
            ))
        );
        let mut events = Vec::new();
        if loaded {
            for callback in 0..CALLBACKS {
                for _ in 0..compiled {
                    events.push(event((callback * 2 + 1) * block));
                }
            }
        }
        let admitted = AdmittedCompiledStream::admit(&plan, &events).unwrap();
        let (mut control, renderer) = StreamControl::open(plan.clone(), ORIGIN).unwrap();
        let scheduler = CompiledEventScheduler::prepare(&mut control, &admitted).unwrap();
        let mut ingress =
            PerformanceIngress::prepare(&profile, &plan, ONLY_PRODUCER, &renderer).unwrap();
        // Latch the actual store without queuing an event. A refused future
        // offer adopts it before horizon admission, and advances no accepted stamp.
        let horizon_end = SampleTime::ZERO
            .checked_add(profile.limits().events().forward_event_horizon())
            .unwrap();
        let time = horizon_end.checked_add(FrameCount::new(1)).unwrap();
        assert_eq!(
            control.offer_parameter(&mut ingress, time, slot, ParameterValue::ONE),
            Err(IngressRefused::BeyondHorizon { time, horizon_end })
        );
        assert_eq!(ingress.adopted_by(), Some(renderer.epoch()));
        assert!(ingress.is_empty());
        assert_eq!(ingress.holds_outstanding(), EventCount::NONE);
        let arbiter = PublicationArbiter::prepare(&profile).unwrap();
        Self {
            profile,
            plan,
            control,
            renderer,
            scheduler,
            ingress,
            arbiter,
            samples: vec![0.0; block as usize * profile.capabilities().channel_layout().channels()],
        }
    }

    fn render(&mut self) {
        let layout = self.profile.capabilities().channel_layout();
        let frames = self.samples.len() / layout.channels();
        self.render_frames(frames);
    }

    fn render_frames(&mut self, frames: usize) {
        let layout = self.profile.capabilities().channel_layout();
        self.scheduler
            .render_with_ingress(
                &mut self.renderer,
                &mut self.arbiter,
                Some(&mut self.ingress),
                AudioBlockMut::new(
                    &mut self.samples[..frames * layout.channels()],
                    frames,
                    layout,
                )
                .unwrap(),
            )
            .unwrap();
        assert!(self.samples.iter().all(|sample| sample.is_finite()));
        assert!(!self.renderer.diagnostics().needs_reprepare());
        assert_eq!(self.renderer.diagnostics().orphan_note_events(), 0);
        assert_eq!(self.ingress.counters().beyond_horizon(), 1);
    }

    fn empty_control(profile: HostProfile) {
        let mut fixture = Self::new(profile, false);
        fixture.render_frames(QUANTUM_FRAMES as usize);
        for _ in 0..CALLBACKS * 2 {
            fixture.render();
        }
        for class in [
            ProducerClass::Compiled,
            ProducerClass::Live,
            ProducerClass::Release,
        ] {
            assert_eq!(fixture.arbiter.high_water(class), EventCount::NONE);
        }
        assert_eq!(
            fixture.arbiter.high_water_external_total(),
            EventCount::NONE
        );
        assert!(fixture.samples.iter().all(|sample| *sample == 0.0));
    }
}

fn loaded(profile: HostProfile, releases: bool) {
    let mut fixture = Fixture::new(profile, true);
    // Consume only the initial Q-frame output carry before offering anything.
    // This makes even a 64-frame setup callback actually render its eight notes.
    fixture.render_frames(QUANTUM_FRAMES as usize);
    let slot = fixture
        .plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
        .unwrap();
    let note = fixture.plan.resolve_note(ENVELOPE).unwrap();
    let depth = profile
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .get();
    let held = if releases { NOTES.get() } else { 0 };
    let ordinary = depth - held;
    let mut identities = Vec::with_capacity(NOTES.get() as usize);
    for callback in 0..CALLBACKS {
        identities.clear();
        for key in 0..held {
            identities.push(
                fixture
                    .control
                    .offer_note_on(
                        &mut fixture.ingress,
                        fixture.renderer.clock(),
                        note,
                        KeyIdentity::new(u8::try_from(60 + key).unwrap()).unwrap(),
                        NoteVelocity::FULL,
                    )
                    .unwrap(),
            );
        }
        fixture.render();
        if releases {
            assert!(fixture.samples.iter().any(|sample| *sample != 0.0));
        }
        assert_eq!(
            fixture.ingress.holds_outstanding(),
            EventCount::measured(held)
        );
        let at = fixture.renderer.clock();
        assert_eq!(
            at.as_u64(),
            (callback * 2 + 1) * profile.capabilities().maximum_block_size().as_u64()
        );
        for _ in 0..ordinary {
            fixture
                .control
                .offer_parameter(&mut fixture.ingress, at, slot, ParameterValue::ONE)
                .unwrap();
        }
        // Occupied + reservations == depth. Refuse the extra ordinary entry,
        // then redeem every reservation without making space by dropping work.
        assert_eq!(
            fixture
                .control
                .offer_parameter(&mut fixture.ingress, at, slot, ParameterValue::ONE),
            Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Slot
            })
        );
        for identity in identities.iter().copied() {
            fixture
                .control
                .offer_note_off(&mut fixture.ingress, at, identity)
                .unwrap();
        }
        assert_eq!(fixture.ingress.len(), depth as usize);
        fixture.render();
        assert!(fixture.samples.iter().all(|sample| *sample == 0.0));
        assert!(fixture.ingress.is_empty());
        assert_eq!(fixture.ingress.holds_outstanding(), EventCount::NONE);
    }
    fixture.render();
    assert!(fixture.samples.iter().all(|sample| *sample == 0.0));
    let compiled = ProducerClass::Compiled.share_of(&profile).get();
    for (class, expected) in [
        (ProducerClass::Compiled, compiled),
        (ProducerClass::Live, ordinary),
        (ProducerClass::Release, held),
    ] {
        assert_eq!(
            fixture.arbiter.high_water(class),
            EventCount::measured(expected)
        );
    }
    assert_eq!(
        fixture.arbiter.high_water_external_total(),
        EventCount::measured(compiled + depth)
    );
    assert_eq!(fixture.ingress.counters().dropped(), CALLBACKS);
    assert_eq!(fixture.ingress.counters().dropped_slot(), CALLBACKS);
    assert_eq!(fixture.ingress.counters().orphan_releases(), 0);
    println!(
        "capacity,rate={},layout={:?},block={},arm={},callbacks={CALLBACKS},compiled={compiled},live={ordinary},release={held},external_total={},queue_refusals={CALLBACKS},final_holds=0",
        profile.capabilities().sample_rate().as_f32(),
        profile.capabilities().channel_layout(),
        profile.capabilities().maximum_block_size().as_u64(),
        if releases {
            "reserved_release"
        } else {
            "ordinary"
        },
        fixture.arbiter.high_water_external_total().get(),
    );
}

#[test]
fn capacity_probe_controls() {
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::QUANTUM,
        ChannelLayout::Mono,
    )
    .unwrap();
    Fixture::empty_control(profile);
    loaded(profile, false);
    loaded(profile, true);
}

#[test]
#[ignore = "EVD-0024 explicit capacity matrix; not production qualification"]
fn evd_0024_capacity_matrix() {
    for hz in [44100.0, 48000.0, 96000.0] {
        for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
            for block in [64, 256, 8192] {
                let profile = HostProfile::harness(
                    SampleRate::new(hz).unwrap(),
                    FrameCount::new(block),
                    layout,
                )
                .unwrap();
                Fixture::empty_control(profile);
                loaded(profile, false);
                loaded(profile, true);
            }
        }
    }
    println!("scope=component_occupancy; production_qualification=not_evaluated");
}
