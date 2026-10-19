//! Crate-internal ADR-0078 transfer fixtures: what the opaque public API cannot forge or
//! practically exhaust, and allocation around the complete callback.

use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use synth_core::MidiChannel;

use super::super::{
    MixedAudioQueues, MixedBendScopeCell, MixedIngressCommand, MixedIngressCommandId,
    MixedIngressOriginId, MixedIngressQueueCount, MixedIngressRequest, history_tests,
};
use super::*;
use crate::{
    quantities::{ChannelLayout, KeyIdentity, NoteVelocity},
    render::AudioBlockMut,
    render_allocation::count_allocs,
    time::FrameCount,
};

/// A preallocated lock-free host ring. It can report a larger capacity than it has, and
/// its consumer can hide its packets from the first `hidden_pops` pops.
pub(super) struct Ring<T> {
    ring: HeapRb<T>,
    reported_capacity: Option<usize>,
    hidden_pops: usize,
}

impl<T> Ring<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            ring: HeapRb::new(capacity),
            reported_capacity: None,
            hidden_pops: 0,
        }
    }
}

pub(super) struct Prod<T>(HeapProd<T>);

pub(super) struct Cons<T> {
    ring: HeapCons<T>,
    hidden_pops: usize,
}

impl<T> MixedTransferQueue<T> for Ring<T> {
    type Producer = Prod<T>;
    type Consumer = Cons<T>;

    fn capacity(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(
            self.reported_capacity
                .unwrap_or_else(|| self.ring.capacity().get()),
        )
    }

    fn occupied(&self) -> MixedTransferPlaces {
        MixedTransferPlaces::new(self.ring.occupied_len())
    }

    fn split(self) -> (Prod<T>, Cons<T>) {
        let (producer, consumer) = self.ring.split();
        (
            Prod(producer),
            Cons {
                ring: consumer,
                hidden_pops: self.hidden_pops,
            },
        )
    }
}

impl<T> MixedTransferProducer<T> for Prod<T> {
    fn try_push(&mut self, packet: T) -> Result<(), T> {
        self.0.try_push(packet)
    }
}

impl<T> MixedTransferConsumer<T> for Cons<T> {
    fn try_pop(&mut self) -> Option<T> {
        if self.hidden_pops > 0 {
            self.hidden_pops -= 1;
            return None;
        }
        self.ring.try_pop()
    }
}

pub(super) type Armed = MixedTransferArmed<Ring<MixedTransferCommand>, Ring<MixedTransferResult>>;

fn profile() -> HostProfile {
    history_tests::mixed_profile()
}

fn depth() -> usize {
    MixedJoinedPrepared::transfer_capacity(&profile()).get()
}

fn arm_with(commands: Ring<MixedTransferCommand>, results: Ring<MixedTransferResult>) -> Armed {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    prepared
        .arm_transfer_rehearsal(candidate, &profile(), commands, results)
        .expect("transfer arms")
}

fn arm() -> Armed {
    arm_with(Ring::new(depth()), Ring::new(depth()))
}

fn channel(number: u8) -> MidiChannel {
    MidiChannel::new(number)
}

pub(super) fn note_on(frames: u64, raw: u8) -> MixedSourceNoteOn {
    MixedSourceNoteOn {
        at: SampleTime::new(frames),
        channel: MidiChannel::CH1,
        key: KeyIdentity::new(raw).expect("key"),
        velocity: NoteVelocity::FULL,
    }
}

fn bend_on(frames: u64, on: MidiChannel, cents: f32) -> MixedSourceBend {
    MixedSourceBend {
        at: SampleTime::new(frames),
        channel: on,
        cents: Cents::new(cents).expect("finite"),
    }
}

pub(super) fn callback<C, R>(
    audio: &mut MixedTransferAudio,
    port: &mut MixedTransferAudioPort<C, R>,
    samples: &mut [f32],
) -> Result<(), MixedTransferRenderError>
where
    C: MixedTransferConsumer<MixedTransferCommand>,
    R: MixedTransferProducer<MixedTransferResult>,
{
    samples.fill(0.5);
    let frames = samples.len();
    let output = AudioBlockMut::new(samples, frames, ChannelLayout::Mono).expect("block");
    audio.render(port, output)
}

fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|sample| *sample == 0.0)
}

fn refused<O: std::fmt::Debug>(
    result: Result<impl std::fmt::Debug, MixedTransferSubmitError<O>>,
) -> (MixedTransferRefusal, O) {
    match result {
        Err(MixedTransferSubmitError::Refused { reason, original }) => (reason, original),
        other => panic!("expected a refusal before minting, found {other:?}"),
    }
}

/// A producer that refills as quickly as audio consumes. The prepared service
/// budget must end even though this queue never becomes empty during the call.
struct RefilledCommands {
    popped: usize,
    available: usize,
}

impl MixedAudioQueues for RefilledCommands {
    const TRANSFER_RULES: bool = true;

    fn bound(&self, _audio: &super::super::MixedOneShotAudio) -> bool {
        true
    }

    fn pop_command(
        &mut self,
        audio: &mut super::super::MixedOneShotAudio,
    ) -> Option<MixedIngressCommand> {
        if self.popped == self.available {
            return None;
        }
        self.popped += 1;
        Some(MixedIngressCommand {
            owner: audio.ingress_owner,
            id: MixedIngressCommandId(self.popped as u64),
            request: MixedIngressRequest::Bend {
                origin: MixedIngressOriginId(self.popped as u64),
                scope: MixedLiveScope::new(REHEARSAL_SOURCE, MidiChannel::CH1),
                at: audio.audio.renderer.clock(),
                cents: Cents::ZERO,
            },
            bend_credits: MixedIngressQueueCount::NONE,
        })
    }

    fn push_result(
        &mut self,
        _audio: &mut super::super::MixedOneShotAudio,
        _result: MixedIngressResult,
    ) -> Option<MixedIngressResult> {
        None
    }
}

#[test]
fn concurrent_refill_cannot_extend_one_quantum_service_without_bound() {
    let mut armed = arm();
    let budget = depth() + 1;
    let mut queues = RefilledCommands {
        popped: 0,
        available: budget * 3,
    };

    armed.audio.inner_mut().service_ingress_queue(&mut queues);

    assert_eq!(queues.popped, budget);
    assert!(armed.audio.inner().fault.is_none());
}

#[test]
fn complete_callbacks_allocate_nothing_including_host_queue_operations() {
    let mut armed = arm();
    let initial_usage = armed.audio.publication_usage();
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Live,
        ProducerClass::Session,
        ProducerClass::Release,
    ] {
        assert_eq!(initial_usage.high_water(class), Some(EventCount::NONE));
    }
    assert_eq!(initial_usage.high_water_external_total(), EventCount::NONE);
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(100, 64))
        .expect("note-on");
    let _bend = armed
        .control
        .submit_bend(
            &mut armed.control_port,
            bend_on(150, MidiChannel::CH1, 100.0),
        )
        .expect("bend");
    let _up = armed
        .control
        .submit_key_up(
            &mut armed.control_port,
            MixedSourceKeyUp {
                at: SampleTime::new(300),
                handle,
            },
        )
        .expect("key-up");
    let mut samples = vec![0.0_f32; 512];
    for frames in [64, 1, 63, 128, 256] {
        let block = &mut samples[..frames];
        let events = count_allocs(|| {
            callback(&mut armed.audio, &mut armed.audio_port, block).expect("callback");
            let _usage = std::hint::black_box(armed.audio.publication_usage());
            let _resources =
                std::hint::black_box(armed.audio.resource_usage().expect("resource counters"));
        });
        assert_eq!(
            events, 0,
            "a {frames}-frame callback allocated or deallocated"
        );
    }
    let mut collected = 0;
    while armed
        .control
        .collect(&mut armed.control_port)
        .expect("results")
        .is_some()
    {
        collected += 1;
    }
    assert_eq!(collected, 3);
    let usage = armed.audio.publication_usage();
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Live,
        ProducerClass::Session,
        ProducerClass::Release,
    ] {
        assert!(usage.high_water(class).expect("available producer").get() > 0);
    }
    for class in [ProducerClass::AuthoredRuntime, ProducerClass::Internal] {
        assert_eq!(usage.high_water(class), None);
    }
    // This fixture's live, compiled and release peaks fall in different quanta.
    // The external total must come from its own ledger, not summing class peaks.
    let independent_sum: u32 = ProducerClass::ALL
        .into_iter()
        .filter_map(|class| usage.high_water(class))
        .map(EventCount::get)
        .sum();
    assert!(usage.high_water_external_total().get() < independent_sum);
    assert_eq!(
        usage.high_water_external_total(),
        armed.audio.inner.arbiter.high_water_external_total()
    );
    let teardown = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("joined owner");
    assert_eq!(teardown.publication_usage(), usage);
}

