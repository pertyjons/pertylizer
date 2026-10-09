//! Private source-to-audio bend custody through raw admission and a bounded stage.

use super::*;
use crate::quantities::Cents;
use synth_core::MidiChannel;

use super::super::occurrence::{MixedBendSubmitError, MixedLiveScope, MixedLiveSource};

struct BendBridge {
    raw: [RawSource; 2],
    source_heads: [VecDeque<Packet>; 2],
    raw_pending: [Option<RawHandoff>; 2],
    staged: Vec<(Occurrence, RawHandoff)>,
    stage: Stage,
    next_occurrence: u64,
}

impl BendBridge {
    fn new(cells_per_source: usize) -> Self {
        Self {
            raw: [raw_source_with_cells(0, 128), raw_source_with_cells(1, 128)],
            source_heads: [VecDeque::new(), VecDeque::new()],
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
        assert!(matches!(original.event(), Midi1Event::PitchBend { .. }));
        let heads = &mut self.source_heads[source.index()];
        assert!(heads.len() < 32, "the fixture's source ring is bounded");
        assert!(
            heads
                .back()
                .is_none_or(|tail| tail.at <= SampleTime::new(at))
        );
        let occurrence = Occurrence(self.next_occurrence);
        self.next_occurrence += 1;
        heads.push_back(Packet {
            occurrence,
            original,
            at: SampleTime::new(at),
            release: false,
        });
        occurrence
    }

    /// Raw acceptance is remembered even when the stage refuses its packet.
    fn pump(&mut self, source: Source) -> Result<Option<RawHandoff>, StageRefusal> {
        let index = source.index();
        let Some(packet) = self.source_heads[index].front().copied() else {
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
        assert_eq!(self.source_heads[index].pop_front(), Some(packet));
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
    assert_eq!(bridge.source_heads[0].front().unwrap().occurrence, last);
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
