//! Phase 9 Stage 6's concrete two-source bridge over ADR-0078's transfer rehearsal.
//!
//! The bridge is the only offerer its armed transfer has: it owns the transfer's control
//! half and exposes no direct submit, so every command passes the per-source stage and
//! the one execution-stamp rule below. It carries note-on, the paired key-up of an onset,
//! channel bend and the sustain pedal's down and lift from two bound source connections,
//! and one host Panic at a time through [`MixedBridgeControl::panic`]. Every other
//! message is refused for sound and kept by raw capture; plan replacement stays refused,
//! and `PerformanceIngress::prepare` still refuses a mixed producer plan. The production
//! session's Panic is unchanged. A later compiled Start, seek or loop entry is prepared
//! and collected through [`MixedBridgeControl::prepare_activation`],
//! [`MixedBridgeControl::prepare_loop_activation`] and
//! [`MixedBridgeControl::collect_activation`] beside the stage; it never enters a lane.
//!
//! # Identity and custody
//!
//! The bridge names every packet by its [`MixedSourcePacketId`]: the bound connection and
//! the serial that source's own ring gave the packet. It never reads a raw capture ID or
//! `matched_onset` link, so raw capture can stop while sound continues. The host hands
//! every source-ring head to [`MixedBridgeControl::stage`]: after raw capture admits it
//! while raw capture runs, and directly once raw capture has been interrupted. The host
//! keeps the correlation between a source ID and raw capture's own outcome itself.
//! Stage consumes every handoff: it never asks the host to wait, so a stage shortage
//! never delays or refuses raw capture (rule 1.1). Each source ID is then in exactly one
//! place: settled by the call's return value, held in a stage cell, or journaled as one
//! command until [`MixedBridgeControl::collect`] reports it. Joined teardown names every
//! held source ID and where its command was found. A source's serials strictly increase,
//! so no two places can name the same packet.
//!
//! # Pairing
//!
//! A key-up pairs with the oldest onset of its channel and key on its own source whose
//! key-up has not yet arrived: same-key FIFO. Raw capture's `matched_onset` applies the
//! same rule to the same per-source sequence, so while raw capture admits every packet
//! the two pairings agree. After an interruption the bridge keeps pairing on its own. A
//! key-up that finds no such onset is refused for sound as [`MixedSplit::Unpaired`].
//!
//! # Stage and order
//!
//! Each source owns `stage_cells` cells, fixed at arm. An onset takes one and protects
//! one for its key-up until that key-up is staged; a bend takes one; a key-up spends its
//! onset's protected cell and asks for nothing (rule 1.2). An onset or bend that finds no
//! cell is split: refused for sound, kept by raw. A split onset leaves a tombstone in its
//! FIFO place until its paired key-up arrives; a key-up already staged carries the
//! refusal in its own cell (rule 1.3).
//!
//! # Sustain
//!
//! Pedal edges stay in their source's FIFO, so they keep their order against that
//! source's key edges, and each source's sixteen channels are separate scopes: a lift
//! on one source and channel never ends another's notes. The bridge classifies each
//! edge against its own per-scope pedal state, which follows the source exactly as the
//! source ring's own pedal reservation does. A down while down, or a lift while up, is
//! split as [`MixedSplit::RedundantPedal`] and spends nothing. An effective pedal-down
//! takes one cell and protects one for its lift, exactly as an onset does for its
//! key-up; the effective lift spends that cell and asks for nothing (rule 1.5a). The
//! merger likewise reserves the lift's command place when it mints the down, so a lift
//! is never refused for stage or command capacity once its down was forwarded. A down
//! the bridge refuses leaves a pedal tombstone; its lift is consumed as
//! [`MixedSplit::AfterRefusedPedal`], or carries the refusal in its own cell when it is
//! already staged. A key-up under its scope's pedal is served as
//! [`MixedKeyUpRetired::Sustained`]: it writes no edge, redeems no hold and keeps its
//! handle until the lift's one release group ends every sustained note of the scope and
//! redeems their holds (rule 1.4). Raw capture keeps treating sustain as ordinary input.
//!
//! [`MixedBridgeControl::pump`] selects the older head of the two lanes, the first source
//! winning a tie. A head waits for an empty peer lane until the peer's frontier passes it,
//! or until the peer is fenced: its frontier lags the audio boundary by more than
//! `peer_wait`. It forwards the head at the execution stamp
//! `max(nominal, first unrendered boundary, last forwarded)`,
//! so the command stream, the audio half's order check and the ingress store's monotone
//! fence can never refuse a forwarded packet for its stamp. A head is forwarded only once
//! that stamp lies inside the next publication window: the quanta one callback of the
//! profile's largest block can render from the audio boundary. A later head waits in its
//! stage cell, not in the ingress queue. The window lies inside the forward horizon, so
//! audio never refuses a forwarded stamp as too far ahead. The head leaves its lane only
//! once its command was minted; a command-ring or bend-credit shortage keeps it staged,
//! except that a quiescent ring with no outstanding command splits it.
//!
//! # Panic
//!
//! Panic is a host command, not a source edge (rule 1.5a). The bridge owns one Panic cell
//! and the merger one protected place in each host queue beyond the admitted bound, which
//! arm requires, so ordinary commands, uncollected results and reserved places never take
//! its place. [`MixedBridgeControl::panic`] refuses at the caller boundary, and changes
//! nothing, while the cell holds a Panic whose result is uncollected, before audio opens
//! the quantum after the last applied Panic, or when its stamp precedes the last accepted
//! Panic's. An accepted Panic is never dropped.
//!
//! The cell is a third head in selection, by its nominal stamp. A source head at or
//! before that stamp goes first, so a source edge with the same stamp is applied before
//! the Panic, as compiled events at its sample are applied before its compiled cut. Order
//! comes before capacity: such a head that waits for command places holds the Panic
//! behind it until a collection frees a place, or until a quiescent ring splits the head.
//! The Panic also waits for every empty, unfenced peer whose bound has not passed its
//! stamp, and is forwarded at the same execution stamp and inside the same window as any
//! head. On audio it ends every sounding and sustained live note of both sources as one
//! release group and, in the same quantum, every open compiled note at the same sample
//! (rule 2.4). It leaves pedals, refusal tombstones, the compiled transport and raw
//! capture unchanged. A sustained note's handle retires with it; a held key's handle
//! stays open on a Panic tombstone, and its later key-up is consumed as
//! [`MixedKeyUpRetired::AfterPanic`].
//!
//! A due Panic cancels a sealed reset (Item 2 step 2). The seal holds one atomic phase,
//! and a Panic stamped before `B + Q` tries one compare-and-swap on it as the last step
//! of its acceptance, while audio tries its own at `B`; exactly one wins. If the Panic
//! wins, audio refuses the successor as
//! [`MixedBridgeResetRefusal::PanicCancelled`](crate::stream::MixedBridgeResetRefusal::PanicCancelled), and
//! the lanes and the Panic forward to the old transfer as if no reset were sealed, so a
//! source edge stamped before the Panic still goes first. If audio already decided, or
//! the Panic is stamped later, it waits in the cell: the commit sends it to the successor,
//! whose own notes it ends after the `Reset` group ended the old ones, and the abandon
//! sends it to the old owner. The bridge still refuses to seal while its Panic cell is
//! occupied, so a sealed reset never has a Panic in flight to the old transfer.
//!
//! Winning decides only which owner the Panic reaches, not when it executes. Its report
//! keeps the nominal stamp and names the execution sample, which is later whenever the
//! Panic was accepted or pumped after audio had already served that stamp's quantum. A
//! Panic is sample-exact only when the host accepts and pumps it before audio opens the
//! quantum of its stamp, as for any head.
//!
//! # Bounded custody
//!
//! Every list the control half keeps is prepared at arm and never grows: a lane holds
//! `stage_cells` packets and protected places, a source's pairing table at most
//! `held_onsets` onsets, the handle table at most the merger's occurrence cells and the
//! command journal at most the admitted command bound; the Panic cell holds one Panic.
//! Every onset whose key-up has not reached the stage is in the pairing table, awaiting
//! its key-up or a tombstone, and the source ring itself reserves a release place for
//! each and refuses an onset beyond its bound. Pump checks for room before it mints, so
//! a full list makes the head wait or split exactly as a full command ring does, and no
//! obligation is dropped to make room.
//! A source that hands over more held onsets than arm prepared is a terminal fault
//! naming the onset.
//!
//! # Faults
//!
//! A foreign source, a source-order or source-serial violation, an expired source retry,
//! a pre-ring source refusal, a raw-admission refusal while raw capture runs, a source
//! beyond its declared holds and an audio clock whose next publication window is
//! unrepresentable join the transfer's closed list, beside its command-ID and
//! occurrence-cell generation exhaustion. Each is terminal: it publishes the transfer's
//! one halt, and audio silences the complete callback that observes it and every later
//! one (rule 1.8). Raw capture interrupting itself, for example when its recorder
//! refuses a late packet, is not a bridge fault: the host stops raw capture for both
//! sources and keeps staging both. Waiting is not a fault: a head beyond the window,
//! behind an unfenced peer, or short of command places or bend credit is an
//! [`MixedPumpStep::Idle`] with its reason. A sound-path shortage is never terminal; it
//! is a split or an audio refusal with a named reason.
//!
//! # Stopped-owner reset
//!
//! One bridge spans a whole-owner reset at a quantum boundary `B`; only the transfer under
//! it changes. The source lanes, serials, frontiers and pairing tables never move, so no
//! source FIFO is split or copied. [`MixedBridgeControl::seal_reset`] takes a freshly
//! armed successor bridge of the same sources and bounds, stops forwarding to the old
//! transfer, and publishes the old transfer's last minted command and the successor's
//! binding to audio. Audio refuses the successor at its offer and again at `B` unless the
//! seal names it and the old owner has served every sealed command. Staging continues
//! while sealed: every handoff is still consumed, and only a key-up the bridge already
//! refused leaves its lane. [`MixedBridgeControl::reset_ready`] reports `B` once every
//! sealed command has been collected.
//!
//! Once the wrapper reports the applied reset, [`MixedBridgeControl::commit_reset`] takes
//! its receipt, ends every old onset whose key-up has not been forwarded as a
//! [`MixedOnsetRefusal::Reset`] tombstone, swaps in the successor's transfer, journal,
//! handle table and audio clock, and returns the old ones as a
//! [`MixedBridgeRetiredControl`] that collects the old results and joins the old arm.
//! Heads still staged go to the successor at execution stamps from its own clock, which
//! is at or after `B`. A successor the wrapper refused, withdrew or never received is
//! handed back with [`MixedBridgeControl::abandon_reset`], which clears the seal and
//! resumes forwarding to the old transfer. Commit and abandon are both allowed after a
//! fault, so teardown is always reachable; a sealed bridge refuses to join until one of
//! them has run. A reset a due Panic cancelled, which
//! [`MixedBridgeControl::reset_phase`] reports, refuses commit, a new seal and join until
//! the abandon returns its successor. A compiled Stop cannot cancel a sealed reset in this
//! shape: the old owner is already stopped, so the wrapper answers `AlreadyStopped` and
//! changes nothing.

use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use synth_core::MidiChannel;
use thiserror::Error;

use super::super::{
    MixedBendTables, MixedBridgeResetEnd, MixedBridgeResetOwner, MixedBridgeResetReceipt,
    MixedIngressCommandId, MixedIngressQueueCount, MixedIngressRequest, MixedJoinedPrepared,
    MixedPanicCapability, MixedPedalSubmitted, MixedStampedCandidate,
    occurrence::{
        LiveOccurrenceId, MixedBendSubmitError, MixedLiveScope, MixedLiveSource,
        MixedOnsetSubmitError, MixedPanicSubmitError, MixedPedalSubmitError,
        MixedReleaseSubmitError,
    },
    source_set::{MixedBoundSource, MixedSourceSet, MixedSourceSetError},
};
use super::{
    MixedActivationCollectRefusal, MixedActivationKind, MixedActivationOffer,
    MixedActivationPrepareError, MixedActivationReceipt, MixedActivationRetired, MixedKeyUpRetired,
    MixedTransferArmError, MixedTransferArmRefusal, MixedTransferArmed, MixedTransferAudio,
    MixedTransferAudioPort, MixedTransferBendApplied, MixedTransferBendRefused,
    MixedTransferCollectError, MixedTransferCommand, MixedTransferCommandId, MixedTransferConsumer,
    MixedTransferControl, MixedTransferControlPort, MixedTransferFault, MixedTransferHandleState,
    MixedTransferHeldCount, MixedTransferJoinError, MixedTransferOutcome, MixedTransferPanicEnd,
    MixedTransferPanicRefused, MixedTransferPedalDown, MixedTransferPedalRefused,
    MixedTransferPlaces, MixedTransferProducer, MixedTransferQueue, MixedTransferResult,
    MixedTransferScopeCount, MixedTransferShape, MixedTransferTeardown,
};
use crate::{
    host::{ConnectionGeneration, input::InputError},
    identity::NoteIdentity,
    ingress::{IngressRefused, RELEASE_GROUP_CAPACITY},
    profile::HostProfile,
    quantities::{Cents, KeyIdentity, NoteVelocity},
    recording::notes::{Midi1Event, Midi1Input},
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, QuantumOffset, SampleTime},
    transport::LoopInterval,
};

/// The largest stage lane arm prepares per source.
const MAX_STAGE_CELLS: usize = 256;

/// The largest pairing table arm prepares, per source.
const MAX_HELD_ONSETS: usize = 65_536;

/// The channel scopes of one MIDI1 source, each with its own pedal.
const MIDI_CHANNELS: usize = 16;

/// Both bend tables must cover every channel of both sources.
const BRIDGE_SCOPES: MixedTransferScopeCount = MixedTransferScopeCount(MixedSourceSet::MAX_SCOPES);

/// A MIDI1 pitch-bend message spans this many cents either side of centre, as the
/// exclusive live owner maps it.
const BEND_RANGE_CENTS: f32 = 200.0;

/// The two source connections one bridge binds. The first wins a stamp tie.
#[derive(Clone, Copy, Debug)]
#[must_use]
pub struct MixedTransferSources {
    set: MixedSourceSet,
}

/// Why two connections cannot be bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedTransferSourcesError {
    /// Both connections are the same generation.
    #[error("the two bridge sources must be distinct connections")]
    Duplicate,
}

impl MixedTransferSources {
    /// Bind two distinct source connections.
    ///
    /// # Errors
    ///
    /// The two generations are equal.
    pub fn new(
        first: ConnectionGeneration,
        second: ConnectionGeneration,
    ) -> Result<Self, MixedTransferSourcesError> {
        match MixedSourceSet::prepare(first, second) {
            Ok(set) => Ok(Self { set }),
            Err(MixedSourceSetError::Duplicate | MixedSourceSetError::Unbound) => {
                Err(MixedTransferSourcesError::Duplicate)
            }
        }
    }

    /// The first source, which wins a stamp tie.
    pub const fn first(self) -> ConnectionGeneration {
        self.set.generation(MixedBoundSource::First)
    }

    /// The second source.
    pub const fn second(self) -> ConnectionGeneration {
        self.set.generation(MixedBoundSource::Second)
    }
}

/// The stage's per-source capacity, the sources' hold bound and the peer wait bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedTransferFence {
    /// Cells per source lane: an onset takes one and protects one for its key-up, an
    /// effective pedal-down takes one and protects one for its lift, a bend takes one.
    /// Arm refuses fewer than two.
    pub stage_cells: MixedTransferPlaces,
    /// The most onsets one source holds at once without having sent their key-ups: the
    /// bound its source ring reserves release places for. Each source's pairing table is
    /// prepared this large. Arm refuses none or more than 65 536.
    pub held_onsets: MixedTransferPlaces,
    /// `W`: a peer whose frontier lags the audio boundary by more than this is fenced for
    /// the sound path, and the other source stops waiting for it. Unmeasured; a host
    /// passes the value it is testing.
    pub peer_wait: FrameCount,
}

/// The identity a bound source's own ring gave one packet, message or frontier: its
/// connection and the ring's serial for it. Raw capture issues its own IDs; the bridge
/// never reads them, so it keeps working after raw capture stops.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct MixedSourcePacketId {
    connection: ConnectionGeneration,
    serial: u64,
}

/// An invalid source-ring packet identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedSourcePacketIdError {
    /// Source-ring packet serials start at one.
    #[error("source packet serial must be nonzero")]
    ZeroSerial,
}

impl MixedSourcePacketId {
    /// The packet `serial` of `connection`'s source ring.
    ///
    /// # Errors
    ///
    /// `serial` is zero; a prepared source ring starts its packet series at one.
    pub const fn new(
        connection: ConnectionGeneration,
        serial: u64,
    ) -> Result<Self, MixedSourcePacketIdError> {
        if serial == 0 {
            Err(MixedSourcePacketIdError::ZeroSerial)
        } else {
            Ok(Self { connection, serial })
        }
    }

    /// The source connection.
    pub const fn connection(self) -> ConnectionGeneration {
        self.connection
    }

    /// The source ring's serial; a source's serials strictly increase.
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

/// One source message, handed to [`MixedBridgeControl::stage`] in its source's ring order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixedSourceHandoff {
    /// The source ring's identity for the packet; its connection names the source.
    pub packet: MixedSourcePacketId,
    /// The nominal stamp the source ring validated; never moved by the bridge.
    pub at: SampleTime,
    /// The message.
    pub input: Midi1Input,
}