#[test]
fn an_unexpected_authored_charge_is_visible_instead_of_reported_unavailable() {
    let mut armed = arm();
    let mut publication = armed
        .audio
        .inner
        .arbiter
        .open(SampleTime::ZERO, 1)
        .expect("window");
    publication
        .charge_operation(ProducerClass::AuthoredRuntime, SampleTime::ZERO)
        .expect("inject an otherwise unavailable producer");
    let _sealed = publication.seal();
    assert_eq!(
        armed
            .audio
            .publication_usage()
            .high_water(ProducerClass::AuthoredRuntime),
        Some(EventCount::measured(1))
    );
}

/// Render the renderer's initial carry, which opens no quantum and serves no command,
/// so each later 64-frame callback opens exactly one quantum.
fn drain_carry(armed: &mut Armed) {
    let carry = armed.audio.inner().audio.renderer.carry_frames();
    if carry > 0 {
        let mut samples = vec![0.0_f32; carry];
        callback(&mut armed.audio, &mut armed.audio_port, &mut samples).expect("carry");
    }
    assert_eq!(armed.audio.inner().audio.renderer.carry_frames(), 0);
    assert_eq!(armed.audio.inner().completed_quanta.get(), 0);
}

#[test]
fn sixteen_first_nonzero_bends_hold_every_scope_in_both_tables() {
    let mut armed = arm();
    drain_carry(&mut armed);
    let mut samples = vec![0.0_f32; 64];
    for number in 1..=16 {
        let at = 64 * u64::from(number);
        let id = armed
            .control
            .submit_bend(&mut armed.control_port, bend_on(at, channel(number), 25.0))
            .expect("control accepts each first non-zero bend");
        callback(&mut armed.audio, &mut armed.audio_port, &mut samples).expect("render");
        let report = armed
            .control
            .collect(&mut armed.control_port)
            .expect("ordered")
            .expect("one result per bend");
        assert_eq!(report.id(), id);
        assert_eq!(
            report.outcome(),
            MixedTransferOutcome::Bend {
                channel: channel(number),
                result: Ok(MixedTransferBendApplied {
                    members: MixedTransferHeldCount::NONE,
                }),
            },
            "audio applies the bend on channel {number}"
        );
    }
    let control_cells = armed
        .control
        .inner()
        .bend_scopes
        .iter()
        .flatten()
        .filter(|cell| cell.accepted_nonzero)
        .count();
    assert_eq!(control_cells, 16);
    assert_eq!(armed.audio.inner().occurrences.bend_cells_held(), 16);
}

#[test]
fn each_undersized_bend_table_refuses_arm_and_returns_both_queues() {
    for (tables, expected) in [
        (
            MixedBendTables::widths(15, 32),
            MixedTransferArmError::ControlBendTable {
                width: MixedTransferScopeCount(15),
                required: REHEARSAL_SCOPES,
            },
        ),
        (
            MixedBendTables::widths(32, 15),
            MixedTransferArmError::AudioBendTable {
                width: MixedTransferScopeCount(15),
                required: REHEARSAL_SCOPES,
            },
        ),
    ] {
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
        let refusal = prepared
            .arm_transfer_with_tables(
                candidate,
                &profile(),
                Ring::<MixedTransferCommand>::new(depth()),
                Ring::<MixedTransferResult>::new(depth()),
                tables,
            )
            .expect_err("an undersized table refuses");
        assert_eq!(refusal.reason(), expected);
        let (_, _, commands, results) = refusal.into_parts();
        assert_eq!(commands.ring.capacity().get(), depth());
        assert_eq!(results.ring.capacity().get(), depth());
    }
    // Exactly sixteen cells in each table arm.
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let armed = prepared
        .arm_transfer_with_tables(
            candidate,
            &profile(),
            Ring::<MixedTransferCommand>::new(depth()),
            Ring::<MixedTransferResult>::new(depth()),
            MixedBendTables::widths(16, 16),
        )
        .expect("sixteen scopes arm");
    assert_eq!(armed.control.inner().bend_scopes.len(), 16);
}

