//! Offline rendering: the same quanta, latency-compensated.
//!
//! ADR-0001 clause 9 is the whole of this module. An offline request for `N` frames
//! starting at plan sample `S` returns exactly `N` frames whose first sample **is**
//! plan sample `S`: the renderer discards the `Q` priming frames from the head and
//! drains `Q` frames past the end to fill the tail. The live path cannot do this —
//! there is nothing to discard into — so live output carries the `Q`-frame latency of
//! clause 7 while offline output carries none.
//!
//! Clause 10 is what that buys: content is identical between live and offline, and
//! only the real-time delay differs. That is a stricter requirement than V1 meets,
//! whose offline path renders at a different control rate than its live path.
//!
//! The asymmetry is permanent, and it has a failure mode worth naming: **a render path
//! that forgets the trim emits audio shifted by `Q` frames**, silently, because the
//! result is still valid audio. ADR-0001's risk control for that is an impulse at plan
//! sample 0 landing at output sample 0, and this module is what that test drives.

use crate::plan::CompiledPlan;
use crate::render::{AudioBlockMut, PreparedRenderer, Renderer, TimedEvent, TimedEvents};
use crate::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

/// An offline render that could not be produced.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum OfflineError {
    /// Preparation failed.
    #[error("offline preparation failed: {0}")]
    Prepare(#[from] crate::diagnostics::CompileError),
    /// Rendering failed.
    #[error("offline rendering failed: {0}")]
    Render(#[from] crate::diagnostics::RenderError),
    /// A compiled list could not be stamped: an unmatched release, or no producer to mint
    /// an occurrence from.
    #[error("offline stamping failed: {0}")]
    Stamp(#[from] crate::schedule::SchedulePrepareError),
    /// The requested length does not fit this platform's index type.
    #[error("a {frames}-frame render does not fit this platform's index type")]
    FramesUnrepresentable {
        /// The length that was asked for.
        frames: u64,
    },
}

/// An event for an offline render, **without an epoch**.
///
/// The epoch is deliberately absent. It is issued by preparation, which happens *inside*
/// [`render_offline`], so a caller assembling `TimedEvent`s beforehand could not know the
/// value to stamp them with — and an event carrying the wrong epoch is discarded as stale
/// under ADR-0032 clause 20, which would have made every non-empty offline event list
/// silently do nothing. An earlier revision of this function took stamped events and had
/// exactly that shape.
///
/// Provenance is `Compiled` by construction: these events come from a plan and a
/// timeline, where the timestamp is exact (ADR-0032 clause 18). An offline render has no
/// adapter, so nothing here could honestly be `Hardware` or `Arrival`.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct OfflineEvent {
    time: SampleTime,
    payload: crate::schedule::CompiledPayload,
}

impl OfflineEvent {
    /// An event at an engine time within the render.
    pub const fn new(time: SampleTime, payload: crate::schedule::CompiledPayload) -> Self {
        Self { time, payload }
    }

    /// When the event happens.
    pub const fn time(&self) -> SampleTime {
        self.time
    }

    /// What it does.
    pub const fn payload(&self) -> crate::schedule::CompiledPayload {
        self.payload
    }
}

/// Render `frames` frames of `plan`, starting at plan sample `start`.
///
/// Runs off the audio thread, so allocating the output here is not a real-time
/// concern; what must stay allocation-free is [`PreparedRenderer::render`], which this
/// only drives.
///
/// `events` must be in ascending time order. They are stamped with the epoch preparation
/// issues and presented to the call that renders their quantum — Phase 1's span rule makes
/// that the harness's job, because the scheduler that releases events as their quanta
/// approach is Phase 3's.
pub fn render_offline(
    plan: CompiledPlan,
    frames: FrameCount,
    start: PlanPosition,
    events: &[OfflineEvent],
) -> Result<Vec<f32>, OfflineError> {
    // A raw `usize` here would let a caller pass a sample count, a byte count, or a
    // duration in the wrong unit without a type error, and its range would vary by
    // platform. The checked conversion happens once, inside.
    let frames = frames
        .as_usize()
        .ok_or(OfflineError::FramesUnrepresentable {
            frames: frames.as_u64(),
        })?;
    let layout = plan.channel_layout();
    let channels = layout.channels();
    let block = plan
        .maximum_block_size()
        .as_usize()
        .unwrap_or(QUANTUM_FRAMES as usize)
        .max(1);
    let priming = QUANTUM_FRAMES as usize;

    let mut renderer = PreparedRenderer::prepare(plan, StreamAnchor::new(SampleTime::ZERO, start))?;

    // Stamped only now, with the epoch this stream actually has — and through the same
    // helper the compiled scheduler uses, so a note-on's occurrence and its release's pairing
    // are decided identically on both paths.
    let compiled: Vec<crate::schedule::CompiledEvent> = events
        .iter()
        .map(|event| crate::schedule::CompiledEvent::new(event.time(), event.payload()))
        .collect();
    let stamped = crate::schedule::stamp_compiled(&mut renderer, &compiled)?;

    // Render the requested frames plus the priming head, then drop the head. The
    // drained tail is the same fact seen from the other end: the last `Q` frames of
    // real content only leave the carry once `Q` further frames have been asked for.
    let total = frames.saturating_add(priming);
    let mut out = Vec::with_capacity(total.saturating_mul(channels));
    let mut scratch = vec![0.0_f32; block.saturating_mul(channels)];
    let mut produced = 0;

    while produced < total {
        let this_block = block.min(total - produced);
        let samples = this_block * channels;
        let Some(region) = scratch.get_mut(..samples) else {
            break;
        };
        region.fill(0.0);
        let due = events_for(&stamped, &renderer, this_block);
        let output = AudioBlockMut::new(region, this_block, layout)?;
        renderer.render(output, TimedEvents::new(due))?;
        out.extend_from_slice(&scratch[..samples]);
        produced += this_block;
    }

    // The trim. Forgetting it is the defect the impulse-alignment test exists for.
    let head = priming.saturating_mul(channels);
    if head <= out.len() {
        out.drain(..head);
    }
    out.truncate(frames.saturating_mul(channels));
    Ok(out)
}

/// The events whose quanta this call renders.
///
/// Phase 1's span is prevalidated: an event outside the quanta a call renders is a
/// contract violation rather than something the renderer holds. The renderer owns no
/// future-event store; Phase 3's publication arbiter instead presents only sealed
/// batches for the imminent call. Selecting here keeps the harness honest instead of
/// pushing that decision into the renderer.
///
fn events_for<'a>(
    events: &'a [TimedEvent],
    renderer: &PreparedRenderer,
    frames: usize,
) -> &'a [TimedEvent] {
    // Not `frames / Q`: the carry decides how many quanta a call renders, and the
    // priming head means the first calls render fewer than they serve.
    let quanta = renderer.quanta_needed_for(frames);
    if quanta == 0 {
        return &[];
    }
    let first = renderer.clock().quantum_index();
    let last = first.saturating_add(quanta as u64 - 1);

    let start = events
        .iter()
        .position(|event| event.envelope().time().quantum_index() >= first)
        .unwrap_or(events.len());
    let end = events
        .iter()
        .rposition(|event| event.envelope().time().quantum_index() <= last)
        .map_or(start, |index| index + 1);
    events.get(start..end.max(start)).unwrap_or(&[])
}
