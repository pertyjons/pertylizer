//! Bounded source-order reporting across stage-full sound splits.

use std::collections::VecDeque;

use super::*;

#[derive(Debug, PartialEq)]
enum JournalOutcome {
    Note(CombinedResult),
    Bend(BendCombinedResult),
    Split(BridgeRefused),
}

#[derive(Debug, PartialEq)]
enum JournalState {
    Staged {
        occurrence: Occurrence,
        release: bool,
    },
    AwaitStage,
    Ready(JournalOutcome),
}

#[derive(Debug, PartialEq)]
struct JournalEntry {
    source: SourceHandoff,
    raw: InputEventId,
    state: JournalState,
}

/// This private note/bend/key-up proof keeps one charge from raw admission
/// until ordered delivery. Service is synchronous; production must also charge
/// commands and results that have left the stage but remain in flight.
struct SourceOutcomeJournal {
    entries: VecDeque<JournalEntry>,
    capacity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalJournalState {
    Unoffered,
    PendingBend,
    Resolved,
}

impl SourceOutcomeJournal {
    fn new(stage_cells: usize) -> Self {
        let capacity = stage_cells + 1;
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    fn is_full(&self) -> bool {
        self.entries.len() == self.capacity
    }

    fn record(
        &mut self,
        source: SourceHandoff,
        raw: InputEventId,
        state: JournalState,
        retry: bool,
    ) {
        if retry {
            let last = self
                .entries
                .back_mut()
                .expect("a pending bend owns its journal place");
            assert_eq!((last.source, last.raw), (source, raw));
            assert_eq!(last.state, JournalState::AwaitStage);
            last.state = state;
            return;
        }
        assert!(
            !self.is_full(),
            "raw admission preflight reserved one journal place"
        );
        if let Some(last) = self.entries.back() {
            assert!(last.source.stamp().queue().serial() < source.stamp().queue().serial());
        }
        self.entries.push_back(JournalEntry { source, raw, state });
    }

    fn after_pump(&mut self, step: &MixedSourceStep, bridge: &RawStageBridge, retry: bool) {
        match *step {
            MixedSourceStep::Staged(source, occurrence) => {
                let handoff = bridge
                    .handoffs
                    .last()
                    .expect("staged head owns a raw handoff")
                    .2;
                let release = matches!(handoff.original.event(), Midi1Event::KeyRelease { .. });
                assert_eq!(
                    handoff.original,
                    match source.observation() {
                        InputObservation::Message { input, .. } => input,
                        InputObservation::Frontier { .. } =>
                            panic!("a staged message has an original"),
                    }
                );
                self.record(
                    source,
                    handoff.raw,
                    JournalState::Staged {
                        occurrence,
                        release,
                    },
                    retry,
                );
            }
            MixedSourceStep::AwaitStage(source, handoff) => {
                self.record(source, handoff.raw, JournalState::AwaitStage, retry);
            }
            MixedSourceStep::Split(source, BridgeRefused::Lane { handoff, reason }) => {
                self.record(
                    source,
                    handoff.raw,
                    JournalState::Ready(JournalOutcome::Split(BridgeRefused::Lane {
                        handoff,
                        reason,
                    })),
                    retry,
                );
            }
            MixedSourceStep::Split(_, BridgeRefused::Raw { .. }) => {
                panic!("raw refusal has no admitted journal charge");
            }
            MixedSourceStep::Frontier(_) | MixedSourceStep::AwaitRaw(_, _) => {}
        }
    }

    fn finish_note(&mut self, result: CombinedResult) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.raw == result.handoff.raw)
            .expect("note result owns an admitted journal entry");
        assert_eq!(
            entry.state,
            JournalState::Staged {
                occurrence: result.occurrence,
                release: matches!(
                    result.handoff.original.event(),
                    Midi1Event::KeyRelease { .. }
                ),
            }
        );
        entry.state = JournalState::Ready(JournalOutcome::Note(result));
    }

