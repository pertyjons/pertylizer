//! Bounded latest-wins preparation and off-thread retirement for the experimental host.
use super::*;
use synth_engine_v2::{
    host::live::{LiveInputError, PreparedLivePlan, SwappingLiveStream},
    ir::{GraphIr, NodeId},
    plan::PlanId,
    profile::HostProfile,
    quantities::EventCount,
};

pub(super) const PAYLOAD_BYTES: PreparedBytes = PreparedBytes::measured(8_000_000);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SwapReport {
    /// Renders from `at` on (or from preparation before any swap); only the retiring
    /// plan's one-quantum fade tail may also sound after that boundary.
    pub active: Option<PlanId>,
    pub at: Option<synth_engine_v2::time::SampleTime>,
    pub clock: Option<synth_engine_v2::time::SampleTime>,
    /// The acknowledged callback delivered no audio from either plan.
    pub callback_failed: bool,
    pub rejected: u64,
    pub cancelled: u64,
}

pub struct SwapControl {
    feedback: triple_buffer::Output<SwapReport>,
    last_report: SwapReport,
    mailbox: triple_buffer::Input<Option<PreparedLivePlan>>,
    retired: HeapCons<PreparedLivePlan>,
    _retired: Arc<HeapRb<PreparedLivePlan>>,
    profile: HostProfile,
    sources: [ConnectionGeneration; 2],
    quota: EventCount,
}
pub struct SwapAudio {
    feedback: triple_buffer::Input<SwapReport>,
    report: SwapReport,
    mailbox: triple_buffer::Output<Option<PreparedLivePlan>>,
    retired: HeapProd<PreparedLivePlan>,
    retained: Option<PreparedLivePlan>,
}
impl SwapControl {
    pub fn prepare(
        profile: HostProfile,
        sources: [ConnectionGeneration; 2],
        quota: EventCount,
    ) -> (Self, SwapAudio) {
        let (writer, reader) = triple_buffer::TripleBuffer::default().split();
        let (feedback_writer, feedback_reader) = triple_buffer::TripleBuffer::default().split();
        let backing = Arc::new(HeapRb::new(1));
        let (retired, collector) = Arc::clone(&backing).split();
        (
            Self {
                feedback: feedback_reader,
                last_report: SwapReport::default(),
                mailbox: writer,
                retired: collector,
                _retired: backing,
                profile,
                sources,
                quota,
            },
            SwapAudio {
                feedback: feedback_writer,
                report: SwapReport::default(),
                mailbox: reader,
                retired,
                retained: None,
            },
        )
    }
    /// Seven payload positions plus one conservative failure-custody reserve:
    /// active, secondary, three mailbox cells, retirement queue, compiler local.
    pub fn bytes() -> PreparedBytes {
        PreparedBytes::measured(8 * PAYLOAD_BYTES.get() + 65_536)
    }
    pub fn publish(&mut self, graph: &GraphIr) -> Result<PlanId, LiveInputError> {
        let candidate = PreparedLivePlan::prepare(
            graph,
            self.profile,
            NodeId::new(2),
            &self.sources,
            &[],
            self.quota,
            PAYLOAD_BYTES,
        )?;
        let plan = candidate.plan_id();
        // Only the publisher replaces/drops unpublished superseded payloads.
        self.mailbox.write(Some(candidate));
        Ok(plan)
    }
    /// The newest audio-side acknowledgement, without collecting retired owners.
    #[cfg(test)]
    pub fn acknowledgement(&mut self) -> SwapReport {
        *self.feedback.read()
    }
    pub fn collect(&mut self) -> usize {
        let report = *self.feedback.read();
        if report != self.last_report {
            println!("live_plan_status={report:?}");
            self.last_report = report;
        }
        let prefix = self.retired.occupied_len();
        let mut count = 0;
        for _ in 0..prefix {
            if self.retired.try_pop().is_some() {
                count += 1;
            }
        }
        count
    }
}
impl SwapAudio {
    /// After every callback, including one that failed and delivered no audio.
    pub fn acknowledge(&mut self, renderer: &SwappingLiveStream, succeeded: bool) {
        self.report.active = Some(renderer.active().plan_id());
        self.report.clock = Some(renderer.clock());
        self.report.callback_failed = !succeeded;
        if let Some(synth_engine_v2::host::live::SwapOutcome::Installed { at, .. }) =
            renderer.swap_outcome()
        {
            self.report.at = Some(at);
        }
        self.feedback.write(self.report);
    }
    pub fn service(&mut self, renderer: &mut SwappingLiveStream) -> Result<(), LiveInputError> {
        if self.retained.is_none() && !self.retired.is_full() {
            self.retained = renderer.take_retired();
        }
        if !self.retired.is_full()
            && let Some(owner) = self.retained.take()
            && let Err(owner) = self.retired.try_push(owner)
        {
            self.retained = Some(owner);
        }
        if self.retained.is_none() && !self.retired.is_full() && renderer.replacement_closed() {
            let _changed = self.mailbox.update();
            self.retained = self.mailbox.output_buffer_mut().take();
            if self.retained.is_some() {
                self.report.cancelled = self.report.cancelled.saturating_add(1);
            }
        }
        if self.retained.is_none() && !self.retired.is_full() && renderer.can_accept_prepared() {
            let _changed = self.mailbox.update();
            if renderer
                .accept_prepared(self.mailbox.output_buffer_mut())
                .is_err()
            {
                self.report.rejected = self.report.rejected.saturating_add(1);
                // Invalid candidates retain custody and cannot fault the sounding plan.
                self.retained = self.mailbox.output_buffer_mut().take();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_engine_v2::{
        host::{
            EndpointId,
            input::{InputCapacity, InputLimits, SimulatedNoteInput},
            live::{AuditionId, AuditionOutcome},
        },
        quantities::{ChannelLayout, SampleRate},
        recording::notes::Midi1Input,
        render::AudioBlockMut,
        time::{FrameCount, SampleTime},
    };
    fn fixture() -> (
        SwapControl,
        SwapAudio,
        SwappingLiveStream,
        [ConnectionGeneration; 2],
    ) {
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(128),
            ChannelLayout::Mono,
        )
        .unwrap();
        let mut sources = Vec::new();
        for id in ["first", "second"] {
            sources.push(
                SimulatedNoteInput::new(
                    EndpointId::new(id.into()).unwrap(),
                    InputLimits {
                        cells: InputCapacity::new(16).unwrap(),
                        bytes: PreparedBytes::measured(65536),
                    },
                )
                .unwrap()
                .begin()
                .unwrap(),
            );
        }
        let sources = [sources[0], sources[1]];
        let (control, audio) = SwapControl::prepare(profile, sources, EventCount::measured(16));
        let renderer = SwappingLiveStream::prepare(
            &super::super::audition::live_graph().unwrap(),
            profile,
            NodeId::new(2),
            &sources,
            &[],
            EventCount::measured(16),
            PreparedBytes::measured(16_100_000),
        )
        .unwrap();
        (control, audio, renderer, sources)
    }
    fn render(audio: &mut SwapAudio, renderer: &mut SwappingLiveStream) -> [f32; 128] {
        let mut block = [0.0; 128];
        let measured = allocation_counter::measure(|| {
            for chunk in block.chunks_mut(64) {
                audio.service(renderer).unwrap();
                renderer
                    .render_deferred(AudioBlockMut::new(chunk, 64, ChannelLayout::Mono).unwrap())
                    .unwrap();
            }
            renderer.commit_outcomes();
            audio.acknowledge(renderer, true);
        });
        assert_eq!(measured.count_total, 0);
        assert_eq!(
            measured.count_current, 0,
            "no final destruction on callback"
        );
        block
    }
    #[test]
    fn latest_publication_moves_future_input_and_preserves_collector_custody() {
        let (mut control, mut audio, mut renderer, sources) = fixture();
        let graph = super::super::audition::live_graph().unwrap();
        let _first = control.publish(&graph).unwrap();
        let latest = control.publish(&graph).unwrap();
        let id = AuditionId::new(sources[0], 1).unwrap();
        renderer
            .queue(
                id,
                SampleTime::new(128),
                Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            )
            .unwrap();
        render(&mut audio, &mut renderer);
        assert_eq!(renderer.active().plan_id(), latest);
        render(&mut audio, &mut renderer);
        assert!(
            matches!(renderer.take_outcome(id), Some(AuditionOutcome::Executed { at, .. }) if at == SampleTime::new(128))
        );
        let newer = control.publish(&graph).unwrap();
        for _ in 0..16 {
            render(&mut audio, &mut renderer);
        }
        assert_eq!(
            renderer.active().plan_id(),
            latest,
            "stalled retirement prevents another installation"
        );
        assert_eq!(control.collect(), 1);
        render(&mut audio, &mut renderer);
        assert_eq!(renderer.active().plan_id(), newer);
    }
    #[test]
    fn invalid_candidate_and_compile_failure_keep_the_active_voice_sounding() {
        let (mut control, mut audio, mut renderer, sources) = fixture();
        let original = renderer.active().plan_id();
        renderer
            .queue(
                AuditionId::new(sources[0], 1).unwrap(),
                SampleTime::ZERO,
                Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            )
            .unwrap();
        render(&mut audio, &mut renderer);
        let wrong = HostProfile::harness(
            SampleRate::new(44100.0).unwrap(),
            FrameCount::new(128),
            ChannelLayout::Mono,
        )
        .unwrap();
        control.mailbox.write(Some(
            PreparedLivePlan::prepare(
                &super::super::audition::live_graph().unwrap(),
                wrong,
                NodeId::new(2),
                &sources,
                &[],
                EventCount::measured(16),
                PAYLOAD_BYTES,
            )
            .unwrap(),
        ));
        assert!(
            render(&mut audio, &mut renderer)
                .iter()
                .any(|v| v.abs() > 0.001)
        );
        let invalid = GraphIr::builder().build().unwrap();
        assert!(control.publish(&invalid).is_err());
        assert!(
            render(&mut audio, &mut renderer)
                .iter()
                .any(|v| v.abs() > 0.001)
        );
        assert_eq!(renderer.active().plan_id(), original);
        assert_eq!(
            control.collect(),
            1,
            "the rejected payload returns to off-thread custody"
        );
        assert_eq!(control.last_report.rejected, 1);
        assert_eq!(control.last_report.cancelled, 0);
    }
    #[test]
    fn stop_latches_against_late_publications_and_join_needs_no_callback() {
        let (mut control, mut audio, mut renderer, _) = fixture();
        let original = renderer.active().plan_id();
        renderer
            .end_at(
                SampleTime::ZERO,
                synth_engine_v2::ingress::ReleaseCause::Panic,
            )
            .unwrap();
        let _candidate = control
            .publish(&super::super::audition::live_graph().unwrap())
            .unwrap();
        for _ in 0..4 {
            render(&mut audio, &mut renderer);
        }
        assert_eq!(renderer.active().plan_id(), original);
        assert_eq!(control.collect(), 1);
        assert_eq!(control.last_report.cancelled, 1);
        assert_eq!(control.last_report.rejected, 0);
        let _late = control
            .publish(&super::super::audition::live_graph().unwrap())
            .unwrap();
        renderer.recover_after_join();
        // Both endpoints are retained until join and all destruction is now off-thread.
        drop(audio);
        drop(control);
        drop(renderer);
    }
    #[test]
    fn a_retired_payload_cannot_be_reinstalled_as_a_fresh_candidate() {
        let (mut control, mut audio, mut renderer, _) = fixture();
        let graph = super::super::audition::live_graph().unwrap();
        let current = control.publish(&graph).unwrap();
        for _ in 0..4 {
            render(&mut audio, &mut renderer);
        }
        let mut retired = Some(control.retired.try_pop().unwrap());
        assert!(renderer.accept_prepared(&mut retired).is_err());
        assert!(retired.is_some(), "refusal retains the retired owner");
        assert_eq!(renderer.active().plan_id(), current);
        render(&mut audio, &mut renderer);
    }
    #[test]
    fn compiler_and_audio_run_concurrently_with_bounded_retirement() {
        let (mut control, mut audio, mut renderer, _) = fixture();
        let count = std::sync::atomic::AtomicU64::new(0);
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                while !stop.load(std::sync::atomic::Ordering::Acquire) {
                    render(&mut audio, &mut renderer);
                    count.fetch_add(1, std::sync::atomic::Ordering::Release);
                }
            });
            let before = count.load(std::sync::atomic::Ordering::Acquire);
            for _ in 0..16 {
                let _plan = control
                    .publish(&super::super::audition::live_graph().unwrap())
                    .unwrap();
                let _collected = control.collect();
            }
            assert!(count.load(std::sync::atomic::Ordering::Acquire) > before);
            stop.store(true, std::sync::atomic::Ordering::Release);
            handle.join().unwrap();
        });
    }

    fn edited_graph() -> GraphIr {
        use synth_engine_v2::{
            ir::{
                ExecutionScope, IrNodeKind, NoteProducerDeclaration, PlanDeclarations, PortId,
                SignalDomain,
            },
            quantities::{Amplitude, HeldNoteCount, NormalizedLevel, Seconds},
            tuning::PreparedTuning,
        };
        // Structurally different from `live_graph`: a second amplifier stage fed by a new
        // fan-out of the envelope, so a partial topology would be audible as a wrong level.
        GraphIr::builder()
            // A constant, not a free-running sine, so the comparison measures topology and
            // note timing rather than oscillator phase since plan installation.
            .node(
                NodeId::new(1),
                IrNodeKind::Constant {
                    level: Amplitude::new(0.05).unwrap(),
                },
                ExecutionScope::Voice,
            )
            .node(
                NodeId::new(2),
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
            .node(NodeId::new(6), IrNodeKind::Amplifier, ExecutionScope::Voice)
            .connect(
                (NodeId::new(1), PortId::FIRST),
                (NodeId::new(3), PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (NodeId::new(2), PortId::FIRST),
                (NodeId::new(3), synth_engine_v2::node::AMPLIFIER_CONTROL),
                SignalDomain::Control,
            )
            .connect(
                (NodeId::new(3), PortId::FIRST),
                (NodeId::new(6), PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (NodeId::new(2), PortId::FIRST),
                (NodeId::new(6), synth_engine_v2::node::AMPLIFIER_CONTROL),
                SignalDomain::Control,
            )
            .connect(
                (NodeId::new(6), PortId::FIRST),
                (NodeId::new(4), PortId::FIRST),
                SignalDomain::Audio,
            )
            .tuning(
                ExecutionScope::Voice,
                PreparedTuning::equal_temperament().unwrap(),
            )
            .declaring(PlanDeclarations {
                note_producers: vec![NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(4),
                    simultaneous_holds: EventCount::measured(4),
                }],
                ..PlanDeclarations::default()
            })
            .build()
            .unwrap()
    }
    /// Phase 9A items 2 and 11: a structural edit lands as one whole plan. A note sounding
    /// across the swap is the old plan's exactly until the boundary, then only its
    /// one-quantum fade tail; afterwards only the edited plan sounds, exactly as that plan
    /// prepared alone. The acknowledgement names the edited plan from its boundary.
    #[test]
    fn a_structural_edit_sounds_as_one_whole_plan_and_is_acknowledged() {
        let (mut control, mut audio, mut renderer, sources) = fixture();
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(128),
            ChannelLayout::Mono,
        )
        .unwrap();
        let reference = |graph: &GraphIr| {
            SwappingLiveStream::prepare(
                graph,
                profile,
                NodeId::new(2),
                &sources,
                &[],
                EventCount::measured(16),
                PreparedBytes::measured(16_100_000),
            )
            .unwrap()
        };
        let plain = |stream: &mut SwappingLiveStream| {
            let mut block = [0.0; 128];
            for chunk in block.chunks_mut(64) {
                stream
                    .render(AudioBlockMut::new(chunk, 64, ChannelLayout::Mono).unwrap())
                    .unwrap();
            }
            block
        };
        let note = |stream: &mut SwappingLiveStream, serial| {
            stream
                .queue(
                    AuditionId::new(sources[0], serial).unwrap(),
                    stream.clock(),
                    Midi1Input::from_bytes([0x90, 69, 100]).unwrap(),
                )
                .unwrap();
        };
        let original = renderer.active().plan_id();
        let mut old = reference(&super::super::audition::live_graph().unwrap());
        render(&mut audio, &mut renderer);
        plain(&mut old);
        note(&mut renderer, 1);
        note(&mut old, 1);
        for _ in 0..2 {
            let (actual, expected) = (render(&mut audio, &mut renderer), plain(&mut old));
            assert!(
                actual == expected,
                "before the edit the old plan sounds unchanged"
            );
            assert!(actual.iter().any(|sample| *sample != 0.0));
        }
        assert_eq!(control.acknowledgement().active, Some(original));

        let edited = edited_graph();
        let plan = control.publish(&edited).unwrap();
        let boundary = renderer.clock();
        let swapped = render(&mut audio, &mut renderer);
        let tail = plain(&mut old);
        for frame in 0..64 {
            let blend = f32::from(u16::try_from(frame + 1).unwrap()) / 64.0;
            assert_eq!(
                swapped[frame],
                tail[frame] * (1.0 - blend),
                "the boundary quantum is only the old plan's fade tail"
            );
        }
        assert!(tail[..64].iter().any(|sample| *sample != 0.0));
        assert!(
            swapped[64..].iter().all(|sample| *sample == 0.0),
            "after the fade only the edited plan sounds, and it holds no note yet"
        );
        assert_eq!(renderer.active().plan_id(), plan);
        let ack = control.acknowledgement();
        assert_eq!(ack.active, Some(plan));
        assert_eq!(
            ack.at,
            Some(boundary),
            "the acknowledged boundary is the fade's start"
        );
        assert!(!ack.callback_failed);

        let mut fresh = reference(&edited);
        plain(&mut fresh);
        note(&mut renderer, 2);
        note(&mut fresh, 1);
        let mut sounding = false;
        for _ in 0..8 {
            let (actual, expected) = (render(&mut audio, &mut renderer), plain(&mut fresh));
            assert!(
                actual == expected,
                "the edited plan sounds as prepared alone"
            );
            sounding |= actual.iter().any(|sample| *sample != 0.0);
        }
        assert!(sounding, "the comparison covers an audible note");
        assert_eq!(control.acknowledgement().active, Some(plan));
    }
    /// Phase 9A item 11: the acknowledgement follows the rendered plan across refused
    /// candidates and failed compiles, and a failed callback claims no rendered output.
    #[test]
    fn acknowledgement_survives_refusals_and_reports_a_failed_callback() {
        let (mut control, mut audio, mut renderer, sources) = fixture();
        let original = renderer.active().plan_id();
        render(&mut audio, &mut renderer);
        let wrong = HostProfile::harness(
            SampleRate::new(44100.0).unwrap(),
            FrameCount::new(128),
            ChannelLayout::Mono,
        )
        .unwrap();
        control.mailbox.write(Some(
            PreparedLivePlan::prepare(
                &super::super::audition::live_graph().unwrap(),
                wrong,
                NodeId::new(2),
                &sources,
                &[],
                EventCount::measured(16),
                PAYLOAD_BYTES,
            )
            .unwrap(),
        ));
        render(&mut audio, &mut renderer);
        let ack = control.acknowledgement();
        assert_eq!((ack.active, ack.rejected), (Some(original), 1));
        assert!(
            control
                .publish(&GraphIr::builder().build().unwrap())
                .is_err()
        );
        render(&mut audio, &mut renderer);
        let before = control.acknowledgement();
        assert_eq!(before.active, Some(original));
        assert!(!before.callback_failed);

        let mut oversized = [9.0; 129];
        let measured = allocation_counter::measure(|| {
            audio.service(&mut renderer).unwrap();
            let result = renderer.render_deferred(
                AudioBlockMut::new(&mut oversized, 129, ChannelLayout::Mono).unwrap(),
            );
            audio.acknowledge(&renderer, result.is_ok());
            assert!(result.is_err());
        });
        assert_eq!((measured.count_total, measured.count_current), (0, 0));
        assert_eq!(oversized, [0.0; 129], "a failed callback delivers no audio");
        let failed = control.acknowledgement();
        assert!(failed.callback_failed);
        assert_eq!(failed.active, Some(original));
        assert_eq!(failed.clock, before.clock, "no rendered output is claimed");
        render(&mut audio, &mut renderer);
        assert!(!control.acknowledgement().callback_failed);
    }
}
