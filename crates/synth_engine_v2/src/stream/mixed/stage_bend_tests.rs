//! Private source-to-audio bend custody through raw admission and a bounded stage.

use super::*;
use crate::quantities::Cents;
use synth_core::MidiChannel;

#[path = "../../../tests/support/source_queue.rs"]
mod source_queue;
use source_queue::{SourceQueueReader, SourceQueueWriter};

use super::super::{
    MixedPedalSubmitted,
    occurrence::{MixedBendSubmitError, MixedLiveScope, MixedLiveSource},
};

/// The fixture owns both endpoints, but packets cross an actual bounded SPSC
/// source ring before raw admission. A refused head stays in that ring.
struct BendSourceRing {
    writer: SourceQueueWriter<Packet>,
    reader: SourceQueueReader<Packet>,
    last_sent: Option<SampleTime>,
}

impl BendSourceRing {
    fn new() -> Self {
        let (writer, reader) = SourceQueueWriter::prepare(32);
        Self {
            writer,
            reader,
            last_sent: None,
        }
    }

    fn send(&mut self, packet: Packet) -> Result<(), Packet> {
        assert!(self.last_sent.is_none_or(|last| last <= packet.at));
        self.writer.try_push(packet)?;
        self.last_sent = Some(packet.at);
        Ok(())
    }

    fn peek(&self) -> Option<Packet> {
        self.reader.peek()
    }

    fn commit(&mut self, expected: Packet) {
        assert_eq!(self.reader.commit_exact(expected), Ok(expected));
    }

    fn len(&self) -> usize {
        self.reader.occupied_len()
    }

    fn is_empty(&self) -> bool {
        self.reader.is_empty()
    }
}

struct BendBridge {
    raw: [RawSource; 2],
    source_heads: [BendSourceRing; 2],
    raw_pending: [Option<RawHandoff>; 2],
    staged: Vec<(Occurrence, RawHandoff)>,
    stage: Stage,
    next_occurrence: u64,
}

impl BendBridge {
    fn new(cells_per_source: usize) -> Self {
        Self {
            raw: [raw_source_with_cells(0, 128), raw_source_with_cells(1, 128)],
            source_heads: [BendSourceRing::new(), BendSourceRing::new()],
            raw_pending: [None, None],
            staged: Vec::new(),
            stage: Stage::new(cells_per_source),
            next_occurrence: 1,
        }
    }

    fn scope(source: Source) -> MixedLiveScope {
        MixedLiveScope::new(
            MixedLiveSource::new(u64::try_from(source.index() + 1).unwrap()),
            MidiChannel::new(1),
        )
    }

    fn enqueue(&mut self, source: Source, at: u64, original: Midi1Input) -> Occurrence {
        self.try_enqueue(source, at, original)
            .expect("source ring has a free cell")
    }

    fn try_enqueue(
        &mut self,
        source: Source,
        at: u64,
        original: Midi1Input,
    ) -> Result<Occurrence, Packet> {
        assert!(matches!(original.event(), Midi1Event::PitchBend { .. }));
        let heads = &mut self.source_heads[source.index()];
        let occurrence = Occurrence(self.next_occurrence);
        let packet = Packet {
            occurrence,
            original,
            at: SampleTime::new(at),
            release: false,
        };
        heads.send(packet)?;
        self.next_occurrence += 1;
        Ok(occurrence)
    }

    /// Raw acceptance is remembered even when the stage refuses its packet.
    fn pump(&mut self, source: Source) -> Result<Option<RawHandoff>, StageRefusal> {
        let index = source.index();
        let Some(packet) = self.source_heads[index].peek() else {
            return Ok(None);
        };
        if self.raw_pending[index].is_none() {
            let owner = &mut self.raw[index];
            let raw = owner
                .input
                .offer_message(
                    owner.generation,
                    InputTick::new(packet.at.as_u64()),
                    packet.at,
                    packet.original,
                )
                .expect("raw source has prepared room");
            self.raw_pending[index] = Some(RawHandoff {
                source,
                raw,
                link: None,
                original: packet.original,
                at: packet.at,
            });
        }
        self.stage.bend(source, packet)?;
        let handoff = self.raw_pending[index].take().unwrap();
        self.staged.push((packet.occurrence, handoff));
        self.source_heads[index].commit(packet);
        Ok(Some(handoff))
    }

