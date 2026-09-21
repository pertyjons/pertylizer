//! Fixed-storage source publication, FIFO pairing and ordered capture finalization.

use super::{
    ActiveCapture, AuditionTrace, CaptureBuffer, CaptureDisposition, CaptureError, CaptureOutcome,
    CaptureStage, CaptureStamp, CaptureStopReason, ConnectionGeneration, HeldInput, Midi1Event,
    Midi1Input, NoteCaptureError, NoteCell, PerformedOccurrenceId, PublicationReceipt,
    PublicationSequence, RecordedInput, RecordedSourceState, SampleTime, SimulatedNoteRecorder,
    SourceState, StreamEpoch, SyntheticClosure, TakeReservation,
};

impl SimulatedNoteRecorder {
    pub fn source_sequence(
        &self,
        source: ConnectionGeneration,
    ) -> Result<PublicationSequence, NoteCaptureError> {
        Ok(self.source_copy(source)?.sequence)
    }

    /// Freeze the serial host's capture selection before acknowledging source shutdown.
    /// A missing fence establishes no captured prefix, even if raw input is retained.
    pub(crate) fn interrupt_host(
        &mut self,
        reason: CaptureStopReason,
    ) -> Result<(), NoteCaptureError> {
        if self.host_interrupted {
            return Ok(());
        }
        if let Some(active) = self.active {
            let mut boundary = self.window(active.ticket)?.end();
            for source in self.sources.iter().flatten() {
                if self.participates(active.ticket, source.generation) {
                    boundary = boundary.min(source.fence.unwrap_or(SampleTime::ZERO));
                }
            }
            self.stop_at(active.ticket, boundary, CaptureOutcome::Interrupted, reason)?;
            self.try_seal()?;
        }
        self.host_interrupted = true;
        Ok(())
    }

    /// The caller supplies a backend fence, not a later GUI timestamp. Publication
    /// has been closed by the host; the already acknowledged frontier cannot advance.
    pub(crate) fn acknowledge_host_source(
        &mut self,
        source: ConnectionGeneration,
    ) -> Result<(), NoteCaptureError> {
        let state = self.source_copy(source)?;
        if state.quiescent {
            return Ok(());
        }
        self.quiesce(source, self.epoch, state.fence.unwrap_or(SampleTime::ZERO))
    }

    pub(crate) fn host_quiescent(&self) -> bool {
        self.host_interrupted
            && self.active.is_none()
            && self.sources.iter().flatten().all(|source| source.quiescent)
    }

    pub(super) fn source_index(
        &self,
        generation: ConnectionGeneration,
    ) -> Result<usize, NoteCaptureError> {
        for (at, source) in self.sources.iter().enumerate() {
            if source.is_some_and(|s| s.generation == generation) {
                return Ok(at);
            }
        }
        Err(NoteCaptureError::ForeignSource)
    }
    fn source_copy(
        &self,
        generation: ConnectionGeneration,
    ) -> Result<SourceState, NoteCaptureError> {
        self.sources
            .get(self.source_index(generation)?)
            .copied()
            .flatten()
            .ok_or(NoteCaptureError::ForeignSource)
    }
    fn participates(&self, ticket: TakeReservation, generation: ConnectionGeneration) -> bool {
        self.store.slots.get(ticket.slot).is_some_and(|slot| {
            slot.state.is_some_and(|state| state.id == ticket.id)
                && slot
                    .sources
                    .iter()
                    .flatten()
                    .any(|s| s.generation == generation)
        })
    }
    fn active_for(&self, ticket: TakeReservation) -> Result<ActiveCapture, NoteCaptureError> {
        self.active
            .filter(|active| active.ticket == ticket)
            .ok_or(NoteCaptureError::NotActive)
    }
    fn window(&self, ticket: TakeReservation) -> Result<super::CaptureWindow, NoteCaptureError> {
        self.store
            .slots
            .get(ticket.slot)
            .and_then(|slot| slot.state)
            .filter(|state| state.id == ticket.id)
            .map(|state| state.window)
            .ok_or(NoteCaptureError::Storage(CaptureError::StaleReservation))
    }

    /// Apply the explicit session start before any source input at this boundary.
    /// Every selected source must have consumed its fence at exactly this boundary.
    pub fn start(&mut self, ticket: TakeReservation) -> Result<(), NoteCaptureError> {
        if self.session_lane.is_some() {
            return Err(NoteCaptureError::OrderedSession);
        }
        self.start_boundary(ticket)
    }

