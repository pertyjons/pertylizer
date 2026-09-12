//! Non-shipping Linux ALSA output, transport, stopped-plan and loop harness.
//! Runs a silent V2 graph. This does not qualify physical timing, input or MIDI.

#[cfg(test)]
#[path = "support/v2_loop_journal.rs"]
mod loop_journal;

#[cfg(target_os = "linux")]
#[path = "support/v2_plan_transfer.rs"]
mod plan_transfer;

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        marker::PhantomData,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use crate::plan_transfer::{PlanAudio, PlanControl};
    use cpal::{
        Device, HostId, SampleFormat, Stream,
        traits::{DeviceTrait, HostTrait, StreamTrait},
    };
    use ringbuf::{
        HeapCons, HeapProd, HeapRb,
        traits::{Consumer, Observer, Producer, Split},
    };
    use synth_core::audio::DeviceSampleRate;
    use synth_engine_v2::{
        compile::{RenderConfig, compile},
        host::session::{
            SessionCommand, SessionCommandCapacity, SessionLimits, SessionOutcome, SessionReceipt,
            transfer::{
                CompletedSessionCommand, PreparedSessionCommand, SessionAudio, SessionControl,
                SessionTransferLimits,
            },
        },
        ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
        looping::{CompiledLoopStream, LoopSettings, journal::JournaledLoopStream},
        profile::HostProfile,
        quantities::{Amplitude, ChannelLayout, LoopPassCount, PreparedBytes, SampleRate},
        render::AudioBlockMut,
        schedule::AdmittedCompiledStream,
        time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime},
        transport::LoopInterval,
    };
    use thiserror::Error;

    // A preparation budget, not a claim about any device's maximum callback.
    const PREPARATION_LIMIT: FrameCount = FrameCount::new(8192);
    const TIMEOUT: Duration = Duration::from_secs(30);

    #[derive(Debug, Error)]
    enum HarnessError {
        #[error(
            "usage: v2_cpal_output list | run|transport|plans|loops <exact ALSA device ID> <positive callback count>"
        )]
        Usage,
        #[error("endpoint not found: {0}")]
        Endpoint(String),
        #[error("unsupported output configuration: {0}")]
        Configuration(&'static str),
        #[error("backend error: {0}")]
        Backend(#[from] cpal::Error),
        #[error("V2 preparation failed: {0}")]
        Preparation(String),
        #[error("callback failed: {0:?}")]
        Callback(Fault),
        #[error("callback target was not reached before timeout")]
        Timeout,
        #[error("run failed: {run}; closing also failed: {close}")]
        RunAndClose { run: Box<Self>, close: Box<Self> },
        #[error("callback storage was still shared after the ALSA stream joined")]
        Custody,
    }

    impl HarnessError {
        fn after_run(self, result: Result<(), Self>) -> Self {
            match result {
                Ok(()) => self,
                Err(run) => Self::RunAndClose {
                    run: Box::new(run),
                    close: Box::new(self),
                },
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    #[must_use]
    struct CallbackTarget(u64);

    impl CallbackTarget {
        fn parse(value: &str) -> Result<Self, HarnessError> {
            match value.parse::<u64>() {
                Ok(count) if count > 0 => Ok(Self(count)),
                _ => Err(HarnessError::Usage),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    #[must_use]
    struct CallbackCount(u64);

    impl CallbackCount {
        const ZERO: Self = Self(0);

        fn checked_next(self) -> Option<Self> {
            self.0.checked_add(1).map(Self)
        }

        const fn as_u64(self) -> u64 {
            self.0
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Fault {
        None,
        Backend,
        Buffer,
        Render,
        Counter,
        Protocol,
    }

    struct Progress {
        ready: AtomicBool,
        callbacks: AtomicU64,
        clock: AtomicU64,
        backend_error: AtomicBool,
        backend_kinds: AtomicU64,
        callback_error: AtomicBool,
    }

    const BACKEND_KINDS: [cpal::ErrorKind; 14] = [
        cpal::ErrorKind::DeviceBusy,
        cpal::ErrorKind::DeviceChanged,
        cpal::ErrorKind::DeviceNotAvailable,
        cpal::ErrorKind::HostUnavailable,
        cpal::ErrorKind::InvalidInput,
        cpal::ErrorKind::PermissionDenied,
        cpal::ErrorKind::RealtimeDenied,
        cpal::ErrorKind::ResourceExhausted,
        cpal::ErrorKind::StreamInvalidated,
        cpal::ErrorKind::UnsupportedConfig,
        cpal::ErrorKind::UnsupportedOperation,
        cpal::ErrorKind::Xrun,
        cpal::ErrorKind::BackendError,
        cpal::ErrorKind::Other,
    ];

    impl Progress {
        fn note_backend_error(&self, kind: cpal::ErrorKind) {
            // Known categories get distinct bits; a future CPAL kind uses the last bit.
            let index = BACKEND_KINDS
                .iter()
                .position(|known| *known == kind)
                .unwrap_or(63);
            self.backend_kinds
                .fetch_or(1_u64 << index, Ordering::Relaxed);
            self.backend_error.store(true, Ordering::Release);
        }

        fn backend_diagnostics(&self) -> Vec<cpal::ErrorKind> {
            let bits = self.backend_kinds.load(Ordering::Acquire);
            let mut kinds: Vec<_> = BACKEND_KINDS
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(index, kind)| (bits & (1_u64 << index) != 0).then_some(kind))
                .collect();
            if bits & (1_u64 << 63) != 0 {
                kinds.push(cpal::ErrorKind::Other);
            }
            kinds
        }

        fn new() -> Self {
            Self {
                ready: AtomicBool::new(false),
                callbacks: AtomicU64::new(0),
                clock: AtomicU64::new(0),
                backend_error: AtomicBool::new(false),
                backend_kinds: AtomicU64::new(0),
                callback_error: AtomicBool::new(false),
            }
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum RunMode {
        Output,
        Transport,
        Plans,
        Loops,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PlanExercise {
        Initial,
        Latest,
        Withdrawal,
    }

    fn silent_graph() -> Result<GraphIr, HarnessError> {
        GraphIr::builder()
            .node(
                NodeId::new(1),
                IrNodeKind::Constant {
                    level: Amplitude::new(0.0)
                        .map_err(|error| HarnessError::Preparation(error.to_string()))?,
                },
                ExecutionScope::Voice,
            )
            .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (NodeId::new(1), PortId::FIRST),
                (NodeId::new(2), PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .map_err(|error| HarnessError::Preparation(error.to_string()))
    }

    const COMMAND_SLOTS: usize = 4;
    const TRANSFER_BUDGET: PreparedBytes = PreparedBytes::measured(16_384);
    const COMMAND_LEAD: FrameCount = FrameCount::new(8192);

    // The owner keeps these backing Arcs through the backend join. Queue endpoints
    // inside callback storage are therefore never their final allocation owners.
    struct ControlLane {
        control: SessionControl,
        plans: PlanControl,
        profile: HostProfile,
        limits: SessionTransferLimits,
        commands: HeapProd<PreparedSessionCommand>,
        completed: HeapCons<CompletedSessionCommand>,
        _commands: Arc<HeapRb<PreparedSessionCommand>>,
        _completed: Arc<HeapRb<CompletedSessionCommand>>,
        // Off-thread diagnostic history; it owns no command credit or activation.
        receipts: Vec<SessionReceipt>,
        failed_commands: Vec<PreparedSessionCommand>,
        failed_completions: Vec<CompletedSessionCommand>,
        bytes: PreparedBytes,
    }

    struct AudioLane {
        audio: SessionAudio,
        plans: PlanAudio,
        commands: HeapCons<PreparedSessionCommand>,
        completed: HeapProd<CompletedSessionCommand>,
        failed_command: Option<PreparedSessionCommand>,
        pending_completion: Option<CompletedSessionCommand>,
    }

    impl ControlLane {
        fn queue_bytes<T>() -> usize {
            // Conservative Arc header plus alignment padding, ring object and cells.
            // The allocator regression checks this bound against both actual queues.
            size_of::<HeapRb<T>>()
                + COMMAND_SLOTS * size_of::<T>()
                + 2 * size_of::<usize>()
                + align_of::<HeapRb<T>>()
        }

        fn prepare(
            plan: synth_engine_v2::plan::CompiledPlan,
            profile: HostProfile,
        ) -> Result<(Self, AudioLane), HarnessError> {
            // Includes queue objects, all packet cells, both owner structures and
            // Arc control blocks. Command-box storage is charged by the core.
            let bytes = usize::try_from(PlanControl::storage_bytes().get())
                .map_err(|_| HarnessError::Configuration("plan mailbox layout overflow"))?
                + size_of::<Self>()
                + size_of::<CallbackEngine>()
                + Self::queue_bytes::<PreparedSessionCommand>()
                + Self::queue_bytes::<CompletedSessionCommand>()
                + COMMAND_SLOTS
                    * (size_of::<PreparedSessionCommand>() + size_of::<CompletedSessionCommand>());
            let bytes = u64::try_from(bytes)
                .map_err(|_| HarnessError::Configuration("transfer layout overflow"))?;
            if PreparedBytes::measured(bytes) > TRANSFER_BUDGET {
                return Err(HarnessError::Configuration("transfer budget exceeded"));
            }
            let stream = AdmittedCompiledStream::admit(&plan, &[])
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let limits = SessionTransferLimits {
                session: SessionLimits {
                    commands: SessionCommandCapacity::new(
                        u32::try_from(COMMAND_SLOTS).map_err(|_| {
                            HarnessError::Configuration("command capacity overflow")
                        })?,
                    )
                    .map_err(|error| HarnessError::Preparation(error.to_string()))?,
                    command_bytes: PreparedBytes::limit(16_384)
                        .map_err(|error| HarnessError::Preparation(error.to_string()))?,
                },
                control_bytes: PreparedBytes::limit(16_384)
                    .map_err(|error| HarnessError::Preparation(error.to_string()))?,
            };
            let (control, audio) = SessionControl::prepare(plan, stream, profile, limits)
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let (plans, plan_audio) = PlanControl::prepare(PreparedBytes::measured(8192))
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let commands = Arc::new(HeapRb::new(COMMAND_SLOTS));
            let completed = Arc::new(HeapRb::new(COMMAND_SLOTS));
            let (command_producer, command_consumer) = Arc::clone(&commands).split();
            let (completed_producer, completed_consumer) = Arc::clone(&completed).split();
            Ok((
                Self {
                    control,
                    plans,
                    profile,
                    limits,
                    commands: command_producer,
                    completed: completed_consumer,
                    _commands: commands,
                    _completed: completed,
                    receipts: Vec::new(),
                    failed_commands: Vec::with_capacity(COMMAND_SLOTS),
                    failed_completions: Vec::with_capacity(COMMAND_SLOTS),
                    bytes: PreparedBytes::measured(bytes),
                },
                AudioLane {
                    audio,
                    plans: plan_audio,
                    commands: command_consumer,
                    completed: completed_producer,
                    failed_command: None,
                    pending_completion: None,
                },
            ))
        }

        fn record(&mut self, receipt: SessionReceipt) {
            println!(
                "session_command={:?} kind={:?} at={} outcome={:?}",
                receipt.boundary.id,
                receipt.boundary.command,
                receipt.boundary.at.as_u64(),
                receipt.outcome
            );
            self.receipts.push(receipt);
        }

        fn collect(&mut self, packet: CompletedSessionCommand) -> Result<(), HarnessError> {
            match self.control.collect(packet) {
                Ok(receipt) => {
                    self.record(receipt);
                    Ok(())
                }
                Err((packet, error)) => {
                    let message = format!("collect {:?}: {error}", packet.boundary().id);
                    self.failed_completions.push(packet);
                    Err(HarnessError::Preparation(message))
                }
            }
        }

        fn cancel(&mut self, packet: PreparedSessionCommand) -> Result<(), HarnessError> {
            match self.control.cancel(packet) {
                Ok(receipt) => {
                    self.record(receipt);
                    Ok(())
                }
                Err((packet, error)) => {
                    let message = format!("cancel {:?}: {error}", packet.boundary().id);
                    self.failed_commands.push(packet);
                    Err(HarnessError::Preparation(message))
                }
            }
        }

        fn publish_plan(&mut self, graph: &GraphIr) -> Result<(), HarnessError> {
            let plan = compile(graph, &RenderConfig::new(self.profile))
                .into_plan()
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let stream = AdmittedCompiledStream::admit(&plan, &[])
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let candidate = self
                .control
                .prepare_replacement(plan, stream, self.profile, self.limits)
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            self.plans
                .publish(&mut self.control, candidate)
                .map_err(|error| HarnessError::Preparation(error.to_string()))
        }

        fn collect_ready(&mut self) -> Result<(), HarnessError> {
            self.plans
                .collect_ready(&mut self.control)
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            while let Some(packet) = self.completed.try_pop() {
                self.collect(packet)?;
            }
            Ok(())
        }

        fn submit(&mut self, command: SessionCommand, at: SampleTime) -> Result<(), HarnessError> {
            let packet = match command {
                SessionCommand::Play => self.control.prepare_play(at),
                SessionCommand::Stop => self.control.prepare_stop(at),
            }
            .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            if let Err(packet) = self.commands.try_push(packet) {
                self.cancel(packet)?;
                return Err(HarnessError::Configuration(
                    "command queue full; command cancelled",
                ));
            }
            Ok(())
        }

        fn finish(&mut self, lane: &mut AudioLane) -> Result<(), HarnessError> {
            self.control.close_admission();
            let mut errors = Vec::new();
            if let Err(error) = lane.audio.close_after_quiescence() {
                errors.push(error.to_string());
            }
            // Process every location even if a separate collection fails.
            while let Some(packet) = self.completed.try_pop() {
                if let Err(error) = self.collect(packet) {
                    errors.push(error.to_string());
                }
            }
            if let Some(packet) = lane.pending_completion.take()
                && let Err(error) = self.collect(packet)
            {
                errors.push(error.to_string());
            }
            while let Some(packet) = lane.audio.take_completed() {
                if let Err(error) = self.collect(packet) {
                    errors.push(error.to_string());
                }
            }
            if let Some(packet) = lane.failed_command.take()
                && let Err(error) = self.cancel(packet)
            {
                errors.push(error.to_string());
            }
            while let Some(packet) = lane.commands.try_pop() {
                if let Err(error) = self.cancel(packet) {
                    errors.push(error.to_string());
                }
            }
            if let Err(error) = self.plans.finish(&mut self.control, &mut lane.plans) {
                errors.push(error.to_string());
            }
            if self.control.has_outstanding() {
                errors.push(format!(
                    "unresolved commands: {:?}; unresolved completions: {:?}",
                    self.failed_commands
                        .iter()
                        .map(PreparedSessionCommand::boundary)
                        .collect::<Vec<_>>(),
                    self.failed_completions
                        .iter()
                        .map(CompletedSessionCommand::boundary)
                        .collect::<Vec<_>>()
                ));
            }
            if errors.is_empty() {
                Ok(())
            } else {
                Err(HarnessError::Preparation(errors.join("; ")))
            }
        }
    }

    #[derive(Clone, Copy)]
    #[must_use]
    struct IngressPrefix(usize);

    impl AudioLane {
        fn render_partitioned(&mut self, mut block: AudioBlockMut<'_>) -> Result<(), Fault> {
            loop {
                let carry = usize::try_from(self.audio.frames_until_plan_boundary().as_u64())
                    .map_err(|_| Fault::Counter)?;
                if carry == 0 && !self.plans.boundary(&mut self.audio) {
                    return Err(Fault::Protocol);
                }
                let step = if carry == 0 {
                    QUANTUM_FRAMES as usize
                } else {
                    carry
                };
                if block.frames() <= step {
                    return self.audio.render(block).map_err(|_| Fault::Render);
                }
                let (head, tail) = block.split_at_frame(step).map_err(|_| Fault::Buffer)?;
                self.audio.render(head).map_err(|_| Fault::Render)?;
                block = tail;
            }
        }

        fn ingress_cut(&self) -> IngressPrefix {
            IngressPrefix(self.commands.occupied_len().min(COMMAND_SLOTS))
        }

        fn admit_prefix(&mut self, prefix: IngressPrefix) -> bool {
            if self.failed_command.is_some() {
                return false;
            }
            // Fix the cut before the first pop; later publications wait one callback.
            for _ in 0..prefix.0 {
                let Some(packet) = self.commands.try_pop() else {
                    break;
                };
                if let Err((packet, _error)) = self.audio.enqueue(packet) {
                    self.failed_command = Some(packet);
                    return false;
                }
            }
            true
        }

        fn return_completed(&mut self) {
            for _ in 0..COMMAND_SLOTS {
                if self.completed.is_full() {
                    break;
                }
                let packet = self
                    .pending_completion
                    .take()
                    .or_else(|| self.audio.take_completed());
                let Some(packet) = packet else {
                    break;
                };
                if let Err(packet) = self.completed.try_push(packet) {
                    self.pending_completion = Some(packet);
                    break;
                }
            }
        }
    }

    // Chosen off-thread before publication and never replaced by a callback.
    enum CallbackEngine {
        Session(AudioLane),
        Loop(JournaledLoopStream),
    }

    impl CallbackEngine {
        fn prepare_loop(
            plan: synth_engine_v2::plan::CompiledPlan,
            profile: HostProfile,
        ) -> Result<Self, HarnessError> {
            let inline_bytes = u64::try_from(size_of::<Self>())
                .map_err(|_| HarnessError::Configuration("loop owner layout overflow"))?;
            if PreparedBytes::measured(inline_bytes) > TRANSFER_BUDGET {
                return Err(HarnessError::Configuration("loop owner budget exceeded"));
            }
            let events = AdmittedCompiledStream::admit(&plan, &[])
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let interval = LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(513))
                .ok_or(HarnessError::Configuration("invalid loop interval"))?;
            let settings = LoopSettings::new(
                interval,
                PlanPosition::ZERO,
                PreparedBytes::measured(262_144),
            )
            .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let stream = CompiledLoopStream::prepare(plan, events, profile, settings)
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let owner = JournaledLoopStream::prepare(
                stream,
                LoopPassCount::limit(4)
                    .map_err(|error| HarnessError::Preparation(error.to_string()))?,
                PreparedBytes::measured(8192),
            )
            .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            Ok(Self::Loop(owner))
        }

        fn clock(&self) -> SampleTime {
            match self {
                Self::Session(lane) => lane.audio.clock(),
                Self::Loop(owner) => owner.acknowledged().clock,
            }
        }

        fn session_mut(&mut self) -> Option<&mut AudioLane> {
            match self {
                Self::Session(lane) => Some(lane),
                Self::Loop(_) => None,
            }
        }

        fn render(&mut self, block: AudioBlockMut<'_>) -> Result<(), Fault> {
            match self {
                Self::Session(lane) => {
                    let prefix = lane.ingress_cut();
                    if !lane.admit_prefix(prefix) {
                        return Err(Fault::Protocol);
                    }
                    lane.render_partitioned(block)
                }
                // Preserve the whole backend callback as the journal's success unit.
                Self::Loop(owner) => owner.render(block).map_err(|_| Fault::Render),
            }
        }

        fn return_completed(&mut self) {
            if let Self::Session(lane) = self {
                lane.return_completed();
            }
        }

        fn finish_loop(&mut self) {
            if let Self::Loop(owner) = self {
                let _retained_end = owner.finish();
            }
        }

        // Off-thread after join, before any run failure is propagated.
        fn report_loop(&self) {
            if let Self::Loop(owner) = self {
                println!(
                    "loop_initial={:?} loop_interval={:?} journal_bytes={} acknowledged={:?}",
                    owner.initial(),
                    owner.interval(),
                    owner.storage_bytes().get(),
                    owner.acknowledged()
                );
                for boundary in owner.boundaries() {
                    println!("loop_boundary={boundary:?}");
                }
                println!("loop_observation_end={:?}", owner.end());
            }
        }
    }

    struct CallbackState {
        engine: CallbackEngine,
        receipts: Vec<SessionReceipt>,
        plan_receipts:
            Vec<synth_engine_v2::host::session::transfer::replacement::PlanReplacementReceipt>,
        scratch: Box<[f32]>,
        layout: ChannelLayout,
        bound: FrameCount,
        callbacks: CallbackCount,
        delivered: FrameCount,
        fault: Fault,
        #[cfg(test)]
        drop_thread: Option<Arc<std::sync::Mutex<Option<thread::ThreadId>>>>,
    }

    impl CallbackState {
        #[cfg(test)]
        fn lane_mut(&mut self) -> &mut AudioLane {
            self.engine
                .session_mut()
                .expect("the fixture owns a session")
        }

        #[cfg(test)]
        fn prepare(
            rate: SampleRate,
            layout: ChannelLayout,
            bound: FrameCount,
        ) -> Result<(Self, ControlLane), HarnessError> {
            let (state, control) = Self::prepare_mode(rate, layout, bound, RunMode::Output)?;
            Ok((state, control.ok_or(HarnessError::Custody)?))
        }

        fn prepare_mode(
            rate: SampleRate,
            layout: ChannelLayout,
            bound: FrameCount,
            mode: RunMode,
        ) -> Result<(Self, Option<ControlLane>), HarnessError> {
            if bound.as_u64() == 0 || bound.as_u64() > PREPARATION_LIMIT.as_u64() {
                return Err(HarnessError::Configuration(
                    "negotiated period exceeds preparation budget or is zero",
                ));
            }
            let profile = HostProfile::harness(rate, bound, layout)
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let graph = silent_graph()?;
            let plan = compile(&graph, &RenderConfig::new(profile))
                .into_plan()
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let (engine, control) = match mode {
                RunMode::Loops => (CallbackEngine::prepare_loop(plan, profile)?, None),
                RunMode::Output | RunMode::Transport | RunMode::Plans => {
                    let (control, lane) = ControlLane::prepare(plan, profile)?;
                    (CallbackEngine::Session(lane), Some(control))
                }
            };
            let samples = usize::try_from(bound.as_u64())
                .ok()
                .and_then(|frames| frames.checked_mul(layout.channels()))
                .ok_or(HarnessError::Configuration("sample count overflow"))?;
            Ok((
                Self {
                    engine,
                    receipts: Vec::new(),
                    plan_receipts: Vec::new(),
                    scratch: vec![0.0; samples].into_boxed_slice(),
                    layout,
                    bound,
                    callbacks: CallbackCount::ZERO,
                    delivered: FrameCount::new(0),
                    fault: Fault::None,
                    #[cfg(test)]
                    drop_thread: None,
                },
                control,
            ))
        }

        fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(&mut self, output: &mut [T]) {
            // Whole-buffer validation precedes rendering: a bad callback advances no clock.
            if self.fault != Fault::None {
                return;
            }
            let count = self.layout.channels();
            let frames = output.len() / count;
            if output.is_empty()
                || !output.len().is_multiple_of(count)
                || output.len() > self.scratch.len()
            {
                self.fault = Fault::Buffer;
                return;
            }
            let Ok(frames_u64) = u64::try_from(frames) else {
                self.fault = Fault::Counter;
                return;
            };
            let (Some(callbacks), Some(delivered)) = (
                self.callbacks.checked_next(),
                self.delivered.as_u64().checked_add(frames_u64),
            ) else {
                self.fault = Fault::Counter;
                return;
            };
            let Some(scratch) = self.scratch.get_mut(..output.len()) else {
                self.fault = Fault::Buffer;
                return;
            };
            let Ok(block) = AudioBlockMut::new(scratch, frames, self.layout) else {
                self.fault = Fault::Buffer;
                return;
            };
            if let Err(fault) = self.engine.render(block) {
                self.fault = fault;
                return;
            }
            for (destination, value) in output.iter_mut().zip(scratch.iter()) {
                *destination = T::from_sample(*value);
            }
            self.engine.return_completed();
            self.callbacks = callbacks;
            self.delivered = FrameCount::new(delivered);
        }
    }

    // The production alternative is always ALSA. The test alternative runs the same
    // owner destructor against a worker which destroys its closure without a final call.
    enum JoinedStream {
        Alsa(Stream),
        #[cfg(test)]
        Worker(Option<thread::JoinHandle<()>>),
    }

    impl Drop for JoinedStream {
        fn drop(&mut self) {
            #[cfg(test)]
            if let Self::Worker(handle) = self
                && let Some(handle) = handle.take()
            {
                handle.join().expect("test worker completes");
            }
            // CPAL's ALSA Stream destructor signals, wakes and joins its worker.
        }
    }

    struct LinuxOutputOwner {
        // Storage precedes stream to exercise cleanup independent of field order.
        pool: Arc<HeapRb<CallbackState>>,
        control: Option<ControlLane>,
        progress: Arc<Progress>,
        stream: Option<JoinedStream>,
        _control_thread: PhantomData<Rc<()>>,
    }

    impl LinuxOutputOwner {
        fn open(device: &Device, mode: RunMode) -> Result<Self, HarnessError> {
            let supported = device.default_output_config()?;
            let layout = match supported.channels() {
                1 => ChannelLayout::Mono,
                2 => ChannelLayout::Stereo,
                _ => return Err(HarnessError::Configuration("only mono/stereo is supported")),
            };
            if !matches!(
                supported.sample_format(),
                SampleFormat::F32 | SampleFormat::I32
            ) {
                return Err(HarnessError::Configuration("only F32/I32 is supported"));
            }
            let raw_rate = supported.sample_rate();
            if !(8000..=DeviceSampleRate::MAX_SUPPORTED.as_u32()).contains(&raw_rate) {
                return Err(HarnessError::Configuration(
                    "sample rate is outside the V2 range",
                ));
            }
            let rate = SampleRate::new(DeviceSampleRate::new(raw_rate).as_f32())
                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
            let pool = Arc::new(HeapRb::<CallbackState>::new(1));
            let (mut producer, mut consumer) = Arc::clone(&pool).split();
            let progress = Arc::new(Progress::new());
            let callback_progress = Arc::clone(&progress);
            let error_progress = Arc::clone(&progress);
            let stream = device.build_output_stream_raw(
                supported.config(),
                supported.sample_format(),
                move |data, _| {
                    // CPAL 0.18.2 supplies silence, including before Ready and on faults.
                    if !callback_progress.ready.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(state) = consumer.first_mut() else {
                        callback_progress
                            .callback_error
                            .store(true, Ordering::Release);
                        return;
                    };
                    match data.sample_format() {
                        SampleFormat::F32 => match data.as_slice_mut::<f32>() {
                            Some(samples) => state.render(samples),
                            None => state.fault = Fault::Buffer,
                        },
                        SampleFormat::I32 => match data.as_slice_mut::<i32>() {
                            Some(samples) => state.render(samples),
                            None => state.fault = Fault::Buffer,
                        },
                        _ => state.fault = Fault::Buffer,
                    }
                    callback_progress
                        .clock
                        .store(state.engine.clock().as_u64(), Ordering::Release);
                    callback_progress
                        .callbacks
                        .store(state.callbacks.as_u64(), Ordering::Release);
                    if state.fault != Fault::None {
                        callback_progress
                            .callback_error
                            .store(true, Ordering::Release);
                    }
                },
                move |error| {
                    // Backend errors can own strings. This is not a whole-worker
                    // allocation claim; no SoundCore resource is owned by this closure.
                    error_progress.note_backend_error(error.kind());
                },
                None,
            )?;
            // Every subsequent failure uses the stream-first destructor, even before play.
            let mut owner = Self {
                pool,
                control: None,
                progress,
                stream: Some(JoinedStream::Alsa(stream)),
                _control_thread: PhantomData,
            };
            let Some(JoinedStream::Alsa(stream)) = owner.stream.as_ref() else {
                return Err(HarnessError::Custody);
            };
            let bound = FrameCount::new(u64::from(stream.buffer_size()?));
            let (state, control) = CallbackState::prepare_mode(rate, layout, bound, mode)?;
            producer
                .try_push(state)
                .map_err(|_state| HarnessError::Custody)?;
            drop(producer);
            owner.control = control;
            Ok(owner)
        }

        fn fence(&mut self) {
            self.progress.ready.store(false, Ordering::Release);
            drop(self.stream.take());
        }

        fn start(&self) -> Result<(), HarnessError> {
            let Some(JoinedStream::Alsa(stream)) = self.stream.as_ref() else {
                return Err(HarnessError::Custody);
            };
            self.progress.ready.store(true, Ordering::Release);
            stream.play()?;
            Ok(())
        }

        fn close(mut self) -> Result<CallbackState, HarnessError> {
            self.fence();
            let pool = Arc::get_mut(&mut self.pool).ok_or(HarnessError::Custody)?;
            let mut state = pool.try_pop().ok_or(HarnessError::Custody)?;
            if let Some(control) = self.control.as_mut() {
                let lane = state.engine.session_mut().ok_or(HarnessError::Custody)?;
                let result = control.finish(lane);
                state.receipts = std::mem::take(&mut control.receipts);
                state.plan_receipts = control.plans.take_receipts();
                self.control = None;
                result?;
            }
            state.engine.finish_loop();
            Ok(state)
        }

        fn next_boundary(&self) -> Result<SampleTime, HarnessError> {
            let control = self.control.as_ref().ok_or(HarnessError::Custody)?;
            let clock = self
                .progress
                .clock
                .load(Ordering::Acquire)
                .max(control.control.snapshot().clock.as_u64());
            // The physical harness uses a generous fixed lead; it is not a latency bound.
            let quantum = u64::from(QUANTUM_FRAMES);
            let at = clock
                .checked_add(COMMAND_LEAD.as_u64())
                .and_then(|value| value.checked_add(quantum - 1))
                .map(|value| value / quantum * quantum)
                .ok_or(HarnessError::Configuration("command time overflow"))?;
            Ok(SampleTime::new(at))
        }

        fn wait(&mut self, target: CallbackTarget, mode: RunMode) -> Result<(), HarnessError> {
            let start = Instant::now();
            let mut offered = 1;
            let mut plan_stage = PlanExercise::Initial;
            let transport = mode == RunMode::Transport;
            loop {
                if self.progress.backend_error.load(Ordering::Acquire) {
                    return Err(HarnessError::Callback(Fault::Backend));
                }
                if self.progress.callback_error.load(Ordering::Acquire) {
                    return Err(HarnessError::Callback(Fault::Render));
                }
                if mode == RunMode::Loops {
                    if self.control.is_some() {
                        return Err(HarnessError::Custody);
                    }
                    if self.progress.callbacks.load(Ordering::Acquire) >= target.0 {
                        return Ok(());
                    }
                    if start.elapsed() >= TIMEOUT {
                        return Err(HarnessError::Timeout);
                    }
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                let at = self.next_boundary()?;
                let control = self.control.as_mut().ok_or(HarnessError::Custody)?;
                control.collect_ready()?;
                if transport {
                    if control
                        .receipts
                        .iter()
                        .any(|receipt| !matches!(receipt.outcome, SessionOutcome::Applied { .. }))
                    {
                        return Err(HarnessError::Configuration(
                            "transport command did not apply",
                        ));
                    }
                    if control.receipts.len() == offered && offered < 4 {
                        let command = if offered == 2 {
                            SessionCommand::Play
                        } else {
                            SessionCommand::Stop
                        };
                        control.submit(command, at)?;
                        offered += 1;
                    }
                }
                if mode == RunMode::Plans {
                    use synth_engine_v2::host::session::transfer::replacement::PlanReplacementOutcome;
                    if control.plans.receipts().iter().any(|receipt| {
                        matches!(receipt.outcome, PlanReplacementOutcome::Refused(_))
                    }) {
                        return Err(HarnessError::Configuration("plan publication was refused"));
                    }
                    if !control.control.has_replacements() {
                        if plan_stage == PlanExercise::Initial {
                            control.publish_plan(&silent_graph()?)?;
                            control.publish_plan(&silent_graph()?)?;
                            plan_stage = PlanExercise::Latest;
                        } else if plan_stage == PlanExercise::Latest {
                            control.publish_plan(&silent_graph()?)?;
                            let complete = control
                                .plans
                                .withdraw(&mut control.control)
                                .map_err(|error| HarnessError::Preparation(error.to_string()))?;
                            println!("plan_withdrawal_complete={complete}");
                            plan_stage = PlanExercise::Withdrawal;
                        }
                    }
                }
                if self.progress.callbacks.load(Ordering::Acquire) >= target.0 {
                    if transport && control.receipts.len() != 4 {
                        return Err(HarnessError::Configuration(
                            "callback target ended before four transport receipts",
                        ));
                    }
                    if mode == RunMode::Plans
                        && (plan_stage != PlanExercise::Withdrawal
                            || control.control.has_replacements()
                            || control.plans.receipts().len() != 4)
                    {
                        return Err(HarnessError::Configuration(
                            "callback target ended before four plan receipts",
                        ));
                    }
                    return Ok(());
                }
                if start.elapsed() >= TIMEOUT {
                    return Err(HarnessError::Timeout);
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }

    impl Drop for LinuxOutputOwner {
        fn drop(&mut self) {
            self.fence();
            if let Some(control) = self.control.as_mut() {
                if let Some(pool) = Arc::get_mut(&mut self.pool) {
                    if let Some(mut state) = pool.try_pop() {
                        match state.engine.session_mut() {
                            Some(lane) => {
                                if let Err(error) = control.finish(lane) {
                                    eprintln!("implicit close failed: {error}");
                                }
                            }
                            None => eprintln!("implicit close failed: {}", HarnessError::Custody),
                        }
                    }
                } else {
                    eprintln!("implicit close failed: {}", HarnessError::Custody);
                }
            } else if let Some(pool) = Arc::get_mut(&mut self.pool) {
                if let Some(mut state) = pool.try_pop() {
                    state.engine.finish_loop();
                    state.engine.report_loop();
                }
            } else {
                eprintln!("implicit close failed: {}", HarnessError::Custody);
            }
        }
    }

    pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // Never default_host: the lifetime proof is specific to CPAL's ALSA backend.
        let host = cpal::host_from_id(HostId::Alsa)?;
        match args.as_slice() {
            [command] if command == "list" => {
                for device in host.output_devices()? {
                    println!("{} {:?}", device.id()?, device.default_output_config()?);
                }
            }
            [command, id, count]
                if matches!(command.as_str(), "run" | "transport" | "plans" | "loops") =>
            {
                let target = CallbackTarget::parse(count)?;
                let mut selected = None;
                for device in host.output_devices()? {
                    if device.id()?.to_string() == *id {
                        selected = Some(device);
                        break;
                    }
                }
                let device = selected.ok_or_else(|| HarnessError::Endpoint(id.clone()))?;
                let mode = match command.as_str() {
                    "transport" => RunMode::Transport,
                    "plans" => RunMode::Plans,
                    "loops" => RunMode::Loops,
                    _ => RunMode::Output,
                };
                let mut owner = LinuxOutputOwner::open(&device, mode)?;
                let progress = Arc::clone(&owner.progress);
                let result = (|| {
                    if let Some(control) = owner.control.as_mut() {
                        println!("transfer_bytes={}", control.bytes.get());
                        if mode == RunMode::Transport {
                            control.submit(SessionCommand::Play, SampleTime::ZERO)?;
                        }
                        if mode == RunMode::Plans {
                            control.publish_plan(&silent_graph()?)?;
                        }
                    }
                    owner.start()?;
                    owner.wait(target, mode)
                })();
                let state = match owner.close() {
                    Ok(state) => state,
                    Err(close) => {
                        eprintln!(
                            "endpoint={id} backend_error={} backend_kinds={:?} callback_error={} close_failed=true",
                            progress.backend_error.load(Ordering::Acquire),
                            progress.backend_diagnostics(),
                            progress.callback_error.load(Ordering::Acquire)
                        );
                        return Err(close.after_run(result).into());
                    }
                };
                println!(
                    "backend=ALSA cpal=0.18.2 endpoint={id} period_frames={} delivered_frames={} render_clock={} callbacks={} fault={:?} backend_error={} backend_kinds={:?} callback_error={} stream_joined=true",
                    state.bound.as_u64(),
                    state.delivered.as_u64(),
                    state.engine.clock().as_u64(),
                    state.callbacks.as_u64(),
                    state.fault,
                    progress.backend_error.load(Ordering::Acquire),
                    progress.backend_diagnostics(),
                    progress.callback_error.load(Ordering::Acquire)
                );
                state.engine.report_loop();
                result?;
                if state.fault != Fault::None {
                    return Err(HarnessError::Callback(state.fault).into());
                }
                if progress.backend_error.load(Ordering::Acquire) {
                    return Err(HarnessError::Callback(Fault::Backend).into());
                }
                if progress.callback_error.load(Ordering::Acquire) {
                    return Err(HarnessError::Callback(Fault::Render).into());
                }
            }
            _ => return Err(HarnessError::Usage.into()),
        }
        Ok(())
    }

    #[cfg(test)]
    impl Drop for CallbackState {
        fn drop(&mut self) {
            if let Some(observer) = &self.drop_thread {
                *observer.lock().unwrap() = Some(thread::current().id());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::{Barrier, Mutex};

        fn state(bound: u64, layout: ChannelLayout) -> (CallbackState, ControlLane) {
            CallbackState::prepare(
                SampleRate::new(48_000.0).unwrap(),
                layout,
                FrameCount::new(bound),
            )
            .unwrap()
        }

        fn loop_state(bound: u64, layout: ChannelLayout) -> CallbackState {
            let (mut state, control) = CallbackState::prepare_mode(
                SampleRate::new(48_000.0).unwrap(),
                layout,
                FrameCount::new(bound),
                RunMode::Loops,
            )
            .unwrap();
            assert!(control.is_none());
            assert!(state.engine.session_mut().is_none());
            state
        }

        fn journal(state: &CallbackState) -> &JournaledLoopStream {
            match &state.engine {
                CallbackEngine::Loop(owner) => owner,
                CallbackEngine::Session(_) => panic!("expected loop fixture"),
            }
        }

        #[test]
        fn loop_output_retains_passes_and_carry_across_formats_and_partitions() {
            for partition in [1_usize, 37, 64, 256, 512] {
                for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
                    let mut floating = loop_state(512, layout);
                    let mut integer = loop_state(512, layout);
                    let samples = 2624 * layout.channels();
                    let mut f32_output = vec![9.0; samples];
                    let mut i32_output = vec![17_i32; samples];
                    for (floating_output, integer_output) in f32_output
                        .chunks_mut(partition * layout.channels())
                        .zip(i32_output.chunks_mut(partition * layout.channels()))
                    {
                        no_allocations(|| {
                            floating.render(floating_output);
                            integer.render(integer_output);
                        });
                    }
                    assert!(f32_output.iter().all(|sample| *sample == 0.0));
                    assert!(i32_output.iter().all(|sample| *sample == 0));
                    for runtime in [&floating, &integer] {
                        assert_eq!(runtime.fault, Fault::None);
                        assert_eq!(runtime.delivered, FrameCount::new(2624));
                        assert_eq!(runtime.engine.clock(), SampleTime::new(2560));
                        assert_eq!(
                            runtime.callbacks.as_u64(),
                            2624_usize.div_ceil(partition) as u64
                        );
                        let owner = journal(runtime);
                        assert_eq!(
                            owner
                                .boundaries()
                                .map(|b| b.at.as_u64())
                                .collect::<Vec<_>>(),
                            [513, 1026, 1539]
                        );
                        let end = owner.end().unwrap();
                        assert_eq!(end.at, SampleTime::new(2052));
                        assert_eq!(end.pass.as_u64(), 4);
                        assert_eq!(
                            end.reason,
                            synth_engine_v2::looping::journal::LoopJournalEndReason::PassLimit
                        );
                    }
                }
            }
        }

        #[test]
        fn loop_callback_is_not_split_into_smaller_successful_journal_calls() {
            let mut runtime = loop_state(512, ChannelLayout::Mono);
            let profile = HostProfile::harness(
                SampleRate::new(48_000.0).unwrap(),
                FrameCount::new(64),
                ChannelLayout::Mono,
            )
            .unwrap();
            let plan = compile(&silent_graph().unwrap(), &RenderConfig::new(profile))
                .into_plan()
                .unwrap();
            // Off-thread fault injection: the backend buffer fits its scratch, but
            // exceeds this renderer's bound. Splitting it into Q calls would hide it.
            runtime.engine = CallbackEngine::prepare_loop(plan, profile).unwrap();
            no_allocations(|| runtime.render(&mut [0.0_f32; 128]));
            assert_eq!(runtime.fault, Fault::Render);
            assert_eq!(runtime.callbacks.as_u64(), 0);
            assert_eq!(runtime.delivered, FrameCount::new(0));
            assert_eq!(runtime.engine.clock(), SampleTime::ZERO);
            assert_eq!(journal(&runtime).boundaries().count(), 0);
            assert!(matches!(
                journal(&runtime).end().unwrap().reason,
                synth_engine_v2::looping::journal::LoopJournalEndReason::RenderFault(
                    synth_engine_v2::looping::LoopFault::Render(
                        synth_engine_v2::diagnostics::RenderError::OversizedCallback { .. }
                    )
                )
            ));
            let first = journal(&runtime).end();
            no_allocations(|| runtime.render(&mut [0.0_f32; 64]));
            assert_eq!(journal(&runtime).end(), first);
            assert_eq!(runtime.engine.clock(), SampleTime::ZERO);
        }

        #[test]
        fn loop_owner_survives_callback_disappearance_and_both_joined_close_paths() {
            for calls in [0_u64, 1, 48] {
                for explicit_close in [false, true] {
                    let observer = Arc::new(Mutex::new(None));
                    let mut runtime = loop_state(64, ChannelLayout::Mono);
                    runtime.drop_thread = Some(Arc::clone(&observer));
                    let pool = Arc::new(HeapRb::new(1));
                    let (mut producer, mut consumer) = Arc::clone(&pool).split();
                    assert!(producer.try_push(runtime).is_ok());
                    drop(producer);
                    let worker = thread::spawn(move || {
                        no_allocations(|| {
                            for _ in 0..calls {
                                consumer.first_mut().unwrap().render(&mut [0.0_f32; 64]);
                            }
                            drop(consumer);
                        });
                    });
                    let owner = LinuxOutputOwner {
                        pool,
                        control: None,
                        progress: Arc::new(Progress::new()),
                        stream: Some(JoinedStream::Worker(Some(worker))),
                        _control_thread: PhantomData,
                    };
                    if explicit_close {
                        let runtime = owner.close().unwrap();
                        assert_eq!(runtime.callbacks.as_u64(), calls);
                        assert_eq!(
                            runtime.engine.clock().as_u64(),
                            calls.saturating_sub(1) * 64
                        );
                        assert!(journal(&runtime).end().is_some());
                        assert!(observer.lock().unwrap().is_none());
                        drop(runtime);
                    } else {
                        drop(owner);
                    }
                    assert_eq!(*observer.lock().unwrap(), Some(thread::current().id()));
                }
            }
        }

        #[test]
        fn in_flight_loop_borrow_remains_owned_until_shutdown_joins() {
            let observer = Arc::new(Mutex::new(None));
            let mut runtime = loop_state(128, ChannelLayout::Mono);
            runtime.drop_thread = Some(Arc::clone(&observer));
            let pool = Arc::new(HeapRb::new(1));
            let (mut producer, mut consumer) = Arc::clone(&pool).split();
            assert!(producer.try_push(runtime).is_ok());
            drop(producer);
            let progress = Arc::new(Progress::new());
            progress.ready.store(true, Ordering::Release);
            let worker_progress = Arc::clone(&progress);
            let entered = Arc::new(Barrier::new(2));
            let worker_entered = Arc::clone(&entered);
            let worker = thread::spawn(move || {
                let borrowed = consumer.first_mut().unwrap();
                worker_entered.wait();
                while worker_progress.ready.load(Ordering::Acquire) {
                    thread::yield_now();
                }
                no_allocations(|| borrowed.render(&mut [0.0_f32; 128]));
                assert_eq!(borrowed.engine.clock(), SampleTime::new(64));
                no_allocations(|| drop(consumer));
            });
            let owner = LinuxOutputOwner {
                pool,
                control: None,
                progress,
                stream: Some(JoinedStream::Worker(Some(worker))),
                _control_thread: PhantomData,
            };
            entered.wait();
            drop(owner);
            assert_eq!(*observer.lock().unwrap(), Some(thread::current().id()));
        }

        #[test]
        fn callback_closure_can_end_without_a_final_call_and_only_control_drops_state() {
            for calls in [0, 1, 1000] {
                for explicit_close in [false, true] {
                    let observer = Arc::new(Mutex::new(None));
                    let (mut runtime, mut control) = state(64, ChannelLayout::Mono);
                    control
                        .submit(SessionCommand::Play, SampleTime::ZERO)
                        .unwrap();
                    control
                        .submit(SessionCommand::Stop, SampleTime::new(64))
                        .unwrap();
                    runtime.drop_thread = Some(Arc::clone(&observer));
                    let pool = Arc::new(HeapRb::new(1));
                    let (mut producer, mut consumer) = Arc::clone(&pool).split();
                    assert!(producer.try_push(runtime).is_ok());
                    drop(producer);
                    let progress = Arc::new(Progress::new());
                    progress.ready.store(true, Ordering::Release);
                    let worker_progress = Arc::clone(&progress);
                    let entered = Arc::new(Barrier::new(2));
                    let worker_entered = Arc::clone(&entered);
                    let worker = thread::spawn(move || {
                        let mut samples = [0.0_f32; 64];
                        no_allocations(|| {
                            for _ in 0..calls {
                                consumer.first_mut().unwrap().render(&mut samples);
                            }
                        });
                        worker_entered.wait();
                        while worker_progress.ready.load(Ordering::Acquire) {
                            thread::yield_now();
                        }
                        // Closure destruction, with no final call or return message.
                        no_allocations(|| drop(consumer));
                    });
                    let owner = LinuxOutputOwner {
                        pool,
                        control: Some(control),
                        progress,
                        stream: Some(JoinedStream::Worker(Some(worker))),
                        _control_thread: PhantomData,
                    };
                    entered.wait();
                    assert!(observer.lock().unwrap().is_none());
                    if explicit_close {
                        let runtime = owner.close().unwrap();
                        assert_eq!(runtime.callbacks.as_u64(), calls);
                        assert_eq!(runtime.receipts.len(), 2);
                        assert!(observer.lock().unwrap().is_none());
                        drop(runtime);
                    } else {
                        drop(owner);
                    }
                    assert_eq!(*observer.lock().unwrap(), Some(thread::current().id()));
                }
            }
        }

        #[test]
        fn failure_after_open_before_preparation_still_joins_with_empty_storage() {
            let pool = Arc::new(HeapRb::<CallbackState>::new(1));
            let (producer, consumer) = Arc::clone(&pool).split();
            let progress = Arc::new(Progress::new());
            let worker = thread::spawn(move || drop(consumer));
            let mut owner = LinuxOutputOwner {
                pool,
                control: None,
                progress,
                stream: Some(JoinedStream::Worker(Some(worker))),
                _control_thread: PhantomData,
            };
            // Constructor's producer can outlive the stream on this error path.
            owner.fence();
            drop(producer);
            assert!(Arc::get_mut(&mut owner.pool).is_some());
            assert!(owner.stream.is_none());
        }

        #[test]
        fn in_flight_borrow_survives_owner_shutdown() {
            let observer = Arc::new(Mutex::new(None));
            let (mut runtime, control) = state(64, ChannelLayout::Mono);
            runtime.drop_thread = Some(Arc::clone(&observer));
            let pool = Arc::new(HeapRb::new(1));
            let (mut producer, mut consumer) = Arc::clone(&pool).split();
            assert!(producer.try_push(runtime).is_ok());
            drop(producer);
            let progress = Arc::new(Progress::new());
            progress.ready.store(true, Ordering::Release);
            let worker_progress = Arc::clone(&progress);
            let entered = Arc::new(Barrier::new(2));
            let worker_entered = Arc::clone(&entered);
            let worker = thread::spawn(move || {
                let borrowed = consumer.first_mut().unwrap();
                worker_entered.wait();
                while worker_progress.ready.load(Ordering::Acquire) {
                    thread::yield_now();
                }
                no_allocations(|| borrowed.render(&mut [0.0_f32; 64]));
                assert_eq!(borrowed.callbacks.as_u64(), 1);
            });
            let owner = LinuxOutputOwner {
                pool,
                control: Some(control),
                progress,
                stream: Some(JoinedStream::Worker(Some(worker))),
                _control_thread: PhantomData,
            };
            entered.wait();
            drop(owner);
            assert_eq!(*observer.lock().unwrap(), Some(thread::current().id()));
        }

        #[test]
        fn borrowed_renderer_preserves_quantum_carry_for_both_output_formats() {
            for partition in [1, 37, 64, 256, 512] {
                for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
                    let (mut floating, _floating_control) = state(512, layout);
                    let (mut integer, _integer_control) = state(512, layout);
                    let mut f32_output = vec![9.0; 1024 * layout.channels()];
                    let mut i32_output = vec![9; f32_output.len()];
                    for output in f32_output.chunks_mut(partition * layout.channels()) {
                        floating.render(output);
                    }
                    for output in i32_output.chunks_mut(partition * layout.channels()) {
                        integer.render(output);
                    }
                    assert!(f32_output.iter().all(|sample| *sample == 0.0));
                    assert!(i32_output.iter().all(|sample| *sample == 0));
                    assert_eq!(floating.delivered, FrameCount::new(1024));
                    assert_eq!(floating.lane_mut().audio.clock(), SampleTime::new(960));
                    assert_eq!(
                        floating.lane_mut().audio.clock(),
                        integer.lane_mut().audio.clock()
                    );
                    assert_eq!(floating.fault, Fault::None);
                    assert_eq!(integer.fault, Fault::None);
                }
            }
        }

        #[test]
        fn malformed_callback_and_counter_exhaustion_do_not_advance_renderer() {
            for length in [0, 3, 130] {
                let (mut runtime, _control) = state(64, ChannelLayout::Stereo);
                runtime.render(&mut vec![0.0_f32; length]);
                assert_eq!(runtime.fault, Fault::Buffer);
                assert_eq!(runtime.callbacks, CallbackCount::ZERO);
                assert_eq!(runtime.lane_mut().audio.clock(), SampleTime::ZERO);
            }
            for exhausted_frames in [false, true] {
                let (mut runtime, _control) = state(64, ChannelLayout::Mono);
                if exhausted_frames {
                    runtime.delivered = FrameCount::new(u64::MAX);
                } else {
                    runtime.callbacks = CallbackCount(u64::MAX);
                }
                runtime.render(&mut [0.0_f32; 64]);
                assert_eq!(runtime.fault, Fault::Counter);
                assert_eq!(runtime.lane_mut().audio.clock(), SampleTime::ZERO);
            }
        }

        fn constant_graph(level: Amplitude, output: bool) -> GraphIr {
            let builder = GraphIr::builder().node(
                NodeId::new(1),
                IrNodeKind::Constant { level },
                ExecutionScope::Global,
            );
            if output {
                builder
                    .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
                    .connect(
                        (NodeId::new(1), PortId::FIRST),
                        (NodeId::new(2), PortId::FIRST),
                        SignalDomain::Audio,
                    )
                    .build()
                    .unwrap()
            } else {
                builder.build().unwrap()
            }
        }

        #[test]
        fn actual_callback_plan_installation_preserves_audio_clock_and_last_valid_compile() {
            for partition in [1, 37, 64, 256, 512] {
                let (mut runtime, mut control) = state(512, ChannelLayout::Mono);
                let epoch = runtime.lane_mut().audio.epoch();
                control
                    .publish_plan(&constant_graph(Amplitude::new(0.25).unwrap(), true))
                    .unwrap();
                assert!(
                    control
                        .publish_plan(&constant_graph(Amplitude::new(0.75).unwrap(), false))
                        .is_err()
                );
                let mut priming = [9.0_f32; 128];
                no_allocations(|| {
                    for block in priming.chunks_mut(partition) {
                        runtime.render(block);
                    }
                });
                assert_eq!(runtime.fault, Fault::None);
                assert_eq!(runtime.lane_mut().audio.clock(), SampleTime::new(64));
                assert_eq!(runtime.lane_mut().audio.epoch(), epoch);
                control.collect_ready().unwrap();
                assert_eq!(control.plans.receipts().len(), 1);
                assert!(!control.control.has_replacements());
                control
                    .submit(SessionCommand::Play, SampleTime::new(64))
                    .unwrap();
                let mut audible = [9.0_f32; 512];
                no_allocations(|| {
                    for block in audible.chunks_mut(partition) {
                        runtime.render(block);
                    }
                });
                assert_eq!(runtime.fault, Fault::None);
                assert_eq!(audible, [0.25; 512]);
                assert_eq!(runtime.lane_mut().audio.clock(), SampleTime::new(576));
                assert_eq!(runtime.delivered, FrameCount::new(640));
                control.finish(runtime.lane_mut()).unwrap();
            }
        }

        #[test]
        fn owning_plan_mailbox_survives_callback_disappearance_and_is_recovered_after_join() {
            for calls in [0, 1, 2] {
                let (runtime, mut control) = state(64, ChannelLayout::Mono);
                control.publish_plan(&silent_graph().unwrap()).unwrap();
                let pool = Arc::new(HeapRb::new(1));
                let (mut producer, mut consumer) = Arc::clone(&pool).split();
                assert!(producer.try_push(runtime).is_ok());
                drop(producer);
                let worker = thread::spawn(move || {
                    no_allocations(|| {
                        for _ in 0..calls {
                            consumer.first_mut().unwrap().render(&mut [0.0_f32; 64]);
                        }
                        drop(consumer);
                    });
                });
                let owner = LinuxOutputOwner {
                    pool,
                    control: Some(control),
                    progress: Arc::new(Progress::new()),
                    stream: Some(JoinedStream::Worker(Some(worker))),
                    _control_thread: PhantomData,
                };
                let state = owner.close().unwrap();
                assert_eq!(state.plan_receipts.len(), 1);
                use synth_engine_v2::host::session::transfer::replacement::PlanReplacementOutcome;
                if calls < 2 {
                    assert_eq!(
                        state.plan_receipts[0].outcome,
                        PlanReplacementOutcome::Cancelled
                    );
                } else {
                    assert!(matches!(
                        state.plan_receipts[0].outcome,
                        PlanReplacementOutcome::Installed(_)
                    ));
                }
            }
        }

        fn no_allocations(f: impl FnOnce()) {
            let measured = allocation_counter::measure(f);
            assert_eq!(measured.count_total, 0);
            assert_eq!(measured.count_current, 0, "final destruction on audio");
        }

        #[test]
        fn allocation_guard_detects_allocation_and_final_destruction() {
            let owned = Box::new(17_u64);
            let measured = allocation_counter::measure(|| drop(std::hint::black_box(owned)));
            assert_eq!(measured.count_total, 0);
            assert_eq!(measured.count_current, -1);
            let measured = allocation_counter::measure(|| {
                drop(std::hint::black_box(Box::new(17_u64)));
            });
            assert_eq!(measured.count_total, 1);
            assert_eq!(measured.count_current, 0);
        }

        #[test]
        fn full_completion_ring_cannot_delay_stop_or_destroy_its_receipt() {
            let (mut runtime, mut control) = state(512, ChannelLayout::Mono);
            // Narrow this test's return ring to expose pressure before credits run out.
            let completed = Arc::new(HeapRb::new(1));
            let (producer, consumer) = Arc::clone(&completed).split();
            runtime.lane_mut().completed = producer;
            control.completed = consumer;
            control._completed = completed;
            control
                .submit(SessionCommand::Play, SampleTime::ZERO)
                .unwrap();
            control
                .submit(SessionCommand::Stop, SampleTime::new(128))
                .unwrap();
            no_allocations(|| {
                runtime.render(&mut [0.0_f32; 128]);
                assert!(runtime.lane_mut().completed.is_full());
                runtime.render(&mut [0.0_f32; 128]);
            });
            assert_eq!(
                runtime.lane_mut().audio.state(),
                synth_engine_v2::host::session::PlaybackState::Stopped(
                    synth_engine_v2::time::PlanPosition::new(128)
                )
            );
            assert!(runtime.lane_mut().audio.has_retained_commands());
            control.collect_ready().unwrap();
            no_allocations(|| runtime.render(&mut [0.0_f32; 64]));
            control.collect_ready().unwrap();
            assert_eq!(control.receipts.len(), 2);
            assert!(
                control
                    .receipts
                    .iter()
                    .all(|receipt| matches!(receipt.outcome, SessionOutcome::Applied { .. }))
            );
            assert!(!control.control.has_outstanding());
        }

        #[test]
        fn quiescent_close_recovers_every_packet_location() {
            for pending_return in [false, true] {
                let (mut runtime, mut control) = state(512, ChannelLayout::Mono);
                control
                    .submit(SessionCommand::Play, SampleTime::ZERO)
                    .unwrap();
                control
                    .submit(SessionCommand::Stop, SampleTime::new(64))
                    .unwrap();
                let prefix = runtime.lane_mut().ingress_cut();
                assert!(runtime.lane_mut().admit_prefix(prefix));
                no_allocations(|| runtime.render(&mut [0.0_f32; 128]));
                // Initial carry delivers 64 silent frames; this call renders only Q0.
                // Play is returned, but Stop@64 still waits for the next quantum.
                assert_eq!(runtime.lane_mut().audio.clock(), SampleTime::new(64));
                assert!(runtime.lane_mut().audio.has_retained_commands());
                assert_eq!(control.completed.occupied_len(), 1);
                assert!(matches!(
                    control.completed.first().unwrap().outcome(),
                    Some(SessionOutcome::Applied { .. })
                ));
                let packet = control.control.prepare_stop(SampleTime::new(192)).unwrap();
                runtime.lane_mut().failed_command = Some(packet);
                control
                    .submit(SessionCommand::Stop, SampleTime::new(256))
                    .unwrap();
                if pending_return {
                    runtime.lane_mut().pending_completion = control.completed.try_pop();
                }
                control.finish(runtime.lane_mut()).unwrap();
                assert_eq!(control.receipts.len(), 4);
                let mut serials: Vec<_> = control
                    .receipts
                    .iter()
                    .map(|receipt| receipt.boundary.id.serial())
                    .collect();
                serials.sort_unstable();
                assert_eq!(serials, [1, 2, 3, 4]);
                assert!(!control.control.has_outstanding());
                assert_eq!(
                    control
                        .receipts
                        .iter()
                        .filter(|receipt| receipt.outcome == SessionOutcome::Cancelled)
                        .count(),
                    3
                );
                control.finish(runtime.lane_mut()).unwrap();
                assert_eq!(control.receipts.len(), 4);
            }
        }

        #[test]
        fn real_queues_exchange_commands_while_control_and_audio_run_on_separate_threads() {
            let (mut runtime, mut control) = state(512, ChannelLayout::Mono);
            let barrier = Arc::new(Barrier::new(2));
            let worker_barrier = Arc::clone(&barrier);
            thread::scope(|scope| {
                let worker = scope.spawn(move || {
                    for _ in 0..32 {
                        worker_barrier.wait();
                        no_allocations(|| runtime.render(&mut [0.0_f32; 512]));
                        worker_barrier.wait();
                    }
                    runtime
                });
                for round in 0..32 {
                    let command = if round % 2 == 0 {
                        SessionCommand::Play
                    } else {
                        SessionCommand::Stop
                    };
                    let clock = if round == 0 { 0 } else { round * 512 - 64 };
                    control.submit(command, SampleTime::new(clock)).unwrap();
                    barrier.wait();
                    // Core preparation, publication and collection are off audio;
                    // a concurrent reader may observe this receipt now or next poll.
                    control.collect_ready().unwrap();
                    barrier.wait();
                    control.collect_ready().unwrap();
                    assert_eq!(control.receipts.len(), usize::try_from(round + 1).unwrap());
                }
                let mut runtime = worker.join().unwrap();
                control.finish(runtime.lane_mut()).unwrap();
            });
            assert_eq!(control.receipts.len(), 32);
            assert!(
                control
                    .receipts
                    .iter()
                    .all(|receipt| matches!(receipt.outcome, SessionOutcome::Applied { .. }))
            );
        }

        #[test]
        fn malformed_buffers_leave_ingress_untouched_and_protocol_refusal_retains_ownership() {
            let (mut runtime, mut control) = state(64, ChannelLayout::Mono);
            control
                .submit(SessionCommand::Play, SampleTime::ZERO)
                .unwrap();
            no_allocations(|| runtime.render(&mut [0.0_f32; 65]));
            assert_eq!(runtime.fault, Fault::Buffer);
            assert_eq!(runtime.lane_mut().commands.occupied_len(), 1);
            control.finish(runtime.lane_mut()).unwrap();
            assert_eq!(control.receipts[0].outcome, SessionOutcome::Cancelled);

            let (mut runtime, mut control) = state(64, ChannelLayout::Mono);
            let play = control.control.prepare_play(SampleTime::ZERO).unwrap();
            let stop = control.control.prepare_stop(SampleTime::new(64)).unwrap();
            control.commands.try_push(stop).unwrap();
            control.commands.try_push(play).unwrap();
            no_allocations(|| runtime.render(&mut [0.0_f32; 64]));
            assert_eq!(runtime.fault, Fault::Protocol);
            assert!(runtime.lane_mut().failed_command.is_some());
            no_allocations(|| {
                let prefix = runtime.lane_mut().ingress_cut();
                assert!(!runtime.lane_mut().admit_prefix(prefix));
                runtime.render(&mut [0.0_f32; 64]);
            });
            control.finish(runtime.lane_mut()).unwrap();
            assert_eq!(control.receipts.len(), 2);
            assert!(
                control
                    .receipts
                    .iter()
                    .all(|receipt| receipt.outcome == SessionOutcome::Cancelled)
            );
            assert!(!control.control.has_outstanding());
        }

        #[test]
        fn publication_after_the_ingress_cut_cannot_cancel_an_already_offered_play() {
            let (mut runtime, mut control) = state(512, ChannelLayout::Mono);
            control
                .submit(SessionCommand::Play, SampleTime::ZERO)
                .unwrap();
            let prefix = runtime.lane_mut().ingress_cut();
            control
                .submit(SessionCommand::Stop, SampleTime::ZERO)
                .unwrap();
            no_allocations(|| {
                assert!(runtime.lane_mut().admit_prefix(prefix));
                let mut samples = [0.0; 128];
                runtime
                    .lane_mut()
                    .audio
                    .render(AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap())
                    .unwrap();
                runtime.lane_mut().return_completed();
            });
            assert_eq!(runtime.lane_mut().commands.occupied_len(), 1);
            control.collect_ready().unwrap();
            assert!(matches!(
                control.receipts[0].outcome,
                SessionOutcome::Applied { .. }
            ));
            no_allocations(|| runtime.render(&mut [0.0_f32; 128]));
            control.collect_ready().unwrap();
            assert!(matches!(
                control.receipts[1].outcome,
                SessionOutcome::DeliveryRefused(_)
            ));
            control.finish(runtime.lane_mut()).unwrap();
        }

        #[test]
        fn failed_ingress_send_cancels_the_returned_owning_packet() {
            let (mut runtime, mut control) = state(512, ChannelLayout::Mono);
            let commands = Arc::new(HeapRb::new(1));
            let (producer, consumer) = Arc::clone(&commands).split();
            runtime.lane_mut().commands = consumer;
            control.commands = producer;
            control._commands = commands;
            control
                .submit(SessionCommand::Play, SampleTime::ZERO)
                .unwrap();
            assert!(
                control
                    .submit(SessionCommand::Stop, SampleTime::new(64))
                    .is_err()
            );
            assert_eq!(control.receipts.len(), 1);
            assert_eq!(control.receipts[0].outcome, SessionOutcome::Cancelled);
            control.finish(runtime.lane_mut()).unwrap();
            assert_eq!(control.receipts.len(), 2);
            assert!(!control.control.has_outstanding());
        }

        #[test]
        fn backend_diagnostics_retain_every_observed_category_without_allocating() {
            let progress = Progress::new();
            no_allocations(|| {
                for kind in BACKEND_KINDS {
                    progress.note_backend_error(kind);
                }
            });
            assert!(progress.backend_error.load(Ordering::Acquire));
            assert_eq!(progress.backend_diagnostics(), BACKEND_KINDS);
        }

        #[test]
        fn queue_byte_budget_covers_actual_arc_alignment_and_cells() {
            let mut commands = None;
            let mut completed = None;
            let measured = allocation_counter::measure(|| {
                commands = Some(Arc::new(HeapRb::<PreparedSessionCommand>::new(
                    COMMAND_SLOTS,
                )));
                completed = Some(Arc::new(HeapRb::<CompletedSessionCommand>::new(
                    COMMAND_SLOTS,
                )));
            });
            let charged = ControlLane::queue_bytes::<PreparedSessionCommand>()
                + ControlLane::queue_bytes::<CompletedSessionCommand>();
            assert!(measured.bytes_total <= u64::try_from(charged).unwrap());
            assert_eq!(
                measured.count_total, 4,
                "two ring cell allocations and two Arc allocations"
            );
            drop(commands);
            drop(completed);
        }

        fn has_single_locked_version(lock: &str, name: &str, version: &str) -> bool {
            let name_line = format!("name = \"{name}\"");
            let version_line = format!("version = \"{version}\"");
            let mut packages = lock
                .split("[[package]]")
                .filter(|package| package.lines().any(|line| line == name_line));
            packages
                .next()
                .is_some_and(|package| package.lines().any(|line| line == version_line))
                && packages.next().is_none()
        }

        #[test]
        fn dependency_versions_keep_the_reviewed_backend_and_storage_implementations() {
            let lock = include_str!("../../../Cargo.lock");
            for (name, version, decision) in [
                ("cpal", "0.18.2", "ADR-0063"),
                ("ringbuf", "0.5.1", "ADR-0063 and ADR-0064"),
                ("triple_buffer", "9.0.0", "ADR-0064"),
                ("crossbeam-utils", "0.8.22", "ADR-0064"),
            ] {
                assert!(
                    has_single_locked_version(lock, name, version),
                    "requalify {decision} before changing {name} {version}"
                );
                // An old transitive version must not conceal an upgraded direct one.
                let duplicate =
                    format!("{lock}\n[[package]]\nname = \"{name}\"\nversion = \"999.0.0\"\n");
                assert!(!has_single_locked_version(&duplicate, name, version));
            }
        }

        #[test]
        fn close_failure_preserves_an_earlier_run_failure() {
            for run in [
                HarnessError::Timeout,
                HarnessError::Callback(Fault::Backend),
                HarnessError::Callback(Fault::Render),
            ] {
                let message = run.to_string();
                let combined = HarnessError::Custody.after_run(Err(run));
                let HarnessError::RunAndClose { run, close } = combined else {
                    panic!("both failures must be retained");
                };
                assert_eq!(run.to_string(), message);
                assert!(matches!(*close, HarnessError::Custody));
            }
            assert!(matches!(
                HarnessError::Custody.after_run(Ok(())),
                HarnessError::Custody
            ));
        }

        #[test]
        fn invalid_preparation_and_cli_input_refuse() {
            for bound in [0, 8193, u64::MAX] {
                assert!(
                    CallbackState::prepare(
                        SampleRate::new(48_000.0).unwrap(),
                        ChannelLayout::Mono,
                        FrameCount::new(bound)
                    )
                    .is_err()
                );
            }
            for raw in ["0", "-1", "", "18446744073709551616"] {
                assert!(CallbackTarget::parse(raw).is_err());
            }
            assert_eq!(CallbackTarget::parse("100").unwrap().0, 100);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    {
        linux::run()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("v2_cpal_output requires Linux and the ALSA backend".into())
    }
}