#[test]
fn arm_returns_a_seeded_command_queue_with_its_opaque_packet() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let owner = MixedIngressOwner {
        epoch: prepared.epoch(),
        table: prepared.table_id(),
    };
    let seeded = MixedIngressCommand {
        owner,
        id: MixedIngressCommandId(7),
        request: MixedIngressRequest::Bend {
            origin: MixedIngressOriginId(1),
            scope: MixedLiveScope::new(REHEARSAL_SOURCE, MidiChannel::CH1),
            at: SampleTime::new(10),
            cents: Cents::ZERO,
        },
        bend_credits: MixedIngressQueueCount::NONE,
    };
    let mut commands = Ring::<MixedTransferCommand>::new(depth());
    assert!(commands.ring.try_push(MixedTransferCommand(seeded)).is_ok());
    let refusal = prepared
        .arm_transfer_rehearsal(
            candidate,
            &profile(),
            commands,
            Ring::<MixedTransferResult>::new(depth()),
        )
        .expect_err("a non-empty queue refuses");
    assert_eq!(
        refusal.reason(),
        MixedTransferArmError::CommandOccupied {
            occupied: MixedTransferPlaces::new(1),
        }
    );
    let (_, _, mut commands, _) = refusal.into_parts();
    let packet = commands.ring.try_pop().expect("the seeded packet stays");
    assert_eq!(packet.id().get(), 7);
    assert_eq!(packet.0.owner, owner);
    assert!(commands.ring.is_empty());
}

#[test]
fn an_exhausted_command_id_precedes_capacity_and_credit_refusals() {
    // Full command capacity.
    let mut armed = arm();
    armed.control.inner_mut().last_ingress_id = Some(MixedIngressCommandId(u64::MAX));
    let capacity = armed.control.inner().ingress_capacity;
    armed.control.inner_mut().ingress_outstanding = capacity;
    let note = note_on(10, 60);
    assert_eq!(
        refused(armed.control.submit_note_on(&mut armed.control_port, note)),
        (MixedTransferRefusal::CommandIdExhausted, note)
    );
    assert_eq!(armed.control.frontier(), None);
    assert_eq!(
        armed.control.fault(),
        Some(MixedTransferFault::CommandIdExhausted)
    );

    // Short inherited bend credit on a bent scope.
    let mut armed = arm();
    let inner = armed.control.inner_mut();
    inner.last_ingress_id = Some(MixedIngressCommandId(u64::MAX));
    inner.bend_scopes[0] = Some(MixedBendScopeCell {
        scope: MixedLiveScope::new(REHEARSAL_SOURCE, MidiChannel::CH1),
        latest: MixedIngressCommandId(1),
        accepted_nonzero: true,
    });
    inner.bend_credits_free = MixedIngressQueueCount::NONE;
    assert_eq!(
        refused(armed.control.submit_note_on(&mut armed.control_port, note)),
        (MixedTransferRefusal::CommandIdExhausted, note)
    );
    assert_eq!(
        armed.control.inner().bend_credits_free,
        MixedIngressQueueCount::NONE
    );
    let bend = bend_on(10, channel(2), 10.0);
    assert_eq!(
        refused(armed.control.submit_bend(&mut armed.control_port, bend)),
        (MixedTransferRefusal::Halted, bend),
        "the identity fault halted the transfer"
    );
}

