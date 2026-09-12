//! Linux harness plan mailbox. The owner must retain both endpoints through join.

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::Arc;
use synth_engine_v2::{
    host::session::transfer::{
        SessionAudio, SessionControl,
        replacement::{
            PlanPublicationId, PlanReplacementError, PlanReplacementReceipt,
            PreparedPlanReplacement, REPLACEMENT_CREDITS,
        },
    },
    quantities::PreparedBytes,
};
use thiserror::Error;
use triple_buffer::{Input, Output, TripleBuffer};

type Packet = Box<PreparedPlanReplacement>;
type Cell = Option<Packet>;

#[derive(Debug, Error)]
pub(crate) enum PlanLaneError {
    #[error("plan mailbox exceeds the declared byte budget")]
    Budget,
    #[error("plan transfer failed: {0}")]
    Transfer(String),
}

#[must_use]
pub(crate) struct PlanControl {
    input: Input<Cell>,
    returned: HeapCons<Packet>,
    _returned: Arc<HeapRb<Packet>>,
    failed: Vec<Packet>,
    receipts: Vec<PlanReplacementReceipt>,
}

#[must_use]
pub(crate) struct PlanAudio {
    output: Output<Cell>,
    returned: HeapProd<Packet>,
    pending: Cell,
    protocol_error: Option<PlanReplacementError>,
}

