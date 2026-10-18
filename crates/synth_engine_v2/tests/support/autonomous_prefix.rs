//! Disposable, single-arm finite-prefix prototype, shared with the child rolling
//! control. Neither qualifies reset or device deadlines. These use a separately
//! declared maximum block, not EVD-0026/27's measured 64-frame profile.
//!
//! Completion checks: real queued raw frontier custody, explicit publication-window
//! boundaries, bitwise serial audio/receipt equality without per-callback permits,
//! and a latched, completely silent fault before an unprepared quantum can open.
use super::*;
use std::thread;
use synth_engine_v2::{quantities::SampleRate, time::TimeError};

#[path = "rolling_refill.rs"]
mod rolling_refill;

#[path = "continuous_saved_host.rs"]
mod continuous_saved_host;

const PREFIX_END: SampleTime = SampleTime::new(1_024);
const SCRIPT_MESSAGES: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SourcePrefix {
    generation: ConnectionGeneration,
    through: SampleTime,
}

/// Control-side coverage for one fixed arm at startup or a paused burst boundary.
/// These private fixtures never transfer it between arms or across reset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PrefixCoverage {
    sound: SampleTime,
    raw: SampleTime,
    sources: [SourcePrefix; 2],
}
impl PrefixCoverage {
    fn capture(rig: &Rig, profile: HostProfile) -> Self {
        assert_eq!(rig.output_frames(), 0, "startup snapshot only");
        assert!(
            rig.callback.is_some(),
            "before this arm's only worker handoff"
        );
        Self::capture_at_boundary(rig, profile, PREFIX_END)
    }