    fn frontier(&mut self, source: Source, at: u64) {
        assert!(self.source_heads[source.index()].is_empty());
        let owner = &mut self.raw[source.index()];
        let frontier = owner
            .input
            .advance_frontier(owner.generation, InputTick::new(at))
            .unwrap();
        assert_eq!(owner.input.matched_onset(frontier), Ok(None));
        self.stage.frontier(source, SampleTime::new(at)).unwrap();
    }

    /// A refused command does not commit the stage head or its raw handoff.
    fn service(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
    ) -> Result<Option<(Occurrence, RawHandoff, MixedIngressOutcome)>, MixedBendSubmitError> {
        let Some((source, packet)) = self.stage.peek_next() else {
            return Ok(None);
        };
        let Midi1Event::PitchBend { value } = packet.original.event() else {
            panic!("bend stage contains only bends");
        };
        let cents = Cents::new(value.as_f32() * 200.0).unwrap();
        let (command, request) = control.submit_bend(
            MixedIngressOriginId(packet.occurrence.0),
            Self::scope(source),
            packet.at,
            cents,
        )?;
        self.stage.commit_next(source, packet);
        let position = self
            .staged
            .iter()
            .position(|(occurrence, _)| *occurrence == packet.occurrence)
            .unwrap();
        let (_, handoff) = self.staged.remove(position);
        audio.service_test_ingress_queue();
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        Ok(Some((packet.occurrence, handoff, result.outcome)))
    }
}

fn queued_bends(audio: &MixedOneShotAudio) -> Vec<(SampleTime, Cents)> {
    let mut queue = vec![None; audio.test_ingress.queue_capacity_for_test()];
    let len = audio.test_ingress.copy_queue_for_test(&mut queue).unwrap();
    queue
        .into_iter()
        .take(len)
        .flatten()
        .filter_map(|(event, _)| match event.payload() {
            EventPayload::Bend { cents, .. } => Some((event.envelope().time(), cents)),
            _ => None,
        })
        .collect()
}