#[test]
fn key_ups_under_an_exhausted_command_id_split_by_handle_state() {
    let mut armed = arm();
    let (_, retired) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
        .expect("first");
    let _up = armed
        .control
        .submit_key_up(
            &mut armed.control_port,
            MixedSourceKeyUp {
                at: SampleTime::new(20),
                handle: retired,
            },
        )
        .expect("key-up");
    let (_, open) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(30, 62))
        .expect("second");
    drain_carry(&mut armed);
    let mut samples = vec![0.0_f32; 64];
    callback(&mut armed.audio, &mut armed.audio_port, &mut samples).expect("render");
    for _ in 0..3 {
        let _report = armed
            .control
            .collect(&mut armed.control_port)
            .expect("ordered")
            .expect("result");
    }
    armed.control.inner_mut().last_ingress_id = Some(MixedIngressCommandId(u64::MAX));

    let stale = MixedSourceKeyUp {
        at: SampleTime::new(40),
        handle: retired,
    };
    assert_eq!(
        refused(armed.control.submit_key_up(&mut armed.control_port, stale)),
        (MixedTransferRefusal::NotOpen, stale),
        "a retired handle is NotOpen even when no ID remains"
    );
    assert_eq!(armed.control.fault(), None);
    assert_eq!(armed.control.frontier(), Some(SampleTime::new(30)));

    let live = MixedSourceKeyUp {
        at: SampleTime::new(40),
        handle: open,
    };
    assert_eq!(
        refused(armed.control.submit_key_up(&mut armed.control_port, live)),
        (MixedTransferRefusal::CommandIdExhausted, live)
    );
    assert!(
        armed
            .control
            .inner()
            .occurrence_ledger
            .is_open(open.occurrence())
    );
    assert_eq!(
        armed.control.inner().release_reserved,
        MixedIngressQueueCount(1)
    );
}

#[test]
fn a_no_vacancy_onset_faults_only_with_command_room_and_credit() {
    let held = |armed: &mut Armed| {
        let _first = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(10, 60))
            .expect("one held cell");
        armed
            .control
            .inner_mut()
            .occurrence_ledger
            .exhaust_cells_after(1);
    };
    let note = note_on(20, 62);

    // Command capacity short: transient.
    let mut armed = arm();
    held(&mut armed);
    let capacity = armed.control.inner().ingress_capacity;
    armed.control.inner_mut().ingress_outstanding = capacity;
    assert_eq!(
        refused(armed.control.submit_note_on(&mut armed.control_port, note)),
        (MixedTransferRefusal::CommandCapacity, note)
    );
    assert_eq!(armed.control.fault(), None);
    assert_eq!(armed.control.frontier(), Some(SampleTime::new(10)));

    // Inherited bend credit short: transient.
    let mut armed = arm();
    held(&mut armed);
    let inner = armed.control.inner_mut();
    inner.bend_scopes[0] = Some(MixedBendScopeCell {
        scope: MixedLiveScope::new(REHEARSAL_SOURCE, MidiChannel::CH1),
        latest: MixedIngressCommandId(1),
        accepted_nonzero: true,
    });
    inner.bend_credits_free = MixedIngressQueueCount::NONE;
    assert_eq!(
        refused(armed.control.submit_note_on(&mut armed.control_port, note)),
        (MixedTransferRefusal::BendCredit, note)
    );
    assert_eq!(armed.control.fault(), None);

    // Room and credit available: a terminal identity fault that mints nothing.
    let mut armed = arm();
    held(&mut armed);
    let last = armed.control.inner().last_ingress_id;
    assert_eq!(
        refused(armed.control.submit_note_on(&mut armed.control_port, note)),
        (MixedTransferRefusal::OccurrenceCellsExhausted, note)
    );
    assert_eq!(armed.control.inner().last_ingress_id, last);
    assert_eq!(armed.control.frontier(), Some(SampleTime::new(10)));
    assert_eq!(
        armed.control.fault(),
        Some(MixedTransferFault::OccurrenceCellsExhausted)
    );

    // One exhausted cell beside a free one does not fault.
    let mut armed = arm();
    armed
        .control
        .inner_mut()
        .occurrence_ledger
        .exhaust_cells_after(1);
    let _only = armed
        .control
        .submit_note_on(&mut armed.control_port, note)
        .expect("the free cell serves the onset");
    assert_eq!(armed.control.fault(), None);
}