    /// The rolling fixture calls this only while its worker waits between bursts,
    /// immediately after control service. The owner and boundary cannot change.
    fn capture_at_boundary(rig: &Rig, profile: HostProfile, complete: SampleTime) -> Self {
        assert!(
            rig.raw_queued_inputs.is_some(),
            "raw delivery tracing enabled"
        );
        let frontier = |source: usize, queued_only: bool| {
            rig.admitted
                .iter()
                .filter_map(|admitted| {
                    if admitted.source != source {
                        return None;
                    }
                    let InputObservation::Frontier { tick } = admitted.observation else {
                        return None;
                    };
                    if queued_only
                        && !admitted.raw_id().is_some_and(|id| {
                            rig.raw_queued_inputs
                                .as_ref()
                                .is_some_and(|queued| queued.contains(&id))
                        })
                    {
                        return None;
                    }
                    Some(
                        rig.raw.clocks[source]
                            .map(tick)
                            .expect("admitted source clock"),
                    )
                })
                .max()
                .unwrap_or(SampleTime::ZERO)
        };
        let sources = [FIRST, SECOND].map(|source| SourcePrefix {
            generation: rig.generation(source),
            through: frontier(source, false),
        });
        let window = rig
            .control
            .audio_boundary()
            .checked_add(FrameCount::new(
                profile.capabilities().maximum_block_size().quanta_spanned()
                    * u64::from(QUANTUM_FRAMES),
            ))
            .expect("bounded fixture window");
        let sound = match rig.waits.last() {
            Some(MixedBridgeWait::Empty) => window.min(complete),
            Some(MixedBridgeWait::Window { end, .. }) => *end,
            _ => SampleTime::ZERO,
        };
        Self {
            sound,
            raw: frontier(FIRST, true).min(frontier(SECOND, true)),
            sources,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MissingPrefix {
    Source {
        generation: ConnectionGeneration,
        through: SampleTime,
        required: SampleTime,
    },
    Sound {
        through: SampleTime,
        required: SampleTime,
    },
    Raw {
        through: SampleTime,
        required: SampleTime,
    },
    Clock(TimeError),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("unprepared callback at {at}: {missing:?}")]
struct PrefixFault {
    at: SampleTime,
    missing: MissingPrefix,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum CheckedOutcome {
    Rendered(Rendered),
    Refused(PrefixFault),
}

struct PrefixCallback {
    callback: HostCallback,
    coverage: PrefixCoverage,
    maximum_block: FrameCount,
    output_at: SampleTime,
    fault: Option<PrefixFault>,
}

fn quantum_end(time: SampleTime) -> Result<SampleTime, TimeError> {
    if time.quantum_offset().as_u16() == 0 {
        Ok(time)
    } else {
        time.quantum_start().checked_add(FrameCount::QUANTUM)
    }
}

impl PrefixCallback {
    fn refuse(&mut self, samples: &mut [f32], missing: MissingPrefix) -> CheckedOutcome {
        let fault = *self.fault.get_or_insert(PrefixFault {
            at: self.output_at,
            missing,
        });
        self.callback.host_halt.request();
        self.callback.raw_halt.request_invalid();
        samples.fill(0.0);
        CheckedOutcome::Refused(fault)
    }

    fn render(&mut self, samples: &mut [f32]) -> CheckedOutcome {
        if let Some(fault) = self.fault {
            samples.fill(0.0);
            return CheckedOutcome::Refused(fault);
        }
        let channels = self.callback.mixed_layout.channels();
        if !samples.len().is_multiple_of(channels) {
            samples.fill(0.0);
            return CheckedOutcome::Rendered(Rendered {
                mixed: Err(MixedHostRenderError::Host(
                    MixedTransferRenderError::OutputShape,
                )),
                raw_ok: false,
            });
        }
        let Ok(frames) = u64::try_from(samples.len() / channels) else {
            return self.refuse(
                samples,
                MissingPrefix::Clock(TimeError::ClockExhausted {
                    position: self.output_at.as_u64(),
                    frames: u64::MAX,
                }),
            );
        };
        if frames > self.maximum_block.as_u64() {
            samples.fill(0.0);
            return CheckedOutcome::Rendered(Rendered {
                mixed: Err(MixedHostRenderError::Host(
                    MixedTransferRenderError::OutputShape,
                )),
                raw_ok: false,
            });
        }
        let (end, raw, sound) = match self
            .output_at
            .checked_add(FrameCount::new(frames))
            .and_then(|end| {
                // Both renderers begin with Q64 carry. Their first opened quantum
                // starts at engine frame zero, after that initial silent output.
                let opened = quantum_end(SampleTime::new(
                    end.as_u64().saturating_sub(u64::from(QUANTUM_FRAMES)),
                ))?;
                Ok((end, opened, opened))
            }) {
            Ok(required) => required,
            Err(error) => return self.refuse(samples, MissingPrefix::Clock(error)),
        };
        for source in self.coverage.sources {
            if source.through < raw {
                return self.refuse(
                    samples,
                    MissingPrefix::Source {
                        generation: source.generation,
                        through: source.through,
                        required: raw,
                    },
                );
            }
        }
        if self.coverage.sound < sound {
            return self.refuse(
                samples,
                MissingPrefix::Sound {
                    through: self.coverage.sound,
                    required: sound,
                },
            );
        }
        if self.coverage.raw < raw {
            return self.refuse(
                samples,
                MissingPrefix::Raw {
                    through: self.coverage.raw,
                    required: raw,
                },
            );
        }
        let rendered = self.callback.render(samples);
        self.output_at = end;
        CheckedOutcome::Rendered(rendered)
    }
}

fn profile(rate: SampleRate, layout: ChannelLayout, maximum: FrameCount) -> HostProfile {
    HostProfile::harness(rate, maximum, layout).expect("separate prefix fixture profile")
}
fn fresh(profile: HostProfile) -> Rig {
    fresh_with(profile, Setup::default())
}
fn fresh_with(profile: HostProfile, setup: Setup) -> Rig {
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(setup, prepared, candidate, profile);
    rig.raw_queued_inputs = Some(Vec::with_capacity(SCRIPT_MESSAGES + 2));
    rig
}
fn script(source: usize) -> Vec<InputObservation> {
    let (on, bend_at, pedal, off, lift, again, end, key) = match source {
        FIRST => (80, 160, 200, 240, 300, 420, 720, 60),
        SECOND => (90, 180, 220, 260, 320, 500, 800, 64),
        _ => panic!("two sources"),
    };
    vec![
        message(on, note_on(key)),
        message(bend_at, bend(8_400)),
        message(pedal, sustain(true)),
        message(off, note_off(key)),
        message(lift, sustain(false)),
        message(again, note_on(key)),
        message(end, note_off(key)),
        frontier(PREFIX_END.as_u64()),
    ]
}
fn stage_script(rig: &mut Rig) {
    for source in [FIRST, SECOND] {
        rig.send_all(source, &script(source));
    }
    rig.service();
    assert!(rig.errors.is_empty() && rig.raw_errors.is_empty());
    assert_eq!(rig.messages().count(), SCRIPT_MESSAGES);
}

#[test]
fn declared_maximum_block_controls_prepared_sound_prefix_and_preserves_raw_custody() {
    let rate = SampleRate::new(48_000.0).expect("rate");
    for (maximum, first_unprepared) in [(64, Some(80)), (256, Some(260)), (1_024, None)] {
        let profile = profile(rate, ChannelLayout::Stereo, FrameCount::new(maximum));
        let mut rig = fresh(profile);
        stage_script(&mut rig);
        let coverage = PrefixCoverage::capture(&rig, profile);
        assert_eq!(coverage.sound, at(maximum));
        assert_eq!(
            coverage.raw, PREFIX_END,
            "raw admission is not enough; both frontiers were queued"
        );
        assert!(
            coverage
                .sources
                .iter()
                .all(|source| source.through == PREFIX_END)
        );
        if let Some(stamp) = first_unprepared {
            assert_eq!(
                rig.waits.last(),
                Some(&MixedBridgeWait::Window {
                    stamp: at(stamp),
                    end: at(maximum)
                })
            );
        } else {
            assert_eq!(rig.waits.last(), Some(&MixedBridgeWait::Empty));
        }
        let joined = rig.join();
        joined.assert_each_packet_once();
        assert!(joined.unexamined.is_empty() && joined.retired.is_empty());
    }
}

fn checked(rig: &mut Rig, profile: HostProfile) -> PrefixCallback {
    let coverage = PrefixCoverage::capture(rig, profile);
    PrefixCallback {
        callback: rig.callback.take().expect("one audio handoff"),
        coverage,
        maximum_block: profile.capabilities().maximum_block_size(),
        output_at: SampleTime::ZERO,
        fault: None,
    }
}
fn require_rendered(outcome: CheckedOutcome) {
    assert_eq!(
        outcome,
        CheckedOutcome::Rendered(Rendered {
            mixed: Ok(()),
            raw_ok: true
        })
    );
}

#[test]
fn both_renderers_share_initial_carry_and_open_whole_quanta_on_partial_callbacks() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    stage_script(&mut rig);
    let mut audio = checked(&mut rig, profile);
    for (frames, output_end, engine_end) in [
        (1, 1, 0),
        (62, 63, 0),
        (1, 64, 0),
        (1, 65, 64),
        (63, 128, 64),
        (1, 129, 128),
    ] {
        let mut samples = vec![0.25; frames * 2];
        require_rendered(callback_probe::without_heap(|| audio.render(&mut samples)));
        assert_eq!(audio.output_at, at(output_end));
        assert_eq!(rig.control.audio_boundary(), at(engine_end));
        assert_eq!(audio.callback.raw.acknowledged().clock, at(engine_end));
    }
    rig.callback = Some(audio.callback);
    let joined = rig.join();
    joined.assert_each_packet_once();
}

struct PrefixReturn {
    audio: PrefixCallback,
    samples: Vec<f32>,
    outcomes: Vec<CheckedOutcome>,
}
struct AudioTask {
    worker: Option<thread::JoinHandle<PrefixReturn>>,
    stop: Arc<AtomicBool>,
    joined: Arc<AtomicBool>,
}
impl AudioTask {
    fn spawn(mut audio: PrefixCallback, partitions: Vec<usize>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let audio_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let channels = audio.callback.mixed_layout.channels();
            let mut samples = vec![0.25; 1_024 * channels];
            let mut outcomes = Vec::with_capacity(partitions.len());
            let mut offset = 0;
            for frames in partitions {
                if audio_stop.load(Ordering::Acquire) {
                    break;
                }
                let end = offset + frames * channels;
                let outcome =
                    callback_probe::without_heap(|| audio.render(&mut samples[offset..end]));
                outcomes.push(outcome);
                offset = end;
            }
            // No permits, clock reads, sleeps, yields or control service between calls.
            PrefixReturn {
                audio,
                samples,
                outcomes,
            }
        });
        Self {
            worker: Some(worker),
            stop,
            joined: Arc::new(AtomicBool::new(false)),
        }
    }
    fn join(mut self) -> PrefixReturn {
        let returned = self
            .worker
            .take()
            .expect("audio handle")
            .join()
            .expect("bounded prefix worker");
        self.joined.store(true, Ordering::Release);
        returned
    }
}
impl Drop for AudioTask {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(returned) => drop(returned),
                Err(reason) => eprintln!("prefix worker failed during cleanup: {reason:?}"),
            }
            self.joined.store(true, Ordering::Release);
        }
    }
}

