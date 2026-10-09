//! Private source-to-audio bend custody through raw admission and a bounded stage.

use super::*;
use crate::quantities::Cents;
use synth_core::MidiChannel;

#[allow(
    dead_code,
    reason = "shared source custody is exercised here through its owned path"
)]
#[path = "../../../tests/support/source_custody.rs"]
mod source_custody;
use source_custody::{
    SOURCE_CELLS, SourceHandoff, SourceHeadError, SourceInbox, SourceOwnedSendError,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BendPumpRefusal {
    AwaitRaw(InputError),
    RawPreflight(InputError),
    RawOffer(InputError),
    Stage(StageRefusal),
}

struct BendBridge<R = [RawSource; 2]> {
    raw: R,
    source_heads: [BendSourceRing; 2],
    raw_pending: [Option<RawHandoff>; 2],
    raw_issued: Vec<(Source, InputEventId)>,
    staged: Vec<(Occurrence, RawHandoff)>,
    stage: Stage,
    next_occurrence: u64,
}

impl BendBridge {
    fn new(cells_per_source: usize) -> Self {
        Self::with_raw_cells(cells_per_source, 128)
    }

    fn with_raw_cells(cells_per_source: usize, raw_cells: u32) -> Self {
        Self::with_raw(
            cells_per_source,
            [
                raw_source_with_cells(0, raw_cells),
                raw_source_with_cells(1, raw_cells),
            ],
        )
    }

    fn scope(source: Source) -> MixedLiveScope {
        source_scope(source, input(0xe0, 0, 64))
    }
}

impl<R: RawBackend> BendBridge<R> {
    fn with_raw(cells_per_source: usize, raw: R) -> Self {
        Self {
            raw,
            source_heads: [BendSourceRing::new(), BendSourceRing::new()],
            raw_pending: [None, None],
            raw_issued: Vec::new(),
            staged: Vec::new(),
            stage: Stage::new(cells_per_source),
            next_occurrence: 1,
        }
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
    fn pump(&mut self, source: Source) -> Result<Option<RawHandoff>, BendPumpRefusal> {
        let index = source.index();
        let Some(packet) = self.source_heads[index].peek() else {
            return Ok(None);
        };
        if self.raw_pending[index].is_none() {
            match self.raw.preflight(source, packet.at, packet.original) {
                Ok(InputCustodyPreflight::Admit) => {}
                Ok(InputCustodyPreflight::Await(error)) => {
                    return Err(BendPumpRefusal::AwaitRaw(error));
                }
                Err(error) => return Err(BendPumpRefusal::RawPreflight(error)),
            }
            // This bridge has exclusive mutable custody of the raw owner across
            // preflight and offer. No other source can spend this observed room.
            let raw = self
                .raw
                .offer(source, packet.at, packet.original)
                .map_err(BendPumpRefusal::RawOffer)?;
            self.raw_issued.push((source, raw));
            self.raw_pending[index] = Some(RawHandoff {
                source,
                raw,
                link: None,
                original: packet.original,
                at: packet.at,
            });
        }
        let pending = self.raw_pending[index].expect("raw handoff remains in bridge custody");
        assert_eq!(
            (pending.source, pending.original, pending.at),
            (source, packet.original, packet.at)
        );
        self.stage
            .bend(source, packet)
            .map_err(BendPumpRefusal::Stage)?;
        let handoff = self.raw_pending[index].take().unwrap();
        self.staged.push((packet.occurrence, handoff));
        self.source_heads[index].commit(packet);
        Ok(Some(handoff))
    }

    fn frontier(&mut self, source: Source, at: u64) {
        assert!(self.source_heads[source.index()].is_empty());
        assert!(self.raw_pending[source.index()].is_none());
        let frontier = self.raw.frontier(source, SampleTime::new(at));
        assert_eq!(self.raw.matched_onset(source, frontier), Ok(None));
        self.stage.frontier(source, SampleTime::new(at)).unwrap();
    }

    /// A refused command submission does not commit the stage head or its raw handoff.
    fn submit_next(
        &mut self,
        control: &mut MixedOneShotControl,
    ) -> Result<
        Option<(
            Occurrence,
            RawHandoff,
            super::super::MixedIngressCommandId,
            MixedIngressRequest,
        )>,
        MixedBendSubmitError,
    > {
        let Some((source, packet)) = self.stage.peek_next() else {
            return Ok(None);
        };
        let Midi1Event::PitchBend { value } = packet.original.event() else {
            panic!("bend stage contains only bends");
        };
        let cents = Cents::new(value.as_f32() * 200.0).unwrap();
        let (command, request) = control.submit_bend(
            MixedIngressOriginId(packet.occurrence.0),
            source_scope(source, packet.original),
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
        Ok(Some((packet.occurrence, handoff, command, request)))
    }

    fn service(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
    ) -> Result<Option<(Occurrence, RawHandoff, MixedIngressOutcome)>, MixedBendSubmitError> {
        let Some((occurrence, handoff, command, request)) = self.submit_next(control)? else {
            return Ok(None);
        };
        audio.service_test_ingress_queue();
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        Ok(Some((occurrence, handoff, result.outcome)))
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
fn a_late_bend_returns_a_terminal_result_and_releases_its_stage_place() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let scope = BendBridge::scope(Source::First);
    let _onset = control
        .submit_onset(
            MixedIngressOriginId(1),
            scope,
            SampleTime::new(200),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));

    let mut bridge = BendBridge::new(2);
    let early = bridge.enqueue(Source::First, 100, input(0xe0, 127, 127));
    let later = bridge.enqueue(Source::First, 201, input(0xe0, 0, 0));
    let early_raw = bridge.pump(Source::First).unwrap().unwrap();
    let later_raw = bridge.pump(Source::First).unwrap().unwrap();
    bridge.frontier(Source::First, 202);
    bridge.frontier(Source::Second, 202);

    let (seen, handoff, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((seen, handoff), (early, early_raw));
    assert!(matches!(
        outcome,
        MixedIngressOutcome::Bend(Err(super::super::occurrence::MixedBendRefused::Ingress(
            IngressRefused::NonMonotoneStamp { time, last }
        ))) if time == SampleTime::new(100) && last == SampleTime::new(200)
    ));
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, later);
    assert_eq!(bridge.staged, vec![(later, later_raw)]);

    let (seen, handoff, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((seen, handoff), (later, later_raw));
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    assert!(bridge.stage.peek_next().is_none());
    assert!(bridge.staged.is_empty());
}

#[test]
fn same_source_bends_keep_channel_and_command_order() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let first_note_scope = source_scope(Source::First, input(0x90, 60, 100));
    let second_note_scope = source_scope(Source::First, input(0x91, 61, 100));
    let submitted: Vec<_> = [
        (1, first_note_scope, 90, 60),
        (2, second_note_scope, 91, 61),
    ]
    .into_iter()
    .map(|(origin, scope, at, key)| {
        control
            .submit_onset(
                MixedIngressOriginId(origin),
                scope,
                SampleTime::new(at),
                crate::quantities::KeyIdentity::new(key).unwrap(),
                crate::quantities::NoteVelocity::FULL,
            )
            .unwrap()
    })
    .collect();
    audio.service_test_ingress_queue();
    let identities: Vec<_> = submitted
        .into_iter()
        .map(|(command, request)| {
            let result = control.collect_ingress_result().unwrap();
            assert_eq!((result.id, result.request), (command, request));
            let MixedIngressOutcome::Onset(Ok(identity)) = result.outcome else {
                panic!("both channel notes must sound");
            };
            identity
        })
        .collect();
    assert_ne!(identities[0], identities[1]);
    let mut bridge = BendBridge::new(2);
    let first_input = input(0xe1, 127, 127);
    let second_input = input(0xe0, 0, 0);
    let first = bridge.enqueue(Source::First, 100, first_input);
    let second = bridge.enqueue(Source::First, 101, second_input);
    let first_raw = bridge.pump(Source::First).unwrap().unwrap();
    let second_raw = bridge.pump(Source::First).unwrap().unwrap();
    bridge.frontier(Source::Second, 102);

    let (first_seen, first_handoff, first_command, first_request) =
        bridge.submit_next(&mut control).unwrap().unwrap();
    let (second_seen, second_handoff, second_command, second_request) =
        bridge.submit_next(&mut control).unwrap().unwrap();
    assert_eq!((first_seen, first_handoff), (first, first_raw));
    assert_eq!((second_seen, second_handoff), (second, second_raw));
    assert_ne!(first_command, second_command);
    let first_scope = source_scope(Source::First, first_input);
    let second_scope = source_scope(Source::First, second_input);
    assert_ne!(first_scope, second_scope);
    assert!(matches!(
        first_request,
        MixedIngressRequest::Bend { scope, at, .. }
            if scope == first_scope && at == SampleTime::new(100)
    ));
    assert!(matches!(
        second_request,
        MixedIngressRequest::Bend { scope, at, .. }
            if scope == second_scope && at == SampleTime::new(101)
    ));
    assert!(bridge.stage.peek_next().is_none() && bridge.staged.is_empty());
    assert_eq!(
        control
            .bend_scopes
            .iter()
            .flatten()
            .filter(|cell| cell.scope == first_scope || cell.scope == second_scope)
            .count(),
        2
    );

    audio.service_test_ingress_queue();
    for (command, request) in [
        (first_command, first_request),
        (second_command, second_request),
    ] {
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        assert!(matches!(result.outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    let mut queue = vec![None; audio.test_ingress.queue_capacity_for_test()];
    let len = audio.test_ingress.copy_queue_for_test(&mut queue).unwrap();
    let written: Vec<_> = queue
        .into_iter()
        .take(len)
        .flatten()
        .filter_map(|(event, _)| match event.payload() {
            EventPayload::Bend { identity, cents } => {
                Some((identity, cents, event.envelope().time()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        written,
        vec![
            (
                identities[1],
                Cents::new(200.0).unwrap(),
                SampleTime::new(100)
            ),
            (
                identities[0],
                Cents::new(-200.0).unwrap(),
                SampleTime::new(101)
            ),
        ]
    );
}

/// Feed the actual producer's validated head into the existing private raw,
/// stage and command fixture. The original observation reaches raw unchanged.
fn pump_concrete_bend(
    bridge: &mut BendBridge<SerialRaw>,
    inbox: &mut SourceInbox,
    pending_head: &mut Option<SourceHandoff>,
) -> Result<Option<(SourceHandoff, RawHandoff)>, BendPumpRefusal> {
    assert!(
        inbox.unresolved_failure().is_none(),
        "producer failure must be resolved before servicing a source head"
    );
    let Some(head) = inbox.peek_handoff() else {
        return Ok(None);
    };
    let InputObservation::Message {
        tick,
        arrival,
        input: original,
    } = head.observation()
    else {
        panic!("concrete bend fixture contains only messages");
    };
    assert!(matches!(original.event(), Midi1Event::PitchBend { .. }));
    let source = Source::First;
    if bridge.raw_pending[0].is_none() {
        assert!(pending_head.is_none());
        let generation = bridge.raw.generations[0];
        let preflight = bridge
            .raw
            .owner
            .input(generation)
            .unwrap()
            .preflight_observation_custody(generation, head.observation())
            .map_err(BendPumpRefusal::RawPreflight)?;
        if let InputCustodyPreflight::Await(error) = preflight {
            return Err(BendPumpRefusal::AwaitRaw(error));
        }
        let raw = bridge
            .raw
            .owner
            .offer_message(generation, tick, arrival, original)
            .map_err(raw_admission)
            .map_err(BendPumpRefusal::RawOffer)?;
        bridge.raw_issued.push((source, raw));
        bridge.raw_pending[0] = Some(RawHandoff {
            source,
            raw,
            link: None,
            original,
            at: head.stamp().mapped_at(),
        });
        *pending_head = Some(head);
    } else {
        assert_eq!(*pending_head, Some(head));
        let pending = bridge.raw_pending[0].unwrap();
        assert_eq!(
            (pending.source, pending.original, pending.at),
            (source, original, head.stamp().mapped_at())
        );
    }
    let packet = Packet {
        occurrence: Occurrence(head.stamp().queue().serial()),
        original,
        at: head.stamp().mapped_at(),
        release: false,
    };
    bridge
        .stage
        .bend(source, packet)
        .map_err(BendPumpRefusal::Stage)?;
    assert_eq!(inbox.commit_handoff(head), Ok(head));
    assert_eq!(pending_head.take(), Some(head));
    let pending = bridge.raw_pending[0].take().unwrap();
    bridge.staged.push((packet.occurrence, pending));
    Ok(Some((head, pending)))
}

#[test]
fn actual_source_head_keeps_its_stamp_and_raw_original_through_stage_refusal() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (onset, request) = control
        .submit_onset(
            MixedIngressOriginId(190),
            BendBridge::scope(Source::First),
            SampleTime::new(190),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    let onset_result = control.collect_ingress_result().unwrap();
    assert_eq!((onset_result.id, onset_result.request), (onset, request));
    assert!(matches!(
        onset_result.outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_rate(
        256,
        InputRate::new(FrameCount::new(2), InputTickSpan::new(1)).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = BendBridge::with_raw(2, SerialRaw { owner, generations });
    let mut pending_head = None;
    let originals = [
        (100, 210, input(0xe0, 127, 127)),
        (101, 211, input(0xe0, 0, 0)),
        (102, 212, input(0xe0, 127, 127)),
    ]
    .map(|(tick, arrival, input)| InputObservation::Message {
        tick: InputTick::new(tick),
        arrival: SampleTime::new(arrival),
        input,
    });
    let queues = originals.map(|observation| producer.submit_owned(observation).unwrap().parts().1);
    assert_eq!(queues.map(|id| id.serial()), [1, 2, 3]);
    bridge.frontier(Source::Second, 220);

    let (first, first_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(first.stamp().queue(), queues[0]);
    assert_eq!(first.stamp().mapped_at(), SampleTime::new(200));
    let (second, second_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(second.stamp().queue(), queues[1]);
    let blocked = inbox.peek_handoff().unwrap();
    assert_eq!(blocked.stamp().queue(), queues[2]);
    assert_eq!(
        pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head),
        Err(BendPumpRefusal::Stage(StageRefusal::NoPacketCredit))
    );
    assert_eq!(inbox.peek_handoff(), Some(blocked));
    assert_eq!(pending_head, Some(blocked));
    assert_eq!(
        inbox.commit_handoff(first),
        Err(SourceHeadError::Mismatched {
            expected: first,
            actual: blocked,
        })
    );
    assert_eq!(inbox.peek_handoff(), Some(blocked));
    assert_eq!(
        bridge.raw_pending[0].unwrap().original,
        input(0xe0, 127, 127)
    );
    assert_eq!(bridge.raw_issued.len(), 3);
    assert_eq!(
        bridge.raw_pending[0].unwrap().at,
        blocked.stamp().mapped_at()
    );
    assert_eq!(bridge.raw_pending[0].unwrap().raw, bridge.raw_issued[2].1);

    let (occurrence, served, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((occurrence, served), (Occurrence(1), first_raw));
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    let (third, third_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(third, blocked);
    assert_eq!(third_raw.raw, bridge.raw_issued[2].1);
    assert_eq!(third_raw.at, blocked.stamp().mapped_at());
    assert_eq!(pending_head, None);
    assert_eq!(bridge.raw_issued.len(), 3);
    let (occurrence, served, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((occurrence, served), (Occurrence(2), second_raw));
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    let (occurrence, served, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((occurrence, served), (Occurrence(3), third_raw));
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    assert!(inbox.is_empty());
    assert!(inbox.close(producer).is_ok());
    assert_eq!(
        queued_bends(&audio)
            .iter()
            .map(|(at, cents)| (at.as_u64(), cents.as_f32()))
            .collect::<Vec<_>>(),
        vec![(200, 200.0), (202, -200.0), (204, 200.0)]
    );
    for generation in generations {
        let tick = if generation == generations[0] {
            128
        } else {
            256
        };
        let frontier = bridge
            .raw
            .owner
            .advance_frontier(generation, InputTick::new(tick))
            .unwrap();
        assert_eq!(frontier.generation(), generation);
    }
    bridge.raw.owner.pump().unwrap();
    let mut samples = [0.0_f32; 512];
    bridge
        .raw
        .owner
        .render(AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    bridge.raw.owner.pump().unwrap();
    let mut receipts = Vec::new();
    while let Some(receipt) = bridge.raw.owner.collect_input(generations[0]).unwrap() {
        if matches!(receipt.observation, InputObservation::Message { .. }) {
            receipts.push((receipt.id, receipt.observation));
        }
    }
    assert_eq!(
        receipts,
        [
            (first_raw.raw, originals[0]),
            (second_raw.raw, originals[1]),
            (third_raw.raw, originals[2]),
        ]
    );
    assert!(
        receipts
            .windows(2)
            .all(|pair| pair[0].0.serial() < pair[1].0.serial())
    );
}

#[test]
fn actual_source_bends_keep_stage_places_and_order_when_the_command_ring_is_full() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let scope = BendBridge::scope(Source::First);
    let (onset, request) = control
        .submit_onset(
            MixedIngressOriginId(100),
            scope,
            SampleTime::new(100),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (onset, request));
    assert!(matches!(result.outcome, MixedIngressOutcome::Onset(Ok(_))));

    let mut pressed = 0;
    while control.ingress_room() > 1 {
        let id = u64::try_from(1000 + pressed).unwrap();
        let peer_scope = MixedLiveScope::new(MixedLiveSource::new(id), MidiChannel::new(1));
        assert!(matches!(
            control
                .submit_pedal(
                    MixedIngressOriginId(id),
                    peer_scope,
                    SampleTime::new(110),
                    true
                )
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        pressed += 1;
    }
    let (filler, _) = control
        .submit_bend(
            MixedIngressOriginId(2000),
            BendBridge::scope(Source::Second),
            SampleTime::new(110),
            Cents::ZERO,
        )
        .unwrap();
    assert_eq!(control.ingress_room(), 0);

    let (owner, generations) = ledger_tests::bridge_serial_owner(512);
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = BendBridge::with_raw(2, SerialRaw { owner, generations });
    let mut pending_head = None;
    let originals = [
        (320, 330, input(0xe0, 127, 127)),
        (321, 331, input(0xe0, 0, 0)),
    ]
    .map(|(tick, arrival, input)| InputObservation::Message {
        tick: InputTick::new(tick),
        arrival: SampleTime::new(arrival),
        input,
    });
    let queues = originals.map(|observation| producer.submit_owned(observation).unwrap().parts().1);
    bridge.frontier(Source::Second, 322);
    let (first, first_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    let (second, second_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(
        (first.stamp().queue(), second.stamp().queue()),
        (queues[0], queues[1])
    );
    assert_eq!(bridge.raw_issued.len(), 2);
    assert!(inbox.is_empty());
    assert!(inbox.close(producer).is_ok());

    let before = control.last_ingress_id;
    let credits_before = control.bend_credits_free;
    assert_eq!(
        bridge.service(&mut control, &mut audio),
        Err(MixedBendSubmitError::Full)
    );
    assert_eq!(control.last_ingress_id, before);
    assert_eq!(control.bend_credits_free, credits_before);
    assert_eq!(
        bridge.stage.peek_next().unwrap().1.occurrence,
        Occurrence(1)
    );
    assert_eq!(
        bridge.staged,
        vec![(Occurrence(1), first_raw), (Occurrence(2), second_raw)]
    );

    audio.service_test_ingress_queue();
    for _ in 0..pressed {
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    let filler_result = control.collect_ingress_result().unwrap();
    assert_eq!(filler_result.id, filler);
    assert!(matches!(
        filler_result.outcome,
        MixedIngressOutcome::Bend(Ok(_))
    ));
    assert!(control.collect_ingress_result().is_none());

    for (expected, raw) in [(Occurrence(1), first_raw), (Occurrence(2), second_raw)] {
        let (occurrence, returned, outcome) =
            bridge.service(&mut control, &mut audio).unwrap().unwrap();
        assert_eq!((occurrence, returned), (expected, raw));
        assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    assert_eq!(bridge.service(&mut control, &mut audio), Ok(None));
    assert!(bridge.staged.is_empty());
    assert_eq!(
        queued_bends(&audio),
        vec![
            (SampleTime::new(320), Cents::new(200.0).unwrap()),
            (SampleTime::new(321), Cents::new(-200.0).unwrap()),
        ]
    );
}

#[test]
fn source_stage_and_command_refusals_keep_seventeen_bends_in_order() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = BendBridge::with_raw(2, SerialRaw { owner, generations });
    let mut pending_head = None;
    let originals: Vec<_> = (0..=SOURCE_CELLS)
        .map(|index| {
            let at = 100 + u64::try_from(index).unwrap();
            InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: if index % 2 == 0 {
                    input(0xe0, 127, 127)
                } else {
                    input(0xe0, 0, 0)
                },
            }
        })
        .collect();
    let mut queue_ids = Vec::new();
    for observation in originals.iter().take(SOURCE_CELLS) {
        queue_ids.push(producer.submit_owned(*observation).unwrap().parts().1);
    }
    let token = match producer.submit_owned(originals[SOURCE_CELLS]) {
        Err(SourceOwnedSendError::Retry(token)) => token,
        other => panic!("source ring must return its owned retry token: {other:?}"),
    };
    assert_eq!(
        token.attempt().serial(),
        u64::try_from(SOURCE_CELLS + 1).unwrap()
    );
    assert_eq!(inbox.peek_handoff().unwrap().stamp().queue(), queue_ids[0]);
    let token = match producer.retry_owned(token) {
        Err(SourceOwnedSendError::Retry(token)) => token,
        other => panic!("full source ring must retain the same attempt: {other:?}"),
    };
    assert_eq!(
        token.attempt().serial(),
        u64::try_from(SOURCE_CELLS + 1).unwrap()
    );
    assert_eq!(inbox.peek_handoff().unwrap().stamp().queue(), queue_ids[0]);
    bridge.frontier(Source::Second, 200);

    // Exhaust ordinary command room while the producer still owns its seventeenth
    // attempt. The same run will also fill the two-cell stage below.
    let mut pressed = 0;
    while control.ingress_room() > 0 {
        let id = u64::try_from(1000 + pressed).unwrap();
        let peer = MixedLiveScope::new(MixedLiveSource::new(id), MidiChannel::CH1);
        assert!(matches!(
            control
                .submit_pedal(MixedIngressOriginId(id), peer, SampleTime::new(90), true)
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        pressed += 1;
    }
    assert_eq!(control.ingress_room(), 0);

    let mut raw_ids = Vec::new();
    let (first, first_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(first.stamp().queue(), queue_ids[0]);
    raw_ids.push(first_raw.raw);
    let (attempt, last_queue) = producer.retry_owned(token).unwrap().parts();
    assert_eq!(attempt.serial(), u64::try_from(SOURCE_CELLS + 1).unwrap());
    assert_eq!(
        last_queue.serial(),
        u64::try_from(SOURCE_CELLS + 1).unwrap()
    );
    queue_ids.push(last_queue);
    assert_eq!(inbox.peek_handoff().unwrap().stamp().queue(), queue_ids[1]);

    let (second, second_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(second.stamp().queue(), queue_ids[1]);
    raw_ids.push(second_raw.raw);
    let blocked = inbox.peek_handoff().unwrap();
    assert_eq!(blocked.stamp().queue(), queue_ids[2]);
    assert_eq!(
        pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head),
        Err(BendPumpRefusal::Stage(StageRefusal::NoPacketCredit))
    );
    assert_eq!(inbox.peek_handoff(), Some(blocked));
    assert_eq!(pending_head, Some(blocked));
    let third_raw = bridge.raw_pending[0].unwrap();
    assert_eq!(bridge.raw_issued.len(), 3);
    let before = control.last_ingress_id;
    assert_eq!(
        bridge.submit_next(&mut control),
        Err(MixedBendSubmitError::Full)
    );
    assert_eq!(control.last_ingress_id, before);
    assert_eq!(
        bridge.stage.peek_next().unwrap().1.occurrence,
        Occurrence(1)
    );
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));

    audio.service_test_ingress_queue();
    for _ in 0..pressed {
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    let (occurrence, handoff, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!((occurrence, handoff), (Occurrence(1), first_raw));
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));

    let (third, retried_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    assert_eq!(third, blocked);
    assert_eq!(retried_raw, third_raw);
    assert_eq!(bridge.raw_issued.len(), 3);
    raw_ids.push(retried_raw.raw);
    for expected in 2..=3 {
        let (occurrence, handoff, outcome) =
            bridge.service(&mut control, &mut audio).unwrap().unwrap();
        assert_eq!(occurrence.0, u64::try_from(expected).unwrap());
        assert_eq!(handoff.raw, raw_ids[expected - 1]);
        assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    for expected in 4..=SOURCE_CELLS + 1 {
        let (head, raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
            .unwrap()
            .unwrap();
        assert_eq!(head.stamp().queue(), queue_ids[expected - 1]);
        raw_ids.push(raw.raw);
        let (occurrence, handoff, outcome) =
            bridge.service(&mut control, &mut audio).unwrap().unwrap();
        assert_eq!(occurrence.0, u64::try_from(expected).unwrap());
        assert_eq!(handoff.raw, raw.raw);
        assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    assert!(inbox.is_empty());
    assert!(inbox.close(producer).is_ok());
    assert_eq!(bridge.raw_issued.len(), originals.len());

    for generation in generations {
        let frontier = bridge
            .raw
            .owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        assert_eq!(frontier.generation(), generation);
    }
    bridge.raw.owner.pump().unwrap();
    let mut samples = [0.0_f32; 512];
    bridge
        .raw
        .owner
        .render(AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    bridge.raw.owner.pump().unwrap();
    let mut receipts = Vec::new();
    while let Some(receipt) = bridge.raw.owner.collect_input(generations[0]).unwrap() {
        if matches!(receipt.observation, InputObservation::Message { .. }) {
            receipts.push((receipt.id, receipt.observation));
        }
    }
    assert_eq!(
        receipts,
        raw_ids.into_iter().zip(originals).collect::<Vec<_>>()
    );
}

#[test]
fn actual_source_bends_retry_a_full_command_ring_into_callback_order() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let scope = BendBridge::scope(Source::First);
    let _onset = control
        .submit_onset(
            MixedIngressOriginId(1),
            scope,
            SampleTime::new(100),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));

    // Fill the ordinary command places while each pedal keeps its lift place.
    let mut pressed = 0;
    while control.ingress_room() > 1 {
        let id = u64::try_from(1000 + pressed).unwrap();
        let peer = MixedLiveScope::new(MixedLiveSource::new(id), MidiChannel::new(1));
        assert!(matches!(
            control
                .submit_pedal(MixedIngressOriginId(id), peer, SampleTime::new(90), true)
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        pressed += 1;
    }
    let (filler, _) = control
        .submit_bend(
            MixedIngressOriginId(2000),
            BendBridge::scope(Source::Second),
            SampleTime::new(100),
            Cents::ZERO,
        )
        .unwrap();
    assert_eq!(control.ingress_room(), 0);

    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = BendBridge::with_raw(2, SerialRaw { owner, generations });
    let mut queue_ids = Vec::new();
    for (tick, wheel) in [(110, input(0xe0, 127, 127)), (111, input(0xe0, 0, 0))] {
        queue_ids.push(
            producer
                .submit_owned(InputObservation::Message {
                    tick: InputTick::new(tick),
                    arrival: SampleTime::new(tick),
                    input: wheel,
                })
                .unwrap()
                .parts()
                .1,
        );
    }
    bridge.frontier(Source::Second, 200);
    let mut pending_head = None;
    let (first_source, first_raw) = pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
        .unwrap()
        .unwrap();
    let (second_source, second_raw) =
        pump_concrete_bend(&mut bridge, &mut inbox, &mut pending_head)
            .unwrap()
            .unwrap();
    assert_eq!(first_source.stamp().queue(), queue_ids[0]);
    assert_eq!(second_source.stamp().queue(), queue_ids[1]);
    assert_eq!(first_source.stamp().mapped_at(), SampleTime::new(110));
    assert_eq!(second_source.stamp().mapped_at(), SampleTime::new(111));
    assert_ne!(first_raw.raw, second_raw.raw);
    assert!(inbox.close(producer).is_ok());

    let before = control.last_ingress_id;
    assert_eq!(
        bridge.submit_next(&mut control),
        Err(MixedBendSubmitError::Full)
    );
    assert_eq!(control.last_ingress_id, before);
    assert_eq!(
        bridge.stage.peek_next().unwrap().1.occurrence,
        Occurrence(1)
    );
    assert_eq!(
        bridge.staged,
        vec![(Occurrence(1), first_raw), (Occurrence(2), second_raw)]
    );

    let mut actual = [0.0_f32; 384];
    audio
        .render_private(AudioBlockMut::new(&mut actual[..128], 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    // The first 64 host frames are priming carry, so this has opened only
    // quantum zero. Both bends are still ahead of the engine clock.
    assert_eq!(audio.audio.renderer.clock(), SampleTime::new(64));
    for _ in 0..pressed {
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    let filler_result = control.collect_ingress_result().unwrap();
    assert_eq!(filler_result.id, filler);
    assert!(matches!(
        filler_result.outcome,
        MixedIngressOutcome::Bend(Ok(_))
    ));

    let first = bridge.submit_next(&mut control).unwrap().unwrap();
    let second = bridge.submit_next(&mut control).unwrap().unwrap();
    assert_eq!((first.0, first.1), (Occurrence(1), first_raw));
    assert_eq!((second.0, second.1), (Occurrence(2), second_raw));
    assert!(bridge.stage.peek_next().is_none());
    assert!(bridge.staged.is_empty());
    audio
        .render_private(AudioBlockMut::new(&mut actual[128..], 256, ChannelLayout::Mono).unwrap())
        .unwrap();
    for submitted in [first, second] {
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (submitted.2, submitted.3));
        let MixedIngressOutcome::Bend(Ok(bent)) = result.outcome else {
            panic!("source bend must write one live occurrence");
        };
        assert_eq!(bent.written().count(), 1);
    }
    assert!(control.collect_ingress_result().is_none());
    assert_eq!(audio.audio.renderer.diagnostics().late_events(), 0);

    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut reference_control, mut reference) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let _note = reference_control
        .submit_onset(
            MixedIngressOriginId(1),
            scope,
            SampleTime::new(100),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    reference.service_test_ingress_queue();
    assert!(matches!(
        reference_control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));
    for (at, cents) in [(110, 200.0), (111, -200.0)] {
        let _bend = reference_control
            .submit_bend(
                MixedIngressOriginId(at),
                scope,
                SampleTime::new(at),
                Cents::new(cents).unwrap(),
            )
            .unwrap();
    }
    reference.service_test_ingress_queue();
    for _ in 0..2 {
        let MixedIngressOutcome::Bend(Ok(bent)) =
            reference_control.collect_ingress_result().unwrap().outcome
        else {
            panic!("reference bend must write one live occurrence");
        };
        assert_eq!(bent.written().count(), 1);
    }
    let mut expected = [0.0_f32; 384];
    reference
        .render_private(AudioBlockMut::new(&mut expected, 384, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(reference.audio.renderer.diagnostics().late_events(), 0);
    assert_eq!(actual, expected);
}

#[test]
fn completed_frontier_receipt_releases_the_same_waiting_source_bend() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (owner, generations) = ledger_tests::bridge_serial_owner(256);
    let mut bridge = BendBridge::with_raw(2, SerialRaw { owner, generations });
    let wheel = input(0xe0, 127, 127);
    for at in 100..107 {
        bridge.enqueue(Source::First, at, wheel);
    }
    bridge.frontier(Source::Second, 200);
    for at in 100..106 {
        let handoff = bridge.pump(Source::First).unwrap().unwrap();
        let (occurrence, served, outcome) =
            bridge.service(&mut control, &mut audio).unwrap().unwrap();
        assert_eq!(occurrence, Occurrence(at - 99));
        assert_eq!(served, handoff);
        assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    let head = bridge.source_heads[0].peek().unwrap();
    assert_eq!(head.occurrence, Occurrence(7));
    assert_eq!(bridge.raw_issued.len(), 6);
    assert_eq!(
        bridge.pump(Source::First),
        Err(BendPumpRefusal::RawPreflight(InputError::Full))
    );

    bridge.raw.owner.pump().unwrap();
    let mut samples = [0.0_f32; 128];
    bridge
        .raw
        .owner
        .render(AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    bridge.raw.owner.pump().unwrap();
    assert_eq!(
        bridge.pump(Source::First),
        Err(BendPumpRefusal::AwaitRaw(InputError::Full))
    );
    assert_eq!(bridge.source_heads[0].peek(), Some(head));
    assert_eq!(bridge.raw_issued.len(), 6);
    let generation = bridge.raw.generations[0];
    let mut receipts = Vec::new();
    while let Some(receipt) = bridge.raw.owner.collect_input(generation).unwrap() {
        receipts.push(receipt);
    }
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].observation,
        InputObservation::Frontier {
            tick: InputTick::new(0),
        }
    );
    let retried = bridge.pump(Source::First).unwrap().unwrap();
    assert_eq!(retried.raw.serial(), bridge.raw_issued[5].1.serial() + 1);
    assert!(bridge.source_heads[0].is_empty());
    let (occurrence, served, outcome) = bridge.service(&mut control, &mut audio).unwrap().unwrap();
    assert_eq!(occurrence, head.occurrence);
    assert_eq!(served, retried);
    assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
}

#[test]
fn raw_full_preserves_the_next_bend_without_minting_or_faulting() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (onset, request) = control
        .submit_onset(
            MixedIngressOriginId(90),
            BendBridge::scope(Source::First),
            SampleTime::new(90),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (onset, request));
    assert!(matches!(result.outcome, MixedIngressOutcome::Onset(Ok(_))));

    let mut bridge = BendBridge::with_raw_cells(2, 6);
    let wheel = input(0xe0, 127, 127);
    for at in 100..106 {
        bridge.enqueue(Source::First, at, wheel);
    }
    bridge.frontier(Source::Second, 200);
    for at in 100..104 {
        let raw = bridge.pump(Source::First).unwrap().unwrap();
        let (occurrence, handed, outcome) =
            bridge.service(&mut control, &mut audio).unwrap().unwrap();
        assert_eq!(occurrence, Occurrence(at - 99));
        assert_eq!(handed, raw);
        assert!(matches!(outcome, MixedIngressOutcome::Bend(Ok(_))));
    }
    assert_eq!(
        queued_bends(&audio)
            .iter()
            .map(|(at, _)| at.as_u64())
            .collect::<Vec<_>>(),
        vec![100, 101, 102, 103]
    );
    assert_eq!(bridge.raw_issued.len(), 4);
    assert!(
        bridge
            .raw_issued
            .windows(2)
            .all(|pair| pair[0].1.serial() < pair[1].1.serial())
    );
    let head = bridge.source_heads[0].peek().unwrap();
    assert_eq!(head.occurrence, Occurrence(5));
    let occupied = bridge.raw[0].input.pressure().occupied();
    for _ in 0..2 {
        assert_eq!(
            bridge.pump(Source::First),
            Err(BendPumpRefusal::RawPreflight(InputError::Full))
        );
        assert_eq!(bridge.source_heads[0].peek(), Some(head));
        assert_eq!(bridge.raw_pending[0], None);
        assert_eq!(bridge.raw_issued.len(), 4);
        assert_eq!(bridge.raw[0].input.pressure().occupied(), occupied);
        assert_eq!(bridge.raw[0].input.discontinuity(), None);
    }
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
    let issued_before = bridge.raw_issued.len();
    let blocked = bridge.enqueue(Source::First, 320, wheel);
    let later = bridge.enqueue(Source::First, 321, input(0xe0, 0, 0));
    let last = bridge.enqueue(Source::First, 322, wheel);
    let first_raw = bridge.pump(Source::First).unwrap().unwrap();
    let second_raw = bridge.pump(Source::First).unwrap().unwrap();
    assert_eq!(
        bridge.pump(Source::First),
        Err(BendPumpRefusal::Stage(StageRefusal::NoPacketCredit))
    );
    let third_raw = bridge.raw_pending[0].unwrap();
    assert_eq!(bridge.raw_issued.len(), issued_before + 3);
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
        Err(BendPumpRefusal::Stage(StageRefusal::NoPacketCredit))
    );
    assert_eq!(bridge.raw_pending[0], Some(third_raw));
    assert_eq!(bridge.raw_issued.len(), issued_before + 3);

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
    assert_eq!(bridge.raw_issued.len(), issued_before + 3);
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

/// Move a source head through raw admission into the bounded note stage. This
/// fixture uses a 1:1 clock and equal tick, arrival and mapped frame stamps.
/// The source place is released only after the stage takes the packet.
fn pump_concrete_note<R: RawBackend>(
    bridge: &mut RawStageBridge<R>,
    inbox: &mut SourceInbox,
) -> (SourceHandoff, Occurrence) {
    assert!(inbox.unresolved_failure().is_none());
    let head = inbox.peek_handoff().expect("source head");
    let InputObservation::Message {
        tick,
        arrival,
        input: original,
    } = head.observation()
    else {
        panic!("note fixture contains only messages");
    };
    let mapped = head.stamp().mapped_at();
    assert_eq!(tick, InputTick::new(mapped.as_u64()));
    assert_eq!(arrival, mapped);
    let occurrence = bridge
        .offer(Source::First, mapped.as_u64(), original)
        .expect("raw owner and stage accept source head");
    assert_eq!(inbox.commit_handoff(head), Ok(head));
    (head, occurrence)
}

#[derive(Debug, PartialEq)]
enum MixedSourceStep {
    Staged(SourceHandoff, Occurrence),
    Frontier(SourceHandoff),
    AwaitRaw(SourceHandoff, InputError),
    AwaitStage(SourceHandoff, RawHandoff),
    Split(SourceHandoff, BridgeRefused),
}

#[derive(Debug, PartialEq)]
struct PendingBendCancellation {
    source: SourceHandoff,
    raw: RawHandoff,
}

/// Joined shutdown names the source packet and its one raw admission before
/// the mixed owner tears down. The raw owner still settles its own receipt.
fn cancel_pending_source_bend<R: RawBackend>(
    bridge: &mut RawStageBridge<R>,
    pending: &mut Option<(SourceHandoff, RawHandoff)>,
) -> Option<PendingBendCancellation> {
    let (source, raw) = pending.take()?;
    assert!(matches!(raw.original.event(), Midi1Event::PitchBend { .. }));
    assert_eq!(
        bridge.pending_source_bends[raw.source.index()],
        Some(raw.raw)
    );
    bridge.pending_source_bends[raw.source.index()] = None;
    Some(PendingBendCancellation { source, raw })
}

/// A raw-admitted bend leaves the source ring and may wait in one source-owned
/// slot. A younger head makes a still-full bend terminally split before that
/// younger head can enter mixed stage. Raw admission faults are not split.
fn pump_concrete_mixed<R: RawBackend>(
    bridge: &mut RawStageBridge<R>,
    inbox: &mut SourceInbox,
    pending: &mut Option<(SourceHandoff, RawHandoff)>,
) -> Option<MixedSourceStep> {
    assert!(inbox.unresolved_failure().is_none());
    if let Some((held_head, handoff)) = pending.take() {
        assert_eq!(bridge.pending_source_bends[0], Some(handoff.raw));
        bridge.pending_source_bends[0] = None;
        return Some(match bridge.stage_handoff(handoff) {
            Ok(occurrence) => MixedSourceStep::Staged(held_head, occurrence),
            Err(
                refusal @ BridgeRefused::Lane {
                    reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
                    ..
                },
            ) if matches!(handoff.original.event(), Midi1Event::PitchBend { .. }) => {
                if inbox.peek_handoff().is_some() {
                    MixedSourceStep::Split(held_head, refusal)
                } else {
                    *pending = Some((held_head, handoff));
                    bridge.pending_source_bends[0] = Some(handoff.raw);
                    MixedSourceStep::AwaitStage(held_head, handoff)
                }
            }
            Err(refusal) => MixedSourceStep::Split(held_head, refusal),
        });
    }
    let head = inbox.peek_handoff()?;
    assert!(bridge.pending_source_bends[0].is_none());
    let at = head.stamp().mapped_at();
    let (tick, arrival, original) = match head.observation() {
        InputObservation::Frontier { tick } => {
            assert_eq!(tick, InputTick::new(at.as_u64()));
            bridge.frontier(Source::First, at.as_u64());
            assert_eq!(inbox.commit_handoff(head), Ok(head));
            return Some(MixedSourceStep::Frontier(head));
        }
        InputObservation::Message {
            tick,
            arrival,
            input,
        } => (tick, arrival, input),
    };
    assert_eq!((tick, arrival), (InputTick::new(at.as_u64()), at));
    match bridge.sources.preflight(Source::First, at, original) {
        Ok(InputCustodyPreflight::Admit) => {}
        Ok(InputCustodyPreflight::Await(error)) => {
            return Some(MixedSourceStep::AwaitRaw(head, error));
        }
        Err(error) => panic!("raw preflight requires a terminal host fault: {error}"),
    }
    let handoff = match bridge.admit_raw(Source::First, at, original) {
        Ok(handoff) => handoff,
        Err(refusal) => panic!("raw admission requires a terminal host fault: {refusal:?}"),
    };
    match bridge.stage_handoff(handoff) {
        Ok(occurrence) => {
            assert_eq!(inbox.commit_handoff(head), Ok(head));
            *pending = None;
            Some(MixedSourceStep::Staged(head, occurrence))
        }
        Err(BridgeRefused::Lane {
            reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
            ..
        }) if matches!(original.event(), Midi1Event::PitchBend { .. }) => {
            assert_eq!(inbox.commit_handoff(head), Ok(head));
            if inbox.peek_handoff().is_some() {
                Some(MixedSourceStep::Split(
                    head,
                    BridgeRefused::Lane {
                        handoff,
                        reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
                    },
                ))
            } else {
                *pending = Some((head, handoff));
                bridge.pending_source_bends[0] = Some(handoff.raw);
                Some(MixedSourceStep::AwaitStage(head, handoff))
            }
        }
        Err(refusal) => {
            assert_eq!(inbox.commit_handoff(head), Ok(head));
            *pending = None;
            Some(MixedSourceStep::Split(head, refusal))
        }
    }
}

#[test]
fn pending_source_bend_shutdown_pairs_its_queue_head_and_raw_receipt() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (control, audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let raw = [raw_source_with_cells(0, 32), raw_source(1)];
    let generation = raw[0].generation;
    let clock = raw[0].input.clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generation, InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(2, raw);
    for (index, (at, input)) in [(100, input(0x90, 60, 100)), (101, input(0xe0, 127, 127))]
        .into_iter()
        .enumerate()
    {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input,
            })
            .unwrap()
            .parts();
        assert_eq!(queue.serial(), u64::try_from(index + 1).unwrap());
    }
    bridge.frontier(Source::Second, 102);
    let mut pending = None;
    assert!(matches!(
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending),
        Some(MixedSourceStep::Staged(_, Occurrence(1)))
    ));
    let Some(MixedSourceStep::AwaitStage(head, handoff)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the bend waits outside an empty source ring");
    };
    assert!(inbox.is_empty());
    assert_eq!(pending, Some((head, handoff)));
    let cancelled = cancel_pending_source_bend(&mut bridge, &mut pending).unwrap();
    assert_eq!(
        cancelled,
        PendingBendCancellation {
            source: head,
            raw: handoff
        }
    );
    assert!(pending.is_none());
    assert!(bridge.pending_source_bends[0].is_none());
    assert!(inbox.close(producer).is_ok());

    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert_eq!(teardown.reservations.len(), 1);
    assert_eq!(teardown.unoffered.len(), 1);
    assert!(
        teardown
            .unoffered
            .iter()
            .all(|entry| entry.raw != cancelled.raw.raw)
    );
    assert!(
        teardown
            .raw_issued
            .contains(&(Source::First, cancelled.raw.raw, None))
    );
    let issued = teardown.raw_issued.clone();
    let settled = settle_raw_by_cancellation(raw, &issued);
    assert_eq!(settled.bridge.len(), issued.len());
    let (_, receipt) = settled
        .bridge
        .iter()
        .find(|(_, receipt)| receipt.id == cancelled.raw.raw)
        .unwrap();
    assert_eq!(receipt.observation, cancelled.source.observation());
    assert_eq!(receipt.matched_onset, None);
}

#[test]
fn a_newer_key_up_splits_an_already_pending_bend_before_raw_service() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let raw = [raw_source_with_cells(0, 32), raw_source(1)];
    let generation = raw[0].generation;
    let clock = raw[0].input.clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generation, InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(2, raw);
    for (index, (at, input)) in [(100, input(0x90, 60, 100)), (101, input(0xe0, 127, 127))]
        .into_iter()
        .enumerate()
    {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input,
            })
            .unwrap()
            .parts();
        assert_eq!(queue.serial(), u64::try_from(index + 1).unwrap());
    }
    bridge.frontier(Source::Second, 103);
    let mut pending = None;
    let Some(MixedSourceStep::Staged(_, note_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the note reserves its key-up place");
    };
    let Some(MixedSourceStep::AwaitStage(bend_head, bend_raw)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the bend is source-owned while no later edge exists");
    };
    assert!(inbox.is_empty());
    let (_, key_up_queue) = producer
        .submit_owned(InputObservation::Message {
            tick: InputTick::new(102),
            arrival: SampleTime::new(102),
            input: input(0x80, 60, 0),
        })
        .unwrap()
        .parts();
    assert_eq!(key_up_queue.serial(), 3);
    assert_eq!(
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending),
        Some(MixedSourceStep::Split(
            bend_head,
            BridgeRefused::Lane {
                handoff: bend_raw,
                reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
            },
        ))
    );
    assert!(pending.is_none());
    assert!(bridge.pending_source_bends[0].is_none());
    let Some(MixedSourceStep::Staged(key_up_head, release_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the protected key-up must pass the split bend");
    };
    assert_eq!(key_up_head.stamp().queue(), key_up_queue);
    assert_eq!(release_id, note_id);
    assert!(inbox.close(producer).is_ok());
    let onset = bridge.service(&mut control, &mut audio).unwrap();
    assert!(onset.accepted());
    let release = bridge.service(&mut control, &mut audio).unwrap();
    assert!(release.accepted());
    assert_eq!(release.handoff.link, Some(onset.handoff.raw));
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn a_source_frontier_splits_a_pending_bend_then_advances_raw_and_stage() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (control, audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(2, SerialRaw { owner, generations });
    for (at, input) in [(100, input(0x90, 60, 100)), (101, input(0xe0, 127, 127))] {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input,
            })
            .unwrap()
            .parts();
        assert_eq!(queue.serial(), at - 99);
    }
    bridge.frontier(Source::Second, 104);
    let mut pending = None;
    assert!(matches!(
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending),
        Some(MixedSourceStep::Staged(_, Occurrence(1)))
    ));
    let Some(MixedSourceStep::AwaitStage(bend_head, bend_raw)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the bend waits outside the source ring");
    };
    let (_, frontier_queue) = producer
        .submit_owned(InputObservation::Frontier {
            tick: InputTick::new(102),
        })
        .unwrap()
        .parts();
    assert_eq!(frontier_queue.serial(), 3);
    assert_eq!(
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending),
        Some(MixedSourceStep::Split(
            bend_head,
            BridgeRefused::Lane {
                handoff: bend_raw,
                reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
            },
        ))
    );
    let Some(MixedSourceStep::Frontier(frontier_head)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the raw and stage frontiers must pass the split bend");
    };
    assert_eq!(frontier_head.stamp().queue(), frontier_queue);
    assert_eq!(
        bridge.stage.lanes[0].last_offered,
        Some(SampleTime::new(102))
    );
    assert!(pending.is_none());
    assert!(inbox.close(producer).is_ok());
    let (teardown, mut raw, _ended) = bridge.tear_down(control, audio);
    assert_eq!(teardown.unoffered.len(), 1);
    assert_eq!(teardown.reservations.len(), 1);
    let mut remaining = teardown.raw_issued;
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        let frontier = raw
            .owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        remaining.push((source, frontier, None));
    }
    let delivered = publish_serial(&mut raw, remaining);
    let bend_receipt = delivered
        .iter()
        .find(|receipt| receipt.id == bend_raw.raw)
        .unwrap();
    assert_eq!(
        bend_receipt.observation,
        InputObservation::Message {
            tick: InputTick::new(101),
            arrival: SampleTime::new(101),
            input: bend_raw.original,
        }
    );
    let InputOutcome::Delivered(SessionSourceOutcome::Published(record)) = &bend_receipt.outcome
    else {
        panic!("the pending split bend must reach the serial recorder: {bend_receipt:?}");
    };
    assert_eq!(
        record.capture,
        crate::recording::notes::CaptureDisposition::Recorded
    );
}

#[test]
fn actual_source_bend_retries_behind_note_and_pedal_without_a_second_raw_id() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(4, SerialRaw { owner, generations });
    let observations = [
        (100, input(0xb0, 64, 127)),
        (101, input(0x90, 60, 100)),
        (102, input(0xe0, 127, 127)),
        (103, input(0x80, 60, 0)),
        (104, input(0xb0, 64, 0)),
    ];
    let mut queues = Vec::new();
    for &(at, input) in observations.iter().take(3) {
        queues.push(
            producer
                .submit_owned(InputObservation::Message {
                    tick: InputTick::new(at),
                    arrival: SampleTime::new(at),
                    input,
                })
                .unwrap()
                .parts()
                .1,
        );
    }
    bridge.frontier(Source::Second, 105);
    let mut pending = None;
    let Some(MixedSourceStep::Staged(first_head, down_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the raw pedal-down takes its protected lift place");
    };
    let Some(MixedSourceStep::Staged(second_head, note_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the raw note-on takes its protected key-up place");
    };
    assert_eq!(first_head.stamp().queue(), queues[0]);
    assert_eq!(second_head.stamp().queue(), queues[1]);
    assert_eq!(bridge.stage.pressure(Source::First), (2, 2));
    let Some(MixedSourceStep::AwaitStage(bend_head, bend_raw)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the bend retains source and raw custody at the full stage");
    };
    assert_eq!(bend_head.stamp().queue(), queues[2]);
    assert_eq!(bend_raw.original, observations[2].1);
    assert!(inbox.peek_handoff().is_none());
    assert_eq!(pending, Some((bend_head, bend_raw)));
    assert_eq!(bridge.pending_source_bends[0], Some(bend_raw.raw));
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bridge.frontier(Source::First, 103)
        }))
        .is_err()
    );
    assert_eq!(bridge.pending_source_bends[0], Some(bend_raw.raw));
    let issued_before_retry = bridge.raw_issued.len();
    let selected = bridge.stage.peek_next();
    assert_eq!(
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending),
        Some(MixedSourceStep::AwaitStage(bend_head, bend_raw))
    );
    assert_eq!(bridge.raw_issued.len(), issued_before_retry);
    assert_eq!(bridge.stage.peek_next(), selected);

    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, down_id);
    assert!(matches!(
        down.outcome,
        Some(MixedIngressOutcome::PedalDown(Ok(_)))
    ));
    assert_eq!(bridge.stage.pressure(Source::First), (1, 2));
    let Some(MixedSourceStep::Staged(retried_head, bend_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the same bend head takes the freed stage cell");
    };
    assert_eq!(retried_head, bend_head);
    assert_eq!(bridge.raw_issued.len(), issued_before_retry);
    assert!(pending.is_none());
    assert!(bridge.pending_source_bends[0].is_none());
    for &(at, input) in observations.iter().skip(3) {
        queues.push(
            producer
                .submit_owned(InputObservation::Message {
                    tick: InputTick::new(at),
                    arrival: SampleTime::new(at),
                    input,
                })
                .unwrap()
                .parts()
                .1,
        );
    }
    for expected in queues.into_iter().skip(3) {
        let Some(MixedSourceStep::Staged(head, _)) =
            pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
        else {
            panic!("protected key-up and lift enter behind the bend");
        };
        assert_eq!(head.stamp().queue(), expected);
    }
    assert_eq!(bridge.stage.pressure(Source::First), (4, 0));
    assert!(inbox.close(producer).is_ok());

    let onset = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(onset.occurrence, note_id);
    assert!(onset.accepted());
    let bend = bridge
        .service_bend(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!((bend.occurrence, bend.handoff), (bend_id, bend_raw));
    let MixedIngressOutcome::Bend(Ok(written)) = bend.outcome else {
        panic!("the selected source bend must write the admitted note");
    };
    assert_eq!(written.written().collect::<Vec<_>>(), vec![onset.handle]);
    let release = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(release.occurrence, note_id);
    assert_eq!(release.handoff.link, Some(onset.handoff.raw));
    assert_eq!(
        release.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Sustained(onset.handle))
    );
    let lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(lift.paired_down_raw, Some(down.handoff.raw));
    let Some(MixedIngressOutcome::Lift(Ok(retired))) = lift.outcome else {
        panic!("the source lift must retire that exact sustained note");
    };
    assert_eq!(retired.retired().collect::<Vec<_>>(), vec![onset.handle]);
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    let (teardown, mut raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unoffered.is_empty() && teardown.pedal_lifts.is_empty());
    let mut remaining = teardown.raw_issued;
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        let frontier = raw
            .owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        remaining.push((source, frontier, None));
    }
    let delivered = publish_serial(&mut raw, remaining);
    let bend_receipt = delivered
        .iter()
        .find(|receipt| receipt.id == bend_raw.raw)
        .unwrap();
    assert_eq!(
        bend_receipt.observation,
        InputObservation::Message {
            tick: InputTick::new(102),
            arrival: SampleTime::new(102),
            input: bend_raw.original,
        }
    );
    let InputOutcome::Delivered(SessionSourceOutcome::Published(record)) = &bend_receipt.outcome
    else {
        panic!("the accepted bend must reach the serial recorder: {bend_receipt:?}");
    };
    assert_eq!(
        record.capture,
        crate::recording::notes::CaptureDisposition::Recorded
    );
    assert_eq!(
        recorded_occurrence(&delivered, onset.handoff.raw),
        recorded_occurrence(&delivered, release.handoff.raw)
    );
}

#[test]
fn younger_source_edges_split_a_full_stage_bend_before_raw_capture_continues() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let raw = [raw_source_with_cells(0, 32), raw_source(1)];
    let generation = raw[0].generation;
    let clock = raw[0].input.clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generation, InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(4, raw);
    for (index, (at, input)) in [
        (100, input(0xb0, 64, 127)),
        (101, input(0x90, 60, 100)),
        (102, input(0xe0, 127, 127)),
        (103, input(0x80, 60, 0)),
        (104, input(0xb0, 64, 0)),
    ]
    .into_iter()
    .enumerate()
    {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input,
            })
            .unwrap()
            .parts();
        assert_eq!(queue.serial(), u64::try_from(index + 1).unwrap());
    }
    bridge.frontier(Source::Second, 105);
    let mut pending = None;
    let Some(MixedSourceStep::Staged(_, down_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("pedal-down must enter the mixed stage");
    };
    let Some(MixedSourceStep::Staged(_, note_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("note-on must enter the mixed stage");
    };
    let Some(MixedSourceStep::Split(
        bend_head,
        BridgeRefused::Lane {
            handoff: bend,
            reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
        },
    )) = pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("a queued successor requires a terminal bend split");
    };
    assert_eq!(bend_head.stamp().queue().serial(), 3);
    assert_eq!(bend.original, input(0xe0, 127, 127));
    assert!(pending.is_none());
    assert_eq!(inbox.peek_handoff().unwrap().stamp().queue().serial(), 4);
    let issued_after_split = bridge.raw_issued.len();
    for expected in 4..=5 {
        let Some(MixedSourceStep::Staged(head, occurrence)) =
            pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
        else {
            panic!("the protected key-up and lift must pass the split bend");
        };
        assert_eq!(head.stamp().queue().serial(), expected);
        assert_eq!(
            occurrence,
            if expected == 4 {
                note_id
            } else {
                Occurrence(3)
            }
        );
    }
    assert_eq!(bridge.raw_issued.len(), issued_after_split + 2);
    assert_eq!(bridge.stage.pressure(Source::First), (4, 0));
    assert!(inbox.close(producer).is_ok());

    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, down_id);
    let onset = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(onset.occurrence, note_id);
    assert!(onset.accepted());
    let release = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(
        release.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Sustained(onset.handle))
    );
    let lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(lift.paired_down_raw, Some(down.handoff.raw));
    let Some(MixedIngressOutcome::Lift(Ok(retired))) = lift.outcome else {
        panic!("the lift must retire the note after the split bend");
    };
    assert_eq!(retired.retired().collect::<Vec<_>>(), vec![onset.handle]);
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn a_split_source_bend_reaches_the_serial_recorder_with_its_note_pair() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(4, SerialRaw { owner, generations });
    for (index, (at, input)) in [
        (100, input(0xb0, 64, 127)),
        (101, input(0x90, 60, 100)),
        (102, input(0xe0, 127, 127)),
        (103, input(0x80, 60, 0)),
        (104, input(0xb0, 64, 0)),
    ]
    .into_iter()
    .enumerate()
    {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input,
            })
            .unwrap()
            .parts();
        assert_eq!(queue.serial(), u64::try_from(index + 1).unwrap());
    }
    bridge.frontier(Source::Second, 105);
    let mut pending = None;
    let Some(MixedSourceStep::Staged(_, down_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("serial pedal-down must reach mixed stage");
    };
    let Some(MixedSourceStep::Staged(_, note_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("serial note-on must reach mixed stage");
    };
    let Some(MixedSourceStep::Split(
        bend_head,
        BridgeRefused::Lane {
            handoff: split_bend,
            reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
        },
    )) = pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the stage-full serial bend splits before the key-up");
    };
    assert_eq!(bend_head.stamp().queue().serial(), 3);
    let Some(MixedSourceStep::Staged(_, release_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the reserved key-up must reach mixed stage");
    };
    assert_eq!(release_id, note_id);
    let Some(MixedSourceStep::Staged(_, _)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the reserved lift must reach mixed stage");
    };
    assert!(inbox.close(producer).is_ok());
    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, down_id);
    let onset = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(onset.occurrence, note_id);
    assert!(onset.accepted());
    let release = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(release.handoff.link, Some(onset.handoff.raw));
    assert_eq!(
        release.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Sustained(onset.handle))
    );
    let lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(lift.paired_down_raw, Some(down.handoff.raw));
    let Some(MixedIngressOutcome::Lift(Ok(retired))) = lift.outcome else {
        panic!("the serial lift must retire the sustained note");
    };
    assert_eq!(retired.retired().collect::<Vec<_>>(), vec![onset.handle]);

    let (teardown, mut raw, _ended) = bridge.tear_down(control, audio);
    assert!(
        teardown.unoffered.is_empty()
            && teardown.reservations.is_empty()
            && teardown.pedal_lifts.is_empty()
            && teardown.unstaged_pedals.is_empty()
            && teardown.refused_onsets.is_empty()
    );
    let mut remaining = teardown.raw_issued;
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        let frontier = raw
            .owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        remaining.push((source, frontier, None));
    }
    let delivered = publish_serial(&mut raw, remaining);
    let bend_receipt = delivered
        .iter()
        .find(|receipt| receipt.id == split_bend.raw)
        .unwrap();
    assert_eq!(bend_receipt.observation, bend_head.observation());
    assert_eq!(bend_receipt.matched_onset, None);
    for raw_id in [split_bend.raw, onset.handoff.raw, release.handoff.raw] {
        let receipt = delivered
            .iter()
            .find(|receipt| receipt.id == raw_id)
            .unwrap();
        let InputOutcome::Delivered(SessionSourceOutcome::Published(record)) = &receipt.outcome
        else {
            panic!("the raw edge must reach the serial recorder: {receipt:?}");
        };
        assert_eq!(
            record.capture,
            crate::recording::notes::CaptureDisposition::Recorded
        );
    }
    assert_eq!(
        recorded_occurrence(&delivered, onset.handoff.raw),
        recorded_occurrence(&delivered, release.handoff.raw)
    );
}

#[test]
fn six_channels_keep_bend_scope_places_with_a_five_place_command_ring() {
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
    assert_eq!(control.ingress_capacity.as_usize(), 5);
    assert_eq!(control.bend_scopes.len(), 32);
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(2, SerialRaw { owner, generations });
    bridge.frontier(Source::Second, 200);
    let mut issued = Vec::new();
    for (at, original) in (100..106)
        .zip(1..=6)
        .map(|(at, channel)| (at, input(0xe0 + channel - 1, 127, 127)))
        .chain([(106, input(0xe0, 0, 64))])
    {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: original,
            })
            .unwrap()
            .parts();
        let mut pending = None;
        let Some(MixedSourceStep::Staged(head, occurrence)) =
            pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
        else {
            panic!("a bend from an admitted source must take its stage place");
        };
        assert_eq!(head.stamp().queue(), queue);
        let result = bridge
            .service_bend(&mut control, &mut audio)
            .unwrap()
            .unwrap();
        assert_eq!(result.occurrence, occurrence);
        assert_eq!(result.handoff.original, original);
        assert!(matches!(result.outcome, MixedIngressOutcome::Bend(Ok(_))));
        issued.push(result.handoff.raw);
    }
    assert_eq!(issued.len(), 7);
    assert_eq!(control.bend_scopes.iter().flatten().count(), 5);
    for channel in 1..=6 {
        let scope = MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::new(channel));
        assert_eq!(
            audio.occurrences.bend_of(scope),
            if channel == 1 {
                Cents::ZERO
            } else {
                Cents::new(200.0).unwrap()
            }
        );
    }
    assert!(inbox.close(producer).is_ok());
    let (teardown, mut raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unoffered.is_empty() && teardown.reservations.is_empty());
    let mut remaining = teardown.raw_issued;
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        let frontier = raw
            .owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        remaining.push((source, frontier, None));
    }
    let delivered = publish_serial(&mut raw, remaining);
    for raw_id in issued {
        let receipt = delivered
            .iter()
            .find(|receipt| receipt.id == raw_id)
            .unwrap();
        assert!(matches!(
            &receipt.outcome,
            InputOutcome::Delivered(SessionSourceOutcome::Published(record))
                if record.capture == crate::recording::notes::CaptureDisposition::Recorded
        ));
    }
}

#[test]
fn source_pedal_split_allows_the_protected_key_up_and_lift_to_advance() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let raw = [raw_source_with_cells(0, 32), raw_source(1)];
    let generation = raw[0].generation;
    let clock = raw[0].input.clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generation, InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(2, raw);
    for (index, (at, original)) in [
        (100, input(0x90, 60, 100)),
        (101, input(0xb0, 64, 127)),
        (102, input(0x80, 60, 0)),
        (103, input(0xb0, 64, 0)),
    ]
    .into_iter()
    .enumerate()
    {
        let (_, queue) = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: original,
            })
            .unwrap()
            .parts();
        assert_eq!(queue.serial(), u64::try_from(index + 1).unwrap());
    }
    bridge.frontier(Source::Second, 104);
    let mut pending = None;
    let Some(MixedSourceStep::Staged(_, note_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the note owns its protected release place");
    };
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
    let Some(MixedSourceStep::Split(
        down_head,
        BridgeRefused::Lane {
            handoff: down,
            reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
        },
    )) = pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("a pedal-down without lift credit must split and release its source head");
    };
    assert_eq!(
        down_head.observation(),
        InputObservation::Message {
            tick: InputTick::new(101),
            arrival: SampleTime::new(101),
            input: down.original,
        }
    );
    assert!(pending.is_none());
    let Some(MixedSourceStep::Staged(_, release_id)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the protected key-up behind the split down must advance");
    };
    assert_eq!(release_id, note_id);
    let Some(MixedSourceStep::Split(
        _,
        BridgeRefused::Lane {
            handoff: lift,
            reason: LaneRefusal::UnstagedPedal { down_raw },
        },
    )) = pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the raw lift must close its split down without a mixed command");
    };
    assert_eq!(down_raw, down.raw);
    assert_eq!(lift.link, None);
    assert!(pending.is_none());
    assert!(inbox.close(producer).is_ok());
    let onset = bridge.service(&mut control, &mut audio).unwrap();
    assert!(onset.accepted());
    let release = bridge.service(&mut control, &mut audio).unwrap();
    assert!(release.accepted());
    assert_eq!(release.handoff.link, Some(onset.handoff.raw));
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unstaged_pedals.is_empty() && teardown.unoffered.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn quiescent_pedal_down_full_splits_and_redeems_its_staged_lift() {
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
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(4, SerialRaw { owner, generations });
    bridge.frontier(Source::First, 100);
    for (source, at, status) in [
        (Source::Second, 90, 0xb0),
        (Source::Second, 91, 0xb1),
        (Source::First, 110, 0xb0),
        (Source::First, 111, 0xb1),
    ] {
        if source == Source::First && at == 110 {
            bridge.frontier(Source::Second, 200);
        }
        let occurrence = bridge.offer(source, at, input(status, 64, 127)).unwrap();
        let down = bridge
            .service_pedal(&mut control, &mut audio)
            .unwrap()
            .unwrap();
        assert_eq!(down.occurrence, occurrence);
        assert!(matches!(
            down.outcome,
            Some(MixedIngressOutcome::PedalDown(Ok(_)))
        ));
    }
    assert_eq!(bridge.stage.pressure(Source::First), (0, 2));
    assert_eq!(bridge.stage.pressure(Source::Second), (0, 2));
    assert_eq!(control.ingress_room(), 1);
    assert_eq!(
        control.ingress_outstanding,
        super::super::MixedIngressQueueCount::NONE
    );
    for (at, value) in [(120, 127), (121, 0)] {
        let _ = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: input(0xb2, 64, value),
            })
            .unwrap();
    }
    let mut pending = None;
    let Some(MixedSourceStep::Staged(_, down)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the effective pedal-down must reserve its stage lift place");
    };
    let Some(MixedSourceStep::Staged(_, lift)) =
        pump_concrete_mixed(&mut bridge, &mut inbox, &mut pending)
    else {
        panic!("the lift must redeem the stage reservation");
    };
    assert_ne!(down, lift);
    assert_eq!(bridge.stage.pressure(Source::First), (2, 2));
    let selected = bridge.stage.peek_next();
    assert_eq!(
        selected.map(|(source, packet)| (source, packet.occurrence)),
        Some((Source::First, down))
    );
    let last_id = control.last_ingress_id;
    assert_eq!(
        bridge.service_pedal(&mut control, &mut audio),
        Err(MixedPedalSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next(), selected);
    assert_eq!(control.last_ingress_id, last_id);
    audio.service_test_ingress_queue();
    assert!(control.collect_ingress_result().is_none());
    assert_eq!(
        bridge.service_pedal(&mut control, &mut audio),
        Err(MixedPedalSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next(), selected);
    let reserved = control.pedal_ledger.reserved();
    let redundant = control.pedal_ledger.redundant();
    let room = control.ingress_room();
    let Some(PedalDecision::Refused(split_down)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("the quiescent Full must split the selected protected down");
    };
    assert_eq!(split_down.occurrence, down);
    assert_eq!(
        split_down.disposition,
        SplitDisposition::PedalDown {
            reason: MixedPedalSubmitError::Full
        }
    );
    assert_eq!(bridge.split_pedals.len(), 1);
    assert_eq!(control.last_ingress_id, last_id);
    assert_eq!(control.pedal_ledger.reserved(), reserved);
    assert_eq!(control.pedal_ledger.redundant(), redundant);
    assert_eq!(control.ingress_room(), room);
    let Some(PedalDecision::Refused(split_lift)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("the reserved lift must close its split down");
    };
    assert_eq!(split_lift.occurrence, lift);
    assert_eq!(
        split_lift.disposition,
        SplitDisposition::PedalLiftAfterDown {
            down_raw: split_down.handoff.raw
        }
    );
    assert_ne!(split_lift.handoff.raw, split_down.handoff.raw);
    assert!(bridge.split_pedals.is_empty());
    assert_eq!(control.last_ingress_id, last_id);
    assert_eq!(control.pedal_ledger.reserved(), reserved);
    assert_eq!(control.pedal_ledger.redundant(), redundant);
    assert_eq!(control.ingress_room(), room);
    let split_scope = MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::new(3));
    assert!(!control.pedal_ledger.is_down(split_scope));
    assert!(!audio.occurrences.pedals_down().contains(&split_scope));
    assert_eq!(bridge.stage.pressure(Source::First), (0, 2));
    assert!(inbox.close(producer).is_ok());
    let (teardown, mut raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unoffered.is_empty());
    assert!(teardown.staged_pedal_lifts.is_empty());
    assert!(teardown.split_pedals.is_empty());
    assert_eq!(teardown.pedal_lifts.len(), 4);
    let mut remaining = teardown.raw_issued;
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        let frontier = raw
            .owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        remaining.push((source, frontier, None));
    }
    let delivered = publish_serial(&mut raw, remaining);
    assert_eq!(
        delivered
            .iter()
            .filter(|receipt| matches!(receipt.observation, InputObservation::Message { .. }))
            .count(),
        6
    );
}