/// Why the bridge refused an onset, or an effective pedal-down, for sound before it
/// reached audio. Each holds a protected release place, so each refusal leaves a
/// tombstone that consumes the paired key-up or lift.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedOnsetRefusal {
    /// The source lane had no cell for the onset or pedal-down and its protected
    /// key-up or lift place.
    StageFull,
    /// The selected onset or pedal-down met a quiescent command ring: no command
    /// outstanding and fewer than two places free, so no collection could make room.
    CommandsFull,
    /// A stopped-owner reset ended the old onset or pedal; its later key-up or lift
    /// belongs to the old source FIFO and must not reach the successor owner.
    Reset,
}

/// A sound-path refusal of one source message. Raw capture, while it runs, keeps the
/// message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedSplit {
    /// The source lane had no cell. A refused onset leaves a tombstone for its key-up.
    StageFull,
    /// The selected head met a quiescent full command ring. A refused onset leaves a
    /// tombstone for its key-up.
    CommandsFull,
    /// A key-up paired with an onset the bridge refused, consumed with the onset's reason.
    AfterRefusedOnset {
        /// The refused onset's source ID.
        onset: MixedSourcePacketId,
        /// Why the onset was refused.
        reason: MixedOnsetRefusal,
    },
    /// A message this rehearsal does not carry.
    Unsupported,
    /// A key-up its source sent while no onset of its channel and key awaited one.
    Unpaired,
    /// A sustain edge that repeats its scope's pedal state, at the stage or at the
    /// merger: consumed and counted here, with no place spent.
    RedundantPedal,
    /// A lift paired with an effective pedal-down the bridge refused, consumed with the
    /// down's reason.
    AfterRefusedPedal {
        /// The refused pedal-down's source ID.
        down: MixedSourcePacketId,
        /// Why the pedal-down was refused.
        reason: MixedOnsetRefusal,
    },
}

/// What [`MixedBridgeControl::stage`] did with a handoff it consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedStaged {
    /// The handoff holds a stage cell; a later pump and collect settle it.
    Staged,
    /// The sound path refused it now; this return settles it.
    Split(MixedSplit),
}

/// A terminal fault a source thread or the raw owner met, reported by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedSourceFault {
    /// The producer retired an owned retry whose deadline expired.
    RetryExpired,
    /// The producer refused an observation before it entered the source ring.
    PreRing(InputError),
    /// The raw owner refused a source-ring packet while raw capture was running. A packet
    /// that meets raw capture already interrupted is not a fault; the host stages it.
    RawAdmission(InputError),
}

/// A terminal bridge fault as the control half recorded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeFault {
    /// A transfer fault: an endpoint, command, result or identity fault, or `Remote`
    /// once the audio half published the halt.
    #[error(transparent)]
    Transfer(MixedTransferFault),
    /// A source packet whose connection is neither bound source.
    #[error("connection {generation:?} is not a bound bridge source")]
    ForeignSource {
        /// The unbound generation.
        generation: ConnectionGeneration,
    },
    /// A source's stamp preceded that source's consumed frontier.
    #[error("source {connection:?} stamp {at} precedes its consumed frontier {frontier}")]
    SourceOrder {
        /// The bound source.
        connection: ConnectionGeneration,
        /// The refused stamp.
        at: SampleTime,
        /// The frontier it preceded.
        frontier: SampleTime,
    },
    /// A source packet whose serial is not the next in its source FIFO.
    #[error("source packet {packet:?} does not follow {previous:?}")]
    SourceSerial {
        /// The refused packet.
        packet: MixedSourcePacketId,
        /// The source's previous packet, or none when the first packet was skipped.
        previous: Option<MixedSourcePacketId>,
    },
    /// The host reported a source or raw-owner fault.
    #[error("source {connection:?} faulted: {fault:?}")]
    Source {
        /// The bound source.
        connection: ConnectionGeneration,
        /// The reported cause.
        fault: MixedSourceFault,
    },
    /// The raw owner halted for a cause other than a classified late raw packet, or
    /// halted before its exact cause could be collected. It is a host fault and is
    /// not attributed to either source connection.
    #[error("raw capture halted without a classified late outcome")]
    RawSelfHalt,
    /// The source handed over an onset while it already held as many onsets without
    /// key-ups as arm prepared pairing places for: the declared `held_onsets` is wrong.
    #[error("source {connection:?} onset {onset:?} exceeds the declared held onsets")]
    SourceHolds {
        /// The bound source.
        connection: ConnectionGeneration,
        /// The onset the bridge could not account for.
        onset: MixedSourcePacketId,
    },
    /// The next publication window from this audio boundary cannot be represented: the
    /// audio clock is exhausted and no later boundary can serve the head.
    #[error("the publication window from audio boundary {boundary} is not representable")]
    ClockExhausted {
        /// The audio boundary the pump read.
        boundary: SampleTime,
    },
    /// Defensive: the bridge's own tables disagreed with the merger's.
    #[error("the bridge ledger disagrees with the merger")]
    Ledger,
}

/// Why a bridge call stopped. The handoff, if the call took one, was consumed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeError {
    /// The call's first read observed the halt; nothing changed.
    #[error("the mixed bridge is halted: {0}")]
    Halted(MixedBridgeFault),
    /// This call detected the terminal fault and published the halt.
    #[error("the mixed bridge faulted: {0}")]
    Fault(MixedBridgeFault),
}

/// Why a pump forwarded nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedBridgeWait {
    /// No stage cell holds a packet.
    Empty,
    /// The only head waits for this unfenced peer's frontier to pass it.
    Peer {
        /// The peer that has not passed the head.
        source: ConnectionGeneration,
    },
    /// The head's execution stamp lies beyond the next publication window; it waits in
    /// its stage cell until the audio boundary brings it inside.
    Window {
        /// The execution stamp the head would carry.
        stamp: SampleTime,
        /// The window's exclusive end: the audio boundary plus one callback's quanta.
        end: SampleTime,
    },
    /// Too few command places are free; collecting results makes room.
    Commands,
    /// Too few bend credits are free; audio returns them as entries drain.
    BendCredit,
    /// A reset at `at` is sealed: nothing is forwarded until it is committed or
    /// abandoned. Only a key-up the bridge already refused leaves its lane.
    ResetSealed {
        /// The sealed reset boundary.
        at: SampleTime,
    },
}

/// One pump step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedPumpStep {
    /// The head became this command at this execution stamp.
    Forwarded {
        /// The handoff's source ID.
        packet: MixedSourcePacketId,
        /// The command.
        id: MixedTransferCommandId,
        /// The execution stamp the command carries.
        stamp: SampleTime,
    },
    /// The head was refused for sound and left its lane; this return settles it.
    Split {
        /// The handoff's source ID.
        packet: MixedSourcePacketId,
        /// Why.
        split: MixedSplit,
    },
    /// The staged host Panic became this command at this execution stamp.
    Panic {
        /// The command.
        id: MixedTransferCommandId,
        /// The execution stamp the command carries.
        stamp: SampleTime,
    },
    /// Nothing was forwarded.
    Idle(MixedBridgeWait),
}

/// What audio did with one forwarded command, named by source IDs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MixedBridgeOutcome {
    /// The onset's identity, or the audio refusal its tombstone now carries.
    NoteOn(Result<NoteIdentity, IngressRefused>),
    /// The key-up of `onset`: its retirement, or an ingress refusal that left it sounding.
    KeyUp {
        /// The paired onset's source ID.
        onset: MixedSourcePacketId,
        /// How it retired.
        result: Result<MixedKeyUpRetired, IngressRefused>,
    },
    /// The bend's members, or a refusal that changed nothing.
    Bend {
        /// The source channel.
        channel: MidiChannel,
        /// The members or refusal.
        result: Result<MixedTransferBendApplied, MixedTransferBendRefused>,
    },
    /// Audio's table held no cell the command could use; nothing changed.
    HandleRefused,
    /// An effective pedal-down: how audio applied it, or a refusal that changed nothing.
    PedalDown {
        /// The source channel.
        channel: MidiChannel,
        /// How audio applied it, or why it refused.
        result: Result<MixedTransferPedalDown, MixedTransferPedalRefused>,
    },
    /// The lift paired with `down`: the sustained notes its release group ended, or a
    /// refusal that left the pedal down and every sustained note sounding.
    Lift {
        /// The source channel.
        channel: MidiChannel,
        /// The paired pedal-down's source ID.
        down: MixedSourcePacketId,
        /// The ended notes by their onsets' source IDs, or why audio refused.
        result: Result<MixedBridgeLifted, MixedTransferPedalRefused>,
    },
    /// The host Panic: the live notes of both sources it ended, or a refusal that changed
    /// nothing.
    Panic(Result<MixedBridgePanicked, MixedTransferPanicRefused>),
}

/// How one applied host Panic ended one live note, named by its onset's source ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedBridgePanicEnd {
    /// The key was up under its scope's pedal: the note's handle retired with the Panic.
    Retired(MixedSourcePacketId),
    /// The key was down: the handle stays open on a Panic tombstone until the paired
    /// key-up, which is consumed as [`MixedKeyUpRetired::AfterPanic`].
    Tombstoned(MixedSourcePacketId),
}

impl MixedBridgePanicEnd {
    /// The ended note's onset.
    pub const fn onset(self) -> MixedSourcePacketId {
        match self {
            Self::Retired(onset) | Self::Tombstoned(onset) => onset,
        }
    }
}

/// The live notes of both sources one applied host Panic ended as one release group, in
/// group order, and the sample it executed at. The compiled producer's open notes ended
/// at the same sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedBridgePanicked {
    executed_at: SampleTime,
    ended: [Option<MixedBridgePanicEnd>; RELEASE_GROUP_CAPACITY],
}

impl MixedBridgePanicked {
    /// The sample the release group and the compiled cut executed at.
    pub const fn executed_at(&self) -> SampleTime {
        self.executed_at
    }

    /// The ended notes, in group order.
    pub fn ended(&self) -> impl Iterator<Item = MixedBridgePanicEnd> + '_ {
        self.ended.iter().flatten().copied()
    }

    /// How many live notes the Panic ended.
    pub fn count(&self) -> MixedTransferHeldCount {
        MixedTransferHeldCount(self.ended.iter().flatten().count())
    }
}

/// Why [`MixedBridgeControl::panic`] refused at the caller boundary. Nothing changed and
/// nothing was accepted, so nothing is dropped later.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgePanicRefusal {
    /// The bridge is halted.
    #[error("the mixed bridge is halted: {0}")]
    Halted(MixedBridgeFault),
    /// An earlier Panic holds the cell until its result is collected.
    #[error("an earlier Panic at {at} is not yet collected")]
    Pending {
        /// The earlier Panic's nominal stamp.
        at: SampleTime,
    },
    /// The stamp precedes the last accepted Panic's.
    #[error("Panic at {at} precedes the last accepted Panic at {previous}")]
    BehindPanic {
        /// The refused stamp.
        at: SampleTime,
        /// The last accepted Panic's nominal stamp.
        previous: SampleTime,
    },
    /// Audio has not opened the quantum after the last applied Panic's execution sample,
    /// so a second compiled cut could fall into that quantum.
    #[error("audio has not opened the quantum after the Panic executed at {previous}")]
    SameServiceQuantum {
        /// The last applied Panic's execution sample.
        previous: SampleTime,
    },
}

/// The sustained notes one applied lift ended, by their onsets' source IDs, in release
/// group order. A lift of a scope that sustained nothing ends none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MixedBridgeLifted {
    /// The lifted scope's source: a lift ends only its own source's notes.
    connection: ConnectionGeneration,
    serials: [Option<NonZeroU64>; RELEASE_GROUP_CAPACITY],
}

impl MixedBridgeLifted {
    /// The ended notes' onsets, in release group order.
    pub fn onsets(&self) -> impl Iterator<Item = MixedSourcePacketId> + '_ {
        let connection = self.connection;
        self.serials
            .iter()
            .flatten()
            .map(move |serial| MixedSourcePacketId {
                connection,
                serial: serial.get(),
            })
    }

    /// How many sustained notes the lift ended.
    pub fn count(&self) -> MixedTransferHeldCount {
        MixedTransferHeldCount(self.serials.iter().flatten().count())
    }
}

/// One settled forwarded message.
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct MixedBridgeReport {
    id: MixedTransferCommandId,
    packet: Option<MixedSourcePacketId>,
    nominal_at: SampleTime,
    stamp: SampleTime,
    executed_at: SampleTime,
    outcome: MixedBridgeOutcome,
}

impl MixedBridgeReport {
    /// The command.
    pub const fn id(&self) -> MixedTransferCommandId {
        self.id
    }

    /// The handoff's source ID; its connection names the source. `None` for a host
    /// Panic, which no source sent.
    pub const fn packet(&self) -> Option<MixedSourcePacketId> {
        self.packet
    }

    /// The source's nominal stamp, which raw capture keeps, or the stamp a host Panic
    /// was accepted with.
    pub const fn nominal_at(&self) -> SampleTime {
        self.nominal_at
    }

    /// The execution stamp the command carried: `max(nominal, first unrendered
    /// boundary, last forwarded)`.
    pub const fn stamp(&self) -> SampleTime {
        self.stamp
    }

    /// Where audio executed it: the stamp, or the boundary that served it when the
    /// stamp was already behind that boundary.
    pub const fn executed_at(&self) -> SampleTime {
        self.executed_at
    }

    /// What audio did.
    pub const fn outcome(&self) -> MixedBridgeOutcome {
        self.outcome
    }
}

/// Where joined teardown found a forwarded command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedCommandPlace {
    /// In control's pending slot after a failed physical push.
    Unsent,
    /// Drained from the command queue.
    Unserved,
    /// In audio's invalid-command slot.
    InvalidCommand,
    /// A Panic audio held unserved until the quantum of its execution stamp opened.
    Parked,
    /// In audio's pending slot after a failed result push.
    PendingResult,
    /// Drained from the result queue.
    Uncollected,
    /// In control's invalid-result slot.
    InvalidResult,
    /// In no queue or slot; the transfer's custody is `Lost`.
    Lost,
    /// A wrapper was missing, so its queue's contents are unknown.
    Unknown,
}

/// What the bridge still held for one source packet, or for its host Panic, at teardown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedSourceCustody {
    /// In a stage cell, never forwarded.
    Staged,
    /// Forwarded as `id` and not yet reported.
    Command {
        /// The command.
        id: MixedTransferCommandId,
        /// Where teardown found it.
        place: MixedCommandPlace,
    },
}

/// Joined teardown of the bridge and its transfer.
#[derive(Debug)]
#[must_use]
pub struct MixedBridgeTeardown {
    transfer: MixedTransferTeardown,
    fault: Option<MixedBridgeFault>,
    held: Vec<(MixedSourcePacketId, MixedSourceCustody)>,
    open: Vec<(Option<MixedSourcePacketId>, MixedTransferHandleState)>,
    tombstones: Vec<(MixedSourcePacketId, MixedOnsetRefusal)>,
    pedals: Vec<(MixedSourcePacketId, Option<MixedOnsetRefusal>)>,
    panic: Option<MixedSourceCustody>,
}

impl MixedBridgeTeardown {
    /// The transfer's own teardown: queues, slots, handles, renderer and counters.
    pub const fn transfer(&self) -> &MixedTransferTeardown {
        &self.transfer
    }

    /// The bridge's terminal fault, if any half recorded one.
    pub const fn fault(&self) -> Option<MixedBridgeFault> {
        self.fault
    }

    /// Every source ID the bridge still held, each exactly once: stage cells in lane
    /// order, then forwarded commands in command order.
    pub fn held(&self) -> &[(MixedSourcePacketId, MixedSourceCustody)] {
        &self.held
    }

    /// Every merger handle still held, by its onset's source ID. `None` only if the
    /// bridge's handle table disagrees with the merger's, which is a defect.
    pub fn open(&self) -> &[(Option<MixedSourcePacketId>, MixedTransferHandleState)] {
        &self.open
    }

    /// Refused onsets whose key-ups have not arrived, oldest first per source.
    pub fn tombstones(&self) -> &[(MixedSourcePacketId, MixedOnsetRefusal)] {
        &self.tombstones
    }

    /// Every effective pedal-down whose lift has not arrived, by source and then channel:
    /// `None` while it protects its lift's cell, or the refusal its pedal tombstone
    /// carries.
    pub fn pedals(&self) -> &[(MixedSourcePacketId, Option<MixedOnsetRefusal>)] {
        &self.pedals
    }

    /// The host Panic the bridge still held: accepted and never forwarded, or forwarded
    /// and not yet reported, with where teardown found its command.
    pub const fn panic(&self) -> Option<MixedSourceCustody> {
        self.panic
    }
}