#[test]
fn full_source_ring_retries_the_same_bend_after_exact_head_commit() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = BendBridge::new(2);
    let wheel = input(0xe0, 127, 127);
    for at in 100..132 {
        bridge.enqueue(Source::First, at, wheel);
    }
    assert_eq!(bridge.source_heads[0].len(), 32);
    let next = bridge.next_occurrence;
    let refused = bridge.try_enqueue(Source::First, 132, wheel).unwrap_err();
    assert_eq!(refused.occurrence, Occurrence(next));
    assert_eq!(bridge.next_occurrence, next);
    assert_eq!(
        bridge.source_heads[0].peek().unwrap().at,
        SampleTime::new(100)
    );

    let raw = bridge.pump(Source::First).unwrap().unwrap();
    assert_eq!(bridge.source_heads[0].len(), 31);
    assert_eq!(
        bridge.try_enqueue(Source::First, 132, wheel),
        Ok(Occurrence(next))
    );
    assert_eq!(bridge.source_heads[0].len(), 32);
    bridge.frontier(Source::Second, 200);
    let (occurrence, handed, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!(occurrence, Occurrence(1));
    assert_eq!(handed, raw);
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    assert_eq!(
        bridge.source_heads[0].peek().unwrap().at,
        SampleTime::new(101)
    );
    for at in 101..=132 {
        let admitted = bridge.pump(Source::First).unwrap().unwrap();
        let (occurrence, handed, outcome) =
            bridge.service(&mut control, &mut audio).unwrap().unwrap();
        assert_eq!(occurrence, Occurrence(at - 99));
        assert_eq!(handed, admitted);
        assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    assert!(bridge.source_heads[0].is_empty());
    assert_eq!(bridge.next_occurrence, 34);
}

#[test]
fn raw_and_stage_keep_exact_bend_heads_until_credits_return() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = BendBridge::new(2);
    let first_scope = BendBridge::scope(Source::First);
    for key in [60, 61] {
        let (_, _) = control
            .submit_onset(
                MixedIngressOriginId(u64::from(key)),
                first_scope,
                SampleTime::new(100),
                crate::quantities::KeyIdentity::new(key).unwrap(),
                crate::quantities::NoteVelocity::FULL,
            )
            .unwrap();
    }
    audio.service_test_ingress_queue();
    while control.collect_ingress_result().is_some() {}

    let wheel = input(0xe0, 127, 127);
    let budget = audio.test_ingress.mixed_bend_budget();
    assert!(budget >= 6 && budget % 2 == 0);
    for at in 110..110 + u64::try_from(budget / 2).unwrap() {
        bridge.enqueue(Source::First, at, wheel);
        bridge.pump(Source::First).unwrap();
        bridge.frontier(Source::Second, at + 1);
        assert!(matches!(
            bridge.service(&mut control, &mut audio),
            Ok(Some((_, _, MixedIngressOutcome::Bend(Ok(_)))))
        ));
    }
    assert_eq!(queued_bends(&audio).len(), budget);
    let blocked = bridge.enqueue(Source::First, 320, wheel);
    let later = bridge.enqueue(Source::First, 321, input(0xe0, 0, 0));
    let last = bridge.enqueue(Source::First, 322, wheel);
    let first_raw = bridge.pump(Source::First).unwrap().unwrap();
    let second_raw = bridge.pump(Source::First).unwrap().unwrap();
    assert_eq!(
        bridge.pump(Source::First),
        Err(StageRefusal::NoPacketCredit)
    );
    let third_raw = bridge.raw_pending[0].unwrap();
    assert_eq!(
        (
            third_raw.source,
            third_raw.link,
            third_raw.original,
            third_raw.at
        ),
        (Source::First, None, wheel, SampleTime::new(322))
    );
    bridge.frontier(Source::Second, 323);
    let id_before = control.last_ingress_id;
    assert!(matches!(
        bridge.service(&mut control, &mut audio),
        Err(MixedBendSubmitError::Credits { .. })
    ));
    assert_eq!(control.last_ingress_id, id_before);
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, blocked);
    assert_eq!(
        bridge.stage.lanes[0]
            .entries
            .back()
            .unwrap()
            .packet
            .occurrence,
        later
    );
    assert_eq!(bridge.source_heads[0].peek().unwrap().occurrence, last);
    assert_eq!(
        bridge.pump(Source::First),
        Err(StageRefusal::NoPacketCredit)
    );
    assert_eq!(bridge.raw_pending[0], Some(third_raw));

    drain(&mut audio);
    assert!(queued_bends(&audio).is_empty());
    let (_, retried_raw, result) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!(retried_raw, first_raw);
    let MixedIngressOutcome::Bend(Ok(retried)) = result else {
        panic!("retained bend was accepted");
    };
    assert_eq!(retried.executed_at(), SampleTime::new(320));
    assert_eq!(queued_bends(&audio).len(), 2);
    assert_eq!(queued_bends(&audio)[0].0, SampleTime::new(320));

    let (_, handed, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!(handed, second_raw);
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    assert_eq!(bridge.pump(Source::First).unwrap(), Some(third_raw));
    assert_eq!(bridge.source_heads[0].len(), 0);
    let (_, handed, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!(handed, third_raw);
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    assert_eq!(
        queued_bends(&audio)
            .iter()
            .map(|(at, cents)| (at.as_u64(), cents.as_f32()))
            .collect::<Vec<_>>(),
        vec![
            (320, 200.0),
            (320, 200.0),
            (321, -200.0),
            (321, -200.0),
            (322, 200.0),
            (322, 200.0),
        ]
    );
    drain(&mut audio);
    assert!(queued_bends(&audio).is_empty());
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    assert!(bridge.staged.is_empty());
}

#[test]
fn a_full_command_ring_keeps_the_selected_bend_ahead_of_its_peer() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let first_scope = BendBridge::scope(Source::First);
    let (onset_id, onset_request) = control
        .submit_onset(
            MixedIngressOriginId(60),
            first_scope,
            SampleTime::new(100),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    let onset_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (onset_result.id, onset_result.request),
        (onset_id, onset_request)
    );
    assert!(matches!(
        onset_result.outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));

    // Typed pedal-downs keep one ring place each for their lifts. The remaining
    // place takes a typed zero bend on an empty scope, then no ordinary room is
    // left for the staged source head.
    let peer_scope = BendBridge::scope(Source::Second);
    let mut pressed = 0;
    while control.ingress_room() > 1 {
        let scope = MixedLiveScope::new(
            MixedLiveSource::new(u64::try_from(1000 + pressed).unwrap()),
            MidiChannel::new(1),
        );
        assert!(matches!(
            control
                .submit_pedal(
                    MixedIngressOriginId(u64::try_from(1000 + pressed).unwrap()),
                    scope,
                    SampleTime::new(110),
                    true,
                )
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        pressed += 1;
    }
    assert_eq!(control.ingress_room(), 1);
    let (filler_id, _) = control
        .submit_bend(
            MixedIngressOriginId(2000),
            peer_scope,
            SampleTime::new(110),
            Cents::ZERO,
        )
        .unwrap();
    assert_eq!(control.last_ingress_id, Some(filler_id));
    assert_eq!(2 * pressed + 2, control.ingress_capacity.as_usize());
    assert_eq!(control.ingress_room(), 0);

    let mut bridge = BendBridge::new(2);
    let first = bridge.enqueue(Source::First, 320, input(0xe0, 127, 127));
    let peer = bridge.enqueue(Source::Second, 321, input(0xe0, 0, 0));
    let first_raw = bridge.pump(Source::First).unwrap().unwrap();
    let peer_raw = bridge.pump(Source::Second).unwrap().unwrap();
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, first);
    let before = control.last_ingress_id;
    let credits_before = control.bend_credits_free;
    let scopes_before = control.bend_scopes.clone();
    assert_eq!(
        bridge.service(&mut control, &mut audio),
        Err(MixedBendSubmitError::Full)
    );
    assert_eq!(control.last_ingress_id, before);
    assert_eq!(control.bend_credits_free, credits_before);
    assert_eq!(control.bend_scopes, scopes_before);
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, first);
    assert_eq!(bridge.staged, vec![(first, first_raw), (peer, peer_raw)]);

    audio.service_test_ingress_queue();
    for _ in 0..pressed {
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::Bend(Ok(_))
    ));
    assert!(control.collect_ingress_result().is_none());
    let (accepted_first, returned_first, first_outcome) =
        bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((accepted_first, returned_first), (first, first_raw));
    let MixedIngressOutcome::Bend(Ok(first_bent)) = first_outcome else {
        panic!("retained first bend must apply");
    };
    assert_eq!(first_bent.executed_at(), SampleTime::new(320));
    assert_eq!(first_bent.written().count(), 1);

    assert_eq!(bridge.service(&mut control, &mut audio), Ok(None));
    assert_eq!(bridge.stage.peek_next(), None);
    assert_eq!(bridge.staged, vec![(peer, peer_raw)]);
    bridge.frontier(Source::First, 322);
    let (accepted_peer, returned_peer, peer_outcome) =
        bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((accepted_peer, returned_peer), (peer, peer_raw));
    let MixedIngressOutcome::Bend(Ok(peer_bent)) = peer_outcome else {
        panic!("retained peer bend must apply");
    };
    assert_eq!(peer_bent.executed_at(), SampleTime::new(321));
    assert_eq!(peer_bent.written().count(), 0);
    assert_eq!(
        queued_bends(&audio),
        vec![(SampleTime::new(320), Cents::new(200.0).unwrap())]
    );
    drain(&mut audio);
    drain(&mut audio);
    assert!(queued_bends(&audio).is_empty());
    assert!(bridge.staged.is_empty());
}