#[test]
fn in_flight_pedal_full_retains_selected_head_until_quiescent_split() {
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
    for channel in 10..13 {
        let scope = MixedLiveScope::new(MixedLiveSource::new(2), MidiChannel::new(channel));
        assert!(matches!(
            control
                .submit_pedal(
                    MixedIngressOriginId(u64::from(channel)),
                    scope,
                    SampleTime::new(90),
                    true
                )
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        audio.service_test_ingress_queue();
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    let fourth = MixedLiveScope::new(MixedLiveSource::new(2), MidiChannel::new(13));
    assert!(matches!(
        control
            .submit_pedal(MixedIngressOriginId(13), fourth, SampleTime::new(95), true)
            .unwrap(),
        MixedPedalSubmitted::Command(..)
    ));
    assert_eq!(control.ingress_room(), 0);
    assert_ne!(
        control.ingress_outstanding,
        super::super::MixedIngressQueueCount::NONE
    );
    let mut bridge = RawStageBridge::new(2);
    bridge.frontier(Source::Second, 200);
    let down = bridge
        .offer(Source::First, 120, input(0xb2, 64, 127))
        .unwrap();
    let selected = bridge.stage.peek_next();
    let handoffs = bridge.handoffs.clone();
    let last_id = control.last_ingress_id;
    let reserved = control.pedal_ledger.reserved();
    let redundant = control.pedal_ledger.redundant();
    assert_eq!(
        bridge.service_pedal_or_refuse_full(&mut control, &mut audio),
        Err(MixedPedalSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next(), selected);
    assert_eq!(bridge.handoffs, handoffs);
    assert!(bridge.split_pedals.is_empty());
    assert_eq!(control.last_ingress_id, last_id);
    assert_eq!(control.pedal_ledger.reserved(), reserved);
    assert_eq!(control.pedal_ledger.redundant(), redundant);
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::PedalDown(Ok(_))
    ));
    let Some(PedalDecision::Refused(split)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("quiescent Full must split the retained down");
    };
    assert_eq!(split.occurrence, down);
    assert_eq!(
        split.disposition,
        SplitDisposition::PedalDown {
            reason: MixedPedalSubmitError::Full
        }
    );
    assert_eq!(bridge.split_pedals.len(), 1);
}

#[test]
fn split_pedal_latches_fill_only_the_prepared_stage_bound() {
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
    for channel in 10..14 {
        let scope = MixedLiveScope::new(MixedLiveSource::new(3), MidiChannel::new(channel));
        assert!(matches!(
            control
                .submit_pedal(
                    MixedIngressOriginId(u64::from(channel)),
                    scope,
                    SampleTime::new(90),
                    true
                )
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        audio.service_test_ingress_queue();
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    assert_eq!(control.ingress_room(), 1);
    let mut bridge = RawStageBridge::new(4);
    bridge.frontier(Source::Second, 115);
    for (source, base) in [(Source::First, 110), (Source::Second, 120)] {
        if source == Source::Second {
            bridge.frontier(Source::First, 200);
        }
        for channel in 0..3 {
            let at = base + u64::from(channel);
            let original = input(0xb0 + channel, 64, 127);
            let occurrence = bridge.offer(source, at, original).unwrap();
            let Some(PedalDecision::Refused(split)) = bridge
                .service_pedal_or_refuse_full(&mut control, &mut audio)
                .unwrap()
            else {
                panic!("each protected down must split at quiescent Full");
            };
            assert_eq!(split.occurrence, occurrence);
            assert_eq!(split.handoff.original, original);
        }
        assert_eq!(bridge.stage.pressure(source), (0, 3));
    }
    assert_eq!(
        bridge.split_pedals.len(),
        2 * (bridge.stage.cells_per_source - 1)
    );
    assert_eq!(control.ingress_room(), 1);
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert_eq!(teardown.split_pedals.len(), 6);
    assert_eq!(teardown.pedal_lifts.len(), 6);
    assert!(teardown.unoffered.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(issued.len(), 8); // Six pedal messages and two ordering frontiers.
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn split_pedal_latch_redeems_a_later_lift_then_allows_a_new_scope() {
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
    for channel in 10..14 {
        let scope = MixedLiveScope::new(MixedLiveSource::new(3), MidiChannel::new(channel));
        assert!(matches!(
            control
                .submit_pedal(
                    MixedIngressOriginId(u64::from(channel)),
                    scope,
                    SampleTime::new(90),
                    true
                )
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        audio.service_test_ingress_queue();
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    let mut bridge = RawStageBridge::new(2);
    bridge.frontier(Source::Second, 200);
    let last_id = control.last_ingress_id;
    let down = bridge
        .offer(Source::First, 120, input(0xb2, 64, 127))
        .unwrap();
    let Some(PedalDecision::Refused(split)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("the protected down must split");
    };
    assert_eq!(split.occurrence, down);
    assert_eq!(bridge.stage.pressure(Source::First), (0, 1));
    let repeat = bridge
        .offer(Source::First, 121, input(0xb2, 64, 127))
        .unwrap();
    let Some(PedalDecision::Refused(repeated)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("a later repeat must use the split latch");
    };
    assert_eq!(repeated.occurrence, repeat);
    assert_eq!(
        repeated.disposition,
        SplitDisposition::RepeatedPedalDown {
            down_raw: split.handoff.raw
        }
    );
    assert_eq!(bridge.stage.pressure(Source::First), (0, 1));
    let lift = bridge
        .offer(Source::First, 122, input(0xb2, 64, 0))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (1, 0));
    let Some(PedalDecision::Refused(lifted)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("a later lift must redeem the split down's stage place");
    };
    assert_eq!(lifted.occurrence, lift);
    assert_eq!(
        lifted.disposition,
        SplitDisposition::PedalLiftAfterDown {
            down_raw: split.handoff.raw
        }
    );
    assert!(bridge.split_pedals.is_empty());
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    assert_eq!(control.ingress_room(), 1);
    assert_eq!(control.last_ingress_id, last_id);
    let split_scope = MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::new(3));
    assert!(!control.pedal_ledger.is_down(split_scope));
    assert!(!audio.occurrences.pedals_down().contains(&split_scope));
    let prior = MixedLiveScope::new(MixedLiveSource::new(3), MidiChannel::new(10));
    assert!(matches!(
        control
            .submit_pedal(
                MixedIngressOriginId(100),
                prior,
                SampleTime::new(122),
                false
            )
            .unwrap(),
        MixedPedalSubmitted::Command(..)
    ));
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::Lift(Ok(_))
    ));
    assert_eq!(control.ingress_room(), 2);
    let admitted = bridge
        .offer(Source::First, 123, input(0xb3, 64, 127))
        .unwrap();
    let Some(PedalDecision::Admitted(result)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("a new scope must reach mixed after room returns");
    };
    assert_eq!(result.occurrence, admitted);
    assert!(matches!(
        result.outcome,
        Some(MixedIngressOutcome::PedalDown(Ok(_)))
    ));
    let admitted_raw = result.handoff.raw;
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.split_pedals.is_empty());
    assert_eq!(
        teardown.pedal_lifts,
        vec![(Source::First, MidiChannel::new(4), admitted, admitted_raw)]
    );
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn split_pedal_latch_consumes_staged_repeat_after_peer_frees_command_room() {
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
    let mut bridge = RawStageBridge::new(5);
    bridge.frontier(Source::First, 100);
    for (source, at, status) in [
        (Source::Second, 90, 0xb0),
        (Source::Second, 91, 0xb1),
        (Source::First, 110, 0xb0),
        (Source::First, 111, 0xb1),
    ] {
        if source == Source::First && at == 110 {
            bridge.frontier(Source::Second, 115);
        }
        bridge.offer(source, at, input(status, 64, 127)).unwrap();
        assert!(matches!(
            bridge.service_pedal(&mut control, &mut audio).unwrap(),
            Some(PedalCombinedResult {
                outcome: Some(MixedIngressOutcome::PedalDown(Ok(_))),
                ..
            })
        ));
    }
    assert_eq!(control.ingress_room(), 1);
    let split_occurrence = bridge
        .offer(Source::First, 120, input(0xb2, 64, 127))
        .unwrap();
    let repeat_occurrence = bridge
        .offer(Source::First, 122, input(0xb2, 64, 127))
        .unwrap();
    let lift_occurrence = bridge
        .offer(Source::First, 123, input(0xb2, 64, 0))
        .unwrap();
    let peer_lift_occurrence = bridge
        .offer(Source::Second, 121, input(0xb0, 64, 0))
        .unwrap();
    let peer_same_channel_down = bridge
        .offer(Source::Second, 121, input(0xb2, 64, 127))
        .unwrap();
    let peer_same_channel_lift = bridge
        .offer(Source::Second, 121, input(0xb2, 64, 0))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (3, 2));
    let Some(PedalDecision::Refused(split_down)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("quiescent Full splits the protected down before its repeat");
    };
    assert_eq!(split_down.occurrence, split_occurrence);
    assert_eq!(
        split_down.disposition,
        SplitDisposition::PedalDown {
            reason: MixedPedalSubmitError::Full
        }
    );
    assert_eq!(bridge.split_pedals.len(), 1);
    let peer_lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(peer_lift.occurrence, peer_lift_occurrence);
    assert!(matches!(
        peer_lift.outcome,
        Some(MixedIngressOutcome::Lift(Ok(_)))
    ));
    assert_eq!(control.ingress_room(), 2);
    let peer_down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(peer_down.occurrence, peer_same_channel_down);
    assert!(matches!(
        peer_down.outcome,
        Some(MixedIngressOutcome::PedalDown(Ok(_)))
    ));
    let peer_same_lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(peer_same_lift.occurrence, peer_same_channel_lift);
    assert_eq!(peer_same_lift.paired_down_raw, Some(peer_down.handoff.raw));
    assert!(matches!(
        peer_same_lift.outcome,
        Some(MixedIngressOutcome::Lift(Ok(_)))
    ));
    assert_eq!(bridge.split_pedals.len(), 1);
    assert_eq!(control.ingress_room(), 2);
    bridge.frontier(Source::Second, 200);
    let last_id = control.last_ingress_id;
    let reserved = control.pedal_ledger.reserved();
    let redundant = control.pedal_ledger.redundant();
    let Some(PedalDecision::Refused(repeat)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("the staged repeat must use its split latch after room returns");
    };
    assert_eq!(repeat.occurrence, repeat_occurrence);
    assert_eq!(
        repeat.disposition,
        SplitDisposition::RepeatedPedalDown {
            down_raw: split_down.handoff.raw
        }
    );
    assert_eq!(control.last_ingress_id, last_id);
    assert_eq!(control.ingress_room(), 2);
    assert_eq!(control.pedal_ledger.reserved(), reserved);
    assert_eq!(control.pedal_ledger.redundant(), redundant);
    let Some(PedalDecision::Refused(lift)) = bridge
        .service_pedal_or_refuse_full(&mut control, &mut audio)
        .unwrap()
    else {
        panic!("the split down's lift must close the latch at its head");
    };
    assert_eq!(lift.occurrence, lift_occurrence);
    assert_eq!(
        lift.disposition,
        SplitDisposition::PedalLiftAfterDown {
            down_raw: split_down.handoff.raw
        }
    );
    assert!(bridge.split_pedals.is_empty());
    assert_eq!(control.last_ingress_id, last_id);
    assert_eq!(control.ingress_room(), 2);
    assert_eq!(control.pedal_ledger.reserved(), reserved);
    assert_eq!(control.pedal_ledger.redundant(), redundant);
    let scope = MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::new(3));
    assert!(!control.pedal_ledger.is_down(scope));
    assert!(!audio.occurrences.pedals_down().contains(&scope));
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unoffered.is_empty() && teardown.split_pedals.is_empty());
    assert_eq!(teardown.pedal_lifts.len(), 3);
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn split_pedal_teardown_names_either_reserved_or_staged_lift() {
    for lift_staged in [false, true] {
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
        let mut bridge = RawStageBridge::new(4);
        bridge.frontier(Source::First, 100);
        for (source, at, status) in [
            (Source::Second, 90, 0xb0),
            (Source::Second, 91, 0xb1),
            (Source::First, 110, 0xb0),
            (Source::First, 111, 0xb1),
        ] {
            if source == Source::First && at == 110 {
                bridge.frontier(Source::Second, 200);
            }
            bridge.offer(source, at, input(status, 64, 127)).unwrap();
            assert!(matches!(
                bridge.service_pedal(&mut control, &mut audio).unwrap(),
                Some(PedalCombinedResult {
                    outcome: Some(MixedIngressOutcome::PedalDown(Ok(_))),
                    ..
                })
            ));
        }
        let down = bridge
            .offer(Source::First, 120, input(0xb2, 64, 127))
            .unwrap();
        if lift_staged {
            bridge
                .offer(Source::First, 121, input(0xb2, 64, 0))
                .unwrap();
        }
        let Some(PedalDecision::Refused(split)) = bridge
            .service_pedal_or_refuse_full(&mut control, &mut audio)
            .unwrap()
        else {
            panic!("the quiescent down must split before teardown");
        };
        assert_eq!(split.occurrence, down);
        let latch = SplitPedalLatch {
            source: Source::First,
            channel: MidiChannel::new(3),
            occurrence: down,
            down_raw: split.handoff.raw,
        };
        assert_eq!(bridge.split_pedals, vec![latch]);
        let (teardown, raw, _ended) = bridge.tear_down(control, audio);
        let pending = if lift_staged {
            assert_eq!(teardown.unoffered.len(), 1);
            let lift_raw = teardown.unoffered[0].raw;
            assert_eq!(
                teardown.staged_pedal_lifts,
                vec![(lift_raw, latch.down_raw)]
            );
            assert_eq!(teardown.pedal_lifts.len(), 4);
            assert_eq!(teardown.unoffered[0].original, input(0xb2, 64, 0));
            SplitPedalPending::StagedLift(lift_raw)
        } else {
            assert!(teardown.unoffered.is_empty());
            assert_eq!(teardown.pedal_lifts.len(), 5);
            SplitPedalPending::ReservedLift
        };
        assert_eq!(teardown.split_pedals, vec![(latch, pending)]);
        let issued = teardown.raw_issued.clone();
        assert_eq!(
            settle_raw_by_cancellation(raw, &issued).bridge.len(),
            issued.len()
        );
    }
}

#[test]
fn actual_source_key_up_waits_behind_an_onset_refused_by_open_handle_reservations() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(20)),
        )
        .unwrap();
    // Direct private pedal commands create the pressure; the source carries
    // only the ordered notes and key-up tested below.
    for channel in 1..=16 {
        let scope = MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::new(channel));
        assert!(matches!(
            control
                .submit_pedal(MixedIngressOriginId(1), scope, SampleTime::new(100), true)
                .unwrap(),
            MixedPedalSubmitted::Command(..)
        ));
        audio.service_test_ingress_queue();
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));
    }
    let (owner, generations) = ledger_tests::bridge_serial_owner_with_first_limits(
        256,
        InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
        InputCapacity::new(64).unwrap(),
    );
    let clock = owner.input(generations[0]).unwrap().clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(8, SerialRaw { owner, generations });
    bridge.frontier(Source::Second, 200);
    let mut queue_ids = Vec::new();
    for (at, original) in [
        (110, input(0x90, 60, 100)),
        (111, input(0x90, 61, 100)),
        (112, input(0x90, 62, 100)),
        (113, input(0x90, 63, 100)),
        (114, input(0x80, 60, 0)),
    ] {
        let accepted = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: original,
            })
            .unwrap();
        queue_ids.push(accepted.parts().1);
    }
    assert_eq!(inbox.occupied_len(), 5);
    let scope = MixedLiveScope::new(MixedLiveSource::new(1), MidiChannel::CH1);
    let mut first_raw = None;
    let mut first_occurrence = None;
    let mut onset_raw = Vec::new();
    let mut onset_occurrences = Vec::new();
    for index in 0..3 {
        let (head, occurrence) = pump_concrete_note(&mut bridge, &mut inbox);
        assert_eq!(head.stamp().queue(), queue_ids[index]);
        assert_eq!(
            head.observation(),
            InputObservation::Message {
                tick: InputTick::new(110 + index as u64),
                arrival: SampleTime::new(110 + index as u64),
                input: input(0x90, 60 + u8::try_from(index).unwrap(), 100),
            }
        );
        let result = bridge
            .try_service_onset_in_scope(&mut control, &mut audio, scope)
            .unwrap()
            .unwrap();
        assert_eq!(result.occurrence, occurrence);
        onset_raw.push(result.handoff.raw);
        onset_occurrences.push(occurrence);
        assert!(
            bridge
                .raw_issued
                .iter()
                .any(|&(source, raw, _)| source == Source::First && raw == result.handoff.raw)
        );
        assert_eq!(
            head.stamp().mapped_at(),
            SampleTime::new(110 + index as u64)
        );
        if index == 0 {
            first_raw = Some(result.handoff.raw);
            first_occurrence = Some(occurrence);
        }
        if index < 2 {
            assert!(matches!(
                result.disposition,
                MixedDisposition::Result(MixedIngressOutcome::Onset(Ok(_)))
            ));
        } else {
            assert_eq!(
                result.disposition,
                MixedDisposition::Result(MixedIngressOutcome::Onset(Err(
                    IngressRefused::Dropped {
                        resource: ExhaustedResource::Hold
                    }
                )))
            );
        }
    }
    let (blocked_head, blocked) = pump_concrete_note(&mut bridge, &mut inbox);
    assert_eq!(blocked_head.stamp().queue(), queue_ids[3]);
    assert_eq!(
        blocked_head.observation(),
        InputObservation::Message {
            tick: InputTick::new(113),
            arrival: SampleTime::new(113),
            input: input(0x90, 63, 100),
        }
    );
    assert_eq!(inbox.occupied_len(), 1);
    assert_eq!(control.ingress_room(), 1);
    assert_eq!(control.ingress_outstanding.as_usize(), 0);
    assert_eq!(control.release_reserved.as_usize(), 3);
    assert_eq!(control.pedal_ledger.reserved(), 16);
    assert_eq!(
        audio.test_ingress.holds_outstanding(),
        EventCount::measured(2)
    );
    let staged = bridge.stage.peek_next();
    let command = control.last_ingress_id;
    assert_eq!(
        bridge.try_service_onset_in_scope(&mut control, &mut audio, scope),
        Err(MixedOnsetSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next(), staged);
    assert_eq!(control.last_ingress_id, command);
    let (release_head, released) = pump_concrete_note(&mut bridge, &mut inbox);
    assert_eq!(release_head.stamp().queue(), queue_ids[4]);
    assert_eq!(
        release_head.observation(),
        InputObservation::Message {
            tick: InputTick::new(114),
            arrival: SampleTime::new(114),
            input: input(0x80, 60, 0),
        }
    );
    assert_eq!(Some(released), first_occurrence);
    assert_eq!(release_head.stamp().mapped_at(), SampleTime::new(114));
    assert_eq!(bridge.handoffs.last().unwrap().2.link, first_raw);
    assert!(inbox.peek_handoff().is_none());
    bridge.frontier(Source::First, 201);
    assert_eq!(bridge.stage.pressure(Source::First), (2, 3));
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, blocked);
    assert_eq!(blocked_head.stamp().mapped_at(), SampleTime::new(113));
    audio.service_test_ingress_queue();
    assert!(control.collect_ingress_result().is_none());
    assert_eq!(
        bridge.try_service_onset_in_scope(&mut control, &mut audio, scope),
        Err(MixedOnsetSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, blocked);
    assert!(inbox.close(producer).is_ok());
    let blocked_raw = bridge
        .handoffs
        .iter()
        .find(|&&(occurrence, release, _)| occurrence == blocked && !release)
        .unwrap()
        .2
        .raw;
    let (teardown, mut raw, ended) = bridge.tear_down(control, audio);
    assert_eq!(teardown.raw_issued.len(), 7);
    assert_eq!(teardown.unoffered.len(), 2);
    assert_eq!(teardown.unoffered[0].original, input(0x90, 63, 100));
    assert_eq!(teardown.unoffered[0].raw, blocked_raw);
    assert_eq!(teardown.unoffered[0].link, None);
    assert_eq!(teardown.unoffered[1].original, input(0x80, 60, 0));
    assert_eq!(teardown.unoffered[1].link, first_raw);
    assert_eq!(teardown.reservations.len(), 3);
    assert!(
        teardown
            .reservations
            .contains(&(Source::First, onset_occurrences[1], onset_raw[1]))
    );
    assert!(
        teardown
            .reservations
            .contains(&(Source::First, onset_occurrences[2], onset_raw[2]))
    );
    assert!(
        teardown
            .reservations
            .contains(&(Source::First, blocked, blocked_raw))
    );
    assert_eq!(teardown.onsets.len(), 3);
    assert!(
        matches!(teardown.onsets[0], (id, OnsetAtTeardown::Live { location: OnsetLocation::Queued, .. }) if id == onset_occurrences[0])
    );
    assert!(
        matches!(teardown.onsets[1], (id, OnsetAtTeardown::Live { location: OnsetLocation::Queued, .. }) if id == onset_occurrences[1])
    );
    assert_eq!(
        teardown.onsets[2],
        (
            onset_occurrences[2],
            OnsetAtTeardown::Refused(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold,
            }),
        )
    );
    assert_eq!(ended.end, MixedCollectionEnd::Pending);
    assert_eq!(ended.ingress.holds_outstanding, EventCount::measured(2));
    let issued_count = teardown.raw_issued.len();
    let mut remaining = teardown.raw_issued;
    raw.owner.pump().unwrap();
    let mut samples = [0.0_f32; 512];
    raw.owner
        .render(AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    raw.owner.pump().unwrap();
    let mut settled = 0;
    let mut prepared_frontiers = Vec::new();
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        while let Some(receipt) = raw.owner.collect_input(generation).unwrap() {
            if let Some(position) = remaining
                .iter()
                .position(|&(owner, id, _)| owner == source && id == receipt.id)
            {
                let (_, _, link) = remaining.swap_remove(position);
                assert_eq!(receipt.matched_onset, link);
                match (source, receipt.observation) {
                    (_, InputObservation::Message { .. })
                    | (Source::Second, InputObservation::Frontier { .. }) => {
                        assert!(matches!(receipt.outcome, InputOutcome::Delivered(_)));
                    }
                    (Source::First, InputObservation::Frontier { tick }) => {
                        assert_eq!(tick, InputTick::new(201));
                        assert!(matches!(receipt.outcome, InputOutcome::Cancelled));
                    }
                }
                settled += 1;
            } else {
                assert_eq!(
                    receipt.observation,
                    InputObservation::Frontier {
                        tick: InputTick::new(0)
                    }
                );
                assert!(matches!(receipt.outcome, InputOutcome::Delivered(_)));
                prepared_frontiers.push(source);
            }
        }
    }
    assert!(remaining.is_empty());
    assert_eq!(settled, issued_count);
    assert_eq!(prepared_frontiers, vec![Source::First, Source::Second]);
}