/// Why [`MixedBridgeControl::seal_reset`] refused; both bridges are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeSealRefusal {
    /// Either bridge recorded or observed a terminal fault.
    #[error("a reset bridge is faulted")]
    Faulted,
    /// A reset is already sealed.
    #[error("a reset at {at} is already sealed")]
    AlreadySealed {
        /// The sealed boundary.
        at: SampleTime,
    },
    /// The successor binds other source connections or another tie order.
    #[error("the reset successor binds different source connections")]
    Sources,
    /// The successor's stage, pairing, journal, handle, peer-wait or window bounds differ.
    #[error("the reset successor uses different bridge bounds")]
    Bounds,
    /// The successor has consumed a source packet, minted a command or is itself sealed.
    #[error("the reset successor is not a fresh bridge")]
    SuccessorBusy,
    /// The boundary is not a quantum boundary.
    #[error("reset boundary {at} is not a quantum boundary")]
    NotBoundary {
        /// The refused boundary.
        at: SampleTime,
    },
    /// The old audio clock has already passed the boundary.
    #[error("reset boundary {at} precedes the audio boundary {boundary}")]
    Passed {
        /// The refused boundary.
        at: SampleTime,
        /// The old owner's published audio boundary.
        boundary: SampleTime,
    },
    /// A command already forwarded executes at or after the boundary, so the old owner
    /// would still hold work at `B`. Stamps never decrease, so this is exact.
    #[error("forwarded stamp {last} is not before reset boundary {at}")]
    Forwarded {
        /// The last forwarded execution stamp.
        last: SampleTime,
        /// The refused boundary.
        at: SampleTime,
    },
    /// Defensive: the old transfer was armed without a reset seal.
    #[error("the old bridge transfer carries no reset seal")]
    Unsealable,
    /// The Panic cell holds a Panic whose result is not yet collected. The seal waits
    /// for it, so a sealed reset never has a Panic in flight to the old transfer.
    #[error("a Panic at {at} is not yet collected")]
    PanicPending {
        /// The Panic's nominal stamp.
        at: SampleTime,
    },
    /// A due Panic cancelled the last sealed reset; abandon it first.
    #[error("a Panic cancelled the reset at {at}; abandon it first")]
    Cancelled {
        /// The cancelled boundary.
        at: SampleTime,
    },
}

/// A refused seal with the successor bridge, unchanged.
#[derive(Debug)]
#[must_use]
pub struct MixedBridgeSealRefused {
    reason: MixedBridgeSealRefusal,
    successor: MixedBridgeControl,
}

impl MixedBridgeSealRefused {
    /// Why the seal was refused.
    pub const fn reason(&self) -> MixedBridgeSealRefusal {
        self.reason
    }

    /// The successor bridge, unchanged.
    pub fn into_successor(self) -> MixedBridgeControl {
        self.successor
    }
}

/// Why [`MixedBridgeControl::commit_reset`] refused; nothing changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeCommitRefusal {
    /// No reset is sealed.
    #[error("no reset is sealed")]
    NotSealed,
    /// A due Panic cancelled the sealed reset before audio decided it, so audio cannot
    /// have applied it; abandon it instead.
    #[error("a Panic cancelled the reset at {at}")]
    Cancelled {
        /// The cancelled boundary.
        at: SampleTime,
    },
    /// The receipt names another old arm, another successor or another boundary.
    #[error("the reset receipt does not name the sealed reset")]
    ForeignReceipt,
    /// The successor's audio has not yet published a boundary at or after `B`, so
    /// staged heads could be stamped before it.
    #[error("the successor's audio boundary {boundary} precedes reset boundary {at}")]
    NotReached {
        /// The successor's published audio boundary.
        boundary: SampleTime,
        /// The sealed boundary.
        at: SampleTime,
    },
}

/// Why [`MixedBridgeControl::abandon_reset`] refused; nothing changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeAbandonRefusal {
    /// No reset is sealed or cancelled.
    #[error("no reset is sealed")]
    NotSealed,
    /// The returned owner is not the sealed successor's audio half.
    #[error("the returned owner is not the sealed successor")]
    ForeignOwner,
    /// The returned owner was installed at a boundary, so the reset was applied and must
    /// be committed.
    #[error("the sealed successor was installed")]
    Installed,
}

/// Why a bridge join returned everything unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum MixedBridgeJoinError {
    /// The transfer refused the halves or wrappers.
    #[error(transparent)]
    Transfer(MixedTransferJoinError),
    /// A reset is sealed; commit or abandon it first.
    #[error("a reset at {at} is sealed; commit or abandon it before joining")]
    ResetSealed {
        /// The sealed boundary.
        at: SampleTime,
    },
    /// A Panic cancelled the reset; abandon it first, so the successor reaches its own
    /// teardown.
    #[error("a Panic cancelled the reset at {at}; abandon it before joining")]
    ResetCancelled {
        /// The cancelled boundary.
        at: SampleTime,
    },
}

/// Where the bridge's stopped-owner reset stands, as the control side last changed it.
/// Only the control thread changes it, so it is exact after each call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedBridgeResetPhase {
    /// No reset is sealed.
    Open,
    /// A reset is sealed for `at`. Audio may already have decided it there; the wrapper
    /// reports that decision.
    Sealed {
        /// The sealed boundary.
        at: SampleTime,
    },
    /// The Panic accepted with nominal stamp `panic` won the seal before audio decided
    /// it: the reset at `at` can no longer apply, and the lanes and the Panic forward to
    /// the old transfer. Abandon returns the successor.
    Cancelled {
        /// The cancelled boundary.
        at: SampleTime,
        /// The cancelling Panic's nominal stamp, not its execution sample.
        panic: SampleTime,
    },
}

/// How retired teardown accounted for one old merger handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MixedRetiredHandle {
    /// The reset's `Reset` group ended its occurrence.
    Ended(MixedBridgeResetEnd),
    /// Audio refused the onset, so the old owner holds only its refusal tombstone; the
    /// key-up was consumed as a reset.
    RefusedOnset,
    /// A command the old journal still holds names it; [`MixedBridgeTeardown::held`]
    /// gives that command's place.
    Journaled,
}

/// The old transfer's custody after a committed reset: its journal, its handle table
/// and the receipt. It collects the old results and joins the old arm off-thread. It
/// cannot be sent to the audio thread.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedBridgeRetiredControl;
/// fn require_send<T: Send>() {}
/// require_send::<MixedBridgeRetiredControl>();
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedBridgeRetiredControl {
    transfer: MixedTransferControl,
    /// A late old-side fault must halt the still-running successor too.
    live_halt: Arc<AtomicBool>,
    forwarded: Fixed<Forwarded>,
    handles: Fixed<Handle>,
    receipt: MixedBridgeResetReceipt,
    fault: Option<MixedBridgeFault>,
}

/// Joined teardown of a retired old transfer.
#[derive(Debug)]
#[must_use]
pub struct MixedBridgeRetiredTeardown {
    bridge: MixedBridgeTeardown,
    receipt: MixedBridgeResetReceipt,
    handles: Vec<(MixedSourcePacketId, MixedRetiredHandle)>,
    unclassified: Vec<(Option<MixedSourcePacketId>, MixedTransferHandleState)>,
    unmatched: Vec<MixedBridgeResetEnd>,
}

impl MixedBridgeRetiredTeardown {
    /// The old transfer's bridge teardown: its uncollected commands in `held`, every
    /// merger handle in `open`, and no lanes or tombstones, which stayed with the bridge.
    pub const fn bridge(&self) -> &MixedBridgeTeardown {
        &self.bridge
    }

    /// The reset that retired the old owner.
    pub const fn receipt(&self) -> MixedBridgeResetReceipt {
        self.receipt
    }

    /// Every old merger handle the receipt or the old journal accounts for, by its
    /// onset's source ID, each exactly once.
    pub fn handles(&self) -> &[(MixedSourcePacketId, MixedRetiredHandle)] {
        &self.handles
    }

    /// Old merger handles neither the receipt nor the old journal accounts for. Not
    /// empty only on a defect.
    pub fn unclassified(&self) -> &[(Option<MixedSourcePacketId>, MixedTransferHandleState)] {
        &self.unclassified
    }

    /// Occurrences the receipt ended that the old merger did not hold. Not empty only
    /// on a defect.
    pub fn unmatched(&self) -> &[MixedBridgeResetEnd] {
        &self.unmatched
    }
}

#[derive(Clone, Copy, Debug)]
enum EntryKind {
    Onset {
        channel: MidiChannel,
        key: KeyIdentity,
        velocity: NoteVelocity,
    },
    KeyUp {
        onset: MixedSourcePacketId,
    },
    RefusedKeyUp {
        onset: MixedSourcePacketId,
        reason: MixedOnsetRefusal,
    },
    Bend {
        channel: MidiChannel,
        cents: Cents,
    },
    PedalDown {
        channel: MidiChannel,
    },
    Lift {
        channel: MidiChannel,
        down: MixedSourcePacketId,
    },
    RefusedLift {
        down: MixedSourcePacketId,
        reason: MixedOnsetRefusal,
    },
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    packet: MixedSourcePacketId,
    at: SampleTime,
    kind: EntryKind,
}

/// A FIFO list prepared at arm that never grows past `limit`. A push into a full list
/// is refused and returns the item, so the caller keeps the obligation it names.
#[derive(Debug)]
struct Fixed<T> {
    items: VecDeque<T>,
    limit: usize,
}

impl<T> Fixed<T> {
    fn new(limit: usize) -> Self {
        Self {
            items: VecDeque::with_capacity(limit),
            limit,
        }
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn is_full(&self) -> bool {
        self.items.len() >= self.limit
    }

    fn push(&mut self, item: T) -> Result<(), T> {
        if self.is_full() {
            return Err(item);
        }
        self.items.push_back(item);
        Ok(())
    }

    fn front(&self) -> Option<&T> {
        self.items.front()
    }

    fn pop_front(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut()
    }

    /// Remove and return the first item `matches` selects, keeping the others in order.
    fn take(&mut self, matches: impl Fn(&T) -> bool) -> Option<T> {
        let index = self.items.iter().position(matches)?;
        self.items.remove(index)
    }
}

/// One onset of a source whose key-up has not reached the stage.
#[derive(Clone, Copy, Debug)]
struct Held {
    onset: MixedSourcePacketId,
    channel: MidiChannel,
    key: KeyIdentity,
    /// `None` while the onset is staged or forwarded and holds a protected cell; once
    /// the bridge refused it, the refusal its key-up is consumed with: a tombstone.
    refused: Option<MixedOnsetRefusal>,
}

/// One channel's effective pedal-down whose lift is not yet staged.
#[derive(Clone, Copy, Debug)]
struct Pedal {
    down: MixedSourcePacketId,
    /// `None` while the down is staged or forwarded and protects a cell for its lift;
    /// once the bridge refused it, the refusal its lift is consumed with: a tombstone.
    refused: Option<MixedOnsetRefusal>,
}

/// One source's custody. `entries` and the protected places share the stage's cells.
/// `held` names, oldest first, every onset of this source whose key-up is not yet
/// staged: each one awaiting its key-up holds one protected cell, and each refused one
/// is a tombstone in its FIFO place. A key-up pairs with the first entry of its channel
/// and key. `pedals` holds, per channel, the source's effective pedal-down whose lift is
/// not yet staged; a scope without one has its pedal up.
#[derive(Debug)]
struct Lane {
    entries: Fixed<Entry>,
    held: Fixed<Held>,
    /// The entries of `held` that are not refused.
    protected: usize,
    pedals: [Option<Pedal>; MIDI_CHANNELS],
    /// The entries of `pedals` that are not refused.
    pedal_protected: usize,
    frontier: Option<SampleTime>,
    last: Option<SampleTime>,
    /// The last packet of this source the bridge consumed, message or frontier.
    previous: Option<MixedSourcePacketId>,
}

impl Lane {
    fn new(cells: usize, held_onsets: usize) -> Self {
        Self {
            entries: Fixed::new(cells),
            held: Fixed::new(held_onsets),
            protected: 0,
            pedals: [None; MIDI_CHANNELS],
            pedal_protected: 0,
            frontier: None,
            last: None,
            previous: None,
        }
    }

    /// No later message of this source can be stamped before this.
    fn bound(&self) -> Option<SampleTime> {
        match (self.frontier, self.last) {
            (Some(frontier), Some(last)) => Some(frontier.max(last)),
            (frontier, last) => frontier.or(last),
        }
    }

    fn charged(&self) -> usize {
        self.entries.len() + self.protected + self.pedal_protected
    }

    fn tombstones(&self) -> usize {
        self.held.len() - self.protected
    }

    /// Turn the awaiting `onset` into a tombstone carrying `reason` in its FIFO place,
    /// releasing its protected cell. `false` if `onset` is not awaiting its key-up here.
    fn refuse_awaiting(&mut self, onset: MixedSourcePacketId, reason: MixedOnsetRefusal) -> bool {
        for held in self.held.iter_mut() {
            if held.onset == onset && held.refused.is_none() {
                held.refused = Some(reason);
                self.protected -= 1;
                return true;
            }
        }
        false
    }

    /// Turn the effective pedal-down `down` on `channel` into a pedal tombstone carrying
    /// `reason`, releasing its protected cell. `false` if `down` is not awaiting its lift.
    fn refuse_pedal(
        &mut self,
        channel: MidiChannel,
        down: MixedSourcePacketId,
        reason: MixedOnsetRefusal,
    ) -> bool {
        if let Some(Some(pedal)) = self.pedals.get_mut(usize::from(channel.as_index()))
            && pedal.down == down
            && pedal.refused.is_none()
        {
            pedal.refused = Some(reason);
            self.pedal_protected -= 1;
            return true;
        }
        false
    }

    /// Make the staged lift paired with `down` carry `reason`. `false` if no staged lift
    /// pairs with it.
    fn refuse_staged_lift(&mut self, down: MixedSourcePacketId, reason: MixedOnsetRefusal) -> bool {
        for staged in self.entries.iter_mut() {
            if let EntryKind::Lift { down: paired, .. } = staged.kind
                && paired == down
            {
                staged.kind = EntryKind::RefusedLift { down, reason };
                return true;
            }
        }
        false
    }

    /// Make the staged key-up paired with `onset` carry `reason`. `false` if no staged
    /// key-up pairs with it.
    fn refuse_staged_key_up(
        &mut self,
        onset: MixedSourcePacketId,
        reason: MixedOnsetRefusal,
    ) -> bool {
        for staged in self.entries.iter_mut() {
            if let EntryKind::KeyUp { onset: paired } = staged.kind
                && paired == onset
            {
                staged.kind = EntryKind::RefusedKeyUp { onset, reason };
                return true;
            }
        }
        false
    }
}

/// Whether `onset` is still staged in this lane, never minted.
fn stages_onset(entries: &Fixed<Entry>, onset: MixedSourcePacketId) -> bool {
    entries
        .iter()
        .any(|entry| entry.packet == onset && matches!(entry.kind, EntryKind::Onset { .. }))
}

/// Whether the pedal-down `down` is still staged in this lane, never minted.
fn stages_pedal(entries: &Fixed<Entry>, down: MixedSourcePacketId) -> bool {
    entries
        .iter()
        .any(|entry| entry.packet == down && matches!(entry.kind, EntryKind::PedalDown { .. }))
}

#[derive(Clone, Copy, Debug)]
struct Handle {
    onset: MixedSourcePacketId,
    occurrence: LiveOccurrenceId,
    /// Audio refused the onset; its handle stays open on a refusal tombstone.
    refused: bool,
}

#[derive(Clone, Copy, Debug)]
enum Forward {
    Onset,
    KeyUp {
        onset: MixedSourcePacketId,
    },
    Bend {
        channel: MidiChannel,
    },
    PedalDown {
        channel: MidiChannel,
    },
    Lift {
        channel: MidiChannel,
        down: MixedSourcePacketId,
    },
}

#[derive(Clone, Copy, Debug)]
struct Forwarded {
    id: MixedIngressCommandId,
    packet: MixedSourcePacketId,
    nominal: SampleTime,
    stamp: SampleTime,
    kind: Forward,
}

/// The bridge's one host Panic cell.
#[derive(Clone, Copy, Debug)]
enum PanicCell {
    Empty,
    /// Accepted at the caller boundary with this nominal stamp and not yet minted.
    Staged {
        at: SampleTime,
    },
    /// Minted as `id` at execution stamp `stamp`; its result is not yet collected.
    Forwarded {
        id: MixedIngressCommandId,
        nominal: SampleTime,
        stamp: SampleTime,
    },
}

impl PanicCell {
    /// The nominal stamp of the Panic the cell holds.
    const fn at(self) -> Option<SampleTime> {
        match self {
            Self::Empty => None,
            Self::Staged { at } | Self::Forwarded { nominal: at, .. } => Some(at),
        }
    }
}

/// The next head selection forwards.
#[derive(Clone, Copy, Debug)]
enum Head {
    Lane(usize),
    Panic(SampleTime),
}

#[derive(Debug)]
struct BridgeState {
    publication_root: super::MixedTransferBinding,
    sources: MixedSourceSet,
    lanes: [Lane; 2],
    cells: usize,
    peer_wait: FrameCount,
    /// One callback of the profile's largest block, in whole quanta.
    window: FrameCount,
    boundary: Arc<AtomicU64>,
    last_stamp: Option<SampleTime>,
    /// One per merger handle the bridge opened and has not seen retired; at most the
    /// merger's occurrence cells.
    handles: Fixed<Handle>,
    /// One per forwarded command not yet collected; at most the admitted command bound.
    forwarded: Fixed<Forwarded>,
    /// The host Panic; its command, once minted, takes the protected place.
    panic: PanicCell,
    /// The nominal stamp of the last accepted Panic.
    last_panic: Option<SampleTime>,
    fault: Option<MixedBridgeFault>,
    reset: BridgeReset,
}

/// Whether a stopped-owner reset is sealed.
#[derive(Debug)]
enum BridgeReset {
    Open,
    /// Nothing is forwarded to the old transfer. The successor is fresh, binds the same
    /// sources and bounds, and is never pumped while here. The Panic cell is never
    /// `Forwarded` here: the seal refuses an occupied cell, and pump forwards nothing.
    Sealed {
        at: SampleTime,
        successor: Box<MixedBridgeControl>,
        /// The successor was already halted when the old bridge found its first fault.
        successor_first: bool,
    },
    /// The Panic stamped `panic` won the seal's compare-and-swap, so audio refuses the
    /// successor at `B`. Everything forwards to the old transfer as when open; only
    /// abandon leaves this state, returning the successor.
    Cancelled {
        at: SampleTime,
        panic: SampleTime,
        successor: Box<MixedBridgeControl>,
    },
}

/// The armed bridge halves and the host's two bound wrappers.
#[must_use]
pub struct MixedBridgeArmed<CQ, RQ>
where
    CQ: MixedTransferQueue<MixedTransferCommand>,
    RQ: MixedTransferQueue<MixedTransferResult>,
{
    /// The off-thread control half: stage, merger, journal and frontiers.
    pub control: MixedBridgeControl,
    /// The audio half, to move to the callback thread.
    pub audio: MixedTransferAudio,
    /// The command producer and result consumer, lent to every pump and collect.
    pub control_port: MixedTransferControlPort<CQ::Producer, RQ::Consumer>,
    /// The command consumer and result producer, lent to every callback.
    pub audio_port: MixedTransferAudioPort<CQ::Consumer, RQ::Producer>,
}

impl<CQ, RQ> std::fmt::Debug for MixedBridgeArmed<CQ, RQ>
where
    CQ: MixedTransferQueue<MixedTransferCommand>,
    RQ: MixedTransferQueue<MixedTransferResult>,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedBridgeArmed")
            .field("control", &self.control)
            .field("audio", &self.audio)
            .field("control_port", &self.control_port)
            .field("audio_port", &self.audio_port)
            .finish()
    }
}