    pub(super) fn start_boundary(
        &mut self,
        ticket: TakeReservation,
    ) -> Result<(), NoteCaptureError> {
        let active = self.active_for(ticket)?;
        if !matches!(active.stage, CaptureStage::Armed) {
            return Err(NoteCaptureError::StartRequired);
        }
        let window = self.window(ticket)?;
        if self.observed > window.start() {
            return Err(NoteCaptureError::PastBoundary);
        }
        for source in self.sources.iter().flatten() {
            if self.participates(ticket, source.generation) {
                if !source.synchronized || source.quiescent {
                    return Err(NoteCaptureError::Unsynchronized);
                }
                if source.fence != Some(window.start()) {
                    return Err(NoteCaptureError::StartFence);
                }
            }
        }
        self.snapshot_sources(ticket)?;
        let mut at = 0;
        for held in self.tracker.iter().flatten() {
            if self.participates(ticket, held.occurrence.source) {
                self.store.write_fixture_metadata(
                    ticket,
                    CaptureBuffer::TrackedInput,
                    at,
                    NoteCell::Held(*held),
                )?;
                at += 1;
            }
        }
        self.observed = window.start();
        self.active = Some(ActiveCapture {
            ticket,
            stage: CaptureStage::Capturing,
            seal_ready: active.seal_ready,
        });
        if window.start() == window.end() {
            self.stop_at(
                ticket,
                window.end(),
                CaptureOutcome::Complete,
                CaptureStopReason::Stop,
            )?;
            self.try_seal()?;
        }
        Ok(())
    }

    pub(super) fn snapshot_sources(
        &mut self,
        ticket: TakeReservation,
    ) -> Result<(), NoteCaptureError> {
        let count = self.store.layout.sources;
        let boundary = self.window(ticket)?.start();
        for at in 0..count {
            let ledger = self
                .store
                .slots
                .get(ticket.slot)
                .and_then(|slot| slot.sources.get(at))
                .copied()
                .flatten();
            if let Some(ledger) = ledger {
                let state = self.source_copy(ledger.generation)?;
                let snapshot = NoteCell::Source(RecordedSourceState {
                    source: state.generation,
                    through: state.sequence,
                    controls: state.controls,
                    boundary,
                    refused_pedals: None,
                });
                self.store.write_fixture_metadata(
                    ticket,
                    CaptureBuffer::InitialSourceState,
                    at,
                    snapshot,
                )?;
                self.store.write_fixture_metadata(
                    ticket,
                    CaptureBuffer::TerminalSourceState,
                    at,
                    snapshot,
                )?;
            }
        }
        Ok(())
    }

    /// A validated source publication. Original timestamps and occurrence pairing are
    /// independent of the supplied audition trace, including explicit playback refusals.
    pub fn publish(
        &mut self,
        generation: ConnectionGeneration,
        stamp: CaptureStamp,
        input: Midi1Input,
        audition: AuditionTrace,
    ) -> Result<PublicationReceipt, NoteCaptureError> {
        if stamp.epoch != self.epoch {
            return Err(NoteCaptureError::ForeignEpoch);
        }
        self.check_session_publication(stamp.published_at)?;
        let before = self.source_copy(generation)?;
        if before.quiescent {
            return Err(NoteCaptureError::RebindRequired);
        }
        if stamp.published_at < self.observed
            || stamp.published_at < before.observed
            || before
                .fence
                .is_some_and(|frontier| stamp.published_at < frontier)
        {
            return Err(NoteCaptureError::PastBoundary);
        }
        // Missing session start is a retryable precondition, not a consumed publication.
        if let Some(active) = self.active
            && matches!(active.stage, CaptureStage::Armed)
            && stamp.published_at >= self.window(active.ticket)?.start()
        {
            return Err(NoteCaptureError::StartRequired);
        }
        let late = before.fence.is_some_and(|fence| stamp.nominal < fence);
        // Quality custody cannot depend on successful pairing or an available identity.
        if late {
            self.attribute_late(generation, stamp)?;
        }
        if !before.synchronized {
            return Err(NoteCaptureError::Unsynchronized);
        }
        self.before_publication(stamp.published_at)?;
        let at = self.source_index(generation)?;
        let sequence = if let Some(next) = before.sequence.0.checked_add(1) {
            PublicationSequence(next)
        } else {
            self.invalidate_source(at, stamp.published_at)?;
            return Err(NoteCaptureError::IdentityExhausted);
        };
        let anomaly = before.last_nominal.is_some_and(|last| stamp.nominal < last);
        if let Some(source) = self.sources.get_mut(at).and_then(Option::as_mut) {
            source.sequence = sequence;
            source.observed = stamp.published_at;
            source.last_nominal = Some(stamp.nominal);
            if anomaly {
                source.diagnostics.timing_anomalies.increment();
            }
        }
        self.observed = stamp.published_at;
        let capture_eligible = self.capture_eligible(generation, stamp)?;
        let occurrence = match self.pair_input(at, input, capture_eligible) {
            Ok(occurrence) => occurrence,
            Err(error) => {
                self.invalidate_source(at, stamp.published_at)?;
                return Err(error);
            }
        };
        let record = RecordedInput {
            source: generation,
            sequence,
            stamp,
            input,
            occurrence,
            audition,
            timing_anomaly: anomaly,
        };
        self.observe_paired_release(record)?;
        let capture = if late {
            CaptureDisposition::Late
        } else {
            self.capture_record(record)?
        };
        if capture != CaptureDisposition::Recorded {
            self.observe_refused_pedal(record)?;
        }
        self.try_seal()?;
        Ok(PublicationReceipt {
            sequence,
            occurrence,
            capture,
        })
    }

