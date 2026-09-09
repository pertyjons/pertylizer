//! Fixed storage, fixed quantum clock, one authority for identity and publication.
use super::{
    AuthoredFault, AuthoredNoteStream, AuthoredRenderError, GeneratedNote, Occurrence, Phase,
};
use crate::{
    identity::Resolution,
    publish::{ProducerClass, Publication},
    quantities::{EventCount, KeyIdentity, NoteVelocity},
    render::{
        AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, Renderer, TimedEvent, TimedEvents,
    },
    script::PreparedInput,
    time::{Located, QUANTUM_FRAMES, SampleTime, TimeSource},
};

impl AuthoredNoteStream {
    /// Render a host block; any producer fault silences the complete outer callback.
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), AuthoredRenderError> {
        if self.renderer.diagnostics().needs_reprepare()
            || output.layout() != self.renderer.plan().channel_layout()
            || output.frames() as u64 > self.renderer.plan().maximum_block_size().as_u64()
        {
            return self
                .renderer
                .render(output, TimedEvents::EMPTY)
                .map_err(Into::into);
        }
        let frames = output.frames();
        let mut start = 0;
        while start < frames {
            let count = (frames - start).min(QUANTUM_FRAMES as usize);
            let result = match output.window(start, count) {
                Some(block) => self.render_quantum(block),
                None => Err(AuthoredRenderError::Source {
                    producer: self.producer,
                    fault: AuthoredFault::Context,
                }),
            };
            if let Err(error) = result {
                if let AuthoredRenderError::Source { fault, .. } = error {
                    self.fault = Some(fault);
                    self.renderer.terminal_authored_fault(&mut output, fault);
                } else if self.renderer.diagnostics().needs_reprepare() {
                    output.silence();
                } else {
                    // Producer state already advanced: a refusal cannot safely be retried.
                    self.fault = Some(AuthoredFault::RendererRefused);
                    self.renderer
                        .terminal_authored_fault(&mut output, AuthoredFault::RendererRefused);
                }
                return Err(error);
            }
            start += count;
        }
        Ok(())
    }

    fn render_quantum(&mut self, output: AudioBlockMut<'_>) -> Result<(), AuthoredRenderError> {
        let producer = self.producer;
        let source_error = |fault| AuthoredRenderError::Source { producer, fault };
        let clock = self.renderer.clock();
        let quanta = self.renderer.quanta_needed_for(output.frames());
        if quanta == 0 {
            return self
                .renderer
                .render(output, TimedEvents::EMPTY)
                .map_err(Into::into);
        }
        let mut publication = self
            .arbiter
            .open(clock, quanta)
            .map_err(|_| source_error(AuthoredFault::Publication))?;
        self.automation
            .drain_authored_automation(self.control.epoch(), clock, &mut publication)
            .map_err(source_error)?;
        let (captured, captured_once) = self
            .renderer
            .note_context(self.script_node)
            .ok_or_else(|| source_error(AuthoredFault::Context))?;
        let program = self
            .renderer
            .plan()
            .prepared_scripts()
            .get(self.script.index())
            .ok_or_else(|| source_error(AuthoredFault::Context))?;
        let mut evaluations = 0_u32;
        // Chronological selection prevents freeing an index at a future end before an earlier start.
        loop {
            let next = self
                .occurrences
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let (time, order) = match entry.phase {
                        Phase::Pending => (entry.start, 1_u8),
                        Phase::Held(_, _) => (entry.generated?.end, 0_u8),
                        Phase::Complete => return None,
                    };
                    (time.quantum_index() <= clock.quantum_index()).then_some((time, order, index))
                })
                .min();
            let Some((time, _, index)) = next else {
                break;
            };
            if time < clock {
                return Err(source_error(AuthoredFault::Late));
            }
            let entry = self
                .occurrences
                .get_mut(index)
                .ok_or_else(|| source_error(AuthoredFault::Context))?;
            match entry.phase {
                Phase::Pending => {
                    evaluations = evaluations.saturating_add(1);
                    let generated = evaluate(
                        program,
                        if captured_once {
                            captured
                        } else {
                            program.initial_sources
                        },
                        *entry,
                        self.maximum_duration,
                        &self.tempo,
                        self.control.anchor(),
                    )
                    .map_err(source_error)?;
                    if let Some(note) = generated {
                        let identity = self
                            .control
                            .minter_mut()
                            .mint_keyed(producer, self.target, note.key)
                            .map_err(|_| source_error(AuthoredFault::Identity))?;
                        let event = stamp(
                            self.control.epoch(),
                            note.start,
                            EventPayload::Note {
                                identity,
                                edge: NoteEdge::On {
                                    slot: self.target,
                                    key: note.key,
                                    velocity: note.velocity,
                                },
                            },
                        );
                        publication
                            .charge(ProducerClass::AuthoredRuntime, event)
                            .map_err(|_| source_error(AuthoredFault::Publication))?;
                        entry.generated = Some(note);
                        entry.phase =
                            Phase::Held(identity, note.end.quantum_index() > clock.quantum_index());
                    } else {
                        entry.phase = Phase::Complete;
                    }
                }
                Phase::Held(identity, held) => {
                    charge_release(&mut publication, self.control.epoch(), identity, time, held)
                        .map_err(source_error)?;
                    if self.control.minter_mut().release_for(producer, identity) != Resolution::Live
                    {
                        return Err(source_error(AuthoredFault::Identity));
                    }
                    entry.phase = Phase::Complete;
                }
                Phase::Complete => return Err(source_error(AuthoredFault::Context)),
            }
            let mut pending = 0_u32;
            let mut holds = 0_u32;
            for entry in &self.occurrences {
                match entry.phase {
                    Phase::Pending => pending += 1,
                    Phase::Held(_, true) => holds += 1,
                    _ => {}
                }
            }
            self.usage.held_releases = self.usage.held_releases.max(EventCount::measured(holds));
            self.usage.future_events = self.usage.future_events.max(EventCount::measured(holds));
            if pending.saturating_add(holds) > self.usage.reserved_obligations.get() {
                return Err(source_error(AuthoredFault::Identity));
            }
        }
        self.usage.evaluations_per_quantum = self
            .usage
            .evaluations_per_quantum
            .max(EventCount::measured(evaluations));
        let batch = publication.seal();
        self.renderer.render(output, batch.events())?;
        self.usage.authored_per_quantum = self.arbiter.high_water(ProducerClass::AuthoredRuntime);
        self.usage.releases_per_quantum = self.arbiter.high_water(ProducerClass::Release);
        Ok(())
    }
}

