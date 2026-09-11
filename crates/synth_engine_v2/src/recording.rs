//! P09-S002: complete recording configuration and bounded reservation/result custody.
//!
//! `SimulatedTakeStore` exercises the storage boundary before a real capture publisher
//! exists. It is not an arm command: destination/revision, tempo snapshot, source state
//! synchronization, note pairing, projection, audio assets and project transactions are
//! still required by their first consumers. No physical input can enter through it.
//! The payload is a fixed-size Copy cell supplied by a synthetic fixture; the store does
//! not interpret it as MIDI or accept an arbitrary byte count as its allocation cost.
//!
//! All allocation and discard run off-thread. The borrowed hot methods mutate only
//! reserved cells/metadata. A sealed payload stays in its slot; notification drain does
//! not release it, and Rust borrows prevent discard while a consumer holds its view.
//! This serial model does not establish a concurrent queue or backend source fence.

mod hot;
mod types;
pub use types::*;

use crate::host::ConnectionGeneration;
use crate::profile::RecordingLimits;
use crate::quantities::PreparedBytes;
use crate::time::{FrameCount, SampleTime};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TAKE: AtomicU64 = AtomicU64::new(0);

fn issue_take(counter: &AtomicU64) -> Result<TakeId, CaptureError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        })
        .map(|last| TakeId(last + 1))
        .map_err(|_| CaptureError::IdentityExhausted)
}

/// Exact typed allocation layout, including every pool slot, source ledger and
/// descriptor. Allocator bookkeeping is outside Rust's requested payload layout.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct CaptureLayout {
    starts: [usize; CaptureBuffer::COUNT],
    lengths: [usize; CaptureBuffer::COUNT],
    cells: usize,
    sources: usize,
    results: usize,
    bytes: PreparedBytes,
}

impl CaptureLayout {
    /// Checked before any allocation, over all simultaneously retained takes.
    pub fn for_payload<T: Copy>(limits: RecordingLimits) -> Result<Self, CaptureError> {
        let capture = limits.capture().ok_or(CaptureError::MissingConfiguration)?;
        let held = u64::from(limits.max_held_notes_per_take().get());
        let passes = u64::from(capture.max_capture_passes().get());
        let sources = u64::from(capture.max_capture_sources().get());
        let results = u64::from(capture.max_pending_capture_results().get());
        let carries = held
            .checked_mul(passes - 1)
            .and_then(|n| n.checked_mul(2))
            .ok_or(CaptureError::LayoutOverflow)?;
        let extents = [
            u64::from(limits.max_recorded_events_per_take().get()),
            u64::from(capture.max_tracked_input_notes().get()),
            held,
            sources,
            sources,
            passes,
            carries,
        ];
        let mut starts = [0; CaptureBuffer::COUNT];
        let mut lengths = [0; CaptureBuffer::COUNT];
        let mut cells = 0_u64;
        for (index, extent) in extents.into_iter().enumerate() {
            starts[index] = usize::try_from(cells).map_err(|_| CaptureError::LayoutOverflow)?;
            lengths[index] = usize::try_from(extent).map_err(|_| CaptureError::LayoutOverflow)?;
            cells = cells
                .checked_add(extent)
                .ok_or(CaptureError::LayoutOverflow)?;
        }
        let data = allocation_bytes::<Option<T>>(cells)?;
        let ledger = allocation_bytes::<Option<SourceAttribution>>(sources)?;
        let slots = allocation_bytes::<TakeSlot<T>>(results)?;
        let total = data
            .checked_add(ledger)
            .and_then(|n| n.checked_mul(results))
            .and_then(|n| n.checked_add(slots))
            .and_then(|n| n.checked_add(size_of::<SimulatedTakeStore<T>>() as u64))
            .ok_or(CaptureError::LayoutOverflow)?;
        let bytes = PreparedBytes::measured(total);
        if bytes > capture.max_capture_bytes() {
            return Err(CaptureError::ByteBudget {
                required: bytes,
                available: capture.max_capture_bytes(),
            });
        }
        Ok(Self {
            starts,
            lengths,
            cells: usize::try_from(cells).map_err(|_| CaptureError::LayoutOverflow)?,
            sources: usize::try_from(sources).map_err(|_| CaptureError::LayoutOverflow)?,
            results: usize::try_from(results).map_err(|_| CaptureError::LayoutOverflow)?,
            bytes,
        })
    }

    /// Collection extent of a named reserved region, independent of current occupancy.
    pub const fn cells_in(self, buffer: CaptureBuffer) -> usize {
        self.lengths[buffer.index()]
    }
    pub const fn bytes(self) -> PreparedBytes {
        self.bytes
    }
}

