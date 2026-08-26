//! The render loop allocates nothing.
//!
//! This lives inside the library's test binary because it installs a
//! `#[global_allocator]`, and a process may only have one. The pattern is V1's
//! `rt_alloc_guard`, which exists for the same reason.
//!
//! **The guard arms before the first render call after preparation**, and stays armed
//! across the calls that follow. Warming up first would hide the failure that matters:
//! a kernel or buffer that allocates lazily on first use passes a steady-state test
//! while the first audio callback glitches.

#![allow(
    unsafe_code,
    reason = "the allocation regression harness must implement and forward the global allocator"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use crate::compile::{RenderConfig, compile};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters};
use crate::plan::CompiledPlan;
use crate::profile::HostProfile;
use crate::quantities::{Amplitude, ChannelLayout, Frequency, ParameterValue, SampleRate};
use crate::render::{
    AudioBlockMut, EventEnvelope, EventPayload, PreparedRenderer, Renderer, TimedEvent, TimedEvents,
};
use crate::schedule::{CompiledEvent, CompiledEventScheduler};
use crate::time::{FrameCount, PlanPosition, SampleTime, StreamAnchor, TimeSource};

thread_local! {
    // `const` init keeps the thread-local off the lazy, potentially allocating
    // initialization path — safe to read from inside `alloc`.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    // Per-thread, because a process-global counter lets an intentionally allocating
    // test on another thread contaminate this one.
    static EVENTS: Cell<u64> = const { Cell::new(0) };
}

struct CountingAlloc;

// SAFETY: every method forwards verbatim to the system allocator; the only addition is
// a thread-local counter update gated on a thread-local flag, neither of which
// allocates or changes allocation semantics.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_event();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note_event();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note_event();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note_event();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn note_event() {
    if ARMED.with(Cell::get) {
        EVENTS.with(|events| events.set(events.get().saturating_add(1)));
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

struct AllocationGuard;

impl AllocationGuard {
    fn arm() -> Self {
        EVENTS.with(|events| events.set(0));
        ARMED.with(|armed| {
            assert!(!armed.replace(true), "allocation counting must not nest");
        });
        Self
    }

    fn count(&self) -> u64 {
        EVENTS.with(Cell::get)
    }
}

impl Drop for AllocationGuard {
    fn drop(&mut self) {
        ARMED.with(|armed| armed.set(false));
    }
}

/// Run `f` with counting armed, and return how many allocator events it caused.
fn count_allocs(f: impl FnOnce()) -> u64 {
    let guard = AllocationGuard::arm();
    f();
    guard.count()
}

const BLOCK: usize = 512;
const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);

fn sine_plan() -> CompiledPlan {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Stereo,
    )
    .expect("valid harness profile");

    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    compile(&ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("the plan fits the default profile")
}