    fn before_publication(&mut self, at: SampleTime) -> Result<(), NoteCaptureError> {
        if let Some(active) = self.active {
            let window = self.window(active.ticket)?;
            match active.stage {
                CaptureStage::Capturing if at >= window.end() => {
                    self.stop_at(
                        active.ticket,
                        window.end(),
                        CaptureOutcome::Complete,
                        CaptureStopReason::Stop,
                    )?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn capture_eligible(
        &self,
        generation: ConnectionGeneration,
        stamp: CaptureStamp,
    ) -> Result<bool, NoteCaptureError> {
        let Some(active) = self.active else {
            return Ok(false);
        };
        if !self.participates(active.ticket, generation)
            || matches!(
                active.stage,
                CaptureStage::Armed
                    | CaptureStage::Stopping {
                        outcome: CaptureOutcome::Partial | CaptureOutcome::Interrupted,
                        ..
                    }
            )
        {
            return Ok(false);
        }
        let window = self.window(active.ticket)?;
        let source = self.source_copy(generation)?;
        Ok(stamp.nominal >= window.start()
            && stamp.nominal < window.end()
            && source.fence.is_none_or(|fence| stamp.nominal >= fence))
    }

    fn unspent_tracker_reserve(&self) -> Result<usize, NoteCaptureError> {
        let Some(active) = self.active else {
            return Ok(0);
        };
        if matches!(
            active.stage,
            CaptureStage::Stopping {
                outcome: CaptureOutcome::Partial | CaptureOutcome::Interrupted,
                ..
            }
        ) {
            return Ok(0);
        }
        let window = self.window(active.ticket)?;
        if window.start() == window.end() {
            return Ok(0);
        }
        let begin = self.store.layout.starts[CaptureBuffer::TerminalOccurrence.index()];
        let length = self.store.layout.lengths[CaptureBuffer::TerminalOccurrence.index()];
        let slot = self
            .store
            .slots
            .get(active.ticket.slot)
            .ok_or(CaptureError::StaleReservation)?;
        let mut unspent = 0;
        for index in 0..length {
            if slot.data.get(begin + index).is_some_and(Option::is_none) {
                unspent += 1;
            }
        }
        Ok(unspent)
    }

    fn pair_input(
        &mut self,
        source_at: usize,
        input: Midi1Input,
        capture_eligible: bool,
    ) -> Result<Option<PerformedOccurrenceId>, NoteCaptureError> {
        if matches!(input.event, Midi1Event::NoteOn { .. }) && !capture_eligible {
            let mut free = 0;
            for held in &self.tracker {
                if held.is_none() {
                    free += 1;
                }
            }
            let reserved = self.unspent_tracker_reserve()?;
            if reserved > 0 && free <= reserved {
                return Err(NoteCaptureError::TrackerReserved);
            }
        }
        let source = self
            .sources
            .get_mut(source_at)
            .and_then(Option::as_mut)
            .ok_or(NoteCaptureError::ForeignSource)?;
        match input.event {
            Midi1Event::NoteOn { key, .. } => {
                let mut free = None;
                for (at, held) in self.tracker.iter().enumerate() {
                    if held.is_none() {
                        free = Some(at);
                        break;
                    }
                }
                let slot = self
                    .tracker
                    .get_mut(free.ok_or(NoteCaptureError::TrackerFull)?)
                    .ok_or(NoteCaptureError::TrackerFull)?;
                let serial = source
                    .last_occurrence
                    .map_or(0, |id| id.serial)
                    .checked_add(1)
                    .ok_or(NoteCaptureError::IdentityExhausted)?;
                let occurrence = PerformedOccurrenceId {
                    source: source.generation,
                    serial,
                };
                source.last_occurrence = Some(occurrence);
                *slot = Some(HeldInput {
                    occurrence,
                    channel: input.channel,
                    key,
                });
                Ok(Some(occurrence))
            }
            Midi1Event::KeyRelease { key, .. } => {
                let mut selected: Option<(usize, PerformedOccurrenceId)> = None;
                for (at, held) in self.tracker.iter().enumerate() {
                    if let Some(held) = held
                        && held.occurrence.source == source.generation
                        && held.channel == input.channel
                        && held.key == key
                        && selected.is_none_or(|(_, old)| held.occurrence.serial < old.serial)
                    {
                        selected = Some((at, held.occurrence));
                    }
                }
                if let Some((at, occurrence)) = selected {
                    if let Some(slot) = self.tracker.get_mut(at) {
                        *slot = None;
                    }
                    Ok(Some(occurrence))
                } else {
                    source.diagnostics.unmatched_releases.increment();
                    Ok(None)
                }
            }
            Midi1Event::Sustain { down } => {
                if let Some(pedal) = source
                    .controls
                    .pedals
                    .get_mut(usize::from(input.channel.as_index()))
                {
                    *pedal = down;
                }
                Ok(None)
            }
            Midi1Event::PitchBend { value } => {
                if let Some(bend) = source
                    .controls
                    .bends
                    .get_mut(usize::from(input.channel.as_index()))
                {
                    *bend = value;
                }
                Ok(None)
            }
        }
    }

    fn invalidate_source(
        &mut self,
        source_at: usize,
        at: SampleTime,
    ) -> Result<(), NoteCaptureError> {
        let source = self
            .sources
            .get_mut(source_at)
            .and_then(Option::as_mut)
            .ok_or(NoteCaptureError::ForeignSource)?;
        source.synchronized = false;
        source.diagnostics.pairing_failures.increment();
        let generation = source.generation;
        if let Some(active) = self.active
            && self.participates(active.ticket, generation)
        {
            self.stop_at(
                active.ticket,
                at,
                CaptureOutcome::Interrupted,
                CaptureStopReason::SourceInvalid,
            )?;
        }
        Ok(())
    }

    /// An input adapter may reject a known exact late observation before serial
    /// admission. Preserve the same quality custody, including already sealed takes.
    pub(super) fn attribute_refused_input(
        &mut self,
        generation: ConnectionGeneration,
        stamp: CaptureStamp,
    ) -> Result<(), NoteCaptureError> {
        if stamp.epoch != self.epoch {
            return Err(NoteCaptureError::ForeignEpoch);
        }
        let source = self.source_copy(generation)?;
        if source.quiescent {
            return Err(NoteCaptureError::RebindRequired);
        }
        if source
            .fence
            .is_some_and(|frontier| stamp.nominal < frontier)
        {
            self.attribute_late(generation, stamp)?;
        }
        Ok(())
    }

    /// Attribute an uncertain refused observation conservatively to any retained
    /// selected interval it could overlap. Never manufacture an exact late time.
    pub(super) fn attribute_uncertain_input(
        &mut self,
        generation: ConnectionGeneration,
        earliest: SampleTime,
        latest: SampleTime,
    ) -> Result<(), NoteCaptureError> {
        if earliest > latest {
            return Err(NoteCaptureError::MappingRange);
        }
        let source = self.source_copy(generation)?;
        if source.quiescent {
            return Err(NoteCaptureError::RebindRequired);
        }
        for index in 0..self.store.slots.len() {
            let state = self.store.slots.get(index).and_then(|slot| slot.state);
            if let Some(state) = state {
                let ticket = TakeReservation {
                    id: state.id,
                    slot: index,
                };
                if self.participates(ticket, generation)
                    && state.window.start < state.window.end
                    && earliest < state.window.end
                    && latest >= state.window.start
                {
                    let slot = self.store.slot_mut(ticket)?;
                    if let Some(state) = &mut slot.state
                        && state.quality.first_uncertain_source.is_none()
                    {
                        state.quality.first_uncertain_source = Some(generation);
                    }
                }
            }
        }
        Ok(())
    }

    fn attribute_late(
        &mut self,
        generation: ConnectionGeneration,
        stamp: CaptureStamp,
    ) -> Result<(), NoteCaptureError> {
        let source_at = self.source_index(generation)?;
        if let Some(source) = self.sources.get_mut(source_at).and_then(Option::as_mut) {
            source.diagnostics.late_refusals.increment();
            if source.diagnostics.first_late.is_none() {
                source.diagnostics.first_late = Some(super::super::LateCaptureInput {
                    source: generation,
                    time: stamp.nominal,
                });
            }
        }
        for at in 0..self.store.slots.len() {
            let state = self.store.slots.get(at).and_then(|slot| slot.state);
            if let Some(state) = state {
                let ticket = TakeReservation {
                    id: state.id,
                    slot: at,
                };
                if self.participates(ticket, generation) {
                    let fault = self.store.attribute_fixture_late(
                        ticket,
                        generation,
                        self.epoch,
                        stamp.nominal,
                    )?;
                    if fault == super::super::LateAttribution::TakeFaulted
                        && self.active.is_some_and(|active| active.ticket == ticket)
                    {
                        self.stop_at(
                            ticket,
                            stamp.published_at,
                            CaptureOutcome::Partial,
                            CaptureStopReason::LateInput,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn observe_paired_release(&mut self, record: RecordedInput) -> Result<(), NoteCaptureError> {
        if !matches!(record.input.event, Midi1Event::KeyRelease { .. }) {
            return Ok(());
        }
        let Some(active) = self.active else {
            return Ok(());
        };
        if !self.participates(active.ticket, record.source) {
            return Ok(());
        }
        let begin = self.store.layout.starts[CaptureBuffer::Ordinary.index()];
        let length = self.store.layout.lengths[CaptureBuffer::Ordinary.index()];
        let slot = self.store.slot_mut(active.ticket)?;
        for index in 0..length {
            if let Some(Some(NoteCell::Input {
                record: onset,
                paired_release,
                ..
            })) = slot.data.get_mut(begin + index)
                && matches!(onset.input.event, Midi1Event::NoteOn { .. })
                && onset.occurrence == record.occurrence
            {
                *paired_release = Some(record.stamp.nominal);
            }
        }
        Ok(())
    }

    fn observe_refused_pedal(&mut self, record: RecordedInput) -> Result<(), NoteCaptureError> {
        let Midi1Event::Sustain { down } = record.input.event else {
            return Ok(());
        };
        let Some(active) = self.active else {
            return Ok(());
        };
        let window = self.window(active.ticket)?;
        if record.stamp.nominal < window.start() || record.stamp.nominal >= window.end() {
            return Ok(());
        }
        let begin = self.store.layout.starts[CaptureBuffer::TerminalSourceState.index()];
        let length = self.store.layout.sources;
        let slot = self.store.slot_mut(active.ticket)?;
        for index in 0..length {
            if let Some(Some(NoteCell::Source(snapshot))) = slot.data.get_mut(begin + index)
                && snapshot.source == record.source
            {
                let mut history = snapshot.refused_pedals.unwrap_or(super::RefusedPedals {
                    earliest: record.stamp.nominal,
                    latest: record.stamp.nominal,
                    seen: [false; 16],
                    last_down: [false; 16],
                });
                history.earliest = history.earliest.min(record.stamp.nominal);
                history.latest = history.latest.max(record.stamp.nominal);
                let channel = usize::from(record.input.channel.as_index());
                *history
                    .seen
                    .get_mut(channel)
                    .ok_or(CaptureError::MetadataBounds)? = true;
                *history
                    .last_down
                    .get_mut(channel)
                    .ok_or(CaptureError::MetadataBounds)? = down;
                snapshot.refused_pedals = Some(history);
            }
        }
        Ok(())
    }

    fn capture_record(
        &mut self,
        record: RecordedInput,
    ) -> Result<CaptureDisposition, NoteCaptureError> {
        let Some(active) = self.active else {
            return Ok(CaptureDisposition::NotRecording);
        };
        if !self.participates(active.ticket, record.source) {
            return Ok(CaptureDisposition::OutsideInterval);
        }
        if matches!(active.stage, CaptureStage::Armed) {
            return Ok(CaptureDisposition::OutsideInterval);
        }
        if let CaptureStage::Stopping {
            reason, outcome, ..
        } = active.stage
            && outcome != CaptureOutcome::Complete
        {
            return Ok(CaptureDisposition::Stopped(reason));
        }
        let window = self.window(active.ticket)?;
        if record.stamp.nominal < window.start() || record.stamp.nominal >= window.end() {
            return Ok(CaptureDisposition::OutsideInterval);
        }
        let held_slot = if matches!(record.input.event, Midi1Event::NoteOn { .. }) {
            let start = self.store.layout.starts[CaptureBuffer::TerminalOccurrence.index()];
            let length = self.store.layout.lengths[CaptureBuffer::TerminalOccurrence.index()];
            let slot = self
                .store
                .slots
                .get(active.ticket.slot)
                .ok_or(CaptureError::StaleReservation)?;
            let mut free = None;
            for index in 0..length {
                if slot.data.get(start + index).is_some_and(Option::is_none) {
                    free = Some(index);
                    break;
                }
            }
            if free.is_none() {
                return self.capacity_stop(active.ticket, record.stamp.published_at);
            }
            free
        } else {
            None
        };
        match self.store.push_fixture(
            active.ticket,
            NoteCell::Input {
                record,
                release: None,
                paired_release: None,
                closure: None,
            },
        ) {
            Ok(()) => {}
            Err(CaptureError::OrdinaryFull | CaptureError::CaptureStopped) => {
                return self.capacity_stop(active.ticket, record.stamp.published_at);
            }
            Err(error) => return Err(NoteCaptureError::Storage(error)),
        }
        self.apply_captured_state(active.ticket, record, held_slot)?;
        Ok(CaptureDisposition::Recorded)
    }

    fn capacity_stop(
        &mut self,
        ticket: TakeReservation,
        at: SampleTime,
    ) -> Result<CaptureDisposition, NoteCaptureError> {
        self.stop_at(
            ticket,
            at,
            CaptureOutcome::Partial,
            CaptureStopReason::Capacity,
        )?;
        Ok(CaptureDisposition::Stopped(CaptureStopReason::Capacity))
    }

    fn apply_captured_state(
        &mut self,
        ticket: TakeReservation,
        record: RecordedInput,
        held_slot: Option<usize>,
    ) -> Result<(), NoteCaptureError> {
        match record.input.event {
            Midi1Event::NoteOn { key, .. } => {
                if let (Some(index), Some(occurrence)) = (held_slot, record.occurrence) {
                    self.store.write_fixture_metadata(
                        ticket,
                        CaptureBuffer::TerminalOccurrence,
                        index,
                        NoteCell::Held(HeldInput {
                            occurrence,
                            channel: record.input.channel,
                            key,
                        }),
                    )?;
                }
            }
            Midi1Event::KeyRelease { .. } => {
                // Retain the paired release alongside its onset before H's live slot is reused.
                // Finalization may select an earlier boundary without losing that relationship.
                let ordinary = self.store.layout.starts[CaptureBuffer::Ordinary.index()];
                let length = self.store.layout.lengths[CaptureBuffer::Ordinary.index()];
                let slot = self.store.slot_mut(ticket)?;
                for index in 0..length {
                    if let Some(Some(NoteCell::Input {
                        record: onset,
                        release,
                        ..
                    })) = slot.data.get_mut(ordinary + index)
                        && matches!(onset.input.event, Midi1Event::NoteOn { .. })
                        && onset.occurrence == record.occurrence
                    {
                        *release = Some(record.stamp.nominal);
                    }
                }
                let start = self.store.layout.starts[CaptureBuffer::TerminalOccurrence.index()];
                let length = self.store.layout.lengths[CaptureBuffer::TerminalOccurrence.index()];
                let slot = self.store.slot_mut(ticket)?;
                for index in 0..length {
                    if let Some(cell) = slot.data.get_mut(start + index)
                        && matches!(cell, Some(NoteCell::Held(held)) if Some(held.occurrence) == record.occurrence)
                    {
                        *cell = None;
                    }
                }
            }
            Midi1Event::Sustain { .. } | Midi1Event::PitchBend { .. } => {}
        }
        let start = self.store.layout.starts[CaptureBuffer::TerminalSourceState.index()];
        let length = self.store.layout.sources;
        let slot = self.store.slot_mut(ticket)?;
        for index in 0..length {
            if let Some(Some(NoteCell::Source(source))) = slot.data.get_mut(start + index)
                && source.source == record.source
            {
                source.through = record.sequence;
                let channel = usize::from(record.input.channel.as_index());
                match record.input.event {
                    Midi1Event::Sustain { down } => {
                        if let Some(value) = source.controls.pedals.get_mut(channel) {
                            *value = down;
                        }
                    }
                    Midi1Event::PitchBend { value } => {
                        if let Some(bend) = source.controls.bends.get_mut(channel) {
                            *bend = value;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Stop, disarm, panic or finalize before a mapping change. This never discards.
    pub fn stop(
        &mut self,
        ticket: TakeReservation,
        at: SampleTime,
        reason: CaptureStopReason,
    ) -> Result<(), NoteCaptureError> {
        if self.session_lane.is_some() {
            return Err(NoteCaptureError::OrderedSession);
        }
        self.stop_boundary(ticket, at, reason)
    }

    pub(super) fn stop_boundary(
        &mut self,
        ticket: TakeReservation,
        at: SampleTime,
        reason: CaptureStopReason,
    ) -> Result<(), NoteCaptureError> {
        self.active_for(ticket)?;
        if at < self.observed {
            return Err(NoteCaptureError::PastBoundary);
        }
        if matches!(
            reason,
            CaptureStopReason::Capacity
                | CaptureStopReason::LateInput
                | CaptureStopReason::LoopCarryCapacity
                | CaptureStopReason::LoopRenderFault
                | CaptureStopReason::SourceInvalid
                | CaptureStopReason::DeviceLost
        ) {
            return Err(NoteCaptureError::InvalidStopReason);
        }
        self.stop_at(ticket, at, CaptureOutcome::Complete, reason)?;
        self.observed = at;
        self.try_seal()?;
        Ok(())
    }

    pub(super) fn stop_at(
        &mut self,
        ticket: TakeReservation,
        at: SampleTime,
        outcome: CaptureOutcome,
        reason: CaptureStopReason,
    ) -> Result<(), NoteCaptureError> {
        let active = self.active_for(ticket)?;
        let mut at = at.min(self.window(ticket)?.end());
        let mut outcome = outcome;
        let mut reason = reason;
        if let CaptureStage::Stopping {
            at: prior,
            outcome: prior_outcome,
            reason: prior_reason,
        } = active.stage
        {
            at = at.min(prior);
            if prior_outcome == CaptureOutcome::Interrupted
                || (prior_outcome == CaptureOutcome::Partial && outcome == CaptureOutcome::Complete)
            {
                outcome = prior_outcome;
                reason = prior_reason;
            }
        }
        let slot = self.store.slot_mut(ticket)?;
        if let Some(state) = &mut slot.state {
            if matches!(active.stage, CaptureStage::Armed) {
                // No capture ever started. Cancellation/loss selects an empty interval,
                // and a normal cancellation needs fences, not source retirement.
                state.window.start = at;
            } else {
                state.window.start = state.window.start.min(at);
            }
            state.window.end = at;
        }
        self.active = Some(ActiveCapture {
            ticket,
            stage: CaptureStage::Stopping {
                at,
                outcome,
                reason,
            },
            seal_ready: active.seal_ready,
        });
        Ok(())
    }

    /// Consume one source's declared publication fence. The sequence must match the
    /// current consumed source sequence; no callback, pending record or worker is inferred.
    pub fn fence(
        &mut self,
        generation: ConnectionGeneration,
        epoch: StreamEpoch,
        frontier: SampleTime,
        through: PublicationSequence,
    ) -> Result<(), NoteCaptureError> {
        self.check_session_fence(frontier)?;
        if epoch != self.epoch {
            return Err(NoteCaptureError::ForeignEpoch);
        }
        let source = self.source_copy(generation)?;
        if source.quiescent {
            return Err(NoteCaptureError::RebindRequired);
        }
        if through != source.sequence
            || frontier < source.observed
            || source.fence.is_some_and(|last| frontier < last)
        {
            return Err(NoteCaptureError::InvalidFence);
        }
        self.propagate_fence(generation, frontier, false)?;
        let at = self.source_index(generation)?;
        if let Some(source) = self.sources.get_mut(at).and_then(Option::as_mut) {
            source.fence = Some(frontier);
        }
        if let Some(active) = self.active
            && matches!(active.stage, CaptureStage::Capturing)
            && self.participates(active.ticket, generation)
        {
            let end = self.window(active.ticket)?.end();
            if frontier >= end {
                self.stop_at(
                    active.ticket,
                    end,
                    CaptureOutcome::Complete,
                    CaptureStopReason::Stop,
                )?;
            }
        }
        self.try_seal()?;
        Ok(())
    }

    /// Close one source without a final callback. Invalid pairing still permits this
    /// lifecycle fence, after which only a fresh generation can publish more input.
    pub fn quiesce(
        &mut self,
        generation: ConnectionGeneration,
        epoch: StreamEpoch,
        last_valid: SampleTime,
    ) -> Result<(), NoteCaptureError> {
        if !self.host_interrupted {
            self.check_session_fence(last_valid)?;
        }
        if epoch != self.epoch {
            return Err(NoteCaptureError::ForeignEpoch);
        }
        let source = self.source_copy(generation)?;
        if source.quiescent {
            return Err(NoteCaptureError::RebindRequired);
        }
        if source.fence.is_some_and(|last| last_valid < last) {
            return Err(NoteCaptureError::InvalidFence);
        }
        self.propagate_fence(generation, last_valid, true)?;
        let index = self.source_index(generation)?;
        if let Some(source) = self.sources.get_mut(index).and_then(Option::as_mut) {
            source.quiescent = true;
            source.fence = Some(last_valid);
        }
        if let Some(active) = self.active
            && self.participates(active.ticket, generation)
        {
            self.stop_at(
                active.ticket,
                last_valid,
                CaptureOutcome::Interrupted,
                CaptureStopReason::DeviceLost,
            )?;
        }
        self.try_seal()?;
        Ok(())
    }

    fn propagate_fence(
        &mut self,
        generation: ConnectionGeneration,
        frontier: SampleTime,
        quiescent: bool,
    ) -> Result<(), NoteCaptureError> {
        for at in 0..self.store.slots.len() {
            if let Some(state) = self.store.slots.get(at).and_then(|slot| slot.state) {
                let ticket = TakeReservation {
                    id: state.id,
                    slot: at,
                };
                if self.participates(ticket, generation) {
                    if quiescent {
                        self.store.acknowledge_fixture_quiescence(
                            ticket, generation, self.epoch, frontier,
                        )?;
                    } else {
                        self.store
                            .acknowledge_fixture_fence(ticket, generation, self.epoch, frontier)?;
                    }
                    break; // S002 updates every retained ledger for this generation.
                }
            }
        }
        Ok(())
    }

    pub(super) fn try_seal(&mut self) -> Result<(), NoteCaptureError> {
        let Some(active) = self.active else {
            return Ok(());
        };
        if !active.seal_ready {
            return Ok(());
        }
        let CaptureStage::Stopping {
            at,
            outcome,
            reason,
        } = active.stage
        else {
            return Ok(());
        };
        let slot = self
            .store
            .slots
            .get(active.ticket.slot)
            .ok_or(CaptureError::StaleReservation)?;
        for source in slot.sources.iter().flatten() {
            if !source.fenced
                || source.watermark < at
                || (outcome == CaptureOutcome::Interrupted && !source.quiescent)
            {
                return Ok(());
            }
        }
        if slot.data.iter().flatten().any(|cell| {
            matches!(cell,
            NoteCell::Input { record, .. } if matches!(record.audition, AuditionTrace::Pending(_)))
        }) {
            return Err(NoteCaptureError::PendingAudition);
        }
        self.finalize_selected_metadata(active.ticket, at, reason)?;
        self.store.seal_fixture(active.ticket, outcome, at)?;
        // A completed session boundary constrains future segments, including fresh sources.
        self.observed = self.observed.max(at);
        self.active = None;
        Ok(())
    }

    fn finalize_selected_metadata(
        &mut self,
        ticket: TakeReservation,
        at: SampleTime,
        reason: CaptureStopReason,
    ) -> Result<(), NoteCaptureError> {
        let window = self.window(ticket)?;
        let initial = self.store.layout.starts[CaptureBuffer::InitialSourceState.index()];
        let terminal = self.store.layout.starts[CaptureBuffer::TerminalSourceState.index()];
        let ordinary = self.store.layout.starts[CaptureBuffer::Ordinary.index()];
        let length = self.store.layout.lengths[CaptureBuffer::Ordinary.index()];
        let source_count = self.store.layout.sources;
        let slot = self.store.slot_mut(ticket)?;
        // Rebuild only the selected controller state, retaining raw out-of-window observations.
        for index in 0..source_count {
            let mut snapshot = match slot.data.get(initial + index).copied().flatten() {
                Some(NoteCell::Source(snapshot)) => snapshot,
                _ => continue,
            };
            if let Some(Some(NoteCell::Source(previous))) = slot.data.get(terminal + index) {
                snapshot.refused_pedals = previous.refused_pedals;
            }
            snapshot.boundary = at;
            for event in 0..length {
                if let Some(Some(NoteCell::Input { record, .. })) = slot.data.get(ordinary + event)
                    && record.source == snapshot.source
                    && record.stamp.nominal >= window.start()
                    && record.stamp.nominal < at
                {
                    snapshot.through = record.sequence;
                    let channel = usize::from(record.input.channel.as_index());
                    match record.input.event {
                        Midi1Event::Sustain { down } => {
                            if let Some(pedal) = snapshot.controls.pedals.get_mut(channel) {
                                *pedal = down;
                            }
                        }
                        Midi1Event::PitchBend { value } => {
                            if let Some(bend) = snapshot.controls.bends.get_mut(channel) {
                                *bend = value;
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(cell) = slot.data.get_mut(terminal + index) {
                *cell = Some(NoteCell::Source(snapshot));
            }
        }
        // Every admitted raw cell already owns its closure field. A shortened interval can
        // expose more than H historical held notes, so H's reusable live slots are insufficient.
        for event in 0..length {
            let Some(NoteCell::Input {
                record,
                release,
                paired_release,
                ..
            }) = slot.data.get(ordinary + event).copied().flatten()
            else {
                continue;
            };
            if !matches!(record.input.event, Midi1Event::NoteOn { .. })
                || record.stamp.nominal < window.start()
                || record.stamp.nominal >= at
                || release.is_some_and(|time| time >= window.start() && time < at)
            {
                continue;
            }
            let Some(occurrence) = record.occurrence else {
                continue;
            };
            let mut pedal_held = None;
            for index in 0..source_count {
                if let Some(Some(NoteCell::Source(snapshot))) = slot.data.get(terminal + index)
                    && snapshot.source == record.source
                {
                    pedal_held = snapshot.observed_pedal(record.input.channel);
                }
            }
            if let Some(Some(NoteCell::Input { closure, .. })) = slot.data.get_mut(ordinary + event)
            {
                *closure = Some(SyntheticClosure {
                    occurrence,
                    time: at,
                    key_held: paired_release.is_none_or(|time| time >= at),
                    pedal_held,
                    reason,
                });
            }
        }
        Ok(())
    }
}

impl RecordedSourceState {
    /// Known pedal state at this snapshot's boundary, including refused observations.
    /// None explicitly marks a cut through coalesced refused history; it is never guessed.
    pub fn observed_pedal(&self, channel: synth_core::MidiChannel) -> Option<bool> {
        let index = usize::from(channel.as_index());
        let captured = self.controls.pedals.get(index).copied()?;
        let Some(refused) = self.refused_pedals else {
            return Some(captured);
        };
        if !refused.seen.get(index).copied()? || self.boundary <= refused.earliest {
            Some(captured)
        } else if self.boundary > refused.latest {
            refused.last_down.get(index).copied()
        } else {
            None
        }
    }
}

impl CaptureStamp {
    pub fn exact_fixture(
        epoch: StreamEpoch,
        nominal: SampleTime,
        published_at: SampleTime,
    ) -> Result<Self, NoteCaptureError> {
        if nominal > published_at {
            return Err(NoteCaptureError::FutureInput);
        }
        Ok(Self {
            epoch,
            nominal,
            published_at,
        })
    }
}

impl SimulatedNoteRecorder {
    /// Reconcile identified audition metadata before sealing; nominal raw data is immutable.
    /// Zero matches means the observation was not retained by this capture consumer.
    pub fn resolve_audition(
        &mut self,
        ticket: TakeReservation,
        id: crate::host::live::AuditionId,
        outcome: crate::host::live::AuditionOutcome,
    ) -> Result<crate::quantities::EventCount, NoteCaptureError> {
        if self.active.is_none_or(|active| active.ticket != ticket) {
            return Err(NoteCaptureError::NotActive);
        }
        let begin = self.store.layout.starts[CaptureBuffer::Ordinary.index()];
        let length = self.store.layout.lengths[CaptureBuffer::Ordinary.index()];
        let slot = self.store.slot_mut(ticket)?;
        let mut resolved = 0_u32;
        for index in begin..begin + length {
            let Some(Some(cell)) = slot.data.get_mut(index) else {
                continue;
            };
            if let NoteCell::Input { record, .. } = cell
                && record.audition == AuditionTrace::Pending(id)
            {
                record.audition = AuditionTrace::Resolved(outcome);
                resolved += 1;
            }
        }
        Ok(crate::quantities::EventCount::measured(resolved))
    }
}
