//! One admitted fixture recipe; callers cannot assert an arbitrary retention footprint.
use super::*;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    host::{
        EndpointId,
        input::{
            InputCapacity, InputLimits, InputRate, InputTick, InputTickSpan, SimulatedInputClock,
            SimulatedNoteInput,
        },
        session::{
            LoopRecordingSession, SessionCommandCapacity, SessionLimits, SessionSourceCapacity,
            SessionSourceLimits,
        },
    },
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    looping::{CompiledLoopStream, LoopSettings},
    profile::{CaptureLimits, CaptureLimitsInput, HostProfile, RecordingLimits},
    quantities::{
        CapturePassCount, CaptureResultCount, CaptureSourceCount, EventCount, HeldNoteCount,
        ProjectionTickCount, TrackedInputNoteCount,
    },
    recording::notes::{
        CaptureMode, CaptureQuantization, ControllerSnapshot, FixtureRevision, FixtureTargetId,
        MusicalInterval,
        loop_capture::{LoopCaptureSession, LoopNoteArmInput},
    },
    report::{ResourceAmount, ResourceField},
    schedule::AdmittedCompiledStream,
    tempo::{Bpm, MusicalTick, TempoMap},
    time::{FrameCount, PlanPosition, StreamEpoch},
    transport::LoopInterval,
};

