//! Off-thread normalization, exact junction admission and retained-storage bounds.

use super::*;
use crate::{
    admit::{admit_linear, admit_loop},
    ir::StealingPolicy,
    plan::CompiledPlan,
    profile::HostProfile,
    render::NoteEdge,
    schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload},
    stream::ActivationRequest,
    time::{QUANTUM_FRAMES, StreamAnchor},
    transport::TransportActivation,
};
use std::collections::HashMap;

fn preparation(error: impl std::fmt::Display) -> LoopPrepareError {
    LoopPrepareError::Preparation(error.to_string())
}

fn empty_slots<T>(count: usize) -> Result<Box<[Option<T>]>, LoopPrepareError> {
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(count)
        .map_err(|_| LoopPrepareError::Allocation)?;
    slots.resize_with(count, || None);
    Ok(slots.into_boxed_slice())
}

fn checked_bytes<T>(count: usize) -> Result<u64, LoopPrepareError> {
    count
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(LoopPrepareError::Layout)
}

impl CompiledLoopStream {
    /// Prepare a standalone compiled loop. This constructor exposes neither raw
    /// ingress nor ordinary TransportActivation, and requires no stealing policy.
    pub fn prepare(
        plan: CompiledPlan,
        stream: AdmittedCompiledStream,
        profile: HostProfile,
        settings: LoopSettings,
    ) -> Result<Self, LoopPrepareError> {
        let caps = profile.capabilities();
        let events = profile.limits().events();
        if plan.id() != stream.plan()
            || plan.sample_rate() != caps.sample_rate()
            || plan.channel_layout() != caps.channel_layout()
            || plan.maximum_block_size() != caps.maximum_block_size()
            || plan.max_events_per_quantum() != events.max_events_per_quantum()
            || plan.compiled_event_share() != events.shares().compiled_event_share()
            || plan.forward_event_horizon() != events.forward_event_horizon()
        {
            return Err(LoopPrepareError::Profile);
        }
        if plan.stealing() != StealingPolicy::None {
            return Err(LoopPrepareError::Stealing);
        }
        // Keep source history, but never normalize or count events at/after the
        // exclusive end: this owner cannot visit that suffix on any pass.
        let end = stream
            .events()
            .partition_point(|event| event.position() < settings.interval.end());
        let stream =
            AdmittedCompiledStream::admit(&plan, &stream.events()[..end]).map_err(preparation)?;
        let interval = settings.interval;
        let length = interval.end().as_u64() - interval.start().as_u64();
        let maximum_frames = u64::from(caps.max_quanta_per_callback().map_err(preparation)?.get())
            .checked_mul(u64::from(QUANTUM_FRAMES))
            .ok_or(LoopPrepareError::Layout)?;
        let boundary_capacity = usize::try_from(maximum_frames.div_ceil(length))
            .map_err(|_| LoopPrepareError::Layout)?;
        let producer = plan.compiled_note_producer();
        let held_capacity = producer
            .and_then(|producer| {
                plan.note_producer_ranges()
                    .get(usize::from(producer.as_u16()))
            })
            .map_or(0, |range| range.get() as usize);
        let count_at = |start: PlanPosition| {
            stream
                .events()
                .iter()
                .filter(|event| event.position() >= start && event.position() < interval.end())
                .count()
        };
        let note_count_at = |start: PlanPosition| {
            stream
                .events()
                .iter()
                .filter(|event| {
                    event.position() >= start
                        && event.position() < interval.end()
                        && matches!(event.payload(), CompiledPayload::NoteOn { .. })
                })
                .count()
        };
        let event_count = count_at(settings.entry)
            .checked_add(count_at(interval.start()))
            .ok_or(LoopPrepareError::Layout)?;
        let token_capacity = note_count_at(settings.entry).max(note_count_at(interval.start()));
        let catch_up_count = plan
            .parameter_addresses()
            .len()
            .checked_mul(2)
            .ok_or(LoopPrepareError::Layout)?;
        let components = [
            checked_bytes::<LoopEvent>(event_count)?,
            checked_bytes::<EventPayload>(catch_up_count)?,
            checked_bytes::<Option<NoteIdentity>>(token_capacity)?,
            checked_bytes::<Option<HeldToken>>(held_capacity)?,
            checked_bytes::<Option<LoopBoundary>>(boundary_capacity)?,
        ];
        let mut total = 0_u64;
        for bytes in components {
            total = total.checked_add(bytes).ok_or(LoopPrepareError::Layout)?;
        }
        let storage_bytes = PreparedBytes::measured(total);
        if storage_bytes > settings.storage_bytes {
            return Err(LoopPrepareError::Storage {
                required: storage_bytes,
                available: settings.storage_bytes,
            });
        }

        let empty = AdmittedCompiledStream::admit(&plan, &[]).map_err(preparation)?;
        let (mut control, renderer) =
            StreamControl::open(plan, StreamAnchor::new(SampleTime::ZERO, settings.entry))
                .map_err(preparation)?;
        // This empty preparation-only scheduler publishes nothing and stamps no
        // note. It lets both templates reuse the existing history/pairing/catch-up
        // compiler. Every template is withdrawn; its working minter is never
        // promoted. The authoritative minter consequently remains fresh.
        let scheduler =
            CompiledEventScheduler::prepare(&mut control, &empty).map_err(preparation)?;
        let initial = normalize(&mut control, &stream, interval, settings.entry)?;
        let repeating = normalize(&mut control, &stream, interval, interval.start())?;
        drop(scheduler);
        admit_programs(&initial, &repeating, interval, &profile)?;
        let arbiter = PublicationArbiter::prepare_one_quantum(&profile).map_err(preparation)?;
        let tokens = empty_slots(initial.note_tokens.max(repeating.note_tokens))?;
        let held = empty_slots(held_capacity)?;
        let boundaries = empty_slots(boundary_capacity)?;
        Ok(Self {
            source: LoopSource {
                control,
                producer,
                initial,
                repeating,
                program: PassProgram::Entry,
                cursor: 0,
                interval,
                position: settings.entry,
                pass: LoopPassId(1),
                started: false,
                tokens,
                held,
                held_len: 0,
                boundaries,
                boundary_len: 0,
            },
            renderer,
            arbiter,
            fault: None,
            storage_bytes,
        })
    }
}