fn partitions(pattern: &[usize]) -> Vec<usize> {
    assert!(!pattern.is_empty() && pattern.iter().all(|frames| *frames > 0 && *frames <= 1_024));
    let mut result = Vec::new();
    let mut frames = 0;
    while frames < 1_024 {
        let next = pattern[result.len() % pattern.len()].min(1_024 - frames);
        result.push(next);
        frames += next;
    }
    result
}
fn stage_from_workers(rig: &mut Rig) {
    let producers = [
        rig.producers[FIRST].take().expect("first producer"),
        rig.producers[SECOND].take().expect("second producer"),
    ];
    let returned = thread::scope(|scope| {
        let workers = producers
            .into_iter()
            .enumerate()
            .map(|(source, mut producer)| {
                scope.spawn(move || {
                    for observation in script(source) {
                        assert!(producer.submit_owned(observation).is_ok());
                    }
                    producer
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .map(|worker| worker.join())
            .collect::<Vec<_>>()
    });
    for (source, producer) in returned.into_iter().enumerate() {
        rig.producers[source] = Some(producer.expect("source worker joined before startup seal"));
    }
    rig.service();
    assert_eq!(rig.messages().count(), SCRIPT_MESSAGES);
    assert!(rig.errors.is_empty() && rig.raw_errors.is_empty());
}
fn healthy_result(mut rig: Rig) -> (Vec<u32>, Vec<String>, Vec<String>) {
    rig.collect();
    assert_eq!(rig.settled.len(), SCRIPT_MESSAGES);
    for (_, settled) in &rig.settled {
        let Settled::Report(report) = settled else {
            panic!("healthy command did not execute: {settled:?}");
        };
        assert_eq!(report.nominal_at(), report.stamp());
        assert_eq!(report.stamp(), report.executed_at());
        match report.outcome() {
            MixedBridgeOutcome::NoteOn(result) => assert!(result.is_ok()),
            MixedBridgeOutcome::KeyUp { result, .. } => assert!(result.is_ok()),
            MixedBridgeOutcome::Bend { result, .. } => assert!(result.is_ok()),
            MixedBridgeOutcome::PedalDown { result, .. } => assert!(result.is_ok()),
            MixedBridgeOutcome::Lift { result, .. } => assert!(result.is_ok()),
            other => panic!("unrequested sound outcome: {other:?}"),
        }
    }
    assert!(rig.rendered.iter().any(|sample| *sample != 0.0));
    finish(rig)
}
fn serial_prefix(profile: HostProfile) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut rig = fresh(profile);
    stage_script(&mut rig);
    // Independent serial oracle calls the existing HostCallback via Rig, without guard.
    for _ in 0..16 {
        rig.render(64).expect("serial prefix");
    }
    healthy_result(rig)
}
fn autonomous_prefix(
    profile: HostProfile,
    pattern: &[usize],
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut rig = fresh(profile);
    stage_from_workers(&mut rig);
    let coverage = PrefixCoverage::capture(&rig, profile);
    assert_eq!(coverage.sound, PREFIX_END);
    assert_eq!(coverage.raw, PREFIX_END);
    assert!(
        coverage
            .sources
            .iter()
            .all(|source| source.through == PREFIX_END)
    );
    assert_eq!(
        rig.raw_queued_inputs.as_ref().expect("trace").len(),
        SCRIPT_MESSAGES + 2
    );
    assert_eq!(
        rig.raw_packets.ring.occupied_len(),
        SCRIPT_MESSAGES + 5,
        "all admitted observations, both start fences, and Play are already sent"
    );
    assert_eq!(
        rig.forwarded.len(),
        SCRIPT_MESSAGES,
        "all sound commands fit before handoff"
    );
    let parts = partitions(pattern);
    let count = parts.len();
    let audio = checked(&mut rig, profile);
    let worker = AudioTask::spawn(audio, parts);
    // Control does not pump, collect or advance a render credit during playback.
    let returned = worker.join();
    assert_eq!(returned.outcomes.len(), count, "no extra final callback");
    for outcome in returned.outcomes {
        require_rendered(outcome);
    }
    assert_eq!(returned.audio.output_at, PREFIX_END);
    assert_eq!(returned.audio.fault, None);
    rig.callback = Some(returned.audio.callback);
    rig.rendered = returned.samples;
    healthy_result(rig)
}

#[test]
fn autonomous_saved_prefix_matches_serial_in_three_partitions_at_four_rates_and_both_layouts() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for layout in [ChannelLayout::Stereo, ChannelLayout::Mono] {
            let profile = profile(
                SampleRate::new(rate).expect("rate"),
                layout,
                FrameCount::new(1_024),
            );
            let reference = serial_prefix(profile);
            for pattern in [&[64][..], &[7, 13, 1, 43][..], &[1_024][..]] {
                assert_eq!(
                    autonomous_prefix(profile, pattern),
                    reference,
                    "{rate} Hz {layout:?} {pattern:?}"
                );
            }
        }
    }
}

/// Once the host gate closes, control may collect but cannot stage or pump a refill.
/// The bridge's own fault remains separate from this host-owned preflight reason.
fn service_prefix_control(rig: &mut Rig) {
    rig.collect();
    if !rig.host_halt.is_requested() {
        rig.service();
    }
}
fn run_closed_prefix(
    mut rig: Rig,
    profile: HostProfile,
    expected_at: SampleTime,
    expected_missing: MissingPrefix,
    expected_staged: usize,
    expected_unserved: usize,
    expected_reports: usize,
) -> Joined {
    let coverage = PrefixCoverage::capture(&rig, profile);
    let audio = checked(&mut rig, profile);
    let returned = AudioTask::spawn(audio, partitions(&[64])).join();
    assert_eq!(returned.outcomes.len(), 16);
    let fault = PrefixFault {
        at: expected_at,
        missing: expected_missing,
    };
    assert_eq!(returned.audio.fault, Some(fault));
    let healthy = usize::try_from(expected_at.as_u64() / 64).expect("bounded prefix");
    for outcome in &returned.outcomes[..healthy] {
        require_rendered(*outcome);
    }
    assert!(
        returned.outcomes[healthy..]
            .iter()
            .all(|outcome| *outcome == CheckedOutcome::Refused(fault))
    );
    let channels = rig.mixed_layout.channels();
    assert!(
        silent(&returned.samples[healthy * Q * channels..]),
        "entire first failing callback and all later output"
    );
    let mut audio = returned.audio;
    assert_eq!(
        audio.output_at, expected_at,
        "failed callbacks advanced neither owner"
    );
    let raw_clock = audio.callback.raw.acknowledged().clock;
    let sound_clock = rig.control.audio_boundary();
    // A late, even fully covered snapshot cannot revive this arm.
    audio.coverage = PrefixCoverage {
        sound: PREFIX_END,
        raw: PREFIX_END,
        sources: coverage.sources.map(|source| SourcePrefix {
            through: PREFIX_END,
            ..source
        }),
    };
    let mut retry = vec![0.25; Q * channels];
    assert_eq!(
        callback_probe::without_heap(|| audio.render(&mut retry)),
        CheckedOutcome::Refused(fault)
    );
    assert!(silent(&retry));
    assert_eq!(audio.callback.raw.acknowledged().clock, raw_clock);
    assert_eq!(rig.control.audio_boundary(), sound_clock);
    rig.callback = Some(audio.callback);
    rig.rendered = returned.samples;
    let admitted = rig.admitted.len();
    let forwarded = rig.forwarded.len();
    let late = message(1_040, note_on(65));
    rig.send(FIRST, late);
    service_prefix_control(&mut rig);
    assert_eq!(
        rig.halted_sends,
        [(FIRST, late)],
        "late producer retained its original"
    );
    assert_eq!(rig.admitted.len(), admitted);
    assert_eq!(
        rig.forwarded.len(),
        forwarded,
        "host halt also blocks already-staged refill"
    );
    assert_eq!(rig.control.audio_boundary(), sound_clock);
    let frames = rig.output_frames();
    let joined = rig.join();
    assert_eq!(
        frames, 1_024,
        "the fixed callback list, including silent faults"
    );
    assert_eq!(
        joined.rendered.len() / channels,
        frames,
        "join performs no extra render"
    );
    joined.assert_each_packet_once();
    assert!(joined.host_halted && joined.raw_halted);
    assert_eq!(
        joined.teardown.fault(),
        None,
        "host reason is PrefixFault, not an invented engine/raw fault"
    );
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert_eq!(
        joined
            .teardown
            .held()
            .iter()
            .filter(|(_, state)| *state == MixedSourceCustody::Staged)
            .count(),
        expected_staged
    );
    assert_eq!(
        joined
            .teardown
            .held()
            .iter()
            .filter(|(_, state)| matches!(
                state,
                MixedSourceCustody::Command {
                    place: MixedCommandPlace::Unserved,
                    ..
                }
            ))
            .count(),
        expected_unserved
    );
    assert_eq!(
        joined
            .settled
            .iter()
            .filter(|(_, state)| matches!(state, Settled::Report(_)))
            .count(),
        expected_reports
    );
    assert!(joined.unexamined.is_empty() && joined.retired.is_empty());
    assert_eq!(
        expected_staged + expected_unserved + expected_reports,
        SCRIPT_MESSAGES
    );
    joined
}

#[test]
fn sound_window_exhaustion_faults_before_missing_commands_and_classifies_held_releases() {
    for (rate, layout) in candidates() {
        for (maximum, cut, required, staged, reports) in
            [(64, 128, 128, 14, 0), (256, 320, 320, 7, 7)]
        {
            let profile = profile(rate, layout, FrameCount::new(maximum));
            let mut rig = fresh(profile);
            stage_script(&mut rig);
            let joined = run_closed_prefix(
                rig,
                profile,
                at(cut),
                MissingPrefix::Sound {
                    through: at(maximum),
                    required: at(required),
                },
                staged,
                0,
                reports,
            );
            if maximum == 256 {
                let first = joined.packet_of(FIRST, 80);
                let second = joined.packet_of(SECOND, 90);
                assert!(
                    joined
                        .teardown
                        .open()
                        .contains(&(Some(first), MixedTransferHandleState::Sustained))
                );
                assert!(
                    joined
                        .teardown
                        .open()
                        .contains(&(Some(second), MixedTransferHandleState::Open))
                );
                assert_eq!(
                    joined.teardown.transfer().holds_outstanding(),
                    EventCount::measured(2)
                );
            }
        }
    }
}

#[test]
fn raw_session_full_and_ring_full_cannot_masquerade_as_prepared_coverage() {
    for (rate, layout) in candidates() {
        for session_full in [false, true] {
            let profile = profile(rate, layout, FrameCount::new(1_024));
            let setup = if session_full {
                Setup {
                    raw_actions: SessionSourceCapacity::new(4).expect("four action places"),
                    ..Setup::default()
                }
            } else {
                Setup {
                    raw_ring: MixedTransferPlaces::new(4),
                    ..Setup::default()
                }
            };
            let mut rig = fresh_with(profile, setup);
            stage_script(&mut rig);
            assert_eq!(
                rig.raw_unsent.is_none(),
                session_full,
                "Full at preparation returns None; ring Full retains the actual packet"
            );
            let coverage = PrefixCoverage::capture(&rig, profile);
            assert_eq!(coverage.sound, PREFIX_END);
            assert_eq!(coverage.raw, SampleTime::ZERO);
            assert!(rig.raw_queued_inputs.as_ref().expect("trace").len() < SCRIPT_MESSAGES + 2);
            let _joined = run_closed_prefix(
                rig,
                profile,
                at(64),
                MissingPrefix::Raw {
                    through: SampleTime::ZERO,
                    required: at(64),
                },
                0,
                SCRIPT_MESSAGES,
                0,
            );
        }
    }
}

#[test]
fn an_unpromised_source_prefix_stops_both_paths_and_keeps_every_source_id() {
    for (rate, layout) in candidates() {
        let profile = profile(rate, layout, FrameCount::new(1_024));
        let mut rig = fresh(profile);
        for source in [FIRST, SECOND] {
            let mut observations = script(source);
            if source == FIRST {
                assert!(matches!(
                    observations.pop(),
                    Some(InputObservation::Frontier { .. })
                ));
            }
            rig.send_all(source, &observations);
        }
        rig.service();
        let first = rig.generation(FIRST);
        let coverage = PrefixCoverage::capture(&rig, profile);
        assert_eq!(coverage.sources[FIRST].through, SampleTime::ZERO);
        assert_eq!(rig.messages().count(), SCRIPT_MESSAGES);
        let staged = rig
            .settled
            .iter()
            .filter(|(_, state)| matches!(state, Settled::Stage(_)))
            .count();
        assert_eq!(staged, 0);
        let forwarded = rig.forwarded.len();
        let _joined = run_closed_prefix(
            rig,
            profile,
            at(64),
            MissingPrefix::Source {
                generation: first,
                through: SampleTime::ZERO,
                required: at(64),
            },
            SCRIPT_MESSAGES - forwarded,
            forwarded,
            0,
        );
    }
}

fn candidates() -> impl Iterator<Item = (SampleRate, ChannelLayout)> {
    [44_100.0, 48_000.0, 96_000.0, 192_000.0]
        .into_iter()
        .flat_map(|rate| {
            [ChannelLayout::Stereo, ChannelLayout::Mono]
                .into_iter()
                .map(move |layout| (SampleRate::new(rate).expect("rate"), layout))
        })
}

#[test]
fn a_frontier_failed_at_ring_push_is_not_credited_as_raw_delivery() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh_with(
        profile,
        Setup {
            raw_ring: MixedTransferPlaces::new(4),
            ..Setup::default()
        },
    );
    for source in [FIRST, SECOND] {
        rig.send(source, frontier(PREFIX_END.as_u64()));
    }
    rig.service();
    let refused = rig
        .raw
        .control
        .packet_input_id(rig.raw_unsent.as_ref().expect("second frontier retained"))
        .expect("refused packet has a raw input ID");
    assert!(
        !rig.raw_queued_inputs
            .as_ref()
            .expect("trace")
            .contains(&refused)
    );
    let coverage = PrefixCoverage::capture(&rig, profile);
    assert_eq!(coverage.raw, SampleTime::ZERO);
    assert!(
        coverage
            .sources
            .iter()
            .all(|source| source.through == PREFIX_END)
    );
    let mut audio = checked(&mut rig, profile);
    let mut samples = vec![0.25; Q * 2];
    require_rendered(callback_probe::without_heap(|| audio.render(&mut samples)));
    let fault = PrefixFault {
        at: at(64),
        missing: MissingPrefix::Raw {
            through: SampleTime::ZERO,
            required: at(64),
        },
    };
    assert_eq!(
        callback_probe::without_heap(|| audio.render(&mut samples)),
        CheckedOutcome::Refused(fault)
    );
    assert!(silent(&samples));
    rig.callback = Some(audio.callback);
    let joined = rig.join();
    joined.assert_each_packet_once();
}

