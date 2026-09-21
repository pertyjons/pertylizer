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
    pub active: Option<PlanId>,
    pub at: Option<synth_engine_v2::time::SampleTime>,
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
    pub fn acknowledge(&mut self, renderer: &SwappingLiveStream) {
        self.report.active = Some(renderer.active().plan_id());
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
            audio.acknowledge(renderer);
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
}