fn allocation_bytes<T>(count: u64) -> Result<u64, CaptureError> {
    let bytes = count
        .checked_mul(size_of::<T>() as u64)
        .ok_or(CaptureError::LayoutOverflow)?;
    if count > usize::MAX as u64 || bytes > isize::MAX as u64 {
        return Err(CaptureError::LayoutOverflow);
    }
    Ok(bytes)
}

fn empty_cells<T: Copy>(count: usize) -> Result<Box<[Option<T>]>, CaptureError> {
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(count)
        .map_err(|_| CaptureError::Allocation)?;
    cells.resize(count, None);
    Ok(cells.into_boxed_slice())
}

struct TakeSlot<T: Copy> {
    state: Option<TakeState>,
    data: Box<[Option<T>]>,
    sources: Box<[Option<SourceAttribution>]>,
}

/// Serialized fixture entry point, explicitly requiring complete capture settings.
/// The fixed-size payload's layout supplies every storage-cell cost; adding audio or
/// projection later must charge their allocations too before an arm API is enabled.
#[must_use]
pub struct SimulatedTakeStore<T: Copy> {
    lateness_allowance: FrameCount,
    layout: CaptureLayout,
    slots: Box<[TakeSlot<T>]>,
    notification: Option<TakeReservation>,
    notification_misses: DiagnosticCount,
    newest_source: Option<ConnectionGeneration>,
}

impl<T: Copy> SimulatedTakeStore<T> {
    pub fn prepare_fixture(limits: RecordingLimits) -> Result<Self, CaptureError> {
        let layout = CaptureLayout::for_payload::<T>(limits)?;
        let capture = limits.capture().ok_or(CaptureError::MissingConfiguration)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(layout.results)
            .map_err(|_| CaptureError::Allocation)?;
        for _ in 0..layout.results {
            slots.push(TakeSlot {
                state: None,
                data: empty_cells(layout.cells)?,
                sources: empty_cells(layout.sources)?,
            });
        }
        Ok(Self {
            lateness_allowance: capture.capture_lateness_allowance(),
            layout,
            slots: slots.into_boxed_slice(),
            notification: None,
            notification_misses: DiagnosticCount::default(),
            newest_source: None,
        })
    }

    pub const fn layout(&self) -> CaptureLayout {
        self.layout
    }

    /// Reserve storage and result/quality entitlement, before any future arm command.
    /// No destination is mutated or inferred here. The caller supplies exact synthetic
    /// source fences later; a physical adapter needs its own accepted proof.
    /// A known live generation inherits its watermark. A first introduction must be
    /// newer than every previously introduced generation; this bounded high-water mark
    /// rejects retired generations even after their final retained take is discarded.
    pub fn reserve(
        &mut self,
        window: CaptureWindow,
        sources: &[ConnectionGeneration],
    ) -> Result<TakeReservation, CaptureError> {
        if sources.is_empty() || sources.len() > self.layout.sources {
            return Err(CaptureError::InvalidSources);
        }
        for (index, source) in sources.iter().enumerate() {
            if sources[..index].contains(source) {
                return Err(CaptureError::InvalidSources);
            }
        }
        for source in sources {
            if let Some((known, epoch)) = self.known_source(*source) {
                if known.quiescent {
                    return Err(CaptureError::SourceQuiescent);
                }
                if epoch != window.epoch() {
                    return Err(CaptureError::ForeignEpoch);
                }
                if known.frontier > window.start() {
                    return Err(CaptureError::WindowBeforeFrontier);
                }
            } else if self
                .newest_source
                .is_some_and(|newest| source.as_u64() <= newest.as_u64())
            {
                return Err(CaptureError::SourceNotFresh);
            }
        }
        let slot = self
            .slots
            .iter()
            .position(|slot| slot.state.is_none())
            .ok_or(CaptureError::ResultsFull)?;
        let id = issue_take(&NEXT_TAKE)?;
        for (index, source) in sources.iter().enumerate() {
            let inherited = self
                .known_source(*source)
                .map(|(state, _)| state)
                .unwrap_or(SourceAttribution {
                    generation: *source,
                    watermark: SampleTime::ZERO,
                    frontier: SampleTime::ZERO,
                    fenced: false,
                    quiescent: false,
                });
            // The vacant slot is excluded from known_source until state is installed;
            // duplicate source inputs were refused before any mutation.
            self.slots[slot].sources[index] = Some(inherited);
        }
        self.slots[slot].state = Some(TakeState {
            id,
            requested_window: window,
            window,
            outcome: None,
            quality: CaptureQuality::default(),
            ordinary_len: 0,
            forced_partial: false,
        });
        for source in sources {
            if self
                .newest_source
                .is_none_or(|newest| source.as_u64() > newest.as_u64())
            {
                self.newest_source = Some(*source);
            }
        }
        Ok(TakeReservation { id, slot })
    }

