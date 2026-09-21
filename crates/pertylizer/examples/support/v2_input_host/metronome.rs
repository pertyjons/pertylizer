//! Independently owned prepared count-in and metronome renderer.
use super::*;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    host::pulse::{PulseError, PulseStream},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    profile::HostProfile,
    quantities::{Amplitude, ChannelLayout, Frequency, NormalizedLevel, Seconds},
    report::{ResourceAmount, ResourceField},
    time::FrameCount,
};
pub struct MetronomeAudio {
    renderer: PulseStream,
    scratch: Box<[f32]>,
    layout: ChannelLayout,
    maximum_frames: usize,
    end_seen: bool,
}
impl MetronomeAudio {
    pub fn prepare(
        profile: HostProfile,
        pulses: &[SampleTime],
        width: FrameCount,
    ) -> Result<(Self, PreparedBytes), Box<dyn std::error::Error>> {
        let graph = GraphIr::builder()
            .node(
                NodeId::new(1),
                IrNodeKind::Sine {
                    frequency: Frequency::new(1500.0)?,
                    amplitude: Amplitude::new(0.05)?,
                },
                ExecutionScope::Global,
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
                ExecutionScope::Global,
            )
            .node(
                NodeId::new(3),
                IrNodeKind::Amplifier,
                ExecutionScope::Global,
            )
            .node(NodeId::new(4), IrNodeKind::Output, ExecutionScope::Global)
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
                (NodeId::new(4), PortId::FIRST),
                SignalDomain::Audio,
            )
            .tuning(
                ExecutionScope::Global,
                synth_engine_v2::tuning::PreparedTuning::equal_temperament()?,
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
                return Err("missing metronome resource charge".into());
            };
            bytes = bytes
                .checked_add(amount.get())
                .ok_or("metronome layout overflow")?;
        }
        let plan = compiled.into_plan()?;
        let gate = plan
            .resolve_parameter(
                NodeId::new(2),
                synth_engine_v2::ir::parameters::ENVELOPE_GATE,
            )
            .ok_or("missing metronome gate")?;
        let renderer = PulseStream::prepare(
            plan,
            profile,
            gate,
            pulses,
            width,
            PreparedBytes::measured(1_000_000),
        )?;
        let maximum_frames = usize::try_from(profile.capabilities().maximum_block_size().as_u64())?;
        let layout = profile.capabilities().channel_layout();
        let samples = maximum_frames
            .checked_mul(layout.channels())
            .ok_or("metronome scratch overflow")?;
        let extra = samples
            .checked_mul(size_of::<f32>())
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .ok_or("metronome scratch overflow")?;
        bytes = bytes
            .checked_add(renderer.bytes().get())
            .and_then(|n| n.checked_add(u64::try_from(extra).ok()?))
            .ok_or("metronome layout overflow")?;
        Ok((
            Self {
                renderer,
                scratch: vec![0.0; samples].into_boxed_slice(),
                layout,
                maximum_frames,
                end_seen: false,
            },
            PreparedBytes::measured(bytes),
        ))
    }
    pub fn validate(&self, output: &AudioBlockMut<'_>) -> Result<(), PulseError> {
        if output.layout() != self.layout
            || output.frames() == 0
            || output.frames() > self.maximum_frames
        {
            return Err(PulseError::Configuration);
        }
        Ok(())
    }
    pub fn render(
        &mut self,
        output: &mut AudioBlockMut<'_>,
        end: Option<(SampleTime, SessionCommand)>,
    ) -> Result<(), PulseError> {
        if !self.end_seen
            && let Some((at, _)) = end
        {
            self.renderer.end_at(at)?;
            self.end_seen = true;
        }
        let frames = output.frames();
        let scratch = self
            .scratch
            .get_mut(..frames * self.layout.channels())
            .ok_or(PulseError::Configuration)?;
        self.renderer.render(
            AudioBlockMut::new(scratch, frames, self.layout)
                .map_err(|_| PulseError::Configuration)?,
        )?;
        for (destination, sample) in output.samples_mut().iter_mut().zip(scratch) {
            *destination += *sample;
        }
        Ok(())
    }
}
