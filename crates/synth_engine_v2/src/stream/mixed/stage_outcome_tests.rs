//! Bounded source-order reporting across stage-full sound splits.

use std::collections::VecDeque;

use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};

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
    SubmittedBend(SubmittedBend),
    TerminalPendingBend(TerminalBendSubmission),
    Ready(JournalOutcome),
}

#[derive(Debug, PartialEq)]
struct JournalEntry {
    source: SourceHandoff,
    raw: InputEventId,
    state: JournalState,
}

/// This private note/bend/key-up proof keeps one charge from raw admission
/// until ordered delivery. Notes are serviced synchronously; a submitted bend
/// keeps its charge through the physical command and result queues.
struct SourceOutcomeJournal {
    entries: VecDeque<JournalEntry>,
    capacity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalJournalState {
    Unoffered,
    PendingBend,
    PendingCommand,
    UnservedCommand,
    UncollectedResult,
    PendingResult,
    InvalidCommand,
    InvalidResult,
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

    fn submit_bend(&mut self, submitted: SubmittedBend) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.raw == submitted.handoff.raw)
            .expect("submitted bend owns an admitted journal entry");
        assert_eq!(
            entry.state,
            JournalState::Staged {
                occurrence: submitted.occurrence,
                release: false,
            }
        );
        entry.state = JournalState::SubmittedBend(submitted);
    }

    fn terminalize_failed_bend_push(&mut self, terminal: TerminalBendSubmission) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.raw == terminal.handoff.raw)
            .expect("failed bend push owns an admitted journal entry");
        assert_eq!(
            entry.state,
            JournalState::Staged {
                occurrence: terminal.occurrence,
                release: false,
            }
        );
        entry.state = JournalState::TerminalPendingBend(terminal);
    }

    fn finish_submitted_bend(&mut self, result: super::super::super::MixedIngressResult) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| {
                matches!(entry.state, JournalState::SubmittedBend(submitted) if submitted.command == result.id)
            })
            .expect("collected result names one submitted bend");
        let JournalState::SubmittedBend(submitted) = entry.state else {
            panic!("the matched journal entry stays submitted");
        };
        assert_eq!(result.request, submitted.request);
        assert!(matches!(result.outcome, MixedIngressOutcome::Bend(_)));
        entry.state = JournalState::Ready(JournalOutcome::Bend(BendCombinedResult {
            handoff: submitted.handoff,
            occurrence: submitted.occurrence,
            outcome: result.outcome,
        }));
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
        ended: Option<&MixedOneShotTeardown>,
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
                    JournalState::TerminalPendingBend(terminal) => {
                        assert_eq!(terminal.handoff.raw, entry.raw);
                        assert_eq!(
                            teardown
                                .terminal_bends
                                .iter()
                                .filter(|held| **held == terminal)
                                .count(),
                            1
                        );
                        let ended = ended.expect("a failed bend push needs mixed terminal custody");
                        assert_eq!(
                            ended
                                .ingress
                                .pending_command
                                .as_ref()
                                .map(|packet| packet.id),
                            Some(terminal.command)
                        );
                        assert_eq!(
                            ended
                                .ingress
                                .terminal_control
                                .as_ref()
                                .expect("terminal control retains the command journal")
                                .outstanding_commands
                                .iter()
                                .filter(|command| command.id == terminal.command)
                                .count(),
                            1
                        );
                        TerminalJournalState::PendingCommand
                    }
                    JournalState::SubmittedBend(submitted) => {
                        assert_eq!(submitted.handoff.raw, entry.raw);
                        assert!(
                            teardown
                                .unoffered
                                .iter()
                                .all(|handoff| handoff.raw != entry.raw)
                        );
                        let ended = ended.expect("a submitted bend needs mixed terminal custody");
                        let unserved = ended
                            .ingress
                            .unserved_commands
                            .iter()
                            .filter(|packet| packet.id == submitted.command)
                            .count();
                        let uncollected = ended
                            .ingress
                            .uncollected_results
                            .iter()
                            .filter(|result| result.id == submitted.command)
                            .count();
                        let pending_result = usize::from(
                            ended
                                .ingress
                                .pending_result
                                .as_ref()
                                .is_some_and(|result| result.id == submitted.command),
                        );
                        let invalid_command = usize::from(
                            ended
                                .ingress
                                .invalid_command
                                .as_ref()
                                .is_some_and(|packet| packet.id == submitted.command),
                        );
                        let invalid_result = usize::from(
                            ended
                                .ingress
                                .invalid_result
                                .as_ref()
                                .is_some_and(|result| result.id == submitted.command),
                        );
                        assert_eq!(
                            unserved
                                + uncollected
                                + pending_result
                                + invalid_command
                                + invalid_result,
                            1
                        );
                        assert_eq!(
                            ended
                                .ingress
                                .terminal_control
                                .as_ref()
                                .expect("terminal control retains the outstanding journal")
                                .outstanding_commands
                                .iter()
                                .filter(|command| command.id == submitted.command)
                                .count(),
                            1
                        );
                        if unserved == 1 {
                            TerminalJournalState::UnservedCommand
                        } else if uncollected == 1 {
                            TerminalJournalState::UncollectedResult
                        } else if pending_result == 1 {
                            TerminalJournalState::PendingResult
                        } else if invalid_command == 1 {
                            TerminalJournalState::InvalidCommand
                        } else {
                            TerminalJournalState::InvalidResult
                        }
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
        let terminal = journal.terminal_records(&teardown, None, None);
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
        let terminal = journal.terminal_records(&teardown, Some(cancelled), None);
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
    assert_eq!(journal.terminal_records(&teardown, None, None).len(), 3);
    let issued = teardown.raw_issued;
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn physical_bend_push_moves_raw_handoff_out_of_stage_into_terminal_command_custody() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (mut bridge, producer, mut inbox, peer) = journal_fixture(Source::First);
    let mut journal = SourceOutcomeJournal::new(2);
    let mut pending = None;
    let original = input(0xe0, 127, 127);
    let mut producer = producer;
    send(&mut producer, 100, original);
    let step = pump_journaled(
        &mut bridge,
        &mut inbox,
        &mut pending,
        Source::First,
        &mut journal,
    );
    let Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, occurrence))) = step else {
        panic!("the bend enters raw and stage with a journal place");
    };
    bridge.frontier(peer, 101);
    assert_eq!(bridge.stage.pressure(Source::First), (1, 0));
    let raw = bridge.handoffs[0].2;
    while control
        .ingress_commands
        .0
        .try_push(super::super::super::MixedIngressCommand {
            owner: control.ingress_owner,
            id: super::super::super::MixedIngressCommandId(u64::MAX),
            request: MixedIngressRequest::Panic {
                origin: MixedIngressOriginId(900),
                at: SampleTime::new(90),
            },
            bend_credits: super::super::super::MixedIngressQueueCount::NONE,
        })
        .is_ok()
    {}
    assert!(control.ingress_room() > 0);
    assert_eq!(
        bridge.service_bend(&mut control, &mut audio),
        Err(MixedBendSubmitError::PhysicalPush)
    );
    let minted = control.ingress_command_pending.as_ref().unwrap().id;
    journal.terminalize_failed_bend_push(bridge.terminal_bends[0]);
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    assert!(bridge.handoffs.is_empty());
    assert_eq!(
        bridge.terminal_bends,
        vec![TerminalBendSubmission {
            occurrence,
            handoff: raw,
            command: minted,
        }]
    );
    assert_eq!(control.last_ingress_id, Some(minted));
    let mut output = [1.0_f32; 64];
    assert_eq!(
        audio.render_private(AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap()),
        Err(MixedOneShotRenderError::IngressTransferHalt)
    );
    assert!(output.iter().all(|sample| *sample == 0.0));
    let (teardown, raw_owners, ended) = bridge.tear_down(control, audio);
    assert_eq!(
        journal
            .terminal_records(&teardown, None, Some(&ended))
            .iter()
            .map(|(_, _, state)| *state)
            .collect::<Vec<_>>(),
        vec![TerminalJournalState::PendingCommand]
    );
    assert_eq!(teardown.terminal_bends.len(), 1);
    assert!(teardown.unoffered.is_empty());
    assert!(teardown.reservations.is_empty());
    assert_eq!(teardown.terminal_bends[0].handoff, raw);
    assert_eq!(
        ended
            .ingress
            .pending_command
            .as_ref()
            .map(|packet| packet.id),
        Some(minted)
    );
    assert_eq!(
        ended
            .ingress
            .unserved_commands
            .iter()
            .filter(|packet| packet.id == minted)
            .count(),
        0
    );
    let issued = teardown.raw_issued;
    assert!(inbox.close(producer).is_ok());
    assert_eq!(issued.iter().filter(|(_, id, _)| *id == raw.raw).count(), 1);
    assert_eq!(
        settle_raw_by_cancellation(raw_owners, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn in_flight_bend_keeps_its_journal_place_through_command_and_result_queues() {
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
        send(&mut producer, 100, input(0xe0, 0, 64));
        send(&mut producer, 101, input(0x90, 60, 100));
        send(&mut producer, 102, input(0xe0, 127, 127));
        send(&mut producer, 103, input(0x80, 60, 0));

        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        bridge.frontier(peer, 101);
        let submitted = bridge.submit_selected_bend(&mut control).unwrap().unwrap();
        journal.submit_bend(submitted);
        assert_eq!(bridge.stage.pressure(source), (0, 0));
        assert_eq!(
            control.ingress_outstanding,
            super::super::super::MixedIngressQueueCount(1)
        );
        assert!(journal.deliver().is_none());
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
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
        assert!(journal.is_full());
        let held = inbox.peek_handoff().unwrap();
        assert_eq!(held.stamp().queue().serial(), 4);
        let issued = bridge.raw_issued.len();
        for _ in 0..2 {
            assert_eq!(
                pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
                Some(JournalPumpStep::AwaitJournal(held))
            );
            assert_eq!(bridge.raw_issued.len(), issued);
            assert_eq!(control.last_ingress_id, Some(submitted.command));
        }
        audio.service_test_ingress_queue();
        assert_eq!(
            control.ingress_results.0.try_peek().map(|result| result.id),
            Some(submitted.command)
        );
        assert!(journal.deliver().is_none());
        assert_eq!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::AwaitJournal(held))
        );
        assert_eq!(bridge.raw_issued.len(), issued);
        let result = control.collect_ingress_result_checked().unwrap().unwrap();
        journal.finish_submitted_bend(result);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 1);
        assert!(journal.deliver().is_none());
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(head, _))) if head == held
        ));
        assert!(inbox.close(producer).is_ok());
        bridge.frontier(peer, 104);
        let onset = bridge.service(&mut control, &mut audio).unwrap();
        assert!(onset.accepted());
        journal.finish_note(onset);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 2);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 3);
        let release = bridge.service(&mut control, &mut audio).unwrap();
        assert!(release.accepted());
        journal.finish_note(release);
        assert_eq!(delivered_serial(journal.deliver().unwrap()), 4);
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
fn terminal_in_flight_bend_names_the_exact_command_or_result_queue_custody() {
    for result_queued in [false, true] {
        let source = Source::First;
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
        send(&mut producer, 100, input(0xe0, 0, 64));
        send(&mut producer, 101, input(0x90, 60, 100));
        send(&mut producer, 102, input(0xe0, 127, 127));
        send(&mut producer, 103, input(0x80, 60, 0));
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        bridge.frontier(peer, 101);
        let submitted = bridge.submit_selected_bend(&mut control).unwrap().unwrap();
        journal.submit_bend(submitted);
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        assert!(matches!(
            pump_journaled(&mut bridge, &mut inbox, &mut pending, source, &mut journal),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Split(_, _)))
        ));
        assert!(journal.is_full());
        let held = inbox.peek_handoff().unwrap();
        assert_eq!(held.stamp().queue().serial(), 4);
        if result_queued {
            audio.service_test_ingress_queue();
            assert_eq!(
                control.ingress_results.0.try_peek().map(|result| result.id),
                Some(submitted.command)
            );
        } else {
            assert!(audio.ingress_commands.0.try_peek().is_some());
        }
        control
            .ingress_halt
            .store(true, std::sync::atomic::Ordering::Release);
        let cancelled = cancel_unadmitted_source_heads(&mut inbox);
        assert_eq!(cancelled, vec![held]);
        assert!(inbox.close(producer).is_ok());
        let (teardown, raw, ended) = bridge.tear_down(control, audio);
        let terminal = journal.terminal_records(&teardown, None, Some(&ended));
        assert_eq!(
            terminal
                .iter()
                .map(|(head, _, state)| (head.stamp().queue().serial(), *state))
                .collect::<Vec<_>>(),
            vec![
                (
                    1,
                    if result_queued {
                        TerminalJournalState::UncollectedResult
                    } else {
                        TerminalJournalState::UnservedCommand
                    }
                ),
                (2, TerminalJournalState::Unoffered),
                (3, TerminalJournalState::Resolved),
            ]
        );
        assert_eq!(teardown.unoffered.len(), 1);
        assert_eq!(teardown.reservations.len(), 1);
        assert!(
            teardown
                .unoffered
                .iter()
                .all(|handoff| handoff.raw != submitted.handoff.raw)
        );
        assert!(
            teardown
                .reservations
                .iter()
                .all(|(_, _, raw)| *raw != submitted.handoff.raw)
        );
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn terminal_bend_faults_name_pending_result_and_invalid_packets() {
    for fault in [
        TerminalJournalState::PendingResult,
        TerminalJournalState::InvalidCommand,
        TerminalJournalState::InvalidResult,
    ] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        if fault == TerminalJournalState::PendingResult {
            let (writer, reader) =
                HeapRb::<super::super::super::MixedIngressResult>::new(1).split();
            control.ingress_results = super::super::super::OpaqueQueue(reader);
            audio.ingress_results = super::super::super::OpaqueQueue(writer);
            let earlier = control
                .submit_bend(
                    MixedIngressOriginId(900),
                    source_scope(Source::First, input(0xe0, 0, 64)),
                    SampleTime::new(90),
                    crate::quantities::Cents::new(0.0).unwrap(),
                )
                .unwrap();
            audio.service_test_ingress_queue();
            assert_eq!(
                control
                    .ingress_results
                    .0
                    .try_peek()
                    .map(|result| (result.id, result.request)),
                Some(earlier)
            );
        }
        let (mut bridge, mut producer, mut inbox, peer) = journal_fixture(Source::First);
        let mut journal = SourceOutcomeJournal::new(2);
        let mut pending = None;
        send(&mut producer, 100, input(0xe0, 127, 127));
        assert!(matches!(
            pump_journaled(
                &mut bridge,
                &mut inbox,
                &mut pending,
                Source::First,
                &mut journal
            ),
            Some(JournalPumpStep::Advanced(MixedSourceStep::Staged(_, _)))
        ));
        bridge.frontier(peer, 101);
        let submitted = bridge.submit_selected_bend(&mut control).unwrap().unwrap();
        journal.submit_bend(submitted);
        if fault == TerminalJournalState::InvalidCommand {
            audio.last_served_ingress_id = Some(submitted.command);
        }
        audio.service_test_ingress_queue();
        match fault {
            TerminalJournalState::PendingResult => {
                assert_eq!(
                    audio
                        .ingress_result_pending
                        .as_ref()
                        .map(|result| result.id),
                    Some(submitted.command)
                );
            }
            TerminalJournalState::InvalidCommand => {
                assert_eq!(
                    audio
                        .invalid_ingress_command
                        .as_ref()
                        .map(|packet| packet.id),
                    Some(submitted.command)
                );
            }
            TerminalJournalState::InvalidResult => {
                let actual = control.ingress_results.0.try_pop().unwrap();
                assert_eq!(actual.id, submitted.command);
                let forged = super::super::super::MixedIngressResult {
                    request: MixedIngressRequest::Panic {
                        origin: MixedIngressOriginId(901),
                        at: SampleTime::new(101),
                    },
                    ..actual
                };
                assert!(audio.ingress_results.0.try_push(forged).is_ok());
                assert!(control.collect_ingress_result_checked().is_err());
                assert_eq!(
                    control
                        .invalid_ingress_result
                        .as_ref()
                        .map(|result| result.id),
                    Some(submitted.command)
                );
            }
            _ => unreachable!(),
        }
        assert!(inbox.close(producer).is_ok());
        let (teardown, raw, ended) = bridge.tear_down(control, audio);
        assert_eq!(
            journal
                .terminal_records(&teardown, None, Some(&ended))
                .iter()
                .map(|(_, _, state)| *state)
                .collect::<Vec<_>>(),
            vec![fault]
        );
        let issued = teardown.raw_issued;
        assert_eq!(
            settle_raw_by_cancellation(raw, &issued).bridge.len(),
            issued.len()
        );
    }
}