fn stamp(epoch: crate::time::StreamEpoch, time: SampleTime, payload: EventPayload) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(epoch, time, TimeSource::Authored),
        payload,
    )
}
fn charge_release(
    publication: &mut Publication<'_>,
    epoch: crate::time::StreamEpoch,
    identity: crate::identity::NoteIdentity,
    time: SampleTime,
    held: bool,
) -> Result<(), AuthoredFault> {
    publication
        .charge(
            if held {
                ProducerClass::Release
            } else {
                ProducerClass::AuthoredRuntime
            },
            stamp(
                epoch,
                time,
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                },
            ),
        )
        .map_err(|_| AuthoredFault::Publication)
}

fn evaluate(
    program: &crate::script::PreparedScript,
    mut sources: [f32; synth_core::script::MAX_SOURCES],
    entry: Occurrence,
    maximum: super::ScriptDuration,
    tempo: &crate::tempo::TempoMap,
    anchor: crate::time::StreamAnchor,
) -> Result<Option<GeneratedNote>, AuthoredFault> {
    let input = entry.raw;
    for (binding, value) in program.inputs.iter().zip(&mut sources) {
        if let PreparedInput::NoteField(field) = binding {
            *value = match field {
                synth_core::script::NoteField::Pitch => f32::from(input.key.as_u8()),
                synth_core::script::NoteField::Vel => input.velocity.as_f32(),
                synth_core::script::NoteField::Dur => {
                    input.duration.map_or(-1.0, super::ScriptDuration::as_f32)
                }
                synth_core::script::NoteField::Tick => input.tick.as_f32(),
            };
        }
    }
    let seed = synth_core::hash::splitmix64(
        program.seed.as_u64() ^ input.occurrence.as_u64() ^ 0x4E4F_5445_4556_5401,
    );
    let mut registers = synth_core::script::RegisterFile::new(0, seed);
    let result = program.code.eval_note(&sources, &mut registers);
    if [result.pitch, result.vel, result.dur, result.gate]
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(AuthoredFault::Output);
    }
    let velocity = result.vel.unwrap_or(input.velocity.as_f32());
    if velocity < 0.0 {
        return Ok(None);
    }
    let key = u8::try_from(rounded(
        result.pitch.unwrap_or(f32::from(input.key.as_u8())),
        127.0,
    ))
    .ok()
    .and_then(|key| KeyIdentity::new(key).ok())
    .ok_or(AuthoredFault::Output)?;
    let velocity =
        NoteVelocity::new(velocity.clamp(0.0, 1.0)).map_err(|_| AuthoredFault::Output)?;
    let duration = result
        .dur
        .unwrap_or(input.duration.map_or(-1.0, super::ScriptDuration::as_f32));
    let end = if duration < 0.0 {
        entry.cut
    } else {
        let duration = u16::try_from(rounded(duration, maximum.as_f32()))
            .map_err(|_| AuthoredFault::Output)?;
        let gated = u16::try_from(rounded(
            f32::from(duration) * result.gate.unwrap_or(1.0).clamp(0.0, 1.0),
            maximum.as_f32(),
        ))
        .map_err(|_| AuthoredFault::Output)?;
        let tick = input
            .start
            .as_u64()
            .checked_add(u64::from(gated))
            .map(crate::tempo::MusicalTick::new)
            .ok_or(AuthoredFault::Output)?;
        let position = tempo.position_of(tick).map_err(|_| AuthoredFault::Output)?;
        match anchor.locate(position) {
            Located::At(time) => time.min(entry.cut),
            _ => return Err(AuthoredFault::Output),
        }
    };
    if end <= entry.start {
        return Ok(None);
    }
    Ok(Some(GeneratedNote {
        occurrence: input.occurrence,
        start: entry.start,
        end,
        key,
        velocity,
    }))
}
fn rounded(value: f32, maximum: f32) -> i32 {
    // Every caller checked finiteness; maxima are 127 or the u16 duration ceiling.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "finite clamped value is at most u16::MAX and fits i32"
    )]
    {
        value.max(0.0).min(maximum).round() as i32
    }
}