impl PlanControl {
    // triple_buffer 9 holds three padded pointer cells and one padded atomic in
    // one Arc. 4096 bytes conservatively covers these plus its aligned Arc header
    // on the audited crossbeam-utils layouts (maximum padding 256 bytes). A test
    // measures the actual allocation. This is not a deep plan-memory measurement.
    const MAILBOX_ALLOCATION: usize = 4096;
    // Heap storage only; ControlLane charges both inline owner layouts itself.
    pub(crate) fn storage_bytes() -> PreparedBytes {
        let bytes = Self::MAILBOX_ALLOCATION
            + size_of::<HeapRb<Packet>>()
            + size_of::<Packet>()
            + 2 * size_of::<usize>()
            + align_of::<HeapRb<Packet>>()
            + REPLACEMENT_CREDITS * size_of::<Packet>();
        // An unrepresentable charge conservatively exceeds this harness's budget.
        PreparedBytes::measured(u64::try_from(bytes).unwrap_or(u64::MAX))
    }
    pub(crate) fn prepare(budget: PreparedBytes) -> Result<(Self, PlanAudio), PlanLaneError> {
        if Self::storage_bytes() > budget {
            return Err(PlanLaneError::Budget);
        }
        let (input, output) = TripleBuffer::<Cell>::default().split();
        let returned = Arc::new(HeapRb::new(1));
        let (producer, consumer) = Arc::clone(&returned).split();
        Ok((
            Self {
                input,
                returned: consumer,
                _returned: returned,
                failed: Vec::with_capacity(REPLACEMENT_CREDITS),
                receipts: Vec::new(),
            },
            PlanAudio {
                output,
                returned: producer,
                pending: None,
                protocol_error: None,
            },
        ))
    }
    pub(crate) fn take_receipts(&mut self) -> Vec<PlanReplacementReceipt> {
        std::mem::take(&mut self.receipts)
    }
    pub(crate) fn receipts(&self) -> &[PlanReplacementReceipt] {
        &self.receipts
    }
    fn resolve(
        &mut self,
        control: &mut SessionControl,
        packet: Packet,
    ) -> Result<(), PlanLaneError> {
        let result = if packet.outcome().is_some() {
            control.collect_replacement(packet)
        } else {
            control.cancel_replacement(packet)
        };
        match result {
            Ok(receipt) => {
                // Report immediately off-thread, including when later close work fails.
                println!(
                    "plan_publication={:?} plan={:?} outcome={:?}",
                    receipt.id, receipt.plan, receipt.outcome
                );
                self.receipts.push(receipt);
                Ok(())
            }
            Err((packet, error)) => {
                let message = format!("{:?}: {error}", packet.id());
                self.failed.push(packet);
                Err(PlanLaneError::Transfer(message))
            }
        }
    }
    pub(crate) fn collect_ready(
        &mut self,
        control: &mut SessionControl,
    ) -> Result<(), PlanLaneError> {
        for _ in 0..REPLACEMENT_CREDITS {
            let Some(packet) = self.returned.try_pop() else {
                break;
            };
            self.resolve(control, packet)?;
        }
        Ok(())
    }
    pub(crate) fn publish(
        &mut self,
        control: &mut SessionControl,
        packet: Packet,
    ) -> Result<(), PlanLaneError> {
        // Caller prepares before calling; a compiler error never reaches publication.
        if let Some(old) = self.input.input_buffer_mut().take()
            && let Err(error) = self.resolve(control, old)
        {
            // Retain or cancel the new candidate too; never discard an owning credit.
            let cancellation = self.resolve(control, packet);
            return match cancellation {
                Ok(()) => Err(error),
                Err(second) => Err(PlanLaneError::Transfer(format!("{error}; {second}"))),
            };
        }
        *self.input.input_buffer_mut() = Some(packet);
        // Overwrite is allowed: the previous back value is now producer-private.
        let _overwrote_unread = self.input.publish();
        if let Some(old) = self.input.input_buffer_mut().take() {
            self.resolve(control, old)?;
        }
        Ok(())
    }
    /// Cancel unread publication without acquiring another credit. A candidate
    /// already taken by audio remains pending until its owning return is collected.
    pub(crate) fn withdraw(&mut self, control: &mut SessionControl) -> Result<bool, PlanLaneError> {
        let mut errors = Vec::new();
        if let Some(old) = self.input.input_buffer_mut().take()
            && let Err(error) = self.resolve(control, old)
        {
            errors.push(error.to_string());
        }
        let _cancelled_unread = self.input.publish(); // Empty producer cell.
        if let Some(old) = self.input.input_buffer_mut().take()
            && let Err(error) = self.resolve(control, old)
        {
            errors.push(error.to_string());
        }
        if let Err(error) = self.collect_ready(control) {
            errors.push(error.to_string());
        }
        if !errors.is_empty() {
            return Err(PlanLaneError::Transfer(errors.join("; ")));
        }
        Ok(!control.has_replacements())
    }
    /// Call only after the backend has joined. Every cell is recovered explicitly.
    pub(crate) fn finish(
        &mut self,
        control: &mut SessionControl,
        audio: &mut PlanAudio,
    ) -> Result<(), PlanLaneError> {
        let mut errors = Vec::new();
        for _ in 0..REPLACEMENT_CREDITS {
            let Some(packet) = self.returned.try_pop() else {
                break;
            };
            if let Err(error) = self.resolve(control, packet) {
                errors.push(error.to_string());
            }
        }
        if let Some(packet) = audio.pending.take()
            && let Err(error) = self.resolve(control, packet)
        {
            errors.push(error.to_string());
        }
        if let Some(packet) = self.input.input_buffer_mut().take()
            && let Err(error) = self.resolve(control, packet)
        {
            errors.push(error.to_string());
        }
        if let Some(packet) = audio.output.output_buffer_mut().take()
            && let Err(error) = self.resolve(control, packet)
        {
            errors.push(error.to_string());
        }
        let _cancelled_unread = self.input.publish();
        if let Some(packet) = self.input.input_buffer_mut().take()
            && let Err(error) = self.resolve(control, packet)
        {
            errors.push(error.to_string());
        }
        if let Some(error) = audio.protocol_error.take() {
            errors.push(error.to_string());
        }
        if control.has_replacements() {
            let unresolved: Vec<PlanPublicationId> =
                self.failed.iter().map(|packet| packet.id()).collect();
            errors.push(format!(
                "unresolved plan credits; retained packets: {unresolved:?}"
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(PlanLaneError::Transfer(errors.join("; ")))
        }
    }
}

impl PlanAudio {
    /// Called only with empty old carry. Returns false on a retained protocol error.
    pub(crate) fn boundary(&mut self, audio: &mut SessionAudio) -> bool {
        if self.protocol_error.is_some() {
            return false;
        }
        if self.returned.is_full() {
            return true;
        }
        if let Some(packet) = self.pending.take() {
            if let Err(packet) = self.returned.try_push(packet) {
                self.pending = Some(packet);
            }
            return true;
        }
        if !self.output.update() {
            return true;
        }
        let Some(packet) = self.output.output_buffer_mut().take() else {
            return true;
        };
        match audio.install_replacement(packet) {
            Ok(packet) => {
                if let Err(packet) = self.returned.try_push(packet) {
                    self.pending = Some(packet);
                }
                true
            }
            Err((packet, error)) => {
                self.pending = Some(packet);
                self.protocol_error = Some(error);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_engine_v2::{
        compile::{RenderConfig, compile},
        host::session::transfer::replacement::PlanReplacementOutcome,
        host::session::{SessionCommandCapacity, SessionLimits, transfer::SessionTransferLimits},
        ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
        profile::HostProfile,
        quantities::{Amplitude, ChannelLayout, SampleRate},
        render::AudioBlockMut,
        schedule::AdmittedCompiledStream,
        time::{FrameCount, SampleTime},
    };
    fn profile() -> HostProfile {
        HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(512),
            ChannelLayout::Mono,
        )
        .unwrap()
    }
    fn limits() -> SessionTransferLimits {
        SessionTransferLimits {
            session: SessionLimits {
                commands: SessionCommandCapacity::new(4).unwrap(),
                command_bytes: PreparedBytes::measured(16384),
            },
            control_bytes: PreparedBytes::measured(16384),
        }
    }
    fn plan(level: f32) -> synth_engine_v2::plan::CompiledPlan {
        let graph = GraphIr::builder()
            .node(
                NodeId::new(1),
                IrNodeKind::Constant {
                    level: Amplitude::new(level).unwrap(),
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
        compile(&graph, &RenderConfig::new(profile()))
            .into_plan()
            .unwrap()
    }
    fn setup() -> (SessionControl, SessionAudio, PlanControl, PlanAudio) {
        let plan = plan(0.25);
        let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
        let (control, mut audio) =
            SessionControl::prepare(plan, stream, profile(), limits()).unwrap();
        audio
            .render(AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap())
            .unwrap();
        let (plans, reader) = PlanControl::prepare(PreparedBytes::measured(8192)).unwrap();
        (control, audio, plans, reader)
    }
    fn candidate(control: &mut SessionControl) -> Packet {
        let plan = plan(0.5);
        let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
        control
            .prepare_replacement(plan, stream, profile(), limits())
            .unwrap()
    }
    fn no_allocations(f: impl FnOnce()) {
        let measured = allocation_counter::measure(f);
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.count_current, 0);
    }
    #[test]
    fn overwrite_immediately_reclaims_old_back_and_only_latest_is_installed() {
        let (mut control, mut audio, mut plans, mut reader) = setup();
        let first = candidate(&mut control);
        let old = first.id();
        plans.publish(&mut control, first).unwrap();
        let second = candidate(&mut control);
        let latest = second.id();
        let latest_plan = second.plan_id();
        plans.publish(&mut control, second).unwrap();
        assert_eq!(plans.receipts()[0].id, old);
        assert_eq!(
            plans.receipts()[0].outcome,
            PlanReplacementOutcome::Cancelled
        );
        no_allocations(|| assert!(reader.boundary(&mut audio)));
        plans.collect_ready(&mut control).unwrap();
        assert_eq!(plans.receipts()[1].id, latest);
        assert!(matches!(
            plans.receipts()[1].outcome,
            PlanReplacementOutcome::Installed(_)
        ));
        assert_eq!(audio.plan_id(), latest_plan);
        assert!(!control.has_replacements());
    }
    #[test]
    fn a_stalled_retirement_preserves_last_valid_publication_and_applies_latest_when_released() {
        let (mut control, mut audio, mut plans, mut reader) = setup();
        let first = candidate(&mut control);
        plans.publish(&mut control, first).unwrap();
        no_allocations(|| assert!(reader.boundary(&mut audio)));
        let installed = audio.plan_id();
        let mut latest = None;
        for _ in 0..20 {
            let next = candidate(&mut control);
            latest = Some(next.plan_id());
            plans.publish(&mut control, next).unwrap();
            no_allocations(|| assert!(reader.boundary(&mut audio)));
            assert_eq!(audio.plan_id(), installed);
        }
        plans.collect_ready(&mut control).unwrap();
        no_allocations(|| assert!(reader.boundary(&mut audio)));
        plans.collect_ready(&mut control).unwrap();
        assert_eq!(Some(audio.plan_id()), latest);
        assert!(!control.has_replacements());
        assert_eq!(plans.receipts().len(), 21);
    }
    #[test]
    fn withdrawal_needs_no_credit_and_quiescent_cleanup_recovers_every_cell() {
        let (mut control, mut audio, mut plans, mut reader) = setup();
        let mut packets: Vec<_> = (0..REPLACEMENT_CREDITS)
            .map(|_| candidate(&mut control))
            .collect();
        *plans.input.input_buffer_mut() = Some(packets.remove(0));
        let _overwrote = plans.input.publish();
        assert!(reader.output.update()); // Keep one consumer-private payload.
        *plans.input.input_buffer_mut() = Some(packets.remove(0));
        let _overwrote = plans.input.publish(); // One unread back payload.
        *plans.input.input_buffer_mut() = Some(packets.remove(0)); // One producer-private.
        assert!(!plans.withdraw(&mut control).unwrap());
        for packet in packets {
            assert_eq!(
                control.cancel_replacement(packet).unwrap().outcome,
                PlanReplacementOutcome::Cancelled
            );
        }
        audio.close_after_quiescence().unwrap();
        plans.finish(&mut control, &mut reader).unwrap();
        assert!(!control.has_replacements());
        assert_eq!(plans.receipts().len(), 3);
    }
    #[test]
    fn close_recovers_mailbox_and_uncollected_retirement_without_another_callback() {
        for install in [false, true] {
            let (mut control, mut audio, mut plans, mut reader) = setup();
            let first = candidate(&mut control);
            plans.publish(&mut control, first).unwrap();
            if install {
                no_allocations(|| assert!(reader.boundary(&mut audio)));
            }
            let second = candidate(&mut control);
            plans.publish(&mut control, second).unwrap();
            control.close_admission();
            audio.close_after_quiescence().unwrap();
            plans.finish(&mut control, &mut reader).unwrap();
            assert!(!control.has_replacements());
            assert_eq!(plans.receipts().len(), 2);
            assert!(plans.input.input_buffer().is_none());
            assert!(reader.output.output_buffer().is_none());
        }
    }
    #[test]
    fn measured_mailbox_and_return_storage_fit_the_declared_charge() {
        let mut owners = None;
        let measured = allocation_counter::measure(|| {
            owners = Some(PlanControl::prepare(PreparedBytes::measured(8192)).unwrap());
        });
        assert!(measured.bytes_total <= PlanControl::storage_bytes().get());
        assert_eq!(measured.count_total, 4); // Mailbox Arc, ring Arc, ring cells, failure cells.
        assert!(measured.count_current > 0);
        drop(owners);
    }
    #[test]
    fn separate_threads_publish_render_and_collect_without_callback_allocation() {
        let (mut control, mut audio, mut plans, mut reader) = setup();
        let packet = candidate(&mut control);
        let expected = packet.plan_id();
        plans.publish(&mut control, packet).unwrap();
        let (send, receive) = std::sync::mpsc::sync_channel(0);
        let worker = std::thread::spawn(move || {
            no_allocations(|| assert!(reader.boundary(&mut audio)));
            send.send(()).unwrap();
            for _ in 0..100 {
                no_allocations(|| {
                    assert!(reader.boundary(&mut audio));
                    audio
                        .render(
                            AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap(),
                        )
                        .unwrap();
                });
            }
            (audio, reader)
        });
        receive.recv().unwrap();
        plans.collect_ready(&mut control).unwrap();
        let (mut audio, mut reader) = worker.join().unwrap();
        assert_eq!(audio.plan_id(), expected);
        assert_eq!(audio.clock(), SampleTime::new(6400));
        assert!(!control.has_replacements());
        audio.close_after_quiescence().unwrap();
        plans.finish(&mut control, &mut reader).unwrap();
    }
    #[test]
    fn retained_failed_return_flushes_before_another_mailbox_adoption() {
        let (mut control, mut audio, mut plans, mut reader) = setup();
        let first = candidate(&mut control);
        plans.publish(&mut control, first).unwrap();
        no_allocations(|| assert!(reader.boundary(&mut audio)));
        reader.pending = plans.returned.try_pop(); // Model a failed return push.
        let current = audio.plan_id();
        let second = candidate(&mut control);
        let next = second.plan_id();
        plans.publish(&mut control, second).unwrap();
        no_allocations(|| assert!(reader.boundary(&mut audio)));
        assert_eq!(audio.plan_id(), current);
        plans.collect_ready(&mut control).unwrap();
        no_allocations(|| assert!(reader.boundary(&mut audio)));
        assert_eq!(audio.plan_id(), next);
        plans.collect_ready(&mut control).unwrap();
        assert!(!control.has_replacements());
    }

    #[test]
    fn publication_overlaps_callback_reads_and_every_credit_is_resolved() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        let (mut control, mut audio, mut plans, mut reader) = setup();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&done);
        let start = Arc::new(std::sync::Barrier::new(2));
        let worker_start = Arc::clone(&start);
        let worker = std::thread::spawn(move || {
            worker_start.wait();
            let started = Instant::now();
            while !worker_done.load(Ordering::Acquire)
                && started.elapsed() < Duration::from_secs(10)
            {
                no_allocations(|| {
                    assert!(reader.boundary(&mut audio));
                    audio
                        .render(
                            AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap(),
                        )
                        .unwrap();
                });
                std::thread::yield_now();
            }
            (audio, reader)
        });
        start.wait();
        let mut latest = None;
        for _ in 0..40 {
            plans.collect_ready(&mut control).unwrap();
            let packet = candidate(&mut control);
            latest = Some(packet.plan_id());
            plans.publish(&mut control, packet).unwrap();
            std::thread::yield_now();
        }
        let started = Instant::now();
        while control.has_replacements() && started.elapsed() < Duration::from_secs(10) {
            plans.collect_ready(&mut control).unwrap();
            std::thread::yield_now();
        }
        done.store(true, Ordering::Release);
        let (mut audio, mut reader) = worker.join().unwrap();
        assert!(!control.has_replacements());
        assert_eq!(Some(audio.plan_id()), latest);
        assert_eq!(plans.receipts().len(), 40);
        assert!(plans.receipts().iter().all(|receipt| matches!(
            receipt.outcome,
            PlanReplacementOutcome::Installed(_) | PlanReplacementOutcome::Cancelled
        )));
        audio.close_after_quiescence().unwrap();
        plans.finish(&mut control, &mut reader).unwrap();
    }

    #[test]
    fn a_protocol_refusal_retains_its_packet_and_is_reported_after_quiescent_recovery() {
        let (mut control, mut audio, mut plans, mut reader) = setup();
        let packet = candidate(&mut control);
        let returned = audio.install_replacement(packet).unwrap();
        // Private misuse seam: publish a packet that was already attempted.
        plans.publish(&mut control, returned).unwrap();
        no_allocations(|| assert!(!reader.boundary(&mut audio)));
        assert!(reader.pending.is_some());
        assert!(reader.protocol_error.is_some());
        audio.close_after_quiescence().unwrap();
        assert!(plans.finish(&mut control, &mut reader).is_err());
        assert_eq!(plans.receipts().len(), 1);
        assert!(!control.has_replacements());
        assert!(reader.pending.is_none());
    }
}