#[test]
fn failed_pushes_keep_every_charge_of_their_retained_packet() {
    let lying = || Ring {
        reported_capacity: Some(depth()),
        ..Ring::<MixedTransferCommand>::new(1)
    };
    let per_bend = |armed: &Armed| armed.control.inner().bend_credits_per_bend.as_usize();
    let free = |armed: &Armed| armed.control.inner().bend_credits_free.as_usize();

    // A first non-zero bend on a new scope keeps its scope cell and credits.
    let mut armed = arm_with(lying(), Ring::new(depth()));
    let _filler = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(5, 60))
        .expect("fills the real ring");
    let budget = free(&armed);
    let Err(MixedTransferSubmitError::PushFailed { id, handle: None }) = armed
        .control
        .submit_bend(&mut armed.control_port, bend_on(10, channel(2), 50.0))
    else {
        panic!("the bend push fails");
    };
    let inner = armed.control.inner();
    assert_eq!(free(&armed), budget - per_bend(&armed));
    assert!(inner.scope_may_bend(MixedLiveScope::new(REHEARSAL_SOURCE, channel(2))));
    let pending = inner.ingress_command_pending.as_ref().expect("retained");
    assert_eq!(
        (pending.id.0, pending.bend_credits.as_usize()),
        (id.get(), per_bend(&armed))
    );
    assert_eq!(inner.ingress_journal.len(), 2);
    assert_eq!(inner.ingress_outstanding, MixedIngressQueueCount(2));

    // An onset on a bent scope keeps its key-up place and inherited credit.
    let mut armed = arm_with(lying(), Ring::new(depth()));
    let _bend = armed
        .control
        .submit_bend(&mut armed.control_port, bend_on(5, MidiChannel::CH1, 50.0))
        .expect("fills the real ring");
    let before = free(&armed);
    let Err(MixedTransferSubmitError::PushFailed {
        handle: Some(handle),
        ..
    }) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
    else {
        panic!("the onset push fails");
    };
    let inner = armed.control.inner();
    assert_eq!(
        free(&armed),
        before - 1,
        "the inherited credit stays charged"
    );
    assert_eq!(
        inner
            .ingress_command_pending
            .as_ref()
            .map(|command| command.bend_credits),
        Some(MixedIngressQueueCount(1))
    );
    assert_eq!(inner.release_reserved, MixedIngressQueueCount(1));
    assert!(inner.occurrence_ledger.is_open(handle.occurrence()));
    assert_eq!(inner.ingress_outstanding, MixedIngressQueueCount(2));

    // A key-up moves its reserved place to the pending command without a second charge.
    let mut armed = arm_with(lying(), Ring::new(depth()));
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(5, 60))
        .expect("fills the real ring");
    let room = armed.control.inner().ingress_room();
    let Err(MixedTransferSubmitError::PushFailed { id, .. }) = armed.control.submit_key_up(
        &mut armed.control_port,
        MixedSourceKeyUp {
            at: SampleTime::new(10),
            handle,
        },
    ) else {
        panic!("the key-up push fails");
    };
    let inner = armed.control.inner();
    assert_eq!(inner.release_reserved, MixedIngressQueueCount::NONE);
    assert_eq!(inner.ingress_outstanding, MixedIngressQueueCount(2));
    assert_eq!(
        inner.ingress_room(),
        room,
        "no place was freed or double-charged"
    );
    assert!(
        inner
            .occurrence_ledger
            .is_releasing(handle.occurrence(), MixedIngressCommandId(id.get()))
    );
}

#[test]
fn a_zero_ingress_width_is_refused_rather_than_asserted() {
    assert_eq!(MixedIngressQueueCount::capacity(0), None);
    assert_eq!(
        MixedIngressQueueCount::capacity(1),
        Some(MixedIngressQueueCount(1))
    );
}

#[test]
fn an_onset_the_ledger_refuses_after_minting_halts_and_keeps_its_command() {
    let mut armed = arm();
    armed.control.inner_mut().occurrence_ledger.forget_scopes();
    let Err(MixedTransferSubmitError::MergerLedger {
        id,
        handle: Some(handle),
    }) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
    else {
        panic!("the ledger refuses the minted onset");
    };
    assert_eq!(id.get(), 1);
    assert_eq!(
        armed.control.fault(),
        Some(MixedTransferFault::MergerLedger { id: Some(id) })
    );
    assert_eq!(armed.control.frontier(), Some(SampleTime::new(10)));
    let inner = armed.control.inner();
    assert_eq!(
        inner.release_reserved,
        MixedIngressQueueCount::NONE,
        "no handle holds a release place"
    );
    assert!(!inner.occurrence_ledger.is_open(handle.occurrence()));
    assert_eq!(inner.ingress_outstanding, MixedIngressQueueCount(1));

    let teardown = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("own halves join");
    assert_eq!(teardown.custody(), MixedTransferCustody::Complete);
    assert_eq!(teardown.outstanding(), [id]);
    assert_eq!(
        teardown.unserved(),
        [id],
        "the pushed command stays in custody"
    );
    assert!(teardown.handles().is_empty());
    assert_eq!(teardown.release_places(), MixedTransferPlaces::NONE);
}