impl MixedJoinedPrepared {
    /// Arm the two-source bridge over the bound initial one-shot plan.
    ///
    /// Both queues must report at least the admitted command/result bound plus Panic's
    /// protected place and be empty, the release share must cover one compiled Panic cut
    /// beside the admitted bound, both bend tables must cover both sources' 32 channel
    /// scopes; each effective
    /// pedal-down owns an ingress place or a reservation for its lift, so the pedal
    /// tables need only cover that depth;
    /// each stage lane must hold between two and 256 cells, and each source's pairing
    /// table between one and 65 536 held onsets. Every refusal returns the owner, the
    /// candidate and both queues unsplit and untouched.
    ///
    /// # Errors
    ///
    /// A stage-lane, held-onset, publication-window, queue, bend-table,
    /// binding or one-shot owner refusal.
    pub fn arm_bridge_rehearsal<CQ, RQ>(
        self,
        candidate: MixedStampedCandidate,
        profile: &HostProfile,
        sources: MixedTransferSources,
        fence: MixedTransferFence,
        commands: CQ,
        results: RQ,
    ) -> Result<MixedBridgeArmed<CQ, RQ>, Box<MixedTransferArmRefusal<CQ, RQ>>>
    where
        CQ: MixedTransferQueue<MixedTransferCommand>,
        RQ: MixedTransferQueue<MixedTransferResult>,
    {
        let refuse = |reason, owner, candidate, commands, results| {
            Err(Box::new(MixedTransferArmRefusal {
                reason,
                owner,
                candidate,
                commands,
                results,
            }))
        };
        let cells = fence.stage_cells.get();
        if !(2..=MAX_STAGE_CELLS).contains(&cells) {
            let reason = MixedTransferArmError::StageCells {
                cells: fence.stage_cells,
                maximum: MixedTransferPlaces(MAX_STAGE_CELLS),
            };
            return refuse(reason, self, candidate, commands, results);
        }
        let held_onsets = fence.held_onsets.get();
        if !(1..=MAX_HELD_ONSETS).contains(&held_onsets) {
            let reason = MixedTransferArmError::HeldOnsets {
                onsets: fence.held_onsets,
                maximum: MixedTransferPlaces(MAX_HELD_ONSETS),
            };
            return refuse(reason, self, candidate, commands, results);
        }
        let block = profile.capabilities().maximum_block_size();
        let horizon = profile.limits().events().forward_event_horizon();
        let window = block
            .quanta_spanned()
            .checked_mul(u64::from(QUANTUM_FRAMES))
            .map(FrameCount::new)
            .filter(|window| *window <= horizon);
        let Some(window) = window else {
            let reason = MixedTransferArmError::PublicationWindow { block, horizon };
            return refuse(reason, self, candidate, commands, results);
        };
        let tables = MixedBendTables::for_capacity(MixedIngressQueueCount(
            Self::transfer_capacity(profile).get(),
        ));
        let MixedTransferArmed {
            mut control,
            mut audio,
            control_port,
            audio_port,
        } = self.arm_transfer_for_scopes(
            candidate,
            profile,
            commands,
            results,
            MixedTransferShape {
                tables,
                scopes: BRIDGE_SCOPES,
                panic: MixedPanicCapability::Available,
            },
        )?;
        control.attach_reset_seal(&mut audio);
        let state = BridgeState {
            publication_root: control.binding,
            sources: sources.set,
            lanes: [Lane::new(cells, held_onsets), Lane::new(cells, held_onsets)],
            cells,
            peer_wait: fence.peer_wait,
            window,
            boundary: Arc::clone(&audio.boundary),
            last_stamp: None,
            handles: Fixed::new(control.inner.occurrence_ledger.width()),
            forwarded: Fixed::new(Self::transfer_capacity(profile).get()),
            panic: PanicCell::Empty,
            last_panic: None,
            fault: None,
            reset: BridgeReset::Open,
        };
        Ok(MixedBridgeArmed {
            control: MixedBridgeControl {
                transfer: control,
                state,
            },
            audio,
            control_port,
            audio_port,
        })
    }
}

/// The bridge's off-thread control half. It cannot be sent to the audio thread, and it
/// exposes no submit around its stage.
///
/// ```compile_fail
/// use synth_engine_v2::stream::MixedBridgeControl;
/// fn require_send<T: Send>() {}
/// require_send::<MixedBridgeControl>();
/// ```
#[derive(Debug)]
#[must_use]
pub struct MixedBridgeControl {
    transfer: MixedTransferControl,
    state: BridgeState,
}

const fn lane_of(bound: MixedBoundSource) -> usize {
    match bound {
        MixedBoundSource::First => 0,
        MixedBoundSource::Second => 1,
    }
}

const fn bound_of(lane: usize) -> MixedBoundSource {
    if lane == 0 {
        MixedBoundSource::First
    } else {
        MixedBoundSource::Second
    }
}

impl MixedBridgeControl {
    /// Closed source identities in the bridge's deterministic tie order.
    pub const fn sources(&self) -> MixedTransferSources {
        MixedTransferSources {
            set: self.state.sources,
        }
    }
    /// Publish a terminal, source-neutral raw self-halt. The host calls this only
    /// after checking the raw owner's exact collected outcomes and direct render or
    /// preparation errors. An unknown cause is terminal and cannot be revived; it
    /// is never blamed on a source by default.
    pub fn report_raw_self_halt(&mut self) -> MixedBridgeError {
        if let Some(halted) = self.halted() {
            return halted;
        }
        self.halt(MixedBridgeFault::RawSelfHalt)
    }

    /// Seal a stopped-owner reset at the quantum boundary `at` for `successor`: a freshly
    /// armed bridge of the same sources, in the same tie order, with the same bounds.
    /// Off-thread.
    ///
    /// From this call nothing is forwarded to the old transfer. The seal publishes the old
    /// transfer's last minted command and the successor's binding; audio refuses the
    /// successor, at its offer and at `B`, unless the seal names it and the old owner has
    /// served that command. Staging and collection are unchanged. All-or-nothing: a
    /// refusal returns the successor unchanged and leaves this bridge as it was.
    ///
    /// # Errors
    ///
    /// A faulted bridge, a seal already in place, a cancelled reset not yet abandoned, an
    /// uncollected Panic, a successor of other sources or bounds or with consumed work,
    /// a boundary that is not a quantum boundary at or after the audio boundary, a
    /// command already forwarded at or after `at`, or an old transfer armed without a seal.
    pub fn seal_reset(
        &mut self,
        successor: Self,
        at: SampleTime,
    ) -> Result<(), Box<MixedBridgeSealRefused>> {
        let refused = self.seal_refusal(&successor, at).or_else(|| {
            // Publishing is the last step, so a refusal here leaves the seal unset.
            (!self.transfer.seal_reset_for(successor.transfer.binding))
                .then_some(MixedBridgeSealRefusal::Unsealable)
        });
        if let Some(reason) = refused {
            return Err(Box::new(MixedBridgeSealRefused { reason, successor }));
        }
        self.state.reset = BridgeReset::Sealed {
            at,
            successor: Box::new(successor),
            successor_first: false,
        };
        Ok(())
    }

    fn seal_refusal(&self, successor: &Self, at: SampleTime) -> Option<MixedBridgeSealRefusal> {
        if self.fault().is_some() || successor.fault().is_some() {
            return Some(MixedBridgeSealRefusal::Faulted);
        }
        match self.state.reset {
            BridgeReset::Open => {}
            BridgeReset::Sealed { at, .. } => {
                return Some(MixedBridgeSealRefusal::AlreadySealed { at });
            }
            BridgeReset::Cancelled { at, .. } => {
                return Some(MixedBridgeSealRefusal::Cancelled { at });
            }
        }
        if let Some(at) = self.state.panic.at() {
            return Some(MixedBridgeSealRefusal::PanicPending { at });
        }
        if [MixedBoundSource::First, MixedBoundSource::Second]
            .into_iter()
            .any(|source| {
                self.state.sources.generation(source) != successor.state.sources.generation(source)
            })
        {
            return Some(MixedBridgeSealRefusal::Sources);
        }
        let (old, next) = (&self.state, &successor.state);
        if old.cells != next.cells
            || old.peer_wait != next.peer_wait
            || old.window != next.window
            || old.forwarded.limit != next.forwarded.limit
            || old.handles.limit != next.handles.limit
            || old.lanes.iter().zip(&next.lanes).any(|(old, next)| {
                old.entries.limit != next.entries.limit || old.held.limit != next.held.limit
            })
        {
            return Some(MixedBridgeSealRefusal::Bounds);
        }
        let inner = &successor.transfer.inner;
        if next.lanes.iter().any(|lane| {
            !lane.entries.is_empty()
                || !lane.held.is_empty()
                || lane.pedals.iter().any(Option::is_some)
                || lane.previous.is_some()
                || lane.frontier.is_some()
                || lane.last.is_some()
        }) || !next.forwarded.is_empty()
            || !next.handles.is_empty()
            || next.last_stamp.is_some()
            || next.panic.at().is_some()
            || next.last_panic.is_some()
            || !matches!(next.reset, BridgeReset::Open)
            || inner.last_ingress_id.is_some()
            || inner.ingress_outstanding != MixedIngressQueueCount::NONE
            || inner.ingress_command_pending.is_some()
        {
            return Some(MixedBridgeSealRefusal::SuccessorBusy);
        }
        if at.quantum_offset() != QuantumOffset::ZERO {
            return Some(MixedBridgeSealRefusal::NotBoundary { at });
        }
        let boundary = self.audio_boundary();
        if boundary > at {
            return Some(MixedBridgeSealRefusal::Passed { at, boundary });
        }
        if let Some(last) = old.last_stamp
            && last >= at
        {
            return Some(MixedBridgeSealRefusal::Forwarded { last, at });
        }
        None
    }

    /// Where the stopped-owner reset stands. Exact after each call: only the control
    /// thread changes it. Audio's own decision at `B` is the wrapper's to report.
    pub fn reset_phase(&self) -> MixedBridgeResetPhase {
        match self.state.reset {
            BridgeReset::Open => MixedBridgeResetPhase::Open,
            BridgeReset::Sealed { at, .. } => MixedBridgeResetPhase::Sealed { at },
            BridgeReset::Cancelled { at, panic, .. } => {
                MixedBridgeResetPhase::Cancelled { at, panic }
            }
        }
    }

    /// The sealed boundary once every command forwarded to the old transfer has been
    /// collected, so the old owner has served all of them; the host offers the successor
    /// only then. `None` while no reset is sealed, once a Panic cancelled it, after a
    /// fault, or while a sealed command is outstanding. Audio checks the seal itself, at
    /// the offer and at `B`.
    pub fn reset_ready(&self) -> Option<SampleTime> {
        let BridgeReset::Sealed { at, .. } = self.state.reset else {
            return None;
        };
        (self.fault().is_none()
            && self.state.forwarded.is_empty()
            && self.idle_ring()
            && self.transfer.inner.ingress_command_pending.is_none())
        .then_some(at)
    }

    /// Commit the reset the wrapper applied, off-thread, with the receipt the wrapper
    /// reported once `B` opened.
    ///
    /// Every old onset whose key-up has not been forwarded becomes a
    /// [`MixedOnsetRefusal::Reset`] tombstone: in its pairing table while it awaits its
    /// key-up, and in the key-up's own cell once that key-up is staged. Every old
    /// effective pedal-down whose lift has not been forwarded becomes a pedal tombstone
    /// the same way, so its lift is consumed and the successor, whose pedals start up,
    /// never sees it. An onset or pedal-down still staged was never minted by the old
    /// transfer, so it and every other staged head go to the successor. The successor's
    /// transfer, journal, handle table and audio clock then replace the old ones, which
    /// come back as the retired control. The host pumps and collects with the
    /// successor's control port from here; the old port belongs to the retired control.
    ///
    /// A Panic accepted while sealed whose cancel lost, because audio had already decided
    /// the seal, or whose stamp lies at or after `B + Q`, is still staged in the Panic
    /// cell; it goes to the successor like every other staged head, at an execution stamp
    /// from the successor's clock. The old owner's notes were ended by the `Reset` group
    /// at `B + Q`, so that Panic ends only the successor's notes.
    ///
    /// Allowed after a fault, because it records what audio already did, and a fault the
    /// old side recorded halts the successor too. Should the old onsets disagree with the
    /// bridge's own handle table, or the Panic cell hold a Panic forwarded to the old
    /// transfer, source tombstones still move and both transfers halt with
    /// [`MixedBridgeFault::Ledger`] unless an earlier terminal cause already won.
    ///
    /// # Errors
    ///
    /// No sealed reset, a reset a Panic cancelled, a receipt of another arm, successor or
    /// boundary, or a successor audio clock still before `B`. Nothing changed.
    pub fn commit_reset(
        &mut self,
        receipt: MixedBridgeResetReceipt,
    ) -> Result<MixedBridgeRetiredControl, MixedBridgeCommitRefusal> {
        let (at, successor, successor_first) =
            match core::mem::replace(&mut self.state.reset, BridgeReset::Open) {
                BridgeReset::Sealed {
                    at,
                    successor,
                    successor_first,
                } => (at, successor, successor_first),
                BridgeReset::Open => return Err(MixedBridgeCommitRefusal::NotSealed),
                cancelled @ BridgeReset::Cancelled { at, .. } => {
                    self.state.reset = cancelled;
                    return Err(MixedBridgeCommitRefusal::Cancelled { at });
                }
            };
        let boundary = successor.audio_boundary();
        let refused = if receipt.old_binding() != self.transfer.binding
            || receipt.successor_binding() != successor.transfer.binding
            || receipt.at() != at
        {
            Some(MixedBridgeCommitRefusal::ForeignReceipt)
        } else if boundary < at {
            Some(MixedBridgeCommitRefusal::NotReached { boundary, at })
        } else {
            None
        };
        if let Some(refused) = refused {
            self.state.reset = BridgeReset::Sealed {
                at,
                successor,
                successor_first,
            };
            return Err(refused);
        }
        // Check before relabelling; an inconsistent ledger is terminal, but the source
        // IDs must still become reset tombstones after audio applied the reset. A Panic
        // forwarded to the old transfer while sealed would have no journal to settle in.
        let consistent = self.old_onsets_have_handles()
            && !matches!(self.state.panic, PanicCell::Forwarded { .. });
        self.tombstone_old_onsets();
        let Self {
            transfer,
            state: next,
        } = *successor;
        let live_halt = Arc::clone(&transfer.inner.ingress_halt);
        let mut retired = MixedBridgeRetiredControl {
            transfer: core::mem::replace(&mut self.transfer, transfer),
            live_halt,
            forwarded: core::mem::replace(&mut self.state.forwarded, next.forwarded),
            handles: core::mem::replace(&mut self.state.handles, next.handles),
            receipt,
            fault: self.state.fault,
        };
        self.state.boundary = next.boundary;
        let live_prior = if successor_first {
            Some(MixedBridgeFault::Transfer(MixedTransferFault::Remote))
        } else {
            self.state
                .fault
                .or_else(|| self.transfer.fault().map(MixedBridgeFault::Transfer))
        };
        if !consistent {
            let cause = live_prior
                .or_else(|| retired.fault())
                .unwrap_or(MixedBridgeFault::Ledger);
            self.state.fault = Some(cause);
            let _old = retired.halt(MixedBridgeFault::Ledger);
            let _live = self.halt(cause);
        } else if let Some(fault) = retired.fault() {
            // A terminal fault stops the whole host (rule 1.8).
            let cause = live_prior.unwrap_or(fault);
            self.state.fault = Some(cause);
            let _live = self.halt(cause);
        }
        Ok(retired)
    }