#[test]
fn control_unwind_joins_autonomous_worker_before_control_drops() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Mono,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    stage_from_workers(&mut rig);
    let audio = checked(&mut rig, profile);
    let worker = AudioTask::spawn(audio, partitions(&[7, 13, 1, 43]));
    let joined = Arc::clone(&worker.joined);
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _worker = worker;
        panic!("injected control failure during autonomous prefix");
    }));
    assert!(failed.is_err());
    assert!(joined.load(Ordering::Acquire));
    assert!(
        !rig.host_halt.is_requested(),
        "control halves are still alive after worker join"
    );
}

#[test]
fn invalid_output_shape_is_retryable_and_clock_exhaustion_is_a_silent_host_fault() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(64),
    );
    let mut rig = fresh(profile);
    stage_script(&mut rig);
    let packets = rig.raw_packets.ring.occupied_len();
    let mut audio = checked(&mut rig, profile);
    for length in [1, 130] {
        let mut samples = vec![0.25; length];
        assert_eq!(
            callback_probe::without_heap(|| audio.render(&mut samples)),
            CheckedOutcome::Rendered(Rendered {
                mixed: Err(MixedHostRenderError::Host(
                    MixedTransferRenderError::OutputShape
                )),
                raw_ok: false
            })
        );
        assert!(silent(&samples));
        assert_eq!(audio.fault, None);
        assert_eq!(audio.output_at, SampleTime::ZERO);
        assert_eq!(
            audio.callback.packets.occupied_len(),
            packets,
            "shape validation preceded raw enqueue"
        );
    }
    audio.output_at = SampleTime::new(u64::MAX);
    let mut samples = vec![0.25; 2];
    let fault = PrefixFault {
        at: SampleTime::new(u64::MAX),
        missing: MissingPrefix::Clock(TimeError::ClockExhausted {
            position: u64::MAX,
            frames: 1,
        }),
    };
    assert_eq!(
        callback_probe::without_heap(|| audio.render(&mut samples)),
        CheckedOutcome::Refused(fault)
    );
    assert!(silent(&samples));
    rig.callback = Some(audio.callback);
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(joined.teardown.fault(), None);
}