fn normalize(
    control: &mut StreamControl,
    stream: &AdmittedCompiledStream,
    interval: LoopInterval,
    entry: PlanPosition,
) -> Result<LoopProgram, LoopPrepareError> {
    let candidate = control
        .plan_activation(
            stream,
            ActivationRequest {
                at: SampleTime::ZERO,
                position: entry,
                loop_interval: Some(interval),
            },
        )
        .map_err(preparation)?;
    let result = normalize_candidate(&candidate, interval, entry);
    if let Err((candidate, error)) = control.withdraw(candidate) {
        // Internal, never published candidate; construction fails and drops both
        // owners off-thread, so this cannot strand a callback-visible credit.
        drop(candidate);
        return Err(preparation(error));
    }
    result
}

fn normalize_candidate(
    candidate: &TransportActivation,
    interval: LoopInterval,
    entry: PlanPosition,
) -> Result<LoopProgram, LoopPrepareError> {
    let duration = interval.end().as_u64() - entry.as_u64();
    let count = candidate
        .events
        .iter()
        .take_while(|event| event.envelope().time().as_u64() < duration)
        .count();
    let mut events = Vec::new();
    events
        .try_reserve_exact(count)
        .map_err(|_| LoopPrepareError::Allocation)?;
    let mut tokens = HashMap::new();
    let mut held = 0_usize;
    for event in candidate.events.iter().take(count) {
        let payload = match event.payload() {
            EventPayload::ReleaseGroup(_) => return Err(LoopPrepareError::LiveReleaseGroup),
            EventPayload::Note {
                identity,
                edge:
                    NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
            } => {
                let token = NoteToken(tokens.len());
                if tokens.insert(identity, token).is_some() {
                    return Err(LoopPrepareError::Occurrence);
                }
                held = held.checked_add(1).ok_or(LoopPrepareError::Layout)?;
                LoopPayload::On {
                    token,
                    slot,
                    key,
                    velocity,
                }
            }
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            } => {
                let token = *tokens.get(&identity).ok_or(LoopPrepareError::Occurrence)?;
                held = held.checked_sub(1).ok_or(LoopPrepareError::Occurrence)?;
                LoopPayload::Off(token)
            }
            EventPayload::Expression {
                identity,
                expression,
            } => LoopPayload::Expression(
                *tokens.get(&identity).ok_or(LoopPrepareError::Occurrence)?,
                expression,
            ),
            EventPayload::Bend { identity, cents } => LoopPayload::Bend(
                *tokens.get(&identity).ok_or(LoopPrepareError::Occurrence)?,
                cents,
            ),
            payload @ (EventPayload::SetParameter { .. }
            | EventPayload::Controller(_)
            | EventPayload::RestoreController(_)) => LoopPayload::Direct(payload),
            EventPayload::Fade { .. } | EventPayload::Reset { .. } => {
                return Err(LoopPrepareError::Stealing);
            }
        };
        events.push(LoopEvent {
            offset: FrameCount::new(event.envelope().time().as_u64()),
            payload,
        });
    }
    let mut catch_up = Vec::new();
    catch_up
        .try_reserve_exact(candidate.catch_up.len())
        .map_err(|_| LoopPrepareError::Allocation)?;
    for event in &candidate.catch_up {
        match event.payload() {
            payload @ (EventPayload::SetParameter { .. }
            | EventPayload::Controller(_)
            | EventPayload::RestoreController(_)) => catch_up.push(payload),
            _ => return Err(LoopPrepareError::Occurrence),
        }
    }
    Ok(LoopProgram {
        start: entry,
        events: events.into_boxed_slice(),
        catch_up: catch_up.into_boxed_slice(),
        note_tokens: tokens.len(),
        omissions: LoopOmissions {
            releases: LoopOmissionCount(candidate.omitted_releases()),
            expressions: LoopOmissionCount(candidate.omitted_expressions()),
        },
        end_held: HeldNoteCount::measured(
            u32::try_from(held).map_err(|_| LoopPrepareError::Layout)?,
        ),
    })
}

