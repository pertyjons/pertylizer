//! Off-thread pass/carry materialization before raw sealing. No additional allocation.
use super::*;
use crate::looping::journal::LoopJournalEndReason;

fn carried(
    cell: &Option<NoteCell>,
    start: SampleTime,
    at: SampleTime,
) -> Option<PerformedOccurrenceId> {
    match cell {
        Some(NoteCell::Input {
            record, release, ..
        }) if matches!(record.input.event, Midi1Event::NoteOn { .. })
            && record.stamp.nominal >= start
            && record.stamp.nominal < at
            && release.is_none_or(|release| release >= at) =>
        {
            record.occurrence
        }
        _ => None,
    }
}

impl LoopCaptureSession {
    /// Finalize off-thread only after actual audio observation ends and source cuts
    /// cover the selected prefix. Accepted raw outside a shortened prefix is retained.
    /// This creates key-carry metadata, never a musical projection certificate.
    pub fn finalize(&mut self) -> Result<(), LoopCaptureError> {
        let ticket = self.ticket.ok_or(LoopCaptureError::State)?;
        if self.recorder.active.is_none() {
            let _sealed = self.recorder.result(ticket)?;
            return Ok(());
        }
        let terminal = self.journal.end().ok_or(LoopCaptureError::AwaitingAudio)?;
        let (outcome, reason) = match terminal.reason {
            LoopJournalEndReason::Finished | LoopJournalEndReason::PassLimit => {
                (CaptureOutcome::Complete, CaptureStopReason::Stop)
            }
            LoopJournalEndReason::RenderFault(_) => (
                CaptureOutcome::Interrupted,
                CaptureStopReason::LoopRenderFault,
            ),
        };
        self.recorder
            .stop_at(ticket, terminal.at, outcome, reason)?;
        let active = self.recorder.active.ok_or(LoopCaptureError::State)?;
        let CaptureStage::Stopping { at, outcome, .. } = active.stage else {
            return Err(LoopCaptureError::State);
        };
        let slot = self
            .recorder
            .store
            .slot_mut(ticket)
            .map_err(NoteCaptureError::from)?;
        for source in slot.sources.iter().flatten() {
            if !source.fenced
                || source.watermark < at
                || (outcome == CaptureOutcome::Interrupted && !source.quiescent)
            {
                return Err(LoopCaptureError::AwaitingSources);
            }
        }
        self.materialize(ticket)?;
        if let Some(active) = &mut self.recorder.active {
            active.seal_ready = true;
        }
        self.recorder.try_seal()?;
        let _sealed = self.recorder.result(ticket)?;
        Ok(())
    }

    fn materialize(&mut self, ticket: TakeReservation) -> Result<(), LoopCaptureError> {
        let initial = self.journal.initial();
        let mut pass_id = self.first_pass.ok_or(LoopCaptureError::State)?;
        let mut rendered = initial.pass;
        let mut position = initial.position;
        let mut start = initial.clock;
        let layout = &self.recorder.store.layout;
        let ordinary_start = layout.starts[CaptureBuffer::Ordinary.index()];
        let ordinary_end = ordinary_start + layout.lengths[CaptureBuffer::Ordinary.index()];
        let held = self
            .recorder
            .limits
            .max_held_notes_per_take()
            .as_usize()
            .ok_or(NoteCaptureError::from(CaptureError::LayoutOverflow))?;
        let mut pass_index = 0;
        let mut carry_index = 0;
        let window = self
            .recorder
            .store
            .slot_mut(ticket)
            .map_err(NoteCaptureError::from)?
            .state
            .ok_or(LoopCaptureError::State)?
            .window;
        let mut end = window.end();
        // Source interruption can select before initial.clock: ordinary empty-window
        // finalization already moved the start back. Never emit a reversed pass interval.
        start = start.min(end);
        for boundary in self.journal.boundaries() {
            if boundary.at >= end {
                break;
            }
            let slot = self
                .recorder
                .store
                .slot_mut(ticket)
                .map_err(NoteCaptureError::from)?;
            let ordinary = slot
                .data
                .get(ordinary_start..ordinary_end)
                .ok_or(NoteCaptureError::from(CaptureError::MetadataBounds))?;
            let count = ordinary
                .iter()
                .filter_map(|cell| carried(cell, window.start(), boundary.at))
                .count();
            if count > held {
                end = boundary.at;
                self.carry_capacity = Some(boundary.at);
                self.recorder.stop_at(
                    ticket,
                    end,
                    CaptureOutcome::Partial,
                    CaptureStopReason::LoopCarryCapacity,
                )?;
                break;
            }
            let next = CapturePassId(
                pass_id
                    .0
                    .checked_add(1)
                    .ok_or(NoteCaptureError::IdentityExhausted)?,
            );
            self.recorder
                .store
                .write_fixture_metadata(
                    ticket,
                    CaptureBuffer::Pass,
                    pass_index,
                    NoteCell::LoopPass(LoopCapturePass {
                        id: pass_id,
                        rendered,
                        position,
                        window: CaptureWindow::new(initial.epoch, start, boundary.at)
                            .map_err(NoteCaptureError::from)?,
                    }),
                )
                .map_err(NoteCaptureError::from)?;
            for index in ordinary_start..ordinary_end {
                let occurrence = self
                    .recorder
                    .store
                    .slot_mut(ticket)
                    .map_err(NoteCaptureError::from)?
                    .data
                    .get(index)
                    .and_then(|cell| carried(cell, window.start(), boundary.at));
                if let Some(occurrence) = occurrence {
                    for (pass, peer, direction) in [
                        (pass_id, next, LoopCarryDirection::Out),
                        (next, pass_id, LoopCarryDirection::In),
                    ] {
                        self.recorder
                            .store
                            .write_fixture_metadata(
                                ticket,
                                CaptureBuffer::Carry,
                                carry_index,
                                NoteCell::Carry(LoopCarry {
                                    occurrence,
                                    pass,
                                    peer,
                                    at: boundary.at,
                                    direction,
                                }),
                            )
                            .map_err(NoteCaptureError::from)?;
                        carry_index += 1;
                    }
                }
            }
            pass_index += 1;
            pass_id = next;
            rendered = boundary.next;
            position = boundary.interval.start();
            start = boundary.at;
        }
        self.recorder
            .store
            .write_fixture_metadata(
                ticket,
                CaptureBuffer::Pass,
                pass_index,
                NoteCell::LoopPass(LoopCapturePass {
                    id: pass_id,
                    rendered,
                    position,
                    window: CaptureWindow::new(initial.epoch, start, end)
                        .map_err(NoteCaptureError::from)?,
                }),
            )
            .map_err(NoteCaptureError::from)?;
        Ok(())
    }
}