#[test]
fn a_key_up_without_its_reserved_place_halts_before_minting() {
    let mut armed = arm();
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(10, 60))
        .expect("onset");
    armed.control.inner_mut().release_reserved = MixedIngressQueueCount::NONE;
    let last = armed.control.inner().last_ingress_id;
    let key_up = MixedSourceKeyUp {
        at: SampleTime::new(20),
        handle,
    };
    let (reason, original) = refused(armed.control.submit_key_up(&mut armed.control_port, key_up));
    assert_eq!(
        (reason, original),
        (MixedTransferRefusal::MergerLedger, key_up)
    );
    assert!(reason.is_terminal());
    assert_eq!(
        armed.control.fault(),
        Some(MixedTransferFault::MergerLedger { id: None })
    );
    let inner = armed.control.inner();
    assert_eq!(inner.last_ingress_id, last, "nothing was minted");
    assert_eq!(inner.release_reserved, MixedIngressQueueCount::NONE);
    assert!(inner.occurrence_ledger.is_open(handle.occurrence()));
    assert_eq!(armed.control.frontier(), Some(SampleTime::new(10)));
}

/// Push a forged command straight into the host's command producer.
fn forge(armed: &mut Armed, id: u64, request: MixedIngressRequest) {
    let owner = armed.control.inner().ingress_owner;
    let forged = MixedTransferCommand(MixedIngressCommand {
        owner,
        id: MixedIngressCommandId(id),
        request,
        bend_credits: MixedIngressQueueCount::NONE,
    });
    assert!(armed.control_port.commands.try_push(forged).is_ok());
}

#[test]
fn audio_faults_an_earlier_stamped_or_duplicate_packet_before_mutation() {
    // A valid next ID with an earlier stamp than the served packet.
    let mut armed = arm();
    let _on = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(200, 60))
        .expect("note-on");
    forge(
        &mut armed,
        2,
        MixedIngressRequest::Bend {
            origin: MixedIngressOriginId(9),
            scope: MixedLiveScope::new(REHEARSAL_SOURCE, MidiChannel::CH1),
            at: SampleTime::new(100),
            cents: Cents::new(50.0).expect("finite"),
        },
    );
    let mut samples = vec![0.0_f32; 128];
    assert_eq!(
        callback(&mut armed.audio, &mut armed.audio_port, &mut samples),
        Err(MixedTransferRenderError::BackwardTime)
    );
    assert!(silent(&samples));
    let audio = armed.audio.inner();
    assert_eq!(
        audio.occurrences.bend_cells_held(),
        0,
        "the bend changed nothing"
    );
    assert_eq!(
        audio
            .invalid_ingress_command
            .as_ref()
            .map(|command| command.id.0),
        Some(2)
    );
    assert_eq!(audio.last_served_ingress_id, Some(MixedIngressCommandId(1)));

    // A duplicate of the served command.
    let mut armed = arm();
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(20, 60))
        .expect("note-on");
    forge(
        &mut armed,
        1,
        MixedIngressRequest::Onset {
            origin: MixedIngressOriginId(9),
            occurrence: handle.occurrence(),
            scope: MixedLiveScope::new(REHEARSAL_SOURCE, MidiChannel::CH1),
            at: SampleTime::new(20),
            key: KeyIdentity::new(60).expect("key"),
            velocity: NoteVelocity::FULL,
        },
    );
    assert_eq!(
        callback(&mut armed.audio, &mut armed.audio_port, &mut samples),
        Err(MixedTransferRenderError::CommandOrder)
    );
    assert!(silent(&samples));
    assert_eq!(armed.audio.inner().occurrences.occupied().len(), 1);
    assert_eq!(
        armed
            .audio
            .inner()
            .invalid_ingress_command
            .as_ref()
            .map(|command| command.id.0),
        Some(1)
    );
}