    fn finish_bend(&mut self, result: BendCombinedResult) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.raw == result.handoff.raw)
            .expect("bend result owns an admitted journal entry");
        assert_eq!(
            entry.state,
            JournalState::Staged {
                occurrence: result.occurrence,
                release: false,
            }
        );
        entry.state = JournalState::Ready(JournalOutcome::Bend(result));
    }

    fn deliver(&mut self) -> Option<JournalEntry> {
        if self
            .entries
            .front()
            .is_some_and(|entry| matches!(entry.state, JournalState::Ready(_)))
        {
            self.entries.pop_front()
        } else {
            None
        }
    }

    fn terminal_records(
        &self,
        teardown: &BridgeTeardown,
        pending: Option<PendingBendCancellation>,
    ) -> Vec<(SourceHandoff, InputEventId, TerminalJournalState)> {
        self.entries
            .iter()
            .map(|entry| {
                assert_eq!(
                    entry.source.stamp().queue().generation(),
                    entry.raw.generation()
                );
                assert_eq!(
                    teardown
                        .raw_issued
                        .iter()
                        .filter(|(_, raw, _)| *raw == entry.raw)
                        .count(),
                    1,
                    "each journal row names exactly one issued raw observation"
                );
                let state = match entry.state {
                    JournalState::Staged { .. } => {
                        assert_eq!(
                            teardown
                                .unoffered
                                .iter()
                                .filter(|handoff| handoff.raw == entry.raw)
                                .count(),
                            1
                        );
                        TerminalJournalState::Unoffered
                    }
                    JournalState::AwaitStage => {
                        assert_eq!(
                            pending.as_ref().map(|cancelled| cancelled.raw.raw),
                            Some(entry.raw)
                        );
                        TerminalJournalState::PendingBend
                    }
                    JournalState::Ready(JournalOutcome::Split(_)) => {
                        assert!(
                            teardown
                                .unoffered
                                .iter()
                                .all(|handoff| handoff.raw != entry.raw)
                        );
                        assert!(
                            teardown
                                .reservations
                                .iter()
                                .all(|(_, _, raw)| *raw != entry.raw)
                        );
                        TerminalJournalState::Resolved
                    }
                    JournalState::Ready(_) => TerminalJournalState::Resolved,
                };
                (entry.source, entry.raw, state)
            })
            .collect()
    }
}

#[derive(Debug, PartialEq)]
enum JournalPumpStep {
    Advanced(MixedSourceStep),
    AwaitJournal(SourceHandoff),
}

/// Journal capacity is checked before raw preflight and raw admission. A
/// pending bend already owns its charge, so its retry needs no second place.
fn pump_journaled(
    bridge: &mut RawStageBridge,
    inbox: &mut SourceInbox,
    pending: &mut Option<(SourceHandoff, RawHandoff)>,
    source: Source,
    journal: &mut SourceOutcomeJournal,
) -> Option<JournalPumpStep> {
    assert!(
        inbox.unresolved_failure().is_none(),
        "producer failure precedes journal capacity refusal"
    );
    let retry = pending.is_some();
    if !retry {
        let head = inbox.peek_handoff()?;
        if let InputObservation::Message { input, .. } = head.observation() {
            assert!(matches!(
                input.event(),
                Midi1Event::NoteOn { .. }
                    | Midi1Event::PitchBend { .. }
                    | Midi1Event::KeyRelease { .. }
            ));
            if journal.is_full() {
                return Some(JournalPumpStep::AwaitJournal(head));
            }
        }
    }
    let step = pump_concrete_mixed_for_source(bridge, inbox, pending, source)?;
    journal.after_pump(&step, bridge, retry);
    Some(JournalPumpStep::Advanced(step))
}

