//! Executable arithmetic for EVD-0026's draft selection rule. This module neither
//! collects observations nor admits a host, plan, workload or selected profile.
use synth_engine_v2::{
    profile::{HostProfile, ProducerShares, ProfileError},
    publish::ProducerClass,
    quantities::{ChannelLayout, EventCount, HeldNoteCount, QuantumCount, SampleRate},
    stream::{MixedTransferHeldCount, MixedTransferPlaces},
    time::FrameCount,
};
use thiserror::Error;

/// The first capacity candidate's envelope, independent of the graph being compiled.
#[derive(Debug, Error)]
pub enum CandidateRefused {
    #[error("the capacity candidate requires stereo, got {0:?}")]
    Layout(ChannelLayout),
    #[error("the capacity candidate does not admit rate {0:?}")]
    Rate(SampleRate),
    #[error("the capacity candidate requires a 64-frame callback, got {0:?}")]
    Block(FrameCount),
    #[error(transparent)]
    Profile(#[from] ProfileError),
}

/// Refuse an unsupported configuration before compiling or opening any mixed stream.
/// Other rehearsal hosts retain their existing wider envelope.
pub fn candidate_profile(
    rate: SampleRate,
    block: FrameCount,
    layout: ChannelLayout,
) -> Result<HostProfile, CandidateRefused> {
    if layout != ChannelLayout::Stereo {
        return Err(CandidateRefused::Layout(layout));
    }
    if ![44_100.0, 48_000.0, 96_000.0, 192_000.0].contains(&rate.as_f32()) {
        return Err(CandidateRefused::Rate(rate));
    }
    if block != FrameCount::new(64) {
        return Err(CandidateRefused::Block(block));
    }
    Ok(HostProfile::harness(rate, block, layout)?)
}

/// One queue's observations and independently frozen simultaneous accepted demand.
/// The ordinary burst excludes the protected releases, so their sum counts each once.
#[derive(Clone, Copy, Debug)]
pub struct QueueDemand {
    pub observed: MixedTransferPlaces,
    pub ordinary_burst: Option<MixedTransferPlaces>,
    pub protected_releases: Option<MixedTransferPlaces>,
    /// A preparation guard is independent of the accepted burst; it is applied
    /// outside doubling and before rounding, never silently after selection.
    pub guard: QueueGuard,
}

#[derive(Clone, Copy, Debug)]
pub enum QueueGuard {
    None,
    AtLeast(MixedTransferPlaces),
    /// Protected host Panic requires one place beyond selected performance C.
    MixedTransfer,
}

#[derive(Debug, Error)]
pub enum SelectionError {
    #[error("mixed transfer queue selection needs selected performance depth")]
    MissingPerformanceDepth,
    #[error("checked capacity arithmetic overflowed: {0}")]
    Overflow(&'static str),
    #[error("the queue has no independently frozen ordinary accepted burst")]
    MissingOrdinaryBurst,
    #[error("the queue has no independently frozen protected-release demand")]
    MissingProtectedDemand,
    #[error("combined sound hold entitlement {0:?} exceeds one mixed release group")]
    SoundHoldEntitlement(EventCount),
    #[error("{0} has no frozen legal-coincidence bound")]
    MissingProducerBound(ProducerClass),
    #[error("executable producer {0} has no observed high water")]
    MissingObservation(ProducerClass),
    #[error("unavailable producer {0} must have an undefined high water")]
    UnavailableObserved(ProducerClass),
    #[error("unavailable producer {class} must retain frozen bound {required:?}, got {actual:?}")]
    UnavailableBound {
        class: ProducerClass,
        required: EventCount,
        actual: EventCount,
    },
    #[error(transparent)]
    Profile(#[from] ProfileError),
}

fn twice_rounded(value: u32) -> Result<EventCount, SelectionError> {
    let doubled = value.max(1).checked_mul(2);
    let rounded = doubled
        .and_then(|value| value.checked_add(31))
        .map(|value| value / 32 * 32)
        .ok_or(SelectionError::Overflow("twice-rounded event capacity"))?;
    Ok(EventCount::measured(rounded))
}

fn queue_depth(
    demand: QueueDemand,
    extra: MixedTransferPlaces,
    performance: Option<MixedTransferPlaces>,
) -> Result<MixedTransferPlaces, SelectionError> {
    let burst = demand
        .ordinary_burst
        .ok_or(SelectionError::MissingOrdinaryBurst)?
        .get();
    let protected = demand
        .protected_releases
        .ok_or(SelectionError::MissingProtectedDemand)?
        .get();
    let accepted = burst
        .checked_add(protected)
        .ok_or(SelectionError::Overflow("frozen accepted queue demand"))?;
    let guard = match demand.guard {
        QueueGuard::None => 0,
        QueueGuard::AtLeast(value) => value.get(),
        QueueGuard::MixedTransfer => performance
            .ok_or(SelectionError::MissingPerformanceDepth)?
            .get()
            .checked_add(1)
            .ok_or(SelectionError::Overflow("C + 1 command/result guard"))?,
    };
    let rounded = demand
        .observed
        .get()
        .max(accepted)
        .max(extra.get())
        .max(1)
        .checked_mul(2)
        .map(|value| value.max(guard))
        .and_then(|value| value.checked_add(31))
        .map(|value| value / 32 * 32)
        .ok_or(SelectionError::Overflow("twice-rounded queue capacity"))?;
    Ok(MixedTransferPlaces::new(rounded))
}

/// Apply the queue rule to an explicitly supplied queue. The caller must enumerate
/// every transfer, per-source stage and raw queue in its frozen inventory; the fixed
/// physical source rings are outside this numeric reselection.
pub fn select_queue(demand: QueueDemand) -> Result<MixedTransferPlaces, SelectionError> {
    queue_depth(demand, MixedTransferPlaces::NONE, None)
}

/// Same-run observations and independently frozen bounds; class arrays use
/// `ProducerClass::index()`. Missing inputs cause a refusal rather than a default.
#[derive(Debug)]
pub struct SelectionInput<'a> {
    pub high_water: [Option<EventCount>; 6],
    pub frozen_bounds: [Option<EventCount>; 6],
    pub performance_store: QueueDemand,
    pub other_queues: &'a [QueueDemand],
    pub observed_holds: EventCount,
    pub sound_hold_entitlement: EventCount,
    pub compiled_span: HeldNoteCount,
    pub default_scheduled_window: EventCount,
    /// The default profile's authored future entitlement, independent of its compiled
    /// floor. Raising the selected compiled share must preserve this entire headroom.
    pub default_authored_headroom: EventCount,
    pub max_quanta_per_callback: QuantumCount,
}

#[derive(Debug)]
pub struct SelectedCapacities {
    pub performance_store: MixedTransferPlaces,
    /// In the supplied frozen inventory order; these are separate queues, not a sum.
    pub other_queues: Vec<MixedTransferPlaces>,
    pub shares: ProducerShares,
    pub max_events_per_quantum: EventCount,
    pub max_scheduled_events_in_flight: EventCount,
}

/// Compute the draft method's capacities. Profile construction, plan admission,
/// controls and selected-profile rerun remain mandatory separate operations.
pub fn select(input: &SelectionInput<'_>) -> Result<SelectedCapacities, SelectionError> {
    if usize::try_from(input.sound_hold_entitlement.get()).map_or(true, |holds| {
        holds > MixedTransferHeldCount::MAX_SOUND_HOLDS.get()
    }) {
        return Err(SelectionError::SoundHoldEntitlement(
            input.sound_hold_entitlement,
        ));
    }
    let mut demand = [EventCount::NONE; 6];
    for class in ProducerClass::ALL {
        let bound = input.frozen_bounds[class.index()]
            .ok_or(SelectionError::MissingProducerBound(class))?;
        let unavailable_bound = match class {
            ProducerClass::AuthoredRuntime => Some(EventCount::measured(48)),
            ProducerClass::Internal => Some(EventCount::measured(16)),
            _ => None,
        };
        demand[class.index()] = if let Some(required) = unavailable_bound {
            if input.high_water[class.index()].is_some() {
                return Err(SelectionError::UnavailableObserved(class));
            }
            if bound != required {
                return Err(SelectionError::UnavailableBound {
                    class,
                    required,
                    actual: bound,
                });
            }
            bound
        } else {
            bound.max(
                input.high_water[class.index()].ok_or(SelectionError::MissingObservation(class))?,
            )
        };
    }

    let hold_floor = input
        .sound_hold_entitlement
        .get()
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or(SelectionError::Overflow("2G + 1"))?;
    let performance_store = queue_depth(
        input.performance_store,
        MixedTransferPlaces::new(
            usize::try_from(hold_floor).map_err(|_| SelectionError::Overflow("hold floor"))?,
        ),
        None,
    )?;
    let other_queues = input
        .other_queues
        .iter()
        .copied()
        .map(|queue| queue_depth(queue, MixedTransferPlaces::NONE, Some(performance_store)))
        .collect::<Result<Vec<_>, _>>()?;
    let holds = twice_rounded(
        input
            .observed_holds
            .get()
            .max(input.sound_hold_entitlement.get()),
    )?;
    let depth = u32::try_from(performance_store.get())
        .map_err(|_| SelectionError::Overflow("performance depth in event units"))?;
    // Integer ceil division avoids overflowing S + 7 at the representable limit.
    let span = input.compiled_span.get();
    let panic_groups = span.div_ceil(8);
    let panic_floor = depth
        .checked_add(panic_groups)
        .ok_or(SelectionError::Overflow("C + ceil(S / 8)"))?;
    demand[ProducerClass::Live.index()] =
        demand[ProducerClass::Live.index()].max(EventCount::measured(depth));
    demand[ProducerClass::Release.index()] = demand[ProducerClass::Release.index()]
        .max(EventCount::measured(panic_floor))
        .max(holds);
    let mut selected = [EventCount::NONE; 6];
    for class in ProducerClass::ALL {
        selected[class.index()] = twice_rounded(demand[class.index()].get())?;
    }
    let shares = ProducerShares::new(
        selected[ProducerClass::Compiled.index()],
        selected[ProducerClass::AuthoredRuntime.index()],
        selected[ProducerClass::Live.index()],
        selected[ProducerClass::Session.index()],
        selected[ProducerClass::Internal.index()],
        selected[ProducerClass::Release.index()],
        holds,
    )?;
    let max_events_per_quantum = shares
        .share_sum()
        .ok_or(SelectionError::Overflow("sum of the six shares"))?;
    let compiled_floor = shares
        .compiled_event_share()
        .checked_over(input.max_quanta_per_callback)
        .ok_or(SelectionError::Overflow("compiled callback floor"))?;
    let scheduled_window = compiled_floor
        .checked_add(input.default_authored_headroom)
        .ok_or(SelectionError::Overflow(
            "compiled floor plus authored headroom",
        ))?;
    Ok(SelectedCapacities {
        performance_store,
        other_queues,
        shares,
        max_events_per_quantum,
        max_scheduled_events_in_flight: input.default_scheduled_window.max(scheduled_window),
    })
}