#[test]
fn a_horizon_fault_after_the_first_quantum_silences_the_whole_callback() {
    let reference = {
        let mut armed = arm();
        drain_carry(&mut armed);
        let mut samples = vec![0.0_f32; 128];
        callback(&mut armed.audio, &mut armed.audio_port, &mut samples).expect("render");
        samples
    };
    assert!(
        !silent(&reference[..64]),
        "the compiled note sounds in the first quantum"
    );

    // The host's consumer hides the packet from the first quantum's service, so the
    // second quantum of one 128-frame callback is the first to serve it.
    let delayed = Ring {
        hidden_pops: 1,
        ..Ring::<MixedTransferCommand>::new(depth())
    };
    let mut delayed_armed = arm_with(delayed, Ring::new(depth()));
    drain_carry(&mut delayed_armed);
    let horizon = profile().limits().events().forward_event_horizon();
    let far = SampleTime::new(64)
        .checked_add(horizon)
        .and_then(|end| end.checked_add(FrameCount::new(1)))
        .expect("representable");
    let id = delayed_armed
        .control
        .submit_note_on(
            &mut delayed_armed.control_port,
            MixedSourceNoteOn {
                at: far,
                ..note_on(0, 60)
            },
        )
        .expect("control does not judge the horizon")
        .0;
    let mut samples = vec![0.0_f32; 128];
    assert_eq!(
        callback(
            &mut delayed_armed.audio,
            &mut delayed_armed.audio_port,
            &mut samples
        ),
        Err(MixedTransferRenderError::BeyondHorizon)
    );
    assert!(
        silent(&samples),
        "the first quantum was silenced with the rest"
    );
    let audio = delayed_armed.audio.inner();
    assert_eq!(
        audio.completed_quanta.get(),
        1,
        "the fault came after one quantum"
    );
    assert_eq!(audio.test_ingress.counters().beyond_horizon(), 1);
    assert!(audio.occurrences.occupied().is_empty(), "no handle changed");
    assert_eq!(
        audio
            .invalid_ingress_command
            .as_ref()
            .map(|command| command.id.0),
        Some(id.get())
    );
    assert_eq!(
        callback(
            &mut delayed_armed.audio,
            &mut delayed_armed.audio_port,
            &mut samples
        ),
        Err(MixedTransferRenderError::Faulted)
    );
    assert!(silent(&samples));
    assert_eq!(
        delayed_armed
            .audio
            .inner()
            .test_ingress
            .counters()
            .beyond_horizon(),
        1
    );
}

#[test]
fn resource_peaks_retain_an_onset_and_release_served_and_drained_in_one_callback() {
    let mut armed = arm();
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(100, 64))
        .expect("onset");
    let _release = armed
        .control
        .submit_key_up(
            &mut armed.control_port,
            MixedSourceKeyUp {
                at: SampleTime::new(110),
                handle,
            },
        )
        .expect("release");
    let mut samples = [0.0; 512];
    callback(&mut armed.audio, &mut armed.audio_port, &mut samples).expect("one callback");
    assert_eq!(armed.audio.live_holds_outstanding(), EventCount::NONE);
    assert_eq!(armed.audio.inner.test_ingress.len(), 0);
    let usage = armed.audio.resource_usage().expect("usage");
    assert_eq!(usage.ingress_queue_high_water, EventCount::measured(2));
    assert_eq!(usage.live_holds_high_water, EventCount::measured(1));
    assert!(usage.renderer.timed_controls_written.get() > 0);
    assert_eq!(usage.renderer.rejected_control_writes, EventCount::NONE);
    assert!(usage.renderer.event_payloads.get() > 0);
    assert!(usage.renderer.seeded_rows.get() > 0);
    assert_eq!(
        usage.renderer_storage.controls.get(),
        u64::try_from(armed.audio.inner.audio.renderer.control_scratch_bytes()).expect("bytes")
    );
    let teardown = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("join");
    assert_eq!(
        teardown.resource_usage().expect("joined observations"),
        usage
    );
}

#[test]
fn issue_transfer_is_unique_under_concurrent_last_ids_and_refuses_permanently() {
    crate::issuer_controls::assert_u64_issuer_exhaustion(issue_transfer_from);
}

#[path = "transfer/endurance_tests.rs"]
mod endurance_tests;