    /// Whether every onset the commit would end has a handle in the bridge's table: an
    /// awaiting onset that is not staged, and the onset of a staged key-up that is not.
    fn old_onsets_have_handles(&self) -> bool {
        let handled = |onset| self.state.handles.iter().any(|held| held.onset == onset);
        self.state.lanes.iter().all(|lane| {
            lane.held
                .iter()
                .filter(|held| held.refused.is_none() && !stages_onset(&lane.entries, held.onset))
                .all(|held| handled(held.onset))
                && lane.entries.iter().all(|entry| match entry.kind {
                    EntryKind::KeyUp { onset } => {
                        stages_onset(&lane.entries, onset) || handled(onset)
                    }
                    _ => true,
                })
        })
    }

    /// End every old onset whose key-up, and every old pedal-down whose lift, has not
    /// been forwarded as a `Reset` tombstone. An onset or pedal-down always precedes its
    /// key-up or lift in its lane, so one not staged was minted by the old transfer:
    /// nothing is forwarded while sealed.
    fn tombstone_old_onsets(&mut self) {
        let reason = MixedOnsetRefusal::Reset;
        for lane in &mut self.state.lanes {
            let Lane {
                entries,
                held,
                protected,
                pedals,
                pedal_protected,
                ..
            } = lane;
            for pending in held.iter_mut() {
                if pending.refused.is_none() && !stages_onset(entries, pending.onset) {
                    pending.refused = Some(reason);
                    *protected -= 1;
                }
            }
            for pedal in pedals.iter_mut().flatten() {
                if pedal.refused.is_none() && !stages_pedal(entries, pedal.down) {
                    pedal.refused = Some(reason);
                    *pedal_protected -= 1;
                }
            }
            for index in 0..entries.len() {
                let refused = match entries.items.get(index).map(|entry| entry.kind) {
                    Some(EntryKind::KeyUp { onset }) if !stages_onset(entries, onset) => {
                        EntryKind::RefusedKeyUp { onset, reason }
                    }
                    Some(EntryKind::Lift { down, .. }) if !stages_pedal(entries, down) => {
                        EntryKind::RefusedLift { down, reason }
                    }
                    _ => continue,
                };
                if let Some(entry) = entries.items.get_mut(index) {
                    entry.kind = refused;
                }
            }
        }
    }

    /// Take back the sealed or cancelled successor bridge, off-thread, after the wrapper
    /// refused its audio half at the offer or at `B`, after it was withdrawn, or when it
    /// was never offered. `returned` is that audio half, which must still be fresh: an
    /// owner the wrapper installed is refused, so an applied reset can only be committed.
    /// Clears the seal and resumes forwarding to the old transfer, where waiting heads,
    /// and a Panic held while sealed, go out at execution stamps from the old clock.
    /// Allowed after a fault.
    ///
    /// # Errors
    ///
    /// No sealed or cancelled reset, an owner of another arm, or an installed successor.
    /// Nothing changed.
    pub fn abandon_reset<C, R>(
        &mut self,
        returned: &MixedBridgeResetOwner<C, R>,
    ) -> Result<Self, MixedBridgeAbandonRefusal> {
        let (BridgeReset::Sealed { successor, .. } | BridgeReset::Cancelled { successor, .. }) =
            &self.state.reset
        else {
            return Err(MixedBridgeAbandonRefusal::NotSealed);
        };
        if returned.binding() != successor.transfer.binding {
            return Err(MixedBridgeAbandonRefusal::ForeignOwner);
        }
        if !returned.fresh() {
            return Err(MixedBridgeAbandonRefusal::Installed);
        }
        match core::mem::replace(&mut self.state.reset, BridgeReset::Open) {
            BridgeReset::Sealed { successor, .. } | BridgeReset::Cancelled { successor, .. } => {
                self.transfer.clear_reset_seal();
                Ok(*successor)
            }
            BridgeReset::Open => Err(MixedBridgeAbandonRefusal::NotSealed),
        }
    }

    /// The terminal fault this half recorded, or the transfer's, or `Remote` once it
    /// observes the audio half's halt.
    pub fn fault(&self) -> Option<MixedBridgeFault> {
        self.state
            .fault
            .or_else(|| self.transfer.fault().map(MixedBridgeFault::Transfer))
    }

    /// Cells holding packets and cells protected for key-ups and lifts in `source`'s
    /// lane, or `None` for an unbound source.
    pub fn lane_charge(
        &self,
        source: ConnectionGeneration,
    ) -> Option<(MixedTransferPlaces, MixedTransferPlaces)> {
        let lane = &self.state.lanes[lane_of(self.state.sources.resolve(source).ok()?)];
        Some((
            MixedTransferPlaces(lane.entries.len()),
            MixedTransferPlaces(lane.protected + lane.pedal_protected),
        ))
    }

    /// Onsets of `source` the bridge refused whose key-ups have not arrived, or `None` for
    /// an unbound source.
    pub fn tombstone_count(&self, source: ConnectionGeneration) -> Option<MixedTransferPlaces> {
        let lane = &self.state.lanes[lane_of(self.state.sources.resolve(source).ok()?)];
        Some(MixedTransferPlaces(lane.tombstones()))
    }

    /// Onsets of `source` whose key-ups have not arrived, awaiting or refused, or `None`
    /// for an unbound source. Never more than the declared held onsets.
    pub fn held_onsets(&self, source: ConnectionGeneration) -> Option<MixedTransferPlaces> {
        let lane = &self.state.lanes[lane_of(self.state.sources.resolve(source).ok()?)];
        Some(MixedTransferPlaces(lane.held.len()))
    }

    /// The audio clock the last completed callback published: the first sample whose
    /// commands audio has not yet served. The peer fence and the window gate read it.
    pub fn audio_boundary(&self) -> SampleTime {
        SampleTime::new(self.state.boundary.load(Ordering::Acquire))
    }

    /// Prepare an audio-owned strict publication guard, off-thread. It stays bound
    /// to this bridge while its active transfer changes through committed resets.
    pub fn publication_gate(&self) -> super::MixedPublicationGate {
        super::MixedPublicationGate::new(self.state.publication_root)
    }

    /// Allocate one latest-value publication channel for this bridge, off-thread.
    /// Audio reads a complete pair in bounded time without waiting for the writer.
    pub fn publication_mailbox(
        &self,
    ) -> (
        super::MixedPublicationSender,
        super::MixedPublicationReceiver,
    ) {
        super::MixedPublicationSender::new(
            self.state.publication_root,
            [
                self.state.sources.generation(MixedBoundSource::First),
                self.state.sources.generation(MixedBoundSource::Second),
            ],
        )
    }

    /// Prove the prefix of promised source input already forwarded or settled.
    /// Computed from actual lanes, never a caller's remembered pump wait. The
    /// earliest unforwarded head, both source promises and publication window cap
    /// it. Publish after command pushes; audio reads before draining commands.
    /// A host Panic or transport request accepted later is a separate asynchronous
    /// operation and can be late; this proof does not promise its future deadline.
    ///
    /// A sealed transition additionally names its exact successor and boundary,
    /// only once every old command has been collected. Its source/head caps prove
    /// that nothing is due during the otherwise unserviceable reset interval.
    /// `None` means faulted, reset-cancelled, old commands uncollected, or clock
    /// exhaustion. This strict mode does not replace tolerant source fencing.
    pub fn publication(&self) -> Option<super::MixedPublication> {
        if self.fault().is_some() {
            return None;
        }
        let successor = match &self.state.reset {
            BridgeReset::Open => None,
            BridgeReset::Sealed { at, successor, .. } if self.reset_ready() == Some(*at) => Some((
                super::publication::MixedPublicationOwner::of(successor.transfer.binding),
                *at,
            )),
            _ => return None,
        };
        let mut through = self.audio_boundary().checked_add(self.state.window).ok()?;
        for lane in &self.state.lanes {
            through = through.min(lane.frontier.unwrap_or(SampleTime::ZERO));
            if let Some(head) = lane.entries.front() {
                through = through.min(head.at);
            }
        }
        if let PanicCell::Staged { at } = self.state.panic {
            through = through.min(at);
        }
        Some(super::MixedPublication {
            root: super::publication::MixedPublicationOwner::of(self.state.publication_root),
            sources: [
                self.state.sources.generation(MixedBoundSource::First),
                self.state.sources.generation(MixedBoundSource::Second),
            ],
            binding: super::publication::MixedPublicationOwner::of(self.transfer.binding),
            successor,
            through,
        })
    }

    /// The execution stamp of the last forwarded command, if any.
    pub const fn last_stamp(&self) -> Option<SampleTime> {
        self.state.last_stamp
    }

    /// Prepare and seal a later compiled Start or seek for the transfer this bridge
    /// owns, off-thread; see [`MixedTransferControl::prepare_activation`]. It touches no
    /// lane, journal or source state. After a committed reset the bridge owns the
    /// successor's transfer, so an offer prepared before the commit is foreign to it.
    ///
    /// # Errors
    ///
    /// The candidate could not be built, displaced or admitted at that boundary.
    pub fn prepare_activation(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        effective: SampleTime,
        kind: MixedActivationKind,
    ) -> Result<MixedActivationOffer, MixedActivationPrepareError> {
        self.transfer
            .prepare_activation(requested, position, effective, kind)
    }

    /// Prepare and seal a later loop entry for the transfer this bridge owns,
    /// off-thread; see [`MixedTransferControl::prepare_loop_activation`]. Like
    /// [`Self::prepare_activation`] it touches no lane, journal or source state.
    ///
    /// # Errors
    ///
    /// A position outside the loop, or a candidate or loop that could not be built,
    /// displaced or admitted at that boundary.
    pub fn prepare_loop_activation(
        &self,
        requested: SampleTime,
        position: PlanPosition,
        interval: LoopInterval,
        effective: SampleTime,
        kind: MixedActivationKind,
    ) -> Result<MixedActivationOffer, MixedActivationPrepareError> {
        self.transfer
            .prepare_loop_activation(requested, position, interval, effective, kind)
    }

    /// Collect one retired candidate of the transfer this bridge owns, off-thread; see
    /// [`MixedTransferControl::collect_activation`].
    ///
    /// # Errors
    ///
    /// A candidate of another transfer or owner, with the candidate unchanged.
    pub fn collect_activation(
        &mut self,
        retired: MixedActivationRetired,
    ) -> Result<MixedActivationReceipt, Box<MixedActivationCollectRefusal>> {
        self.transfer.collect_activation(retired)
    }

    /// Whether the successor was halted before this old-side control operation began.
    fn sealed_successor_halted(&self) -> bool {
        matches!(
            &self.state.reset,
            BridgeReset::Sealed { successor, .. }
                if successor.transfer.inner.ingress_halt.load(Ordering::Acquire)
        )
    }

    /// Preserve the first cause the control side observed. A successor already halted
    /// at this call's entry wins over a new old-side cause; the retired arm retains its
    /// own cause. Capture this before a transfer call can halt the old arm.
    fn record_sealed_fault(
        &mut self,
        fault: MixedBridgeFault,
        successor_was_halted: bool,
        old_prior: Option<MixedBridgeFault>,
    ) {
        if self.state.fault.is_some() {
            return;
        }
        if let BridgeReset::Sealed {
            successor_first, ..
        } = &mut self.state.reset
        {
            *successor_first = successor_was_halted;
            self.state.fault = Some(old_prior.unwrap_or(fault));
        }
    }

    /// Publish the old bridge's terminal halt to a sealed successor before commit.
    fn halt_sealed_successor(&self) {
        if let BridgeReset::Sealed { successor, .. } = &self.state.reset {
            successor
                .transfer
                .inner
                .ingress_halt
                .store(true, Ordering::Release);
        }
    }

    /// Record this half's cause, unless a cause is already recorded on either half, and
    /// publish the halt to both transfers during a sealed reset. The first cause wins.
    fn halt(&mut self, fault: MixedBridgeFault) -> MixedBridgeError {
        let old_prior = self.transfer.fault().map(MixedBridgeFault::Transfer);
        let successor_was_halted = old_prior.is_none() && self.sealed_successor_halted();
        self.halt_with_prior(fault, successor_was_halted, old_prior)
    }

    fn halt_with_prior(
        &mut self,
        fault: MixedBridgeFault,
        successor_was_halted: bool,
        old_prior: Option<MixedBridgeFault>,
    ) -> MixedBridgeError {
        self.record_sealed_fault(fault, successor_was_halted, old_prior);
        if self.fault().is_none() {
            self.state.fault = Some(fault);
        }
        self.transfer
            .inner
            .ingress_halt
            .store(true, Ordering::Release);
        self.halt_sealed_successor();
        MixedBridgeError::Fault(self.fault().unwrap_or(fault))
    }

    fn halted(&self) -> Option<MixedBridgeError> {
        if self.transfer.inner.ingress_halt.load(Ordering::Acquire) {
            self.halt_sealed_successor();
            Some(MixedBridgeError::Halted(self.fault().unwrap_or(
                MixedBridgeFault::Transfer(MixedTransferFault::Remote),
            )))
        } else {
            None
        }
    }

    /// The halt read, then the source binding, then the source's serial and order. A
    /// packet that passes becomes its source's previous packet.
    fn admit_source(
        &mut self,
        packet: MixedSourcePacketId,
        at: SampleTime,
    ) -> Result<usize, MixedBridgeError> {
        if let Some(halted) = self.halted() {
            return Err(halted);
        }
        let generation = packet.connection();
        let Ok(bound) = self.state.sources.resolve(generation) else {
            return Err(self.halt(MixedBridgeFault::ForeignSource { generation }));
        };
        let lane = lane_of(bound);
        let previous = self.state.lanes[lane].previous;
        let expected = previous.map_or(Some(1), |previous| previous.serial().checked_add(1));
        if expected != Some(packet.serial()) {
            return Err(self.halt(MixedBridgeFault::SourceSerial { packet, previous }));
        }
        if let Some(frontier) = self.state.lanes[lane].bound()
            && at < frontier
        {
            return Err(self.halt(MixedBridgeFault::SourceOrder {
                connection: generation,
                at,
                frontier,
            }));
        }
        self.state.lanes[lane].previous = Some(packet);
        Ok(lane)
    }

    /// Accept one host Panic stamped `at` into the bridge's Panic cell, off-thread.
    ///
    /// This is ADR-0046 clause 3's caller boundary: every refusal happens here, before
    /// acceptance, and changes nothing. An accepted Panic is never dropped; a later
    /// [`Self::pump`] forwards it into its protected place by the selection and stamp
    /// rules of the module documentation, and [`Self::collect`] reports it. The cell is
    /// free again once that report is collected.
    ///
    /// While a reset is sealed for `B`, a Panic due no later than `B`'s quantum, so
    /// stamped before `B + Q`, tries to cancel it as the last step after every refusal:
    /// one compare-and-swap on the seal against audio's own decision at `B`. If it wins,
    /// [`Self::reset_phase`] reports `Cancelled`, audio refuses the successor, and the
    /// lanes and the Panic forward to the old transfer. If audio already decided, or the
    /// stamp is later, the reset stays sealed and the Panic waits in the cell for the
    /// commit, which sends it to the successor, or the abandon, which sends it to the old
    /// owner. Winning decides only which owner the Panic reaches. It executes at its
    /// reported execution sample, which is later than its nominal stamp when it was
    /// accepted or pumped after audio had already rendered that stamp.
    ///
    /// # Errors
    ///
    /// The halt observed first, an uncollected earlier Panic, a stamp before the last
    /// accepted Panic's, or a service quantum audio has not yet left since the last
    /// applied Panic.
    pub fn panic(&mut self, at: SampleTime) -> Result<(), MixedBridgePanicRefusal> {
        if let Some(MixedBridgeError::Halted(fault) | MixedBridgeError::Fault(fault)) =
            self.halted()
        {
            return Err(MixedBridgePanicRefusal::Halted(fault));
        }
        if let Some(pending) = self.state.panic.at() {
            return Err(MixedBridgePanicRefusal::Pending { at: pending });
        }
        if let Some(previous) = self.state.last_panic
            && at < previous
        {
            return Err(MixedBridgePanicRefusal::BehindPanic { at, previous });
        }
        if let Some(previous) = self.transfer.inner.last_panic_executed_at {
            // Audio publishes each quantum it opens before serving commands there.
            let current = SampleTime::new(
                self.transfer
                    .inner
                    .panic_current_service_at
                    .load(Ordering::Acquire),
            );
            if current.quantum_index() == previous.quantum_index() {
                return Err(MixedBridgePanicRefusal::SameServiceQuantum { previous });
            }
        }
        // Accepted from here. The compare-and-swap precedes the Panic's push to the old
        // port, whose Release audio Acquires, so audio holding this Panic has also seen
        // the cancel and can no longer install the successor.
        if let BridgeReset::Sealed { at: boundary, .. } = self.state.reset
            && at.quantum_index() <= boundary.quantum_index()
            && self.transfer.cancel_reset_seal()
        {
            self.state.reset = match core::mem::replace(&mut self.state.reset, BridgeReset::Open) {
                BridgeReset::Sealed {
                    at: boundary,
                    successor,
                    ..
                } => BridgeReset::Cancelled {
                    at: boundary,
                    panic: at,
                    successor,
                },
                other => other,
            };
        }
        self.state.panic = PanicCell::Staged { at };
        self.state.last_panic = Some(at);
        Ok(())
    }

    /// The nominal stamp of the Panic the cell holds, staged or forwarded and not yet
    /// collected.
    pub const fn panic_pending(&self) -> Option<SampleTime> {
        self.state.panic.at()
    }