    /// The replicated ledgers share one source-generation state across retained takes.
    fn known_source(
        &self,
        generation: ConnectionGeneration,
    ) -> Option<(SourceAttribution, crate::time::StreamEpoch)> {
        for slot in &self.slots {
            if let Some(state) = slot.state {
                for source in slot.sources.iter().flatten() {
                    if source.generation == generation {
                        return Some((*source, state.window.epoch()));
                    }
                }
            }
        }
        None
    }

    /// Immutable payload plus current sticky quality, with no payload copy or transfer.
    pub fn result(&self, ticket: TakeReservation) -> Result<CaptureResult<'_, T>, CaptureError> {
        let slot = self
            .slots
            .get(ticket.slot)
            .ok_or(CaptureError::StaleReservation)?;
        let state = slot
            .state
            .as_ref()
            .filter(|state| state.id == ticket.id)
            .ok_or(CaptureError::StaleReservation)?;
        let outcome = state.outcome.ok_or(CaptureError::NotSealed)?;
        Ok(CaptureResult {
            slot,
            state,
            outcome,
            layout: self.layout,
        })
    }

    /// Off-thread recovery when the one-slot notification hint is saturated.
    /// Discoverable results retain their payload whether or not the hint was delivered.
    pub fn retained_results(&self) -> impl Iterator<Item = TakeReservation> + '_ {
        self.slots.iter().enumerate().filter_map(|(slot, value)| {
            value
                .state
                .filter(|state| state.outcome.is_some())
                .map(|state| TakeReservation { id: state.id, slot })
        })
    }

    pub fn take_notification(&mut self) -> Option<TakeReservation> {
        self.notification.take()
    }
    pub const fn notification_misses(&self) -> DiagnosticCount {
        self.notification_misses
    }

    /// Explicit discard and final owner quality acknowledgement, off-thread only.
    /// Sealed reads, notification drain and any later project commit do not call this.
    /// A borrowed result prevents mutation/reclamation until its last borrow ends.
    pub fn discard(
        &mut self,
        ticket: TakeReservation,
        acknowledged: CaptureQuality,
    ) -> Result<(), CaptureError> {
        let slot = self.slot_mut(ticket)?;
        let state = slot.state.as_ref().ok_or(CaptureError::StaleReservation)?;
        if state.outcome.is_none() {
            return Err(CaptureError::NotSealed);
        }
        if slot
            .sources
            .iter()
            .flatten()
            .any(|source| !source.quiescent)
        {
            return Err(CaptureError::SourcesLive);
        }
        if state.quality != acknowledged {
            return Err(CaptureError::QualityChanged);
        }
        slot.data.fill(None);
        slot.sources.fill(None);
        slot.state = None;
        if self.notification == Some(ticket) {
            self.notification = None;
        }
        Ok(())
    }
}

/// A borrowed sealed result, never the sole owner of its payload.
#[must_use]
pub struct CaptureResult<'a, T: Copy> {
    slot: &'a TakeSlot<T>,
    state: &'a TakeState,
    outcome: CaptureOutcome,
    layout: CaptureLayout,
}
impl<T: Copy> CaptureResult<'_, T> {
    /// The caller's original interval, retained even if interruption precedes its start.
    pub const fn requested_window(&self) -> CaptureWindow {
        self.state.requested_window
    }

    /// The selected interval. An interruption before the requested start produces an
    /// empty interval at the last valid boundary, not invented capture at a later time.
    pub const fn window(&self) -> CaptureWindow {
        self.state.window
    }
    pub const fn sealed_outcome(&self) -> CaptureOutcome {
        self.outcome
    }
    pub const fn quality(&self) -> CaptureQuality {
        self.state.quality
    }
    pub const fn effective_outcome(&self) -> CaptureOutcome {
        self.state.quality.effective_outcome(self.outcome)
    }
    pub fn cells(&self, buffer: CaptureBuffer) -> &[Option<T>] {
        let start = self.layout.starts[buffer.index()];
        let length = if buffer == CaptureBuffer::Ordinary {
            self.state.ordinary_len
        } else {
            self.layout.lengths[buffer.index()]
        };
        &self.slot.data[start..start + length]
    }
}

#[cfg(test)]
#[path = "recording/tests.rs"]
mod tests;
