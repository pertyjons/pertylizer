//! Frozen workload inventory. Constants are authored demand bounds, never observations.
use super::capacity_selection_controls::capacity_selection::{
    self as arithmetic, QueueDemand, QueueGuard,
};
use super::*;
use synth_engine_v2::profile::{
    EventLimits, ProducerShares, ProfileError, QueueCapacities, RenderLimits,
};
use synth_engine_v2::publish::ProducerClass;
use synth_engine_v2::quantities::{HeldNoteCount, SampleRate};
use synth_engine_v2::render::RehearsalUsageError;
use thiserror::Error;

/// Independently authored bounds for the fixed script and its full ordinary
/// source-pressure control. These are demand inputs, not configured capacities.
pub const PRODUCER_BOUNDS: [EventCount; 6] = [
    EventCount::measured(6),
    EventCount::measured(48),
    EventCount::measured(96),
    EventCount::measured(64),
    EventCount::measured(16),
    EventCount::measured(8),
];
pub const SOUND_HOLD_BOUND: EventCount = EventCount::measured(4);

#[derive(Clone, Copy, Debug)]
pub struct FrozenQueueBound {
    pub ordinary: MixedTransferPlaces,
    pub protected: MixedTransferPlaces,
}
impl FrozenQueueBound {
    const fn new(ordinary: usize, protected: usize) -> Self {
        Self {
            ordinary: MixedTransferPlaces::new(ordinary),
            protected: MixedTransferPlaces::new(protected),
        }
    }
    fn total(self) -> Result<MixedTransferPlaces, FrozenError> {
        self.ordinary
            .get()
            .checked_add(self.protected.get())
            .map(MixedTransferPlaces::new)
            .ok_or(FrozenError::Inventory("frozen queue bound sum"))
    }
}
// Sixteen ordinary commands per source, each at most two sound entries for
// this script's two held keys/source. The protected suffix owns four key-ups,
// two effective lifts and one host Panic; none is counted as ordinary traffic.
pub const PERFORMANCE_BOUND: FrozenQueueBound = FrozenQueueBound::new(64, 7);
pub const TRANSFER_BOUND: FrozenQueueBound = FrozenQueueBound::new(32, 7);
pub const STAGE_BOUND: FrozenQueueBound = FrozenQueueBound::new(16, 3);
// Preserve existing raw preparation entitlements independently of the timed G4
// script. These totals already include their reservations; do not add them again.
pub const RAW_INPUT_BOUND: FrozenQueueBound = FrozenQueueBound::new(136, 0);
pub const RAW_RING_BOUND: FrozenQueueBound = FrozenQueueBound::new(64, 0);
pub const RAW_COMMAND_BOUND: FrozenQueueBound = FrozenQueueBound::new(4, 0);
pub const RAW_ACTION_BOUND: FrozenQueueBound = FrozenQueueBound::new(256, 0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrozenResource {
    Producer(ProducerClass),
    Holds,
    Performance,
    Commands,
    Results,
    StageFirst,
    StageSecond,
    RawInputFirst,
    RawInputSecond,
    RawPackets,
    RawCompletions,
    RawCommands,
    RawActions,
}

#[derive(Debug, Error)]
pub enum FrozenError {
    #[error(
        "frozen demand bound disproved for {resource:?}: observed {observed:?}, bound {bound:?}"
    )]
    FrozenBound {
        resource: FrozenResource,
        observed: MixedTransferPlaces,
        bound: MixedTransferPlaces,
    },
    #[error("unavailable producer {0} unexpectedly has an observation")]
    UnavailableProducer(ProducerClass),
    #[error("executable producer {0} has no observation")]
    MissingProducer(ProducerClass),
    #[error(transparent)]
    Usage(#[from] RehearsalUsageError),
    #[error(transparent)]
    Selection(#[from] arithmetic::SelectionError),
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Candidate(#[from] arithmetic::CandidateRefused),
    #[error("capacity arithmetic or inventory cannot be represented: {0}")]
    Inventory(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrozenObservations {
    pub producers: [Option<EventCount>; 6],
    pub holds: EventCount,
    pub performance: MixedTransferPlaces,
    pub commands: MixedTransferPlaces,
    pub results: MixedTransferPlaces,
    pub stage: [MixedTransferPlaces; 2],
    pub raw_inputs: [MixedTransferPlaces; 2],
    pub raw_packets: MixedTransferPlaces,
    pub raw_completions: MixedTransferPlaces,
    pub raw_commands: MixedTransferPlaces,
    pub raw_actions: MixedTransferPlaces,
}

impl FrozenObservations {
    /// Owners have separate partitions and stores. Retired/current maxima are
    /// combined componentwise, never summed across the reset overlap.
    pub fn from_joined(joined: &Joined) -> Result<Self, FrozenError> {
        if let Some(error) = joined.raw_usage_error {
            return Err(error.into());
        }
        let mut owners = vec![joined.teardown.transfer()];
        owners.extend(
            joined
                .retired_reset
                .iter()
                .map(|owner| owner.bridge().transfer()),
        );
        if joined.queue_owners.len() != owners.len() {
            return Err(FrozenError::Inventory("owner queue snapshot missing"));
        }
        let mut producers = [None; 6];
        let mut holds = EventCount::NONE;
        let mut performance = MixedTransferPlaces::NONE;
        for owner in owners {
            let publication = owner.publication_usage();
            for class in ProducerClass::ALL {
                if let Some(value) = publication.high_water(class) {
                    producers[class.index()] = Some(
                        producers[class.index()].map_or(value, |old: EventCount| old.max(value)),
                    );
                }
            }
            let usage = owner.resource_usage()?;
            holds = holds.max(usage.live_holds_high_water);
            performance = performance.max(places_from_events(usage.ingress_queue_high_water)?);
        }
        let maximum = |get: fn(&OwnerQueuePeaks) -> &QueuePeak| {
            joined
                .queue_owners
                .iter()
                .map(|owner| get(owner).get())
                .max()
                .ok_or(FrozenError::Inventory("no owner queues"))
        };
        Ok(Self {
            producers,
            holds,
            performance,
            commands: maximum(|owner| &owner.commands)?,
            results: maximum(|owner| &owner.results)?,
            stage: joined.stage_peaks.map(|peak| peak.charged),
            raw_inputs: joined.raw_input_peaks.map(|peak| peak.charged),
            raw_packets: joined.raw_packet_peak.get(),
            raw_completions: joined.raw_completion_peak.get(),
            raw_commands: joined.raw_command_peak.get(),
            raw_actions: joined.raw_action_peak.get(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrozenHostConfig {
    pub profile: HostProfile,
    pub commands: MixedTransferPlaces,
    pub results: MixedTransferPlaces,
    pub stage: MixedTransferPlaces,
    pub raw_inputs: MixedTransferPlaces,
    pub raw_packets: MixedTransferPlaces,
    pub raw_completions: MixedTransferPlaces,
    pub raw_commands: SessionCommandCapacity,
    pub raw_actions: SessionSourceCapacity,
    pub raw_action_bytes: PreparedBytes,
}

impl FrozenHostConfig {
    pub fn setup(&self) -> Setup {
        Setup {
            cells: self.stage.get(),
            peer_wait: 64,
            commands: HostQueue::new(self.commands.get()),
            results: HostQueue::new(self.results.get()),
            raw_cells: u32::try_from(self.raw_inputs.get()).expect("validated raw cells"),
            raw_ring: self.raw_packets,
            raw_completion_ring: self.raw_completions,
            raw_commands: self.raw_commands,
            raw_actions: self.raw_actions,
            raw_action_bytes: self.raw_action_bytes,
            declared_holds: None,
            bridge: true,
        }
    }
    pub fn validate_observations(&self, observed: &FrozenObservations) -> Result<(), FrozenError> {
        let shares = self.profile.limits().events().shares();
        for class in ProducerClass::ALL {
            if let Some(value) = observed.producers[class.index()]
                && value
                    > [
                        shares.compiled_event_share(),
                        shares.authored_runtime_event_share(),
                        shares.live_event_share(),
                        shares.session_event_share(),
                        shares.internal_event_share(),
                        shares.release_event_share(),
                    ][class.index()]
            {
                return Err(FrozenError::Inventory("producer exceeds capacity"));
            }
        }
        let queues = [
            (observed.commands, self.commands),
            (observed.results, self.results),
            (observed.stage[0], self.stage),
            (observed.stage[1], self.stage),
            (observed.raw_inputs[0], self.raw_inputs),
            (observed.raw_inputs[1], self.raw_inputs),
            (observed.raw_packets, self.raw_packets),
            (observed.raw_completions, self.raw_completions),
            (
                observed.raw_commands,
                MixedTransferPlaces::new(self.raw_commands.as_u32() as usize),
            ),
            (
                observed.raw_actions,
                MixedTransferPlaces::new(self.raw_actions.as_u32() as usize),
            ),
            (
                observed.performance,
                places_from_events(
                    self.profile
                        .limits()
                        .events()
                        .queues()
                        .performance_ingress_capacity(),
                )?,
            ),
        ];
        if queues.iter().any(|(used, cap)| used > cap)
            || observed.holds > shares.release_hold_capacity()
        {
            return Err(FrozenError::Inventory(
                "queue or hold observation exceeds capacity",
            ));
        }
        Ok(())
    }
}

fn places_from_events(value: EventCount) -> Result<MixedTransferPlaces, FrozenError> {
    usize::try_from(value.get())
        .map(MixedTransferPlaces::new)
        .map_err(|_| FrozenError::Inventory("event places"))
}
fn events_from_places(value: MixedTransferPlaces) -> Result<EventCount, FrozenError> {
    u32::try_from(value.get())
        .map(EventCount::measured)
        .map_err(|_| FrozenError::Inventory("places in event units"))
}
pub(super) fn replace_events(
    base: HostProfile,
    shares: ProducerShares,
    performance: EventCount,
    window: EventCount,
) -> Result<HostProfile, FrozenError> {
    let limits = base.limits();
    let old = limits.events();
    let queues = old.queues();
    let events = EventLimits::new(
        shares
            .share_sum()
            .ok_or(FrozenError::Inventory("share sum"))?,
        old.max_note_expansion_per_tick(),
        window,
        old.forward_event_horizon(),
        QueueCapacities::new(
            performance,
            queues.command_queue_capacity(),
            queues.event_egress_capacity(),
        )?,
        shares,
    )?;
    Ok(HostProfile::new(
        base.capabilities(),
        RenderLimits::new(
            limits.stream(),
            limits.graph(),
            limits.voices(),
            events,
            limits.observation(),
            limits.mixing(),
            limits.memory(),
            limits.script(),
            limits.recording(),
            limits.cost(),
        )?,
    )?)
}

/// Exactly eight times each default executable share, hold entitlement, and
/// physical host queue depth. Source rings remain fixed128/16 custody contracts;
/// legacy profile command/event-egress fields are reporting-only and unchanged.
pub fn reference_host(rate: SampleRate) -> Result<FrozenHostConfig, FrozenError> {
    let base = arithmetic::candidate_profile(rate, FrameCount::new(64), ChannelLayout::Stereo)?;
    reference_host_for_profile(base)
}

/// Apply the same independently frozen reference inventory to an admitted layout.
/// The caller owns its candidate envelope; no graph is compiled by this helper.
pub fn reference_host_for_profile(base: HostProfile) -> Result<FrozenHostConfig, FrozenError> {
    let old = base.limits().events();
    let shares = old.shares();
    let scale = |value: EventCount| {
        value
            .get()
            .checked_mul(8)
            .map(EventCount::measured)
            .ok_or(FrozenError::Inventory("reference eightfold"))
    };
    let shares = ProducerShares::new(
        scale(shares.compiled_event_share())?,
        scale(shares.authored_runtime_event_share())?,
        scale(shares.live_event_share())?,
        scale(shares.session_event_share())?,
        scale(shares.internal_event_share())?,
        scale(shares.release_event_share())?,
        scale(shares.release_hold_capacity())?,
    )?;
    let window = shares
        .compiled_event_share()
        .checked_over(base.capabilities().max_quanta_per_callback()?)
        .and_then(|floor| floor.checked_add(EventCount::measured(4096)))
        .ok_or(FrozenError::Inventory("reference window"))?
        .max(old.max_scheduled_events_in_flight());
    Ok(FrozenHostConfig {
        profile: replace_events(
            base,
            shares,
            scale(old.queues().performance_ingress_capacity())?,
            window,
        )?,
        commands: MixedTransferPlaces::new(33 * 8),
        results: MixedTransferPlaces::new(33 * 8),
        stage: MixedTransferPlaces::new(8 * 8),
        raw_inputs: MixedTransferPlaces::new(136 * 8),
        raw_packets: MixedTransferPlaces::new(64 * 8),
        raw_completions: MixedTransferPlaces::new(64 * 8),
        raw_commands: SessionCommandCapacity::new(4 * 8)
            .map_err(|_| FrozenError::Inventory("reference raw commands"))?,
        raw_actions: SessionSourceCapacity::new(256 * 8)
            .map_err(|_| FrozenError::Inventory("reference raw actions"))?,
        raw_action_bytes: PreparedBytes::measured(262_144 * 8),
    })
}

fn queue(observed: MixedTransferPlaces, bound: FrozenQueueBound, guard: QueueGuard) -> QueueDemand {
    QueueDemand {
        observed,
        ordinary_burst: Some(bound.ordinary),
        protected_releases: Some(bound.protected),
        guard,
    }
}

fn queue_inventory(observed: &FrozenObservations) -> [(FrozenResource, QueueDemand); 11] {
    use FrozenResource as R;
    let ordinary = QueueGuard::None;
    let raw_guard = QueueGuard::AtLeast(MixedTransferPlaces::new(7));
    [
        (
            R::Performance,
            queue(observed.performance, PERFORMANCE_BOUND, ordinary),
        ),
        (
            R::Commands,
            queue(observed.commands, TRANSFER_BOUND, QueueGuard::MixedTransfer),
        ),
        (
            R::Results,
            queue(observed.results, TRANSFER_BOUND, QueueGuard::MixedTransfer),
        ),
        (
            R::StageFirst,
            queue(observed.stage[0], STAGE_BOUND, ordinary),
        ),
        (
            R::StageSecond,
            queue(observed.stage[1], STAGE_BOUND, ordinary),
        ),
        (
            R::RawInputFirst,
            queue(observed.raw_inputs[0], RAW_INPUT_BOUND, raw_guard),
        ),
        (
            R::RawInputSecond,
            queue(observed.raw_inputs[1], RAW_INPUT_BOUND, raw_guard),
        ),
        (
            R::RawPackets,
            queue(observed.raw_packets, RAW_RING_BOUND, ordinary),
        ),
        (
            R::RawCompletions,
            queue(observed.raw_completions, RAW_RING_BOUND, ordinary),
        ),
        (
            R::RawCommands,
            queue(observed.raw_commands, RAW_COMMAND_BOUND, ordinary),
        ),
        (
            R::RawActions,
            queue(observed.raw_actions, RAW_ACTION_BOUND, ordinary),
        ),
    ]
}

/// Falsify an independently frozen derivation before configuration validation or
/// max(H,B) arithmetic can absorb an undercount. This is separate from whether
/// configured storage is large enough to execute a run.
pub fn validate_frozen_bounds(observed: &FrozenObservations) -> Result<(), FrozenError> {
    let check = |resource, used, bound| {
        if used > bound {
            Err(FrozenError::FrozenBound {
                resource,
                observed: used,
                bound,
            })
        } else {
            Ok(())
        }
    };
    for class in ProducerClass::ALL {
        let unavailable = matches!(
            class,
            ProducerClass::AuthoredRuntime | ProducerClass::Internal
        );
        match observed.producers[class.index()] {
            Some(_) if unavailable => return Err(FrozenError::UnavailableProducer(class)),
            None if !unavailable => return Err(FrozenError::MissingProducer(class)),
            Some(value) => check(
                FrozenResource::Producer(class),
                places_from_events(value)?,
                places_from_events(PRODUCER_BOUNDS[class.index()])?,
            )?,
            None => {}
        }
    }
    check(
        FrozenResource::Holds,
        places_from_events(observed.holds)?,
        places_from_events(SOUND_HOLD_BOUND)?,
    )?;
    for (resource, demand) in queue_inventory(observed) {
        let bound = FrozenQueueBound {
            ordinary: demand
                .ordinary_burst
                .ok_or(FrozenError::Inventory("frozen ordinary bound"))?,
            protected: demand
                .protected_releases
                .ok_or(FrozenError::Inventory("frozen protected bound"))?,
        }
        .total()?;
        check(resource, demand.observed, bound)?;
    }
    Ok(())
}

pub fn select_host(
    base: HostProfile,
    observed: &FrozenObservations,
) -> Result<FrozenHostConfig, FrozenError> {
    validate_frozen_bounds(observed)?;
    let inventory = queue_inventory(observed);
    let performance = inventory[0].1;
    let queues = inventory[1..]
        .iter()
        .map(|(_, demand)| *demand)
        .collect::<Vec<_>>();
    let old = base.limits().events();
    let selected = arithmetic::select(&arithmetic::SelectionInput {
        high_water: observed.producers,
        frozen_bounds: PRODUCER_BOUNDS.map(Some),
        performance_store: performance,
        other_queues: &queues,
        observed_holds: observed.holds,
        sound_hold_entitlement: SOUND_HOLD_BOUND,
        compiled_span: HeldNoteCount::measured(2),
        default_scheduled_window: old.max_scheduled_events_in_flight(),
        default_authored_headroom: EventCount::measured(4096),
        max_quanta_per_callback: base.capabilities().max_quanta_per_callback()?,
    })?;
    let q = &selected.other_queues;
    Ok(FrozenHostConfig {
        profile: replace_events(
            base,
            selected.shares,
            events_from_places(selected.performance_store)?,
            selected.max_scheduled_events_in_flight,
        )?,
        commands: q[0],
        results: q[1],
        stage: q[2].max(q[3]),
        raw_inputs: q[4].max(q[5]),
        raw_packets: q[6],
        raw_completions: q[7],
        raw_commands: SessionCommandCapacity::new(events_from_places(q[8])?.get())
            .map_err(|_| FrozenError::Inventory("selected raw commands"))?,
        raw_actions: SessionSourceCapacity::new(events_from_places(q[9])?.get())
            .map_err(|_| FrozenError::Inventory("selected raw actions"))?,
        raw_action_bytes: PreparedBytes::measured(262_144),
    })
}

fn arithmetic_control_observations() -> FrozenObservations {
    // Arithmetic control inputs, not collected workload high waters.
    FrozenObservations {
        producers: [Some(0), None, Some(0), Some(0), None, Some(0)]
            .map(|value| value.map(EventCount::measured)),
        holds: EventCount::NONE,
        performance: MixedTransferPlaces::NONE,
        commands: MixedTransferPlaces::NONE,
        results: MixedTransferPlaces::NONE,
        stage: [MixedTransferPlaces::NONE; 2],
        raw_inputs: [MixedTransferPlaces::NONE; 2],
        raw_packets: MixedTransferPlaces::NONE,
        raw_completions: MixedTransferPlaces::NONE,
        raw_commands: MixedTransferPlaces::NONE,
        raw_actions: MixedTransferPlaces::NONE,
    }
}

#[test]
fn frozen_inventory_keeps_reference_eightfold_and_selected_guards_explicit() {
    let observed = arithmetic_control_observations();
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let rate = SampleRate::new(hz).expect("rate");
        let reference = reference_host(rate).expect("reference construction");
        assert_eq!(reference.commands.get(), 264);
        assert_eq!(
            reference
                .profile
                .limits()
                .events()
                .queues()
                .performance_ingress_capacity()
                .get(),
            256
        );
        reference
            .validate_observations(&observed)
            .expect("zero control observations");
        let base = arithmetic::candidate_profile(rate, FrameCount::new(64), ChannelLayout::Stereo)
            .expect("candidate");
        let selected = select_host(base, &observed).expect("fixed inventory control");
        selected
            .validate_observations(&observed)
            .expect("control fits");
        assert_eq!(selected.commands.get(), 192);
        assert_eq!(selected.results.get(), 192);
        assert_eq!(
            selected,
            select_host(base, &observed).expect("deterministic selection")
        );
        let _setup = selected.setup();
    }
}

fn set_control_queue(
    observed: &mut FrozenObservations,
    resource: FrozenResource,
    value: MixedTransferPlaces,
) {
    match resource {
        FrozenResource::Performance => observed.performance = value,
        FrozenResource::Commands => observed.commands = value,
        FrozenResource::Results => observed.results = value,
        FrozenResource::StageFirst => observed.stage[0] = value,
        FrozenResource::StageSecond => observed.stage[1] = value,
        FrozenResource::RawInputFirst => observed.raw_inputs[0] = value,
        FrozenResource::RawInputSecond => observed.raw_inputs[1] = value,
        FrozenResource::RawPackets => observed.raw_packets = value,
        FrozenResource::RawCompletions => observed.raw_completions = value,
        FrozenResource::RawCommands => observed.raw_commands = value,
        FrozenResource::RawActions => observed.raw_actions = value,
        other => panic!("not a queue control: {other:?}"),
    }
}

#[test]
fn every_frozen_producer_hold_and_queue_bound_is_falsifiable_despite_spare_capacity() {
    let reference = reference_host(SampleRate::new(48_000.0).expect("rate")).expect("reference");
    let zero = arithmetic_control_observations();
    validate_frozen_bounds(&zero).expect("available literal zeros are valid");
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Live,
        ProducerClass::Session,
        ProducerClass::Release,
    ] {
        let mut observed = zero.clone();
        let bound = PRODUCER_BOUNDS[class.index()];
        observed.producers[class.index()] = Some(bound);
        validate_frozen_bounds(&observed).expect("exact producer bound");
        observed.producers[class.index()] = Some(EventCount::measured(bound.get() + 1));
        reference
            .validate_observations(&observed)
            .expect("configuration can still fit a disproved bound");
        assert!(
            matches!(validate_frozen_bounds(&observed),Err(FrozenError::FrozenBound {resource:FrozenResource::Producer(found),..}) if found==class)
        );
        assert!(
            matches!(
                select_host(reference.profile, &observed),
                Err(FrozenError::FrozenBound { .. })
            ),
            "max(H,B) cannot absorb a disproved bound"
        );
    }
    let mut observed = zero.clone();
    observed.holds = SOUND_HOLD_BOUND;
    validate_frozen_bounds(&observed).expect("exact G4 bound");
    observed.holds = EventCount::measured(SOUND_HOLD_BOUND.get() + 1);
    reference
        .validate_observations(&observed)
        .expect("configured holds exceed G");
    assert!(matches!(
        validate_frozen_bounds(&observed),
        Err(FrozenError::FrozenBound {
            resource: FrozenResource::Holds,
            ..
        })
    ));
    for (resource, demand) in queue_inventory(&zero) {
        let bound = FrozenQueueBound {
            ordinary: demand.ordinary_burst.expect("frozen ordinary"),
            protected: demand.protected_releases.expect("frozen protected"),
        }
        .total()
        .expect("bound sum");
        let mut observed = zero.clone();
        set_control_queue(&mut observed, resource, bound);
        validate_frozen_bounds(&observed).expect("exact queue bound");
        set_control_queue(
            &mut observed,
            resource,
            MixedTransferPlaces::new(bound.get() + 1),
        );
        reference
            .validate_observations(&observed)
            .expect("configuration can still fit a disproved queue bound");
        assert!(
            matches!(validate_frozen_bounds(&observed),Err(FrozenError::FrozenBound {resource:found,..}) if found==resource)
        );
    }
}

#[test]
fn frozen_availability_requires_none_for_unavailable_and_some_for_executable() {
    for class in ProducerClass::ALL {
        let mut observed = arithmetic_control_observations();
        if matches!(
            class,
            ProducerClass::AuthoredRuntime | ProducerClass::Internal
        ) {
            observed.producers[class.index()] = Some(EventCount::NONE);
            assert!(
                matches!(validate_frozen_bounds(&observed),Err(FrozenError::UnavailableProducer(found)) if found==class)
            );
        } else {
            observed.producers[class.index()] = None;
            assert!(
                matches!(validate_frozen_bounds(&observed),Err(FrozenError::MissingProducer(found)) if found==class)
            );
        }
    }
}
