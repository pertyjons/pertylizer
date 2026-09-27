//! Non-shipping, exclusively scheduled live-input/parameter/swap/PCM simulator.
//! Run without an audio device: cargo run -p pertylizer --example v2_live_session
use synth_engine_v2::{
    host::{
        EndpointId, OutputFormat,
        audio_input::{AudioInputConfig, InputFrame, SimulatedAudioInput},
        input::{
            InputCapacity, InputLimits, InputRate, InputTick, InputTickSpan, SimulatedInputClock,
            SimulatedNoteInput,
        },
        live::{AuditionId, SwappingLiveStream, UpdateVersion},
    },
    ingress::ReleaseCause,
    ir::{
        ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations,
        PortId, SignalDomain, parameters,
    },
    profile::HostProfile,
    quantities::{
        Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, NormalizedLevel,
        ParameterValue, PreparedBytes, SampleRate, Seconds,
    },
    recording::notes::Midi1Input,
    render::AudioBlockMut,
    time::{FrameCount, SampleTime},
    tuning::PreparedTuning,
};
type Error = Box<dyn std::error::Error>;
const ENVELOPE: NodeId = NodeId::new(2);
fn graph() -> Result<GraphIr, Error> {
    Ok(GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0)?,
                amplitude: Amplitude::new(0.05)?,
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(NodeId::new(3), IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(NodeId::new(4), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(3), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (NodeId::new(3), synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (NodeId::new(3), PortId::FIRST),
            (NodeId::new(4), PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, PreparedTuning::equal_temperament()?)
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(4),
                simultaneous_holds: EventCount::measured(4),
            }],
            ..PlanDeclarations::default()
        })
        .build()?)
}
fn render(live: &mut SwappingLiveStream, input: &mut SimulatedAudioInput) -> Result<(), Error> {
    let mut synth = [0.0; 64];
    let mut monitor = [0.0; 64];
    live.render(AudioBlockMut::new(&mut synth, 64, ChannelLayout::Mono)?)?;
    input.monitor(AudioBlockMut::new(&mut monitor, 64, ChannelLayout::Mono)?)?;
    for (synth, input) in synth.iter_mut().zip(monitor) {
        *synth += input;
    }
    if synth.iter().any(|sample| !sample.is_finite()) {
        return Err("nonfinite mixed output".into());
    }
    Ok(())
}
fn exercise(cycles: u32) -> Result<(usize, u64), Error> {
    let graph = graph()?;
    let source = SimulatedNoteInput::new(
        EndpointId::new("simulated-midi".into())?,
        InputLimits {
            cells: InputCapacity::new(16)?,
            bytes: PreparedBytes::measured(65536),
        },
    )?
    .begin()?;
    let format = OutputFormat {
        rate: SampleRate::new(48000.0)?,
        layout: ChannelLayout::Mono,
    };
    let profile = HostProfile::harness(format.rate, FrameCount::new(256), format.layout)?;
    let parameters = [(ENVELOPE, parameters::ENVELOPE_SUSTAIN)];
    let mut live = SwappingLiveStream::prepare(
        &graph,
        profile,
        ENVELOPE,
        &[source],
        &parameters,
        EventCount::measured(16),
        PreparedBytes::measured(8_000_000),
    )?;
    let config = AudioInputConfig {
        input: format,
        output: format,
        clock: SimulatedInputClock::new(
            live.active().epoch(),
            SampleTime::ZERO,
            InputTick::new(0),
            InputRate::new(FrameCount::new(1), InputTickSpan::new(1))?,
            InputTickSpan::new(0),
        ),
        first: InputFrame::new(0),
        chunk_frames: FrameCount::new(256),
        chunk_cells: EventCount::measured(4),
        take_frames: FrameCount::new(u64::from(cycles) * 192),
        monitor_frames: FrameCount::new(1024),
        target_frames: FrameCount::new(512),
        bytes: PreparedBytes::measured(32_000_000),
    };
    let mut input = SimulatedAudioInput::prepare(config)?;
    render(&mut live, &mut input)?;
    for cycle in 0..cycles {
        if input
            .input(InputFrame::new(u64::from(cycle) * 192), &[0.01; 192])?
            .is_some()
        {
            return Err("recording discontinuity".into());
        }
        let slot = live
            .active()
            .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
            .ok_or("missing prepared parameter")?;
        let _superseded = live.update_parameter(
            slot,
            UpdateVersion::new(u64::from(cycle) + 1)?,
            ParameterValue::new(0.75)?,
        )?;
        for (index, bytes) in [[0x90, 60, 100], [0xe0, 0, 72], [0x80, 60, 0]]
            .into_iter()
            .enumerate()
        {
            let id = AuditionId::new(source, u64::from(cycle) * 3 + u64::try_from(index)? + 1)?;
            live.queue(id, live.active().clock(), Midi1Input::from_bytes(bytes)?)?;
            render(&mut live, &mut input)?;
            let _outcome = live.take_outcome(id).ok_or("missing audition outcome")?;
            if cycle == cycles / 2 && index == 0 {
                let _plan = live.prepare_candidate(&graph, ENVELOPE, &parameters)?;
                render(&mut live, &mut input)?;
                let mut cleared = false;
                if !live.collect_retired(|old| {
                    cleared = old.holds() == EventCount::NONE && !old.has_sounding_obligations()
                }) || !cleared
                {
                    return Err("plan retirement retained sounding obligations".into());
                }
            }
        }
        input.drain_worker();
    }
    live.end_at(live.active().clock(), ReleaseCause::Panic)?;
    render(&mut live, &mut input)?;
    input.stop(false);
    let dropped = input.monitor_counters().dropped.as_u64();
    let take = input.finish();
    if take.first_gap().is_some() || take.samples().iter().any(|sample| *sample != 0.01) {
        return Err("recorded PCM differs from original source".into());
    }
    Ok((take.samples().len(), dropped))
}
fn main() -> Result<(), Error> {
    let (samples, monitor_dropped) = exercise(2048)?;
    println!(
        "cycles=2048 recorded_samples={samples} monitor_dropped={monitor_dropped} plan_swap=complete"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_later_source_release_does_not_free_an_earlier_onset_hold() {
        use synth_engine_v2::{
            host::live::AuditionOutcome,
            ingress::{ExhaustedResource, IngressRefused},
        };

        let source = |name: &str| {
            SimulatedNoteInput::new(
                EndpointId::new(name.into()).unwrap(),
                InputLimits {
                    cells: InputCapacity::new(4).unwrap(),
                    bytes: PreparedBytes::measured(65536),
                },
            )
            .unwrap()
            .begin()
            .unwrap()
        };
        let [first, second] = [source("hold-a"), source("hold-b")];
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(256),
            ChannelLayout::Mono,
        )
        .unwrap();
        let mut live = SwappingLiveStream::prepare(
            &graph().unwrap(),
            profile,
            ENVELOPE,
            &[first, second],
            &[],
            EventCount::measured(16),
            PreparedBytes::measured(8_000_000),
        )
        .unwrap();
        for (index, key) in (60..64).enumerate() {
            live.queue(
                AuditionId::new(first, u64::try_from(index + 1).unwrap()).unwrap(),
                SampleTime::new(10),
                Midi1Input::from_bytes([0x90, key, 100]).unwrap(),
            )
            .unwrap();
        }
        let mut samples = [0.0; 256];
        live.render(AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).unwrap())
            .unwrap();
        assert_eq!(live.active().holds(), EventCount::measured(4));

        let clock = live.active().clock();
        let release = AuditionId::new(first, 5).unwrap();
        let earlier_onset = AuditionId::new(second, 1).unwrap();
        // Queue the release first; the renderer must still stage the earlier onset first.
        live.queue(
            release,
            clock.checked_add(FrameCount::new(20)).unwrap(),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
        )
        .unwrap();
        live.queue(
            earlier_onset,
            clock.checked_add(FrameCount::new(10)).unwrap(),
            Midi1Input::from_bytes([0x90, 64, 100]).unwrap(),
        )
        .unwrap();
        live.render(AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).unwrap())
            .unwrap();
        assert!(matches!(
            live.take_outcome(earlier_onset),
            Some(AuditionOutcome::Refused(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold
            }))
        ));
        assert!(matches!(
            live.take_outcome(release),
            Some(AuditionOutcome::Executed { .. })
        ));
        assert_eq!(live.active().holds(), EventCount::measured(3));
    }
    #[test]
    fn complete_simulated_session_retains_original_pcm_through_running_swap() {
        let (samples, _) = exercise(256).unwrap();
        assert_eq!(samples, 256 * 192);
    }
    #[test]
    fn live_swap_and_pcm_callbacks_neither_allocate_nor_destroy_owners() {
        let graph = graph().unwrap();
        let source = SimulatedNoteInput::new(
            EndpointId::new("allocation".into()).unwrap(),
            InputLimits {
                cells: InputCapacity::new(4).unwrap(),
                bytes: PreparedBytes::measured(65536),
            },
        )
        .unwrap()
        .begin()
        .unwrap();
        let format = OutputFormat {
            rate: SampleRate::new(48000.0).unwrap(),
            layout: ChannelLayout::Mono,
        };
        let profile =
            HostProfile::harness(format.rate, FrameCount::new(256), format.layout).unwrap();
        let mut live = SwappingLiveStream::prepare(
            &graph,
            profile,
            ENVELOPE,
            &[source],
            &[],
            EventCount::measured(16),
            PreparedBytes::measured(8_000_000),
        )
        .unwrap();
        let mut input = SimulatedAudioInput::prepare(AudioInputConfig {
            input: format,
            output: format,
            clock: SimulatedInputClock::new(
                live.active().epoch(),
                SampleTime::ZERO,
                InputTick::new(0),
                InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
                InputTickSpan::new(0),
            ),
            first: InputFrame::new(0),
            chunk_frames: FrameCount::new(256),
            chunk_cells: EventCount::measured(2),
            take_frames: FrameCount::new(4096),
            monitor_frames: FrameCount::new(1024),
            target_frames: FrameCount::new(512),
            bytes: PreparedBytes::measured(1_000_000),
        })
        .unwrap();
        render(&mut live, &mut input).unwrap();
        live.queue(
            AuditionId::new(source, 1).unwrap(),
            SampleTime::ZERO,
            Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
        )
        .unwrap();
        render(&mut live, &mut input).unwrap();
        let _plan = live.prepare_candidate(&graph, ENVELOPE, &[]).unwrap();
        let measurement = allocation_counter::measure(|| {
            input.input(InputFrame::new(0), &[0.1; 256]).unwrap();
            render(&mut live, &mut input).unwrap();
            live.end_at(live.active().clock(), ReleaseCause::Panic)
                .unwrap();
            render(&mut live, &mut input).unwrap();
            input.stop(true);
        });
        assert_eq!(measurement.count_total, 0);
        assert_eq!(measurement.count_current, 0);
        assert!(live.collect_retired(|old| assert!(!old.has_sounding_obligations())));
    }
    #[test]
    fn two_renderer_peak_and_audio_pool_fit_their_reported_preparation_grants() {
        let graph = graph().unwrap();
        let source = SimulatedNoteInput::new(
            EndpointId::new("bytes".into()).unwrap(),
            InputLimits {
                cells: InputCapacity::new(4).unwrap(),
                bytes: PreparedBytes::measured(65536),
            },
        )
        .unwrap()
        .begin()
        .unwrap();
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(256),
            ChannelLayout::Mono,
        )
        .unwrap();
        let mut prepared = None;
        let first = allocation_counter::measure(|| {
            prepared = Some(
                SwappingLiveStream::prepare(
                    &graph,
                    profile,
                    ENVELOPE,
                    &[source],
                    &[(ENVELOPE, parameters::ENVELOPE_SUSTAIN)],
                    EventCount::measured(16),
                    PreparedBytes::measured(8_000_000),
                )
                .unwrap(),
            )
        });
        let mut live = prepared.unwrap();
        let second = allocation_counter::measure(|| {
            let _plan = live
                .prepare_candidate(
                    &graph,
                    ENVELOPE,
                    &[(ENVELOPE, parameters::ENVELOPE_SUSTAIN)],
                )
                .unwrap();
        });
        assert!(
            u64::try_from(first.bytes_current + second.bytes_current).unwrap()
                <= live.prepared_bytes().get(),
            "first={} second={} charged={}",
            first.bytes_current,
            second.bytes_current,
            live.prepared_bytes().get()
        );
        let format = OutputFormat {
            rate: SampleRate::new(48000.0).unwrap(),
            layout: ChannelLayout::Mono,
        };
        let config = AudioInputConfig {
            input: format,
            output: format,
            clock: SimulatedInputClock::new(
                live.active().epoch(),
                SampleTime::ZERO,
                InputTick::new(0),
                InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
                InputTickSpan::new(0),
            ),
            first: InputFrame::new(0),
            chunk_frames: FrameCount::new(256),
            chunk_cells: EventCount::measured(4),
            take_frames: FrameCount::new(4096),
            monitor_frames: FrameCount::new(1024),
            target_frames: FrameCount::new(512),
            bytes: PreparedBytes::measured(1_000_000),
        };
        let mut prepared = None;
        let measured = allocation_counter::measure(|| {
            prepared = Some(SimulatedAudioInput::prepare(config).unwrap())
        });
        assert!(u64::try_from(measured.bytes_current).unwrap() <= prepared.unwrap().bytes().get());
    }
}