#[test]
fn the_first_render_after_preparation_allocates_nothing() {
    let mut renderer = PreparedRenderer::prepare(
        sine_plan(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let mut samples = vec![0.0_f32; BLOCK * 2];

    // Armed before the first call, not after a warm-up: lazy first-use allocation is
    // exactly what a warmed test cannot see.
    let allocs = count_allocs(|| {
        let output = AudioBlockMut::new(&mut samples, BLOCK, ChannelLayout::Stereo)
            .expect("a correctly shaped block");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("the first call renders");
    });

    assert_eq!(
        allocs, 0,
        "the first render call after preparation allocated {allocs} time(s); every buffer the \
         loop touches is allocated at preparation"
    );
}

#[test]
fn repeated_renders_at_varying_block_sizes_allocate_nothing() {
    let mut renderer = PreparedRenderer::prepare(
        sine_plan(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let mut samples = vec![0.0_f32; BLOCK * 2];

    let allocs = count_allocs(|| {
        // Irregular sizes on purpose: a size the carry has never seen is where a
        // resize would happen if any survived.
        for frames in [1_usize, 64, 63, 65, 512, 7, 128] {
            let region = samples
                .get_mut(..frames * 2)
                .expect("the scratch covers every size used here");
            let output = AudioBlockMut::new(region, frames, ChannelLayout::Stereo)
                .expect("a correctly shaped block");
            renderer
                .render(output, TimedEvents::EMPTY)
                .expect("every size up to the maximum renders");
        }
    });

    assert_eq!(allocs, 0, "rendering allocated {allocs} time(s)");
}

#[test]
fn resolving_and_applying_events_allocates_nothing() {
    let mut renderer = PreparedRenderer::prepare(
        sine_plan(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();
    let slot = renderer
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency parameter");
    let mut samples = vec![0.0_f32; BLOCK * 2];

    // Presented in descending position order, so the in-place sort has work to do —
    // the one sort the loop performs, and the one that must not allocate.
    let events: Vec<TimedEvent> = (0..8)
        .rev()
        .map(|index| {
            TimedEvent::new(
                EventEnvelope::new(epoch, SampleTime::new(index * 8), TimeSource::Compiled),
                EventPayload::SetParameter {
                    slot,
                    value: ParameterValue::new(220.0 + index as f32).expect("finite"),
                },
            )
        })
        .collect();

    let allocs = count_allocs(|| {
        let region = samples
            .get_mut(..128 * 2)
            .expect("the scratch covers 128 frames");
        let output = AudioBlockMut::new(region, 128, ChannelLayout::Stereo)
            .expect("a correctly shaped block");
        renderer
            .render(output, TimedEvents::new(&events))
            .expect("the span fits the quanta this call renders");
    });

    assert_eq!(
        allocs, 0,
        "event resolution allocated {allocs} time(s); the scratch is preallocated and the sort \
         is in place"
    );
}

#[test]
fn selecting_a_compiled_schedule_for_the_first_call_allocates_nothing() {
    let mut renderer = PreparedRenderer::prepare(
        sine_plan(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let slot = renderer
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency parameter");
    let events = [CompiledEvent::new(
        SampleTime::ZERO,
        EventPayload::SetParameter {
            slot,
            value: ParameterValue::new(880.0).expect("finite"),
        },
    )];
    let mut scheduler =
        CompiledEventScheduler::prepare(&renderer, &events).expect("the schedule is valid");
    // Prepared outside the counted region, like every other store the loop uses.
    let mut arbiter = crate::publish::PublicationArbiter::prepare(
        &crate::profile::HostProfile::harness(
            crate::quantities::SampleRate::new(48_000.0).expect("a valid rate"),
            crate::time::FrameCount::new(BLOCK as u64),
            ChannelLayout::Stereo,
        )
        .expect("the harness profile is valid"),
    )
    .expect("the publication store is preparable");
    let mut samples = vec![0.0_f32; 128 * 2];

    let allocs = count_allocs(|| {
        let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Stereo)
            .expect("a correctly shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the first scheduled call renders");
    });

    assert_eq!(
        allocs, 0,
        "compiled schedule selection and rendering allocated {allocs} time(s)"
    );
}

#[test]
fn a_complete_publication_pass_allocates_nothing() {
    // The behavioural half of the arbiter's real-time guarantee. The source scan can be
    // escaped by moving work out of the scanned file or by naming an allocating method
    // like an accessor; this one cannot, because it counts what the allocator actually
    // did. It drives a full pass — open, fill every publishable class to its share across
    // two quanta, and seal — so a store that grew rather than being written by index shows
    // up here whatever the call is named.
    use crate::profile::HostProfile;
    use crate::publish::{ProducerClass, PublicationArbiter};
    use crate::quantities::SampleRate;
    use crate::render::{EventEnvelope, TimedEvent};
    use crate::time::{FrameCount, QUANTUM_FRAMES, StreamEpoch, TimeSource};

    let host = HostProfile::harness(
        SampleRate::new(48_000.0).expect("a valid rate"),
        FrameCount::new(256),
        ChannelLayout::Stereo,
    )
    .expect("the default harness profile is valid");
    let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");
    let slot = crate::plan::ParameterSlot::new(crate::plan::PlanId::FILL, 0);
    let event = |quantum: u64| {
        TimedEvent::new(
            EventEnvelope::new(
                StreamEpoch::from_raw(1),
                SampleTime::new(quantum * u64::from(QUANTUM_FRAMES) + 1),
                TimeSource::Compiled,
            ),
            EventPayload::SetParameter {
                slot,
                value: ParameterValue::ZERO,
            },
        )
    };

    let allocs = count_allocs(|| {
        let mut publication = arbiter
            .open(SampleTime::ZERO, 2)
            .expect("a two-quantum window");
        for class in ProducerClass::ALL {
            if class == ProducerClass::Internal {
                continue;
            }
            let share = class.share_of(&host).get();
            for quantum in 0..2 {
                for _ in 0..share {
                    publication
                        .charge(class, event(quantum))
                        .expect("inside the share");
                }
            }
        }
        let batch = publication.seal();
        assert!(!batch.is_empty(), "the pass published something to measure");
    });

    assert_eq!(
        allocs, 0,
        "a publication pass allocated {allocs} time(s); the store is preallocated and written \
         by index"
    );
}