#[test]
fn bent_onset_keeps_its_raw_and_stage_custody_at_the_minimum_bend_budget() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(5)),
        )
        .unwrap();
    assert_eq!(audio.test_ingress.mixed_bend_budget(), 2);
    let scope = BendBridge::scope(Source::First);
    let (old_command, old_request) = control
        .submit_onset(
            MixedIngressOriginId(60),
            scope,
            SampleTime::new(100),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    let old_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (old_result.id, old_result.request),
        (old_command, old_request)
    );
    let MixedIngressOutcome::Onset(Ok(old_identity)) = old_result.outcome else {
        panic!("old onset accepted");
    };
    drain(&mut audio);

    let (bend_command, bend_request) = control
        .submit_bend(
            MixedIngressOriginId(70),
            scope,
            SampleTime::new(300),
            Cents::new(30.0).unwrap(),
        )
        .unwrap();
    assert_eq!(control.bend_credits_free.as_usize(), 0);
    let mut bridge = RawStageBridge::new(2);
    let offered = bridge
        .offer(Source::First, 300, input(0x90, 61, 100))
        .unwrap();
    bridge.frontier(Source::Second, 301);
    let original_handoff = bridge.handoffs[0].2;
    let old_id = control.last_ingress_id;
    let old_handles = control.occurrence_ledger.held();
    assert_eq!(
        bridge.try_service_onset_in_scope(&mut control, &mut audio, scope),
        Err(MixedOnsetSubmitError::BendCredit)
    );
    assert_eq!(control.last_ingress_id, old_id);
    assert_eq!(control.occurrence_ledger.held(), old_handles);
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, offered);
    assert_eq!(bridge.handoffs, vec![(offered, false, original_handoff)]);

    audio.service_test_ingress_queue();
    let bend_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (bend_result.id, bend_result.request),
        (bend_command, bend_request)
    );
    let MixedIngressOutcome::Bend(Ok(bent)) = bend_result.outcome else {
        panic!("the earlier bend must apply");
    };
    assert_eq!(bent.written().count(), 1);
    assert_eq!(control.bend_credit_return.pending().as_usize(), 1);
    let accepted = bridge
        .try_service_onset_in_scope(&mut control, &mut audio, scope)
        .unwrap()
        .unwrap();
    assert_eq!(accepted.handoff, original_handoff);
    assert_eq!(accepted.occurrence, offered);
    let MixedDisposition::Result(MixedIngressOutcome::Onset(Ok(new_identity))) =
        accepted.disposition
    else {
        panic!("retained onset accepted after unused credit returned");
    };
    assert_ne!(old_identity, new_identity);
    assert_eq!(bridge.stage.pressure(Source::First), (0, 1));
    let mut queue = vec![None; audio.test_ingress.queue_capacity_for_test()];
    let len = audio.test_ingress.copy_queue_for_test(&mut queue).unwrap();
    let live: Vec<_> = queue
        .into_iter()
        .take(len)
        .flatten()
        .map(|(event, _)| event)
        .collect();
    assert_eq!(live.len(), 3);
    assert!(
        matches!(live[0].payload(), EventPayload::Bend { identity, cents } if identity == old_identity && cents == Cents::new(30.0).unwrap())
    );
    assert!(
        matches!(live[1].payload(), EventPayload::Note { identity, edge: NoteEdge::On { .. } } if identity == new_identity)
    );
    assert!(
        matches!(live[2].payload(), EventPayload::Bend { identity, cents } if identity == new_identity && cents == Cents::new(30.0).unwrap())
    );
    assert!(
        live.iter()
            .all(|event| event.envelope().time() == SampleTime::new(300))
    );
}