#[test]
fn actual_source_split_refusal_releases_its_same_key_head_without_mixed_identity() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(3)),
        )
        .unwrap();
    let scope = super::super::TEST_SCOPE;
    let pedal = control
        .submit_pedal(MixedIngressOriginId(1), scope, SampleTime::new(100), true)
        .unwrap();
    assert!(matches!(pedal, MixedPedalSubmitted::Command(..)));
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::PedalDown(Ok(_))
    ));

    let raw = [raw_source_with_cells(0, 32), raw_source(1)];
    let generation = raw[0].generation;
    let clock = raw[0].input.clock().unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generation, InputCaptureHalt::detached_for_test(), clock);
    let mut bridge = RawStageBridge::with_raw(4, raw);
    bridge.frontier(Source::Second, 200);
    let mut queue_ids = Vec::new();
    for (at, original) in [
        (110, input(0x90, 60, 100)),
        (111, input(0x90, 60, 100)),
        (112, input(0x80, 60, 0)),
        (113, input(0x80, 60, 0)),
    ] {
        let accepted = producer
            .submit_owned(InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: original,
            })
            .unwrap();
        queue_ids.push(accepted.parts().1);
    }
    assert_eq!(inbox.occupied_len(), 4);

    let (first_head, first) = pump_concrete_note(&mut bridge, &mut inbox);
    assert_eq!(first_head.stamp().queue(), queue_ids[0]);
    let admitted = bridge
        .service_onset_or_refuse_full(&mut control, &mut audio, scope)
        .unwrap()
        .unwrap();
    let OnsetDecision::Admitted(admitted) = admitted else {
        panic!("the first same-key onset has room");
    };
    assert_eq!(admitted.occurrence, first);
    assert_eq!(admitted.handoff.original, input(0x90, 60, 100));
    assert_eq!(admitted.handoff.at, first_head.stamp().mapped_at());
    assert!(matches!(
        admitted.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Onset(Ok(_)))
    ));
    let (second_head, second) = pump_concrete_note(&mut bridge, &mut inbox);
    assert_eq!(second_head.stamp().queue(), queue_ids[1]);
    let (first_up_head, first_up) = pump_concrete_note(&mut bridge, &mut inbox);
    let (second_up_head, second_up) = pump_concrete_note(&mut bridge, &mut inbox);
    assert_eq!(first_up_head.stamp().queue(), queue_ids[2]);
    assert_eq!(second_up_head.stamp().queue(), queue_ids[3]);
    assert_eq!((first_up, second_up), (first, second));
    assert_eq!(bridge.handoffs[1].2.link, Some(admitted.handoff.raw));
    assert_eq!(bridge.handoffs[2].2.link, Some(bridge.handoffs[0].2.raw));
    assert_eq!(bridge.stage.pressure(Source::First), (3, 0));
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, second);
    assert!(bridge.service_refused_key_up().is_none());
    assert_eq!(
        (
            control.ingress_room(),
            control.ingress_outstanding.as_usize()
        ),
        (1, 0)
    );
    let command = control.last_ingress_id;
    let held = control.occurrence_ledger.held();
    let split = bridge
        .service_onset_or_refuse_full(&mut control, &mut audio, scope)
        .unwrap()
        .unwrap();
    let OnsetDecision::Refused(split) = split else {
        panic!("the selected second onset has no command place");
    };
    assert_eq!(split.occurrence, second);
    assert_eq!(
        split.disposition,
        SplitDisposition::Onset {
            reason: MixedOnsetSubmitError::Full,
        }
    );
    assert_eq!(control.last_ingress_id, command);
    assert_eq!(control.occurrence_ledger.held(), held);
    assert_eq!(split.handoff.original, input(0x90, 60, 100));
    assert_eq!(split.handoff.at, second_head.stamp().mapped_at());
    assert_eq!(bridge.handoffs.len(), 2);
    assert_eq!(bridge.refused_onsets[0].onset_raw, split.handoff.raw);
    assert_eq!(bridge.handoffs[0].2.link, Some(admitted.handoff.raw));
    assert_eq!(bridge.handoffs[1].2.link, Some(split.handoff.raw));
    assert_eq!(bridge.handoffs[0].2.original, input(0x80, 60, 0));
    assert_eq!(bridge.handoffs[0].2.at, first_up_head.stamp().mapped_at());
    assert_eq!(bridge.handoffs[1].2.original, input(0x80, 60, 0));
    assert_eq!(bridge.handoffs[1].2.at, second_up_head.stamp().mapped_at());
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
    assert_eq!(bridge.stage.peek_next().unwrap().1.occurrence, first);
    assert!(bridge.service_refused_key_up().is_none());
    let first_result = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(first_result.occurrence, first);
    assert_eq!(first_result.handle, admitted.handle);
    assert_eq!(first_result.handoff.link, Some(admitted.handoff.raw));
    assert_eq!(
        first_result.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Sustained(admitted.handle))
    );
    let second_result = bridge.service_refused_key_up().unwrap();
    assert_eq!(second_result.occurrence, second);
    assert_eq!(second_result.handoff.link, Some(split.handoff.raw));
    assert_eq!(
        second_result.disposition,
        SplitDisposition::KeyUpAfterOnset {
            onset_raw: split.handoff.raw,
            reason: MixedOnsetSubmitError::Full,
        }
    );
    assert!(bridge.refused_onsets.is_empty());
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    assert!(inbox.peek_handoff().is_none());
    assert!(inbox.close(producer).is_ok());

    let (teardown, raw, ended) = bridge.tear_down(control, audio);
    assert_eq!(ended.end, MixedCollectionEnd::Pending);
    assert!(teardown.unoffered.is_empty());
    assert!(teardown.reservations.is_empty());
    assert!(teardown.onsets.is_empty());
    assert!(teardown.refused_onsets.is_empty());
    let issued = teardown.raw_issued.clone();
    let settlement = settle_raw_by_cancellation(raw, &issued);
    assert_eq!(settlement.bridge.len(), issued.len());
}