    /// Consume one source message into a stage cell, or refuse it for sound now.
    ///
    /// The handoff is consumed in every case, so the host commits its source head after
    /// this call whatever it returns. A key-up pairs with the oldest onset of its channel
    /// and key on its source whose key-up has not arrived; a lift pairs with its source
    /// and channel's effective pedal-down.
    ///
    /// # Errors
    ///
    /// The halt observed first, or a foreign source, source-serial, source-order or
    /// held-onset fault this call published.
    pub fn stage(&mut self, handoff: MixedSourceHandoff) -> Result<MixedStaged, MixedBridgeError> {
        let lane_index = self.admit_source(handoff.packet, handoff.at)?;
        let cells = self.state.cells;
        let lane = &mut self.state.lanes[lane_index];
        lane.last = Some(handoff.at);
        let channel = handoff.input.channel();
        let entry = |kind| Entry {
            packet: handoff.packet,
            at: handoff.at,
            kind,
        };
        let staged = match handoff.input.event() {
            Midi1Event::NoteOn { key, velocity } => {
                // Every onset of this source without its key-up is in the pairing table,
                // so this one needs a place the prepared table still has.
                if lane.held.is_full() {
                    return Err(self.halt(MixedBridgeFault::SourceHolds {
                        connection: handoff.packet.connection(),
                        onset: handoff.packet,
                    }));
                }
                let mut held = Held {
                    onset: handoff.packet,
                    channel,
                    key,
                    refused: None,
                };
                if lane.charged() + 2 > cells {
                    held.refused = Some(MixedOnsetRefusal::StageFull);
                    if lane.held.push(held).is_err() {
                        return Err(self.halt(MixedBridgeFault::Ledger));
                    }
                    return Ok(MixedStaged::Split(MixedSplit::StageFull));
                }
                let onset = entry(EntryKind::Onset {
                    channel,
                    key,
                    velocity,
                });
                if lane.entries.push(onset).is_err() || lane.held.push(held).is_err() {
                    return Err(self.halt(MixedBridgeFault::Ledger));
                }
                lane.protected += 1;
                MixedStaged::Staged
            }
            Midi1Event::KeyRelease { key, .. } => {
                let Some(held) = lane
                    .held
                    .take(|held| held.channel == channel && held.key == key)
                else {
                    return Ok(MixedStaged::Split(MixedSplit::Unpaired));
                };
                let onset = held.onset;
                if let Some(reason) = held.refused {
                    return Ok(MixedStaged::Split(MixedSplit::AfterRefusedOnset {
                        onset,
                        reason,
                    }));
                }
                // The onset's protected cell becomes this key-up's cell.
                lane.protected -= 1;
                if lane
                    .entries
                    .push(entry(EntryKind::KeyUp { onset }))
                    .is_err()
                {
                    return Err(self.halt(MixedBridgeFault::Ledger));
                }
                MixedStaged::Staged
            }
            Midi1Event::PitchBend { value } => {
                let Ok(cents) = Cents::new(value.as_f32() * BEND_RANGE_CENTS) else {
                    return Ok(MixedStaged::Split(MixedSplit::Unsupported));
                };
                if lane.charged() + 1 > cells {
                    return Ok(MixedStaged::Split(MixedSplit::StageFull));
                }
                let bend = entry(EntryKind::Bend {
                    channel: handoff.input.channel(),
                    cents,
                });
                if lane.entries.push(bend).is_err() {
                    return Err(self.halt(MixedBridgeFault::Ledger));
                }
                MixedStaged::Staged
            }
            Midi1Event::Sustain { down } => {
                let Some(pedal) = lane.pedals.get(usize::from(channel.as_index())).copied() else {
                    return Err(self.halt(MixedBridgeFault::Ledger));
                };
                let (next, staged) = match (down, pedal) {
                    (true, Some(_)) | (false, None) => {
                        return Ok(MixedStaged::Split(MixedSplit::RedundantPedal));
                    }
                    (true, None) => {
                        let mut pressed = Pedal {
                            down: handoff.packet,
                            refused: None,
                        };
                        if lane.charged() + 2 > cells {
                            pressed.refused = Some(MixedOnsetRefusal::StageFull);
                            (Some(pressed), MixedStaged::Split(MixedSplit::StageFull))
                        } else {
                            if lane
                                .entries
                                .push(entry(EntryKind::PedalDown { channel }))
                                .is_err()
                            {
                                return Err(self.halt(MixedBridgeFault::Ledger));
                            }
                            lane.pedal_protected += 1;
                            (Some(pressed), MixedStaged::Staged)
                        }
                    }
                    (
                        false,
                        Some(Pedal {
                            down,
                            refused: Some(reason),
                        }),
                    ) => (
                        None,
                        MixedStaged::Split(MixedSplit::AfterRefusedPedal { down, reason }),
                    ),
                    (
                        false,
                        Some(Pedal {
                            down,
                            refused: None,
                        }),
                    ) => {
                        // The down's protected cell becomes this lift's cell.
                        lane.pedal_protected -= 1;
                        if lane
                            .entries
                            .push(entry(EntryKind::Lift { channel, down }))
                            .is_err()
                        {
                            return Err(self.halt(MixedBridgeFault::Ledger));
                        }
                        (None, MixedStaged::Staged)
                    }
                };
                if let Some(slot) = lane.pedals.get_mut(usize::from(channel.as_index())) {
                    *slot = next;
                }
                staged
            }
        };
        Ok(staged)
    }

    /// Advance `packet`'s source frontier to `at`: the source sends nothing stamped
    /// earlier. `packet` is the source ring's ID for the frontier observation.
    ///
    /// # Errors
    ///
    /// The halt observed first, or a foreign source, source-serial or source-order fault.
    pub fn advance_frontier(
        &mut self,
        packet: MixedSourcePacketId,
        at: SampleTime,
    ) -> Result<(), MixedBridgeError> {
        let lane = self.admit_source(packet, at)?;
        self.state.lanes[lane].frontier = Some(at);
        Ok(())
    }

    /// Report a terminal source or raw-owner fault for `source` and publish the halt.
    ///
    /// Returns `Halted` with the earlier cause if either half already halted, and
    /// otherwise `Fault` with the cause this call recorded: `fault`, or a foreign-source
    /// fault for an unbound `source`.
    pub fn report_source_fault(
        &mut self,
        source: ConnectionGeneration,
        fault: MixedSourceFault,
    ) -> MixedBridgeError {
        if let Some(halted) = self.halted() {
            return halted;
        }
        if self.state.sources.resolve(source).is_err() {
            return self.halt(MixedBridgeFault::ForeignSource { generation: source });
        }
        self.halt(MixedBridgeFault::Source {
            connection: source,
            fault,
        })
    }

    fn fenced(&self, lane: usize, boundary: SampleTime) -> bool {
        let bound = self.state.lanes[lane].bound().unwrap_or(SampleTime::ZERO);
        bound
            .checked_add(self.state.peer_wait)
            .is_ok_and(|end| boundary > end)
    }

    /// The head that is next, the Panic cell included, or why none is. A source head at
    /// or before the Panic's stamp goes first; otherwise the Panic waits for every empty,
    /// unfenced lane whose bound has not passed its stamp.
    fn select(&self, boundary: SampleTime) -> Result<Head, MixedBridgeWait> {
        let source = self.select_source(boundary);
        let PanicCell::Staged { at: panic } = self.state.panic else {
            return source.map(Head::Lane);
        };
        let mut waiting = None;
        for (lane, state) in self.state.lanes.iter().enumerate() {
            match state.entries.front() {
                Some(entry) if entry.at <= panic => return source.map(Head::Lane),
                Some(_) => {}
                None => {
                    if waiting.is_none()
                        && !state.bound().is_some_and(|bound| bound > panic)
                        && !self.fenced(lane, boundary)
                    {
                        waiting = Some(lane);
                    }
                }
            }
        }
        match waiting {
            Some(lane) => Err(MixedBridgeWait::Peer {
                source: self.state.sources.generation(bound_of(lane)),
            }),
            None => Ok(Head::Panic(panic)),
        }
    }

    /// The lane whose source head is next, or why none is.
    fn select_source(&self, boundary: SampleTime) -> Result<usize, MixedBridgeWait> {
        let [first, second] = &self.state.lanes;
        let heads = (
            first.entries.front().map(|entry| entry.at),
            second.entries.front().map(|entry| entry.at),
        );
        let peer = |lane: usize| MixedBridgeWait::Peer {
            source: self.state.sources.generation(bound_of(lane)),
        };
        match heads {
            (Some(left), Some(right)) => Ok(if left <= right { 0 } else { 1 }),
            // The first source wins a tie, so its head may pass a peer bound equal to it.
            (Some(left), None) => {
                if second.bound().is_some_and(|bound| bound >= left) || self.fenced(1, boundary) {
                    Ok(0)
                } else {
                    Err(peer(1))
                }
            }
            (None, Some(right)) => {
                if first.bound().is_some_and(|bound| bound > right) || self.fenced(0, boundary) {
                    Ok(1)
                } else {
                    Err(peer(0))
                }
            }
            (None, None) => Err(MixedBridgeWait::Empty),
        }
    }

    fn scope(&self, lane: usize, channel: MidiChannel) -> MixedLiveScope {
        MixedLiveScope::new(MixedLiveSource::new(bound_of(lane).ordinal()), channel)
    }

    /// Whether no collection can free a command place: nothing is outstanding.
    fn quiescent(&self, needed: usize) -> bool {
        self.idle_ring() && self.transfer.inner.ingress_room() < needed
    }

    fn idle_ring(&self) -> bool {
        self.transfer.inner.ingress_outstanding == MixedIngressQueueCount::NONE
    }