fn journal_fixture(source: Source) -> (RawStageBridge, SourceProducer, SourceInbox, Source) {
    let raw = [raw_source_with_cells(0, 32), raw_source_with_cells(1, 32)];
    let generation = raw[source.index()].generation;
    let clock = raw[source.index()].input.clock().unwrap();
    let (producer, inbox) =
        SourceInbox::prepare(generation, InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(2, raw);
    let peer = if source == Source::First {
        Source::Second
    } else {
        Source::First
    };
    bridge.frontier(peer, 99);
    (bridge, producer, inbox, peer)
}

fn send(producer: &mut SourceProducer, at: u64, input: Midi1Input) {
    let accepted = producer
        .submit_owned(InputObservation::Message {
            tick: InputTick::new(at),
            arrival: SampleTime::new(at),
            input,
        })
        .unwrap();
    let (_, queue) = accepted.parts();
    assert!(queue.serial() > 0);
}

/// Off-thread shutdown returns every source head that never reached raw
/// admission. These records own queue identity and original, but no raw ID.
fn cancel_unadmitted_source_heads(inbox: &mut SourceInbox) -> Vec<SourceHandoff> {
    let mut cancelled = Vec::new();
    while let Some(head) = inbox.peek_handoff() {
        assert_eq!(inbox.commit_handoff(head), Ok(head));
        cancelled.push(head);
    }
    cancelled
}

fn delivered_serial(entry: JournalEntry) -> u64 {
    assert!(matches!(entry.state, JournalState::Ready(_)));
    entry.source.stamp().queue().serial()
}

#[test]
fn terminal_bend_split_waits_for_older_onset_result_on_both_sources() {
    for source in [Source::First, Source::Second] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (mut bridge, mut producer, mut inbox, peer) = journal_fixture(source);
        let mut journal = SourceOutcomeJournal::new(2);
        let mut pending = None;
        send(&mut producer, 100, input(0x90, 60, 100));
        send(&mut producer, 101, input(0xe0, 127, 127));
        send(&mut producer, 102, input(0x80, 60, 0));

        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        assert!(journal.deliver().is_none());
        let before = control.last_ingress_id;
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Split(
                _,
                BridgeRefused::Lane {
                    reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
                    ..
                }
            )))
        ));
        assert_eq!(control.last_ingress_id, before);
        assert!(
            journal.deliver().is_none(),
            "the younger bend cannot overtake the onset"
        );
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        assert!(inbox.close(producer).is_ok());
        bridge.frontier(peer, 103);

        let onset = bridge.service(&mut control, &mut audio).unwrap();
        assert!(onset.accepted());
        journal.finish_note(onset);
        let first = journal.deliver().unwrap();
        let split = journal.deliver().unwrap();
        assert_eq!((delivered_serial(first), delivered_serial(split)), (1, 2));
        assert!(journal.deliver().is_none());
        let release = bridge.service(&mut control, &mut audio).unwrap();
        assert!(release.accepted());
        journal.finish_note(release);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 3);
        assert!(journal.entries.is_empty());
        let (teardown, raw, _) = bridge.tear_down(control, audio);
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn full_result_journal_waits_before_raw_admission_and_retries_the_same_head() {
    for source in [Source::First, Source::Second] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (mut bridge, mut producer, mut inbox, peer) = journal_fixture(source);
        let mut journal = SourceOutcomeJournal::new(2);
        let mut pending = None;
        send(&mut producer, 100, input(0x90, 60, 100));
        for at in 101..=103 {
            send(&mut producer, at, input(0xe0, 127, 127));
        }
        send(&mut producer, 104, input(0x80, 60, 0));
        for _ in 0..3 {
            assert!(matches!(
                pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
                Some(JournalPumpStep::Advanced(_))
            ));
        }
        assert_eq!(journal.entries.len(), 3);
        assert!(journal.deliver().is_none());
        let held = inbox.peek_handoff().unwrap();
        assert_eq!(held.stamp().queue().serial(), 4);
        let issued_before = bridge.raw_issued.len();
        let command_before = control.last_ingress_id;
        for _ in 0..2 {
            assert_eq!(
                pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
                Some(JournalPumpStep::AwaitJournal(held))
            );
            assert_eq!(bridge.raw_issued.len(), issued_before);
            assert_eq!(inbox.peek_handoff(), Some(held));
            assert_eq!(control.last_ingress_id, command_before);
        }

        bridge.frontier(peer, 101);
        let issued_after_peer = bridge.raw_issued.len();
        let onset = bridge.service(&mut control, &mut audio).unwrap();
        journal.finish_note(onset);
        let serials: Vec<_> = (0..3)
            .map(|_| delivered_serial(journal.deliver().unwrap()))
            .collect();
        assert_eq!(serials, vec![1, 2, 3]);
        let Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(retried, _))) =
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal)
        else {
            panic!("the held bend takes the freed stage cell");
        };
        assert_eq!(retried, held);
        assert_eq!(bridge.raw_issued.len(), issued_after_peer + 1);
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        assert!(inbox.close(producer).is_ok());
        bridge.frontier(peer, 105);
        let bend = bridge
            .service_bend(&mut control, &mut audio)
            .unwrap()
            .unwrap();
        assert!(matches!(bend.outcome, MixedIngressOutcome::Bend(Ok(_))));
        journal.finish_bend(bend);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 4);
        let release = bridge.service(&mut control, &mut audio).unwrap();
        journal.finish_note(release);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 5);
        assert!(journal.entries.is_empty());
        let (teardown, raw, _) = bridge.tear_down(control, audio);
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn full_result_journal_teardown_keeps_resolved_splits_and_unoffered_onset() {
    for source in [Source::First, Source::Second] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (control, audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (mut bridge, mut producer, mut inbox, _) = journal_fixture(source);
        let mut journal = SourceOutcomeJournal::new(2);
        let mut pending = None;
        send(&mut producer, 100, input(0x90, 60, 100));
        for at in 101..=103 {
            send(&mut producer, at, input(0xe0, 127, 127));
        }
        send(&mut producer, 104, input(0x80, 60, 0));
        for _ in 0..3 {
            assert!(matches!(
                pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
                Some(JournalPumpStep::Advanced(_))
            ));
        }
        let held = inbox.peek_handoff().unwrap();
        assert_eq!(held.stamp().queue().serial(), 4);
        assert_eq!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::AwaitJournal(held))
        );
        assert!(pending.is_none());
        let (teardown, raw, _) = bridge.tear_down(control, audio);
        let terminal = journal.terminal_records(&teardown, None);
        assert_eq!(
            terminal
                .iter()
                .map(|(head, _, state)| (head.stamp().queue().serial(), *state))
                .collect::<Vec<_>>(),
            vec![
                (1, TerminalJournalState::Unoffered),
                (2, TerminalJournalState::Resolved),
                (3, TerminalJournalState::Resolved),
            ]
        );
        assert_eq!(inbox.peek_handoff(), Some(held));
        let cancelled = cancel_unadmitted_source_heads(&mut inbox);
        assert_eq!(
            cancelled
                .iter()
                .map(|head| head.stamp().queue().serial())
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert!(inbox.close(producer).is_ok());
        assert_eq!(teardown.reservations.len(), 1);
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn pending_bend_retry_uses_its_existing_journal_and_raw_places() {
    for source in [Source::First, Source::Second] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (mut bridge, mut producer, mut inbox, peer) = journal_fixture(source);
        let mut journal = SourceOutcomeJournal::new(2);
        let mut pending = None;
        send(&mut producer, 100, input(0x90, 60, 100));
        send(&mut producer, 101, input(0xe0, 127, 127));
        bridge.frontier(peer, 102);
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        let Some(JournalPumpStep::Advanced(MixedSourceStep::AwaitStage(head, raw))) =
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal)
        else {
            panic!("the selected older head keeps the bend pending");
        };
        assert_eq!(journal.entries.len(), 2);
        assert_eq!(pending, Some((head, raw)));
        let issued = bridge.raw_issued.len();
        assert_eq!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::AwaitStage(
                head, raw
            )))
        );
        assert_eq!(bridge.raw_issued.len(), issued);
        assert_eq!(journal.entries.len(), 2);
        let onset = bridge.service(&mut control, &mut audio).unwrap();
        journal.finish_note(onset);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 1);
        assert!(journal.deliver().is_none());
        let Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(retried, _))) =
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal)
        else {
            panic!("the same pending bend takes the freed stage cell");
        };
        assert_eq!(retried, head);
        assert!(pending.is_none());
        assert_eq!(bridge.raw_issued.len(), issued);
        assert!(inbox.close(producer).is_ok());
        let bend = bridge
            .service_bend(&mut control, &mut audio)
            .unwrap()
            .unwrap();
        assert_eq!(bend.handoff.raw, raw.raw);
        journal.finish_bend(bend);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 2);
        assert!(journal.entries.is_empty());
        let (teardown, raw_owner, _) = bridge.tear_down(control, audio);
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw_owner, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn two_staged_bends_and_one_pending_bend_fill_the_per_source_bound() {
    for source in [Source::First, Source::Second] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (control, audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (mut bridge, mut producer, mut inbox, peer) = journal_fixture(source);
        let mut journal = SourceOutcomeJournal::new(2);
        let mut pending = None;
        for at in 100..=102 {
            send(&mut producer, at, input(0xe0, 127, 127));
        }
        bridge.frontier(peer, 200);
        for _ in 0..2 {
            assert!(matches!(
                pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
                Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
            ));
        }
        let Some(JournalPumpStep::Advanced(MixedSourceStep::AwaitStage(head, raw))) =
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal)
        else {
            panic!("the third bend waits in its charged source-owned slot");
        };
        assert_eq!(head.stamp().queue().serial(), 3);
        assert_eq!(journal.entries.len(), 3);
        assert!(journal.is_full());
        assert_eq!(pending, Some((head, raw)));
        assert!(inbox.close(producer).is_ok());
        let cancelled = cancel_pending_source_bend(&mut bridge, &mut pending).unwrap();
        assert_eq!((cancelled.source, cancelled.raw), (head, raw));
        let (teardown, raw_owner, _) = bridge.tear_down(control, audio);
        let terminal = journal.terminal_records(&teardown, Some(cancelled));
        assert_eq!(
            terminal
                .iter()
                .map(|(head, _, state)| (head.stamp().queue().serial(), *state))
                .collect::<Vec<_>>(),
            vec![
                (1, TerminalJournalState::Unoffered),
                (2, TerminalJournalState::Unoffered),
                (3, TerminalJournalState::PendingBend),
            ]
        );
        assert_eq!(teardown.unoffered.len(), 2);
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw_owner, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn producer_fault_precedes_full_journal_backpressure() {
    let source = Source::First;
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (control, audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (mut bridge, mut producer, mut inbox, _) = journal_fixture(source);
    let mut journal = SourceOutcomeJournal::new(2);
    let mut pending = None;
    send(&mut producer, 100, input(0x90, 60, 100));
    for at in 101..=103 {
        send(&mut producer, at, input(0xe0, 127, 127));
    }
    send(&mut producer, 104, input(0x80, 60, 0));
    for _ in 0..3 {
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(_))
        ));
    }
    assert!(journal.is_full());
    let invalid = InputObservation::Message {
        tick: InputTick::new(99),
        arrival: SampleTime::new(99),
        input: input(0xe0, 127, 127),
    };
    assert!(matches!(
        producer.submit_owned(invalid),
        Err(SourceOwnedSendError::Refused(_))
    ));
    assert_eq!(
        inbox.unresolved_failure(),
        Some((invalid, InputError::Order))
    );
    let held = inbox.peek_handoff();
    let issued = bridge.raw_issued.len();
    let fault = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal)
    }));
    assert!(
        fault.is_err(),
        "the terminal source fault must win over journal Full"
    );
    assert_eq!(inbox.peek_handoff(), held);
    assert_eq!(bridge.raw_issued.len(), issued);
    inbox.resolve_failure();
    let cancelled = cancel_unadmitted_source_heads(&mut inbox);
    assert_eq!(
        cancelled
            .iter()
            .map(|head| head.stamp().queue().serial())
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
    assert!(inbox.close(producer).is_ok());
    let (teardown, raw, _) = bridge.tear_down(control, audio);
    assert_eq!(journal.terminal_records(&teardown, None).len(), 3);
    let issued = teardown.raw_issued;
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}
