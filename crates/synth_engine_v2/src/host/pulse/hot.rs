//! Prepared metronome publication and rendering without callback allocation.
use super::{PulseError, PulseStream};
use crate::{
    publish::ProducerClass,
    quantities::ParameterValue,
    render::{AudioBlockMut, EventEnvelope, EventPayload, Renderer, TimedEvent},
    time::{FrameCount, QUANTUM_FRAMES, SampleTime, TimeSource},
};
impl PulseStream {
    pub fn end_at(&mut self, at: SampleTime) -> Result<(), PulseError> {
        if at < self.renderer.clock() || !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES)) {
            return Err(PulseError::Configuration);
        }
        if self.end.is_none_or(|prior| at < prior) {
            self.end = Some(at);
        }
        Ok(())
    }
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), PulseError> {
        if output.frames() == 0
            || output.layout() != self.renderer.plan().channel_layout()
            || u64::try_from(output.frames()).map_or(true, |n| {
                n > self.renderer.plan().maximum_block_size().as_u64()
            })
        {
            output.silence();
            return Err(PulseError::Configuration);
        }
        let result = self.render_inner(output.reborrow());
        if result.is_err() {
            output.silence();
        }
        result
    }
    fn render_inner(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), PulseError> {
        while output.frames() > 0 {
            let carry = self.renderer.carry_frames();
            let clock = self.renderer.clock();
            let frames = output.frames().min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let mut publication = self.arbiter.open(clock, usize::from(carry == 0))?;
            if carry == 0 {
                if !self.closed && self.end.is_some_and(|end| end <= clock) {
                    publication.charge(
                        ProducerClass::Session,
                        TimedEvent::new(
                            EventEnvelope::new(self.renderer.epoch(), clock, TimeSource::Compiled),
                            EventPayload::SetParameter {
                                slot: self.gate,
                                value: ParameterValue::ZERO,
                            },
                        ),
                    )?;
                    self.closed = true;
                    self.next = self.edges.len();
                }
                let end = clock
                    .checked_add(FrameCount::QUANTUM)
                    .map_err(|_| PulseError::Configuration)?;
                while let Some(event) = self.edges.get(self.next) {
                    if event.envelope().time() >= end {
                        break;
                    }
                    publication.charge(ProducerClass::Session, *event)?;
                    self.next += 1;
                }
            }
            let (block, rest) = match output.split_at_frame(frames) {
                Ok((block, rest)) => (block, Some(rest)),
                Err(block) => (block, None),
            };
            self.renderer.render(block, publication.seal().events())?;
            if let Some(rest) = rest {
                output = rest;
            } else {
                break;
            }
        }
        Ok(())
    }
}