fn add_repeated(
    positions: &mut Vec<PlanPosition>,
    at: u64,
    count: usize,
) -> Result<(), LoopPrepareError> {
    positions
        .try_reserve(count)
        .map_err(|_| LoopPrepareError::Allocation)?;
    positions.extend(std::iter::repeat_n(PlanPosition::new(at), count));
    Ok(())
}

pub(super) fn admit_programs(
    initial: &LoopProgram,
    repeating: &LoopProgram,
    interval: LoopInterval,
    profile: &HostProfile,
) -> Result<(), LoopPrepareError> {
    let length = interval.end().as_u64() - interval.start().as_u64();
    let first_wrap = interval.end().as_u64() - initial.start.as_u64();
    let copies = u64::from(QUANTUM_FRAMES)
        .div_ceil(length)
        .checked_add(2)
        .ok_or(LoopPrepareError::Layout)?;
    let shares = profile.limits().events().shares();
    let repeated_cost = repeating
        .catch_up
        .len()
        .checked_add(repeating.end_held.get() as usize)
        .and_then(|value| value.checked_add(1))
        .ok_or(LoopPrepareError::Layout)?;
    let mut periodic_session = Vec::new();
    add_repeated(&mut periodic_session, 0, repeated_cost)?;
    admit_loop(
        &periodic_session,
        PlanPosition::ZERO,
        PlanPosition::new(length),
        shares.session_event_share(),
    )
    .map_err(|source| LoopPrepareError::Admission {
        class: crate::publish::ProducerClass::Session,
        source,
    })?;

    let periodic_compiled: Vec<_> = repeating
        .events
        .iter()
        .map(|event| PlanPosition::new(event.offset.as_u64()))
        .collect();
    admit_loop(
        &periodic_compiled,
        PlanPosition::ZERO,
        PlanPosition::new(length),
        shares.compiled_event_share(),
    )
    .map_err(|source| LoopPrepareError::Admission {
        class: crate::publish::ProducerClass::Compiled,
        source,
    })?;

    let mut session = Vec::new();
    let initial_cost = initial
        .catch_up
        .len()
        .checked_add(1)
        .ok_or(LoopPrepareError::Layout)?;
    add_repeated(&mut session, 0, initial_cost)?;
    let mut compiled: Vec<_> = initial
        .events
        .iter()
        .map(|event| PlanPosition::new(event.offset.as_u64()))
        .collect();
    for pass in 0..copies {
        let at = length
            .checked_mul(pass)
            .and_then(|offset| first_wrap.checked_add(offset))
            .ok_or(LoopPrepareError::Layout)?;
        let cost = if pass == 0 {
            repeating
                .catch_up
                .len()
                .checked_add(initial.end_held.get() as usize)
                .and_then(|value| value.checked_add(1))
                .ok_or(LoopPrepareError::Layout)?
        } else {
            repeated_cost
        };
        add_repeated(&mut session, at, cost)?;
        compiled
            .try_reserve(repeating.events.len())
            .map_err(|_| LoopPrepareError::Allocation)?;
        for event in &repeating.events {
            compiled.push(PlanPosition::new(
                at.checked_add(event.offset.as_u64())
                    .ok_or(LoopPrepareError::Layout)?,
            ));
        }
    }
    admit_linear(&session, shares.session_event_share()).map_err(|source| {
        LoopPrepareError::Admission {
            class: crate::publish::ProducerClass::Session,
            source,
        }
    })?;
    admit_linear(&compiled, shares.compiled_event_share()).map_err(|source| {
        LoopPrepareError::Admission {
            class: crate::publish::ProducerClass::Compiled,
            source,
        }
    })?;
    Ok(())
}