    /// A head that meets a full journal or handle table before minting: it waits while a
    /// collection can still free an entry, and is split once nothing is outstanding.
    /// Each entry stands for an outstanding command or a reserved key-up place, so this
    /// is the command-ring shortage the merger would report at the same bound.
    fn custody_full(
        &mut self,
        lane: usize,
        entry: Entry,
    ) -> Result<MixedPumpStep, MixedBridgeError> {
        if self.idle_ring() {
            self.split(lane, entry, MixedSplit::CommandsFull)
        } else {
            Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands))
        }
    }

    /// Journal the lane head now minted as `id`, then remove it from its lane. Pump
    /// checked the journal for room before minting; should it be full regardless, the
    /// head stays in its cell, so teardown still finds its source ID, and the bridge halts.
    fn journal(
        &mut self,
        lane: usize,
        entry: Entry,
        id: MixedIngressCommandId,
        stamp: SampleTime,
    ) -> Result<(), MixedBridgeError> {
        let kind = match entry.kind {
            EntryKind::Onset { .. } => Forward::Onset,
            EntryKind::KeyUp { onset } => Forward::KeyUp { onset },
            EntryKind::Bend { channel, .. } => Forward::Bend { channel },
            EntryKind::PedalDown { channel } => Forward::PedalDown { channel },
            EntryKind::Lift { channel, down } => Forward::Lift { channel, down },
            EntryKind::RefusedKeyUp { .. } | EntryKind::RefusedLift { .. } => {
                return Err(self.halt(MixedBridgeFault::Ledger));
            }
        };
        let forwarded = Forwarded {
            id,
            packet: entry.packet,
            nominal: entry.at,
            stamp,
            kind,
        };
        if self.state.forwarded.push(forwarded).is_err() {
            return Err(self.halt(MixedBridgeFault::Ledger));
        }
        self.state.lanes[lane].entries.pop_front();
        self.state.last_stamp = Some(stamp);
        Ok(())
    }

    /// Record the merger handle a minted onset opened. Pump checked the table for room
    /// before minting; should it be full regardless, the command stays journaled under
    /// its source ID, teardown names the handle without an onset, and the bridge halts.
    fn open_handle(
        &mut self,
        onset: MixedSourcePacketId,
        occurrence: LiveOccurrenceId,
    ) -> Result<(), MixedBridgeError> {
        if self
            .state
            .handles
            .push(Handle {
                onset,
                occurrence,
                refused: false,
            })
            .is_err()
        {
            return Err(self.halt(MixedBridgeFault::Ledger));
        }
        Ok(())
    }

    fn forward(
        &mut self,
        lane: usize,
        entry: Entry,
        id: MixedIngressCommandId,
        stamp: SampleTime,
    ) -> Result<MixedPumpStep, MixedBridgeError> {
        self.journal(lane, entry, id, stamp)?;
        Ok(MixedPumpStep::Forwarded {
            packet: entry.packet,
            id: MixedTransferCommandId::of(id),
            stamp,
        })
    }

    /// Remove the lane head as a split. A refused onset whose key-up has not arrived
    /// becomes a tombstone in its FIFO place and releases its protected cell. A key-up
    /// already in this lane carries the refusal itself. A refused pedal-down does the
    /// same with its lift.
    fn split(
        &mut self,
        lane: usize,
        entry: Entry,
        split: MixedSplit,
    ) -> Result<MixedPumpStep, MixedBridgeError> {
        let lane_state = &mut self.state.lanes[lane];
        let refusal = MixedOnsetRefusal::CommandsFull;
        let consistent = match (entry.kind, split) {
            (EntryKind::Onset { .. }, MixedSplit::CommandsFull) => {
                lane_state.refuse_awaiting(entry.packet, refusal)
                    || lane_state.refuse_staged_key_up(entry.packet, refusal)
            }
            (EntryKind::PedalDown { channel }, MixedSplit::CommandsFull) => {
                lane_state.refuse_pedal(channel, entry.packet, refusal)
                    || lane_state.refuse_staged_lift(entry.packet, refusal)
            }
            _ => true,
        };
        if !consistent {
            return Err(self.halt(MixedBridgeFault::Ledger));
        }
        lane_state.entries.pop_front();
        Ok(MixedPumpStep::Split {
            packet: entry.packet,
            split,
        })
    }

    /// The command a failed physical push left pending: journal it, keep its handle,
    /// and record the transfer's push fault.
    fn push_failed(&mut self, lane: usize, entry: Entry, stamp: SampleTime) -> MixedBridgeError {
        let pending = self
            .transfer
            .inner
            .ingress_command_pending
            .as_ref()
            .map(|command| (command.id, command.request));
        let Some((id, request)) = pending else {
            return self.halt(MixedBridgeFault::Ledger);
        };
        // The transfer's cause is recorded first, so it wins over any ledger defect below.
        self.transfer.halt(MixedTransferFault::CommandPush {
            id: MixedTransferCommandId::of(id),
        });
        if let Err(error) = self.journal(lane, entry, id, stamp) {
            return error;
        }
        if let MixedIngressRequest::Onset { occurrence, .. } = request
            && let Err(error) = self.open_handle(entry.packet, occurrence)
        {
            return error;
        }
        MixedBridgeError::Fault(
            self.fault()
                .unwrap_or(MixedBridgeFault::Transfer(MixedTransferFault::Remote)),
        )
    }

    /// The minted command the merger's ledger refused: journal it and halt.
    fn ledger_defect(
        &mut self,
        lane: usize,
        entry: Entry,
        id: MixedIngressCommandId,
        stamp: SampleTime,
    ) -> MixedBridgeError {
        self.transfer.halt(MixedTransferFault::MergerLedger {
            id: Some(MixedTransferCommandId::of(id)),
        });
        if let Err(error) = self.journal(lane, entry, id, stamp) {
            return error;
        }
        MixedBridgeError::Fault(
            self.fault()
                .unwrap_or(MixedBridgeFault::Transfer(MixedTransferFault::Remote)),
        )
    }

    /// The command identity space or the occurrence cells are exhausted; the head stays.
    fn identity_exhausted(&mut self) -> MixedBridgeError {
        let _refusal = self.transfer.identity_refusal();
        MixedBridgeError::Fault(self.fault().unwrap_or(MixedBridgeFault::Transfer(
            MixedTransferFault::CommandIdExhausted,
        )))
    }

    /// Forward or split at most one stage head, or forward the staged Panic.
    ///
    /// A head is forwarded only once its execution stamp lies inside the next publication
    /// window, `[boundary, boundary + window)`, where `window` is one callback of the
    /// profile's largest block in whole quanta. Since the last forwarded stamp lay inside
    /// an earlier window and the boundary only grows, this holds exactly when the head's
    /// nominal stamp does. A head beyond it waits as [`MixedBridgeWait::Window`]. While a
    /// reset is sealed, only a key-up or lift the bridge already refused is split; every
    /// other call returns [`MixedBridgeWait::ResetSealed`] after the halt and wrapper
    /// checks, a Panic held while sealed included. Once a Panic cancelled the reset, pump
    /// forwards to the old transfer exactly as when no reset is sealed.
    ///
    /// # Errors
    ///
    /// The halt observed first, a foreign wrapper, or a terminal fault this call met,
    /// including [`MixedBridgeFault::ClockExhausted`] when the window from the audio
    /// boundary cannot be represented and command-ID or occurrence-cell generation
    /// exhaustion in the merger.
    pub fn pump<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
    ) -> Result<MixedPumpStep, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        if let Some(halted) = self.halted() {
            return Err(halted);
        }
        if port.binding != self.transfer.binding {
            let successor_was_halted = self.sealed_successor_halted();
            self.record_sealed_fault(
                MixedBridgeFault::Transfer(MixedTransferFault::ForeignEndpoint),
                successor_was_halted,
                None,
            );
            self.transfer.halt(MixedTransferFault::ForeignEndpoint);
            self.halt_sealed_successor();
            return Err(MixedBridgeError::Fault(MixedBridgeFault::Transfer(
                MixedTransferFault::ForeignEndpoint,
            )));
        }
        let boundary = self.audio_boundary();
        let selected = self.select(boundary);
        if let BridgeReset::Sealed { at, .. } = self.state.reset {
            // A key-up the bridge already refused touches no audio, so it may leave its
            // lane; every other head waits for the commit or the abandon, and so does a
            // Panic whose cancel lost or whose stamp is after `B`'s quantum.
            let head = selected.ok().and_then(|head| match head {
                Head::Lane(lane) => Some((lane, *self.state.lanes[lane].entries.front()?)),
                Head::Panic(_) => None,
            });
            if let Some((lane, entry)) = head {
                match entry.kind {
                    EntryKind::RefusedKeyUp { onset, reason } => {
                        let split = MixedSplit::AfterRefusedOnset { onset, reason };
                        return self.split(lane, entry, split);
                    }
                    EntryKind::RefusedLift { down, reason } => {
                        let split = MixedSplit::AfterRefusedPedal { down, reason };
                        return self.split(lane, entry, split);
                    }
                    _ => {}
                }
            }
            return Ok(MixedPumpStep::Idle(MixedBridgeWait::ResetSealed { at }));
        }
        let head = match selected {
            Ok(head) => head,
            Err(wait) => return Ok(MixedPumpStep::Idle(wait)),
        };
        let nominal = match head {
            Head::Lane(lane) => match self.state.lanes[lane].entries.front() {
                Some(entry) => entry.at,
                None => return Err(self.halt(MixedBridgeFault::Ledger)),
            },
            Head::Panic(at) => at,
        };
        let stamp = self.state.last_stamp.map_or(nominal.max(boundary), |last| {
            last.max(nominal).max(boundary)
        });
        let Ok(end) = boundary.checked_add(self.state.window) else {
            return Err(self.halt(MixedBridgeFault::ClockExhausted { boundary }));
        };
        if stamp >= end {
            return Ok(MixedPumpStep::Idle(MixedBridgeWait::Window { stamp, end }));
        }
        let lane = match head {
            Head::Lane(lane) => lane,
            Head::Panic(at) => return self.pump_panic(port, at, stamp),
        };
        let Some(entry) = self.state.lanes[lane].entries.front().copied() else {
            return Err(self.halt(MixedBridgeFault::Ledger));
        };
        match entry.kind {
            EntryKind::Onset {
                channel,
                key,
                velocity,
            } => self.pump_onset(port, lane, entry, stamp, (channel, key, velocity)),
            EntryKind::KeyUp { onset } => self.pump_key_up(port, lane, entry, stamp, onset),
            EntryKind::RefusedKeyUp { onset, reason } => {
                self.split(lane, entry, MixedSplit::AfterRefusedOnset { onset, reason })
            }
            EntryKind::Bend { channel, cents } => {
                self.pump_bend(port, lane, entry, stamp, channel, cents)
            }
            EntryKind::PedalDown { channel } => {
                self.pump_pedal(port, lane, entry, stamp, channel, true)
            }
            EntryKind::Lift { channel, .. } => {
                self.pump_pedal(port, lane, entry, stamp, channel, false)
            }
            EntryKind::RefusedLift { down, reason } => {
                self.split(lane, entry, MixedSplit::AfterRefusedPedal { down, reason })
            }
        }
    }

    /// Mint the staged Panic into its protected place. Neither a full command ring nor
    /// exhausted bend credit can delay it. The caller boundary already refused every
    /// merger refusal that could change, so any other refusal here is a ledger defect.
    fn pump_panic<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        nominal: SampleTime,
        stamp: SampleTime,
    ) -> Result<MixedPumpStep, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        let origin = self.transfer.origin();
        match self.transfer.inner.submit_panic_via(port, origin, stamp) {
            Ok((id, _)) => {
                self.state.panic = PanicCell::Forwarded { id, nominal, stamp };
                self.state.last_stamp = Some(stamp);
                Ok(MixedPumpStep::Panic {
                    id: MixedTransferCommandId::of(id),
                    stamp,
                })
            }
            Err(MixedPanicSubmitError::Halted) => {
                Err(self
                    .halted()
                    .unwrap_or(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
                        MixedTransferFault::Remote,
                    ))))
            }
            Err(MixedPanicSubmitError::PhysicalPush) => {
                // The merger keeps the minted Panic in its pending slot; teardown finds it.
                let Some(id) = self
                    .transfer
                    .inner
                    .ingress_command_pending
                    .as_ref()
                    .map(|command| command.id)
                else {
                    return Err(self.halt(MixedBridgeFault::Ledger));
                };
                self.state.panic = PanicCell::Forwarded { id, nominal, stamp };
                self.state.last_stamp = Some(stamp);
                self.transfer.halt(MixedTransferFault::CommandPush {
                    id: MixedTransferCommandId::of(id),
                });
                Err(MixedBridgeError::Fault(self.fault().unwrap_or(
                    MixedBridgeFault::Transfer(MixedTransferFault::Remote),
                )))
            }
            Err(MixedPanicSubmitError::IdentityExhausted) => Err(self.identity_exhausted()),
            // Defensive: the cell admits one Panic until its result is collected, the
            // caller boundary checked the service quantum, and stamps never decrease.
            Err(
                MixedPanicSubmitError::Pending(_)
                | MixedPanicSubmitError::BehindPanic(_)
                | MixedPanicSubmitError::SameServiceQuantum { .. },
            ) => Err(self.halt(MixedBridgeFault::Ledger)),
        }
    }

    fn pump_onset<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        lane: usize,
        entry: Entry,
        stamp: SampleTime,
        (channel, key, velocity): (MidiChannel, KeyIdentity, NoteVelocity),
    ) -> Result<MixedPumpStep, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        if self.state.forwarded.is_full() || self.state.handles.is_full() {
            return self.custody_full(lane, entry);
        }
        let scope = self.scope(lane, channel);
        let origin = self.transfer.origin();
        match self
            .transfer
            .inner
            .submit_onset_via(port, origin, scope, stamp, key, velocity)
        {
            Ok((id, MixedIngressRequest::Onset { occurrence, .. })) => {
                let step = self.forward(lane, entry, id, stamp)?;
                self.open_handle(entry.packet, occurrence)?;
                Ok(step)
            }
            // Defensive: the merger stages only an onset request for an onset.
            Ok((id, _)) => Err(self.ledger_defect(lane, entry, id, stamp)),
            Err(MixedOnsetSubmitError::Halted) => {
                Err(self
                    .halted()
                    .unwrap_or(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
                        MixedTransferFault::Remote,
                    ))))
            }
            Err(MixedOnsetSubmitError::Full) if self.quiescent(2) => {
                self.split(lane, entry, MixedSplit::CommandsFull)
            }
            Err(MixedOnsetSubmitError::Full) => Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands)),
            Err(MixedOnsetSubmitError::BendCredit) => {
                Ok(MixedPumpStep::Idle(MixedBridgeWait::BendCredit))
            }
            Err(MixedOnsetSubmitError::IdentityExhausted) => Err(self.identity_exhausted()),
            Err(MixedOnsetSubmitError::PhysicalPush) => Err(self.push_failed(lane, entry, stamp)),
            Err(MixedOnsetSubmitError::LedgerDefect(id)) => {
                Err(self.ledger_defect(lane, entry, id, stamp))
            }
            // Defensive: forwarded stamps never decrease, so no command precedes a Panic.
            Err(MixedOnsetSubmitError::BehindPanic(_)) => Err(self.halt(MixedBridgeFault::Ledger)),
        }
    }

    fn pump_key_up<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        lane: usize,
        entry: Entry,
        stamp: SampleTime,
        onset: MixedSourcePacketId,
    ) -> Result<MixedPumpStep, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        // Defensive: an open handle's reserved place keeps the journal below its bound.
        if self.state.forwarded.is_full() {
            return Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands));
        }
        let Some(handle) = self
            .state
            .handles
            .iter()
            .find(|handle| handle.onset == onset)
            .copied()
        else {
            return Err(self.halt(MixedBridgeFault::Ledger));
        };
        let origin = self.transfer.origin();
        match self
            .transfer
            .inner
            .submit_release_via(port, origin, handle.occurrence, stamp)
        {
            Ok((id, _)) => self.forward(lane, entry, id, stamp),
            Err(MixedReleaseSubmitError::Halted) => {
                Err(self
                    .halted()
                    .unwrap_or(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
                        MixedTransferFault::Remote,
                    ))))
            }
            // Defensive: every open handle owns a reserved place.
            Err(MixedReleaseSubmitError::ReservedPlaceUnavailable(_)) => {
                Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands))
            }
            Err(MixedReleaseSubmitError::IdentityExhausted(_)) => Err(self.identity_exhausted()),
            Err(MixedReleaseSubmitError::PhysicalPush) => Err(self.push_failed(lane, entry, stamp)),
            Err(MixedReleaseSubmitError::LedgerDefect(id)) => {
                Err(self.ledger_defect(lane, entry, id, stamp))
            }
            Err(
                MixedReleaseSubmitError::NotOpen(_) | MixedReleaseSubmitError::ReservationDefect(_),
            ) => {
                self.transfer
                    .halt(MixedTransferFault::MergerLedger { id: None });
                Err(self.halt(MixedBridgeFault::Ledger))
            }
            // Defensive: forwarded stamps never decrease, so no command precedes a Panic.
            Err(MixedReleaseSubmitError::BehindPanic(_)) => {
                Err(self.halt(MixedBridgeFault::Ledger))
            }
        }
    }

    fn pump_bend<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        lane: usize,
        entry: Entry,
        stamp: SampleTime,
        channel: MidiChannel,
        cents: Cents,
    ) -> Result<MixedPumpStep, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        if self.state.forwarded.is_full() {
            return self.custody_full(lane, entry);
        }
        let scope = self.scope(lane, channel);
        let origin = self.transfer.origin();
        match self
            .transfer
            .inner
            .submit_bend_via(port, origin, scope, stamp, cents)
        {
            Ok((id, _)) => self.forward(lane, entry, id, stamp),
            Err(MixedBendSubmitError::Halted) => {
                Err(self
                    .halted()
                    .unwrap_or(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
                        MixedTransferFault::Remote,
                    ))))
            }
            Err(MixedBendSubmitError::Full) if self.quiescent(1) => {
                self.split(lane, entry, MixedSplit::CommandsFull)
            }
            Err(MixedBendSubmitError::Full) => Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands)),
            Err(MixedBendSubmitError::Credits { .. }) => {
                Ok(MixedPumpStep::Idle(MixedBridgeWait::BendCredit))
            }
            // Defensive: arm proved both tables cover every scope of both sources.
            Err(MixedBendSubmitError::ScopeCapacity) => Err(self.halt(MixedBridgeFault::Ledger)),
            Err(MixedBendSubmitError::IdentityExhausted) => Err(self.identity_exhausted()),
            Err(MixedBendSubmitError::PhysicalPush) => Err(self.push_failed(lane, entry, stamp)),
            // Defensive: forwarded stamps never decrease, so no command precedes a Panic.
            Err(MixedBendSubmitError::BehindPanic(_)) => Err(self.halt(MixedBridgeFault::Ledger)),
        }
    }

    /// Forward one effective pedal edge. The merger mints a down only with a second place
    /// it reserves for the scope's lift, so a lift spends that place and never waits for
    /// command capacity. An edge the merger finds redundant, because audio refused the
    /// scope's last down or lift, is split with nothing minted; a down's protected cell
    /// still waits for its lift.
    fn pump_pedal<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
        lane: usize,
        entry: Entry,
        stamp: SampleTime,
        channel: MidiChannel,
        down: bool,
    ) -> Result<MixedPumpStep, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        if self.state.forwarded.is_full() {
            // Defensive for a lift: its down's reserved place keeps the journal below
            // its bound.
            return if down {
                self.custody_full(lane, entry)
            } else {
                Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands))
            };
        }
        let scope = self.scope(lane, channel);
        let origin = self.transfer.origin();
        match self
            .transfer
            .inner
            .submit_pedal_via(port, origin, scope, stamp, down)
        {
            Ok(MixedPedalSubmitted::Command(id, _)) => self.forward(lane, entry, id, stamp),
            Ok(MixedPedalSubmitted::Redundant) => {
                self.split(lane, entry, MixedSplit::RedundantPedal)
            }
            Err(MixedPedalSubmitError::Halted) => {
                Err(self
                    .halted()
                    .unwrap_or(MixedBridgeError::Halted(MixedBridgeFault::Transfer(
                        MixedTransferFault::Remote,
                    ))))
            }
            Err(MixedPedalSubmitError::Full) if self.quiescent(2) => {
                self.split(lane, entry, MixedSplit::CommandsFull)
            }
            // Only a down asks for new places; a lift's reserved place is defensive here.
            Err(MixedPedalSubmitError::Full | MixedPedalSubmitError::ReservedPlaceUnavailable) => {
                Ok(MixedPumpStep::Idle(MixedBridgeWait::Commands))
            }
            Err(MixedPedalSubmitError::IdentityExhausted) => Err(self.identity_exhausted()),
            Err(MixedPedalSubmitError::PhysicalPush) => Err(self.push_failed(lane, entry, stamp)),
            // Defensive: forwarded stamps never decrease, so no command precedes a Panic.
            Err(MixedPedalSubmitError::BehindPanic(_)) => Err(self.halt(MixedBridgeFault::Ledger)),
        }
    }

    /// Pop and settle at most one result against the oldest forwarded command.
    ///
    /// # Errors
    ///
    /// The halt observed first, or a terminal fault this call found.
    pub fn collect<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
    ) -> Result<Option<MixedBridgeReport>, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        let old_prior_at_entry = self.transfer.fault().map(MixedBridgeFault::Transfer);
        let successor_was_halted = old_prior_at_entry.is_none() && self.sealed_successor_halted();
        match settle(
            &mut self.transfer,
            &mut self.state.forwarded,
            &mut self.state.handles,
            Some(&mut self.state.panic),
            port,
        ) {
            Ok(report) => Ok(report),
            Err(Unsettled::Halted(fault)) => {
                let old_prior = self.transfer.fault().map(MixedBridgeFault::Transfer);
                self.record_sealed_fault(
                    MixedBridgeFault::Transfer(fault),
                    successor_was_halted,
                    old_prior,
                );
                self.halt_sealed_successor();
                Err(MixedBridgeError::Halted(
                    self.fault().unwrap_or(MixedBridgeFault::Transfer(fault)),
                ))
            }
            Err(Unsettled::Fault(fault)) => {
                let old_prior = self.transfer.fault().map(MixedBridgeFault::Transfer);
                self.record_sealed_fault(
                    MixedBridgeFault::Transfer(fault),
                    successor_was_halted,
                    old_prior,
                );
                self.halt_sealed_successor();
                Err(MixedBridgeError::Fault(MixedBridgeFault::Transfer(fault)))
            }
            Err(Unsettled::Ledger) => Err(self.halt_with_prior(
                MixedBridgeFault::Ledger,
                successor_was_halted,
                old_prior_at_entry,
            )),
        }
    }

    /// Stop both halves off-thread and account for every source ID the bridge still held.
    ///
    /// A sealed reset refuses the join and returns everything unchanged: commit or abandon
    /// it first, so the sealed successor and the old handles each reach a teardown. A
    /// reset a Panic cancelled likewise refuses until it is abandoned. The
    /// transfer's join rules then apply: a crossed pair or a foreign wrapper refuses the
    /// whole join and returns everything unchanged; a missing wrapper selects the
    /// transfer's `MissingEndpoint` custody and marks its queue's commands `Unknown`.
    ///
    /// # Errors
    ///
    /// A sealed or cancelled reset, a crossed audio half or a foreign wrapper.
    pub fn join<CP, RC, CC, RP>(
        self,
        audio: MixedTransferAudio,
        control_port: Option<MixedTransferControlPort<CP, RC>>,
        audio_port: Option<MixedTransferAudioPort<CC, RP>>,
    ) -> Result<MixedBridgeTeardown, Box<MixedBridgeJoinRefusal<CP, RC, CC, RP>>>
    where
        CP: MixedTransferProducer<MixedTransferCommand>,
        RC: MixedTransferConsumer<MixedTransferResult>,
        CC: MixedTransferConsumer<MixedTransferCommand>,
        RP: MixedTransferProducer<MixedTransferResult>,
    {
        let refused = match self.state.reset {
            BridgeReset::Open => None,
            BridgeReset::Sealed { at, .. } => Some(MixedBridgeJoinError::ResetSealed { at }),
            BridgeReset::Cancelled { at, .. } => Some(MixedBridgeJoinError::ResetCancelled { at }),
        };
        if let Some(reason) = refused {
            return Err(Box::new(MixedBridgeJoinRefusal {
                reason,
                control: self,
                audio,
                control_port,
                audio_port,
            }));
        }
        let fault = self.fault();
        let Self { transfer, state } = self;
        let teardown = match transfer.join(audio, control_port, audio_port) {
            Ok(teardown) => teardown,
            Err(refusal) => {
                let reason = MixedBridgeJoinError::Transfer(refusal.reason());
                let (transfer, audio, control_port, audio_port) = refusal.into_parts();
                return Err(Box::new(MixedBridgeJoinRefusal {
                    reason,
                    control: Self { transfer, state },
                    audio,
                    control_port,
                    audio_port,
                }));
            }
        };
        Ok(bridge_teardown(
            teardown,
            fault,
            &state.lanes,
            &state.forwarded,
            &state.handles,
            state.panic,
        ))
    }
}