#[must_use]
pub struct PreparedAttempt {
    profile: HostProfile,
    start: SampleTime,
    pub(super) metronome: Option<super::metronome::MetronomeAudio>,
    pub(super) audition: Option<(
        super::audition::AuditionControl,
        super::audition::AuditionAudio,
    )>,
    pub(super) owner: InputCaptureSession,
    pub(super) generations: [ConnectionGeneration; 2],
    pub(super) bytes: PreparedBytes,
}
impl PreparedAttempt {
    pub const fn start(&self) -> SampleTime {
        self.start
    }
    /// Whole-quarter count-in at fixed 120 BPM, followed by a one-bar loop.
    /// Profiles whose exact musical start is not quantum aligned are refused.
    pub fn counted(
        profile: HostProfile,
        count_in: MusicalTick,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use synth_engine_v2::tempo::TICKS_PER_QUARTER;
        let quarter = u64::from(TICKS_PER_QUARTER);
        if count_in == MusicalTick::ZERO
            || !count_in.as_u64().is_multiple_of(quarter)
            || count_in.as_u64() > 16 * quarter
        {
            return Err("count-in requires one to sixteen whole quarter notes".into());
        }
        let tempo = TempoMap::new(Bpm::new(120.0)?, &[], profile.capabilities().sample_rate())?;
        let start = SampleTime::new(tempo.position_of(count_in)?.as_u64());
        let mut prepared = Self::new_interval(
            profile,
            start,
            IrNodeKind::Silence,
            MusicalTick::new(4 * quarter),
        )?;
        let beats = count_in.as_u64() / quarter + 32 * 4;
        let mut pulses = Vec::new();
        for beat in 0..beats {
            pulses.push(SampleTime::new(
                tempo
                    .position_of(MusicalTick::new(beat * quarter))?
                    .as_u64(),
            ));
        }
        let width = FrameCount::new(tempo.position_of(MusicalTick::new(quarter / 32))?.as_u64());
        let (metronome, bytes) =
            super::metronome::MetronomeAudio::prepare(profile, &pulses, width)?;
        prepared.bytes = PreparedBytes::measured(
            prepared
                .bytes
                .get()
                .checked_add(bytes.get())
                .ok_or("metronome aggregate overflow")?,
        );
        prepared.metronome = Some(metronome);
        Ok(prepared)
    }
    pub fn with_audition(mut self) -> Result<Self, Box<dyn std::error::Error>> {
        let (control, audio, bytes) = super::audition::AuditionControl::prepare(
            self.profile,
            self.epoch(),
            self.generations,
        )?;
        self.bytes = PreparedBytes::measured(
            self.bytes
                .get()
                .checked_add(bytes.get())
                .ok_or("aggregate live preparation overflow")?,
        );
        self.audition = Some((control, audio));
        Ok(self)
    }
    pub fn epoch(&self) -> StreamEpoch {
        self.owner.acknowledged().epoch
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
    #[cfg(test)]
    pub fn new(
        profile: HostProfile,
        start: SampleTime,
        kind: IrNodeKind,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_interval(profile, start, kind, MusicalTick::new(8))
    }
    fn new_interval(
        profile: HostProfile,
        start: SampleTime,
        kind: IrNodeKind,
        end: MusicalTick,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let graph = GraphIr::builder()
            .node(NodeId::new(1), kind, ExecutionScope::Global)
            .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (NodeId::new(1), PortId::FIRST),
                (NodeId::new(2), PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()?;
        let compiled = compile(&graph, &RenderConfig::new(profile));
        let mut bytes = 0_u64;
        for field in [
            ResourceField::PreparedImmutableBytes,
            ResourceField::MutableStateBytes,
            ResourceField::BufferScratchBytes,
        ] {
            let Some(ResourceAmount::Bytes(amount)) =
                compiled.report().row(field).map(|row| row.requested())
            else {
                return Err("missing renderer byte admission".into());
            };
            bytes = bytes
                .checked_add(amount.get())
                .ok_or("renderer layout overflow")?;
        }
        let plan = compiled.into_plan()?;
        let events = AdmittedCompiledStream::admit(&plan, &[])?;
        let rate = profile.capabilities().sample_rate();
        let tempo = TempoMap::new(Bpm::new(120.0)?, &[], rate)?;
        let interval = MusicalInterval::new(MusicalTick::ZERO, end)?;
        let stream = CompiledLoopStream::prepare(
            plan,
            events,
            profile,
            LoopSettings::new(
                LoopInterval::new(PlanPosition::ZERO, tempo.position_of(interval.end())?)
                    .ok_or("empty loop")?,
                PlanPosition::ZERO,
                PreparedBytes::measured(1_000_000),
            )?,
        )?;
        bytes = bytes
            .checked_add(stream.storage_bytes().get())
            .ok_or("loop layout overflow")?;
        let limits = RecordingLimits::new(HeldNoteCount::limit(4)?, EventCount::limit(64)?)?
            .with_capture(CaptureLimits::new(CaptureLimitsInput {
                max_tracked_input_notes: TrackedInputNoteCount::limit(8)?,
                max_capture_sources: CaptureSourceCount::limit(2)?,
                max_capture_passes: CapturePassCount::limit(32)?,
                max_pending_capture_results: CaptureResultCount::limit(1)?,
                max_capture_bytes: PreparedBytes::measured(1_048_576),
                max_audio_capture_frames: FrameCount::new(1),
                max_projection_ticks: ProjectionTickCount::limit(4096)?,
                capture_lateness_allowance: FrameCount::ZERO,
            })?)?;
        let mut capture =
            LoopCaptureSession::prepare(stream, limits, PreparedBytes::measured(8192))?;
        let mut inputs = Vec::new();
        let mut sources = Vec::new();
        let mut generations = Vec::new();
        for port in 0..2 {
            let mut input = SimulatedNoteInput::new(
                EndpointId::new(format!("simulated-{port}"))?,
                InputLimits {
                    cells: InputCapacity::new(32)?,
                    bytes: PreparedBytes::measured(65536),
                },
            )?;
            let generation = input.begin()?;
            input.prepare(
                generation,
                SimulatedInputClock::new(
                    capture.initial().epoch,
                    SampleTime::ZERO,
                    InputTick::new(0),
                    InputRate::new(FrameCount::new(1), InputTickSpan::new(port + 1))?,
                    InputTickSpan::new(0),
                ),
            )?;
            sources.push(input.bind_capture(
                generation,
                &mut capture,
                ControllerSnapshot::neutral(),
            )?);
            bytes = bytes
                .checked_add(input.bytes().get())
                .ok_or("input layout overflow")?;
            generations.push(generation);
            inputs.push(input);
        }
        let _ticket = capture.arm_at(
            LoopNoteArmInput {
                target: FixtureTargetId::new(1)?,
                expected_revision: FixtureRevision::new(1),
                interval,
                mode: CaptureMode::Overdub,
                quantization: CaptureQuantization::Off,
                tempo,
            },
            &sources,
            start,
        )?;
        // Reserve the complete capture ceiling, including future off-thread projection.
        for amount in [1_048_576, capture.journal_bytes().get()] {
            bytes = bytes.checked_add(amount).ok_or("capture layout overflow")?;
        }
        let session = LoopRecordingSession::prepare(
            capture,
            SessionLimits {
                commands: SessionCommandCapacity::new(4)?,
                command_bytes: PreparedBytes::measured(16384),
            },
            SessionSourceLimits {
                actions: SessionSourceCapacity::new(32)?,
                bytes: PreparedBytes::measured(32768),
            },
        )?;
        for amount in [session.command_bytes().get(), session.source_bytes().get()] {
            bytes = bytes.checked_add(amount).ok_or("session layout overflow")?;
        }
        let owner = InputCaptureSession::prepare(
            session,
            inputs.into_boxed_slice(),
            PreparedBytes::measured(8192),
        )?;
        for amount in [
            owner.bytes().get(),
            65536,
            LiveControl::storage_bytes().get(),
            2 * super::source::SourceInbox::storage_bytes().get(),
            profile
                .capabilities()
                .maximum_block_size()
                .as_u64()
                .checked_mul(u64::try_from(
                    profile.capabilities().channel_layout().channels(),
                )?)
                .and_then(|n| n.checked_mul(4))
                .ok_or("callback scratch overflow")?,
            4096, // Callback descriptor, one-cell pool, Arc headers and progress atomics.
            u64::try_from(size_of::<Self>())?,
        ] {
            bytes = bytes.checked_add(amount).ok_or("host layout overflow")?;
        }
        let [first, second] = generations.as_slice() else {
            return Err("input generation count".into());
        };
        Ok(Self {
            profile,
            start,
            metronome: None,
            audition: None,
            owner,
            generations: [*first, *second],
            bytes: PreparedBytes::measured(bytes),
        })
    }
}