impl MixedBridgeRetiredControl {
    /// The reset that retired the old transfer.
    pub const fn receipt(&self) -> MixedBridgeResetReceipt {
        self.receipt
    }

    /// The fault the bridge had recorded at the commit or this control recorded since,
    /// or the old transfer's, or `Remote` once it observes the old audio half's halt.
    pub fn fault(&self) -> Option<MixedBridgeFault> {
        self.fault
            .or_else(|| self.transfer.fault().map(MixedBridgeFault::Transfer))
    }

    /// Commands forwarded to the old transfer and not yet collected.
    pub fn outstanding(&self) -> MixedTransferPlaces {
        MixedTransferPlaces(self.forwarded.len())
    }

    /// Record the cause unless one is recorded, and halt both the old transfer and
    /// the successor, including after the old owner has retired (rule 1.8).
    fn halt(&mut self, fault: MixedBridgeFault) -> MixedBridgeError {
        if self.fault().is_none() {
            self.fault = Some(fault);
        }
        self.transfer
            .inner
            .ingress_halt
            .store(true, Ordering::Release);
        self.live_halt.store(true, Ordering::Release);
        MixedBridgeError::Fault(self.fault().unwrap_or(fault))
    }

    /// Pop and settle at most one old result with the old arm's control port, as
    /// [`MixedBridgeControl::collect`] does.
    ///
    /// # Errors
    ///
    /// The halt observed first, or a terminal fault this call found.
    pub fn collect<C, R>(
        &mut self,
        port: &mut MixedTransferControlPort<C, R>,
    ) -> Result<Option<MixedBridgeReport>, MixedBridgeError>
    where
        C: MixedTransferProducer<MixedTransferCommand>,
        R: MixedTransferConsumer<MixedTransferResult>,
    {
        match settle(
            &mut self.transfer,
            &mut self.forwarded,
            &mut self.handles,
            None,
            port,
        ) {
            Ok(report) => Ok(report),
            Err(Unsettled::Halted(fault)) => {
                self.live_halt.store(true, Ordering::Release);
                Err(MixedBridgeError::Halted(
                    self.fault().unwrap_or(MixedBridgeFault::Transfer(fault)),
                ))
            }
            Err(Unsettled::Fault(fault)) => {
                self.live_halt.store(true, Ordering::Release);
                Err(MixedBridgeError::Fault(MixedBridgeFault::Transfer(fault)))
            }
            Err(Unsettled::Ledger) => Err(self.halt(MixedBridgeFault::Ledger)),
        }
    }

    /// Stop the old arm off-thread and account for every old command and handle.
    ///
    /// Each old merger handle is classified once: ended by the receipt's `Reset` group,
    /// open on an onset audio refused, or named by a command the old journal still
    /// holds. Anything else is unclassified, and an ended occurrence the old merger did
    /// not hold is unmatched; both are defects. The transfer's join rules apply.
    ///
    /// # Errors
    ///
    /// A crossed audio half or a foreign wrapper; everything comes back unchanged.
    pub fn join<CP, RC, CC, RP>(
        self,
        audio: MixedTransferAudio,
        control_port: Option<MixedTransferControlPort<CP, RC>>,
        audio_port: Option<MixedTransferAudioPort<CC, RP>>,
    ) -> Result<MixedBridgeRetiredTeardown, Box<MixedBridgeRetiredJoinRefusal<CP, RC, CC, RP>>>
    where
        CP: MixedTransferProducer<MixedTransferCommand>,
        RC: MixedTransferConsumer<MixedTransferResult>,
        CC: MixedTransferConsumer<MixedTransferCommand>,
        RP: MixedTransferProducer<MixedTransferResult>,
    {
        let fault = self.fault();
        let Self {
            transfer,
            live_halt,
            forwarded,
            handles,
            receipt,
            fault: recorded,
        } = self;
        let teardown = match transfer.join(audio, control_port, audio_port) {
            Ok(teardown) => teardown,
            Err(refusal) => {
                let reason = MixedBridgeJoinError::Transfer(refusal.reason());
                let (transfer, audio, control_port, audio_port) = refusal.into_parts();
                return Err(Box::new(MixedBridgeJoinRefusal {
                    reason,
                    control: Self {
                        transfer,
                        live_halt,
                        forwarded,
                        handles,
                        receipt,
                        fault: recorded,
                    },
                    audio,
                    control_port,
                    audio_port,
                }));
            }
        };
        let ended_handle = |end: MixedBridgeResetEnd| match end {
            MixedBridgeResetEnd::Retired(handle) | MixedBridgeResetEnd::Tombstoned(handle) => {
                handle
            }
        };
        let journaled = |onset: MixedSourcePacketId, hold: MixedTransferHandleState| {
            forwarded.iter().any(|command| {
                command.packet == onset
                    || hold
                        == MixedTransferHandleState::Releasing(MixedTransferCommandId::of(
                            command.id,
                        ))
            })
        };
        let mut classified = Vec::new();
        let mut unclassified = Vec::new();
        for &(handle, hold) in teardown.handles() {
            let held = handles
                .iter()
                .find(|held| held.occurrence == handle.occurrence)
                .copied();
            let end = receipt.ended().find(|&end| ended_handle(end) == handle);
            let open = hold == MixedTransferHandleState::Open;
            let sustained = hold == MixedTransferHandleState::Sustained;
            let class = match (held, end) {
                (Some(_), Some(end)) if open => Some(MixedRetiredHandle::Ended(end)),
                // A key up under its pedal: the reset group retired the sustained cell.
                (Some(_), Some(end @ MixedBridgeResetEnd::Retired(_))) if sustained => {
                    Some(MixedRetiredHandle::Ended(end))
                }
                (Some(held), None) if open && held.refused => {
                    Some(MixedRetiredHandle::RefusedOnset)
                }
                (Some(held), _) if journaled(held.onset, hold) => {
                    Some(MixedRetiredHandle::Journaled)
                }
                _ => None,
            };
            match (held, class) {
                (Some(held), Some(class)) => classified.push((held.onset, class)),
                (held, _) => unclassified.push((held.map(|held| held.onset), hold)),
            }
        }
        let unmatched = receipt
            .ended()
            .filter(|&end| {
                !teardown
                    .handles()
                    .iter()
                    .any(|&(handle, _)| handle == ended_handle(end))
            })
            .collect();
        let bridge = bridge_teardown(teardown, fault, &[], &forwarded, &handles, PanicCell::Empty);
        Ok(MixedBridgeRetiredTeardown {
            bridge,
            receipt,
            handles: classified,
            unclassified,
            unmatched,
        })
    }
}

/// Why a settlement stopped.
enum Unsettled {
    Halted(MixedTransferFault),
    Fault(MixedTransferFault),
    Ledger,
}

/// Pop and settle at most one result of `transfer` against the oldest entry of
/// `forwarded`, or against the forwarded Panic in `panic`. A retired key-up drops its
/// handle; a sustained key-up keeps it until a lift ends it; an applied lift drops every
/// handle it ended; an onset audio refused marks its handle, which stays open on the
/// refusal tombstone until its key-up. An applied Panic drops every sustained handle it
/// retired and keeps every handle it tombstoned open for its key-up; any Panic result
/// frees the cell.
fn settle<C, R>(
    transfer: &mut MixedTransferControl,
    forwarded: &mut Fixed<Forwarded>,
    handles: &mut Fixed<Handle>,
    panic: Option<&mut PanicCell>,
    port: &mut MixedTransferControlPort<C, R>,
) -> Result<Option<MixedBridgeReport>, Unsettled>
where
    C: MixedTransferProducer<MixedTransferCommand>,
    R: MixedTransferConsumer<MixedTransferResult>,
{
    let report = match transfer.collect(port) {
        Ok(None) => return Ok(None),
        Ok(Some(report)) => report,
        Err(MixedTransferCollectError::Halted(fault)) => return Err(Unsettled::Halted(fault)),
        Err(MixedTransferCollectError::Fault(fault)) => return Err(Unsettled::Fault(fault)),
    };
    if let MixedTransferOutcome::Panic { result } = report.outcome() {
        let Some(cell) = panic else {
            return Err(Unsettled::Ledger);
        };
        let PanicCell::Forwarded { id, nominal, stamp } = *cell else {
            return Err(Unsettled::Ledger);
        };
        if MixedTransferCommandId::of(id) != report.id() {
            return Err(Unsettled::Ledger);
        }
        let result = match result {
            Ok(panicked) => {
                let mut ended = [None; RELEASE_GROUP_CAPACITY];
                for (slot, end) in ended.iter_mut().zip(panicked.ended()) {
                    *slot = Some(match end {
                        // The merger retired the sustained handle with this result.
                        MixedTransferPanicEnd::Retired(handle) => {
                            match handles.take(|held| held.occurrence == handle.occurrence) {
                                Some(held) => MixedBridgePanicEnd::Retired(held.onset),
                                None => return Err(Unsettled::Ledger),
                            }
                        }
                        // The handle stays open on its tombstone for the key-up.
                        MixedTransferPanicEnd::Tombstoned(handle) => {
                            match handles
                                .iter()
                                .find(|held| held.occurrence == handle.occurrence)
                            {
                                Some(held) => MixedBridgePanicEnd::Tombstoned(held.onset),
                                None => return Err(Unsettled::Ledger),
                            }
                        }
                    });
                }
                Ok(MixedBridgePanicked {
                    executed_at: panicked.executed_at(),
                    ended,
                })
            }
            Err(refused) => Err(refused),
        };
        *cell = PanicCell::Empty;
        return Ok(Some(MixedBridgeReport {
            id: report.id(),
            packet: None,
            nominal_at: nominal,
            stamp,
            executed_at: report.executed_at(),
            outcome: MixedBridgeOutcome::Panic(result),
        }));
    }
    let Some(command) = forwarded
        .front()
        .copied()
        .filter(|command| MixedTransferCommandId::of(command.id) == report.id())
    else {
        return Err(Unsettled::Ledger);
    };
    let mut refuse_onset = || {
        if let Some(handle) = handles
            .iter_mut()
            .find(|handle| handle.onset == command.packet)
        {
            handle.refused = true;
        }
    };
    let outcome = match (command.kind, report.outcome()) {
        (Forward::Onset, MixedTransferOutcome::NoteOn { result, .. }) => {
            if result.is_err() {
                refuse_onset();
            }
            MixedBridgeOutcome::NoteOn(result)
        }
        (Forward::Onset, MixedTransferOutcome::HandleRefused { .. }) => {
            refuse_onset();
            MixedBridgeOutcome::HandleRefused
        }
        (Forward::KeyUp { onset }, MixedTransferOutcome::KeyUp { result, .. }) => {
            if result.is_ok_and(|retired| retired != MixedKeyUpRetired::Sustained) {
                // The merger retired the handle with this result.
                let _retired = handles.take(|handle| handle.onset == onset);
            }
            MixedBridgeOutcome::KeyUp { onset, result }
        }
        (Forward::PedalDown { channel }, MixedTransferOutcome::PedalDown { result, .. }) => {
            MixedBridgeOutcome::PedalDown { channel, result }
        }
        (Forward::Lift { channel, down }, MixedTransferOutcome::Lift { result, .. }) => {
            let result = match result {
                Ok(lifted) => {
                    // The merger retired every lifted handle with this result, and every
                    // one belongs to the lift's own source.
                    let connection = down.connection();
                    let mut serials = [None; RELEASE_GROUP_CAPACITY];
                    for (serial, ended) in serials.iter_mut().zip(lifted.handles()) {
                        let Some(held) =
                            handles.take(|handle| handle.occurrence == ended.occurrence)
                        else {
                            return Err(Unsettled::Ledger);
                        };
                        let Some(onset) = NonZeroU64::new(held.onset.serial())
                            .filter(|_| held.onset.connection() == connection)
                        else {
                            return Err(Unsettled::Ledger);
                        };
                        *serial = Some(onset);
                    }
                    Ok(MixedBridgeLifted {
                        connection,
                        serials,
                    })
                }
                Err(refused) => Err(refused),
            };
            MixedBridgeOutcome::Lift {
                channel,
                down,
                result,
            }
        }
        (Forward::KeyUp { .. }, MixedTransferOutcome::HandleRefused { .. }) => {
            MixedBridgeOutcome::HandleRefused
        }
        (Forward::Bend { channel }, MixedTransferOutcome::Bend { result, .. }) => {
            MixedBridgeOutcome::Bend { channel, result }
        }
        _ => return Err(Unsettled::Ledger),
    };
    forwarded.pop_front();
    Ok(Some(MixedBridgeReport {
        id: report.id(),
        packet: Some(command.packet),
        nominal_at: command.nominal,
        stamp: command.stamp,
        executed_at: report.executed_at(),
        outcome,
    }))
}

/// Name every source ID `lanes` and `forwarded` still hold, each once, the Panic `panic`
/// holds, and every merger handle by its onset's source ID.
fn bridge_teardown(
    transfer: MixedTransferTeardown,
    fault: Option<MixedBridgeFault>,
    lanes: &[Lane],
    forwarded: &Fixed<Forwarded>,
    handles: &Fixed<Handle>,
    panic: PanicCell,
) -> MixedBridgeTeardown {
    let place = |id: MixedIngressCommandId| {
        let id = MixedTransferCommandId::of(id);
        if transfer.unsent() == Some(id) {
            MixedCommandPlace::Unsent
        } else if transfer.unserved().contains(&id) {
            MixedCommandPlace::Unserved
        } else if transfer.invalid_command() == Some(id) {
            MixedCommandPlace::InvalidCommand
        } else if transfer.parked() == Some(id) {
            MixedCommandPlace::Parked
        } else if transfer.pending_result() == Some(id) {
            MixedCommandPlace::PendingResult
        } else if transfer.uncollected().contains(&id) {
            MixedCommandPlace::Uncollected
        } else if transfer.invalid_result() == Some(id) {
            MixedCommandPlace::InvalidResult
        } else if transfer.lost().contains(&id) {
            MixedCommandPlace::Lost
        } else {
            MixedCommandPlace::Unknown
        }
    };
    let mut held = Vec::new();
    let mut tombstones = Vec::new();
    let mut pedals = Vec::new();
    for lane in lanes {
        held.extend(
            lane.entries
                .iter()
                .map(|entry| (entry.packet, MixedSourceCustody::Staged)),
        );
        for pending in lane.held.iter() {
            if let Some(reason) = pending.refused {
                tombstones.push((pending.onset, reason));
            }
        }
        pedals.extend(
            lane.pedals
                .iter()
                .flatten()
                .map(|pedal| (pedal.down, pedal.refused)),
        );
    }
    held.extend(forwarded.iter().map(|command| {
        (
            command.packet,
            MixedSourceCustody::Command {
                id: MixedTransferCommandId::of(command.id),
                place: place(command.id),
            },
        )
    }));
    let open = transfer
        .handles()
        .iter()
        .map(|&(handle, hold)| {
            let onset = handles
                .iter()
                .find(|held| held.occurrence == handle.occurrence)
                .map(|held| held.onset);
            (onset, hold)
        })
        .collect();
    let panic = match panic {
        PanicCell::Empty => None,
        PanicCell::Staged { .. } => Some(MixedSourceCustody::Staged),
        PanicCell::Forwarded { id, .. } => Some(MixedSourceCustody::Command {
            id: MixedTransferCommandId::of(id),
            place: place(id),
        }),
    };
    MixedBridgeTeardown {
        transfer,
        fault,
        held,
        open,
        tombstones,
        pedals,
        panic,
    }
}

/// A refused bridge join with both halves and every supplied wrapper, undrained. `B` is
/// the refused control: the live bridge, or a retired old transfer.
#[must_use]
pub struct MixedBridgeJoinRefusal<CP, RC, CC, RP, B = MixedBridgeControl> {
    reason: MixedBridgeJoinError,
    control: B,
    audio: MixedTransferAudio,
    control_port: Option<MixedTransferControlPort<CP, RC>>,
    audio_port: Option<MixedTransferAudioPort<CC, RP>>,
}

impl<CP, RC, CC, RP, B> std::fmt::Debug for MixedBridgeJoinRefusal<CP, RC, CC, RP, B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MixedBridgeJoinRefusal")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// A refused join of a retired old transfer, with every part unchanged.
pub type MixedBridgeRetiredJoinRefusal<CP, RC, CC, RP> =
    MixedBridgeJoinRefusal<CP, RC, CC, RP, MixedBridgeRetiredControl>;

/// Every part a refused bridge join returns, in argument order.
pub type MixedBridgeJoinParts<CP, RC, CC, RP, B = MixedBridgeControl> = (
    B,
    MixedTransferAudio,
    Option<MixedTransferControlPort<CP, RC>>,
    Option<MixedTransferAudioPort<CC, RP>>,
);

impl<CP, RC, CC, RP, B> MixedBridgeJoinRefusal<CP, RC, CC, RP, B> {
    /// Why the join refused.
    pub const fn reason(&self) -> MixedBridgeJoinError {
        self.reason
    }

    /// The control, the audio half and every supplied wrapper, unchanged.
    pub fn into_parts(self) -> MixedBridgeJoinParts<CP, RC, CC, RP, B> {
        (self.control, self.audio, self.control_port, self.audio_port)
    }
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
