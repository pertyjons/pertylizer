//! Tests for `rt_alloc_guard`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of alloc/dealloc/realloc calls seen while the calling thread is
/// armed. Reset by [`no_alloc`] around each guarded region.
static EVENTS: AtomicU64 = AtomicU64::new(0);

thread_local! {
    // `const` init keeps the thread-local off the lazy, potentially
    // allocating initialization path — safe to read from inside `alloc`.
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

struct CountingAlloc;

// SAFETY: every method forwards verbatim to the system allocator; the only
// addition is a relaxed atomic increment gated on a thread-local flag,
// neither of which allocates or changes allocation semantics.
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
        EVENTS.fetch_add(1, Ordering::Relaxed);
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Run `f` with allocation counting armed on this thread and return the
/// number of alloc/dealloc/realloc events it triggered.
fn count_allocs(f: impl FnOnce()) -> u64 {
    ARMED.with(|a| a.set(true));
    EVENTS.store(0, Ordering::Relaxed);
    f();
    let count = EVENTS.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(false));
    count
}

use super::*;
use crate::voice_allocator::{AllocatorConfig, VoiceAllocator};

/// Build an engine with one default instrument holding a single sounding
/// voice, warmed up so all lazy initialization is already done.
fn warmed_engine_with_voice() -> (SynthEngine, EngineHandle, AudioCallbackContext, Vec<f32>) {
    let (mut engine, mut handle) = SynthEngine::new();

    let mut instrument =
        Instrument::with_config(InstrumentId::FIRST, "Default", AllocatorConfig::default());
    instrument.set_midi_channel(MidiChannel::CH1);
    SynthEngine::populate_default_voice_graph(instrument.voice_graph_mut());
    *instrument.allocator_mut() = VoiceAllocator::with_graph_template(
        instrument.allocator().config().clone(),
        instrument.voice_graph(),
    );
    handle.send(EngineCommand::AddInstrument {
        instrument: Box::new(instrument),
    });
    handle.note_on(MidiNote::C4, Velocity::new(0.8));

    let context = AudioCallbackContext {
        sample_rate: synth_core::audio::SampleRate::new(48000),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: Seconds::ZERO,
    };
    let mut out = vec![0.0f32; 256 * 2];

    // Warm up: drain the AddInstrument/NoteOn commands and let any per-module
    // lazy allocation settle so the measured block is genuine steady state.
    for _ in 0..8 {
        out.fill(0.0);
        engine.process(&mut out, &context);
    }

    (engine, handle, context, out)
}

#[test]
fn process_does_not_allocate_in_steady_state() {
    let (mut engine, _handle, context, mut out) = warmed_engine_with_voice();

    let allocs = count_allocs(|| {
        for _ in 0..4 {
            out.fill(0.0);
            engine.process(&mut out, &context);
        }
    });

    assert_eq!(
        allocs, 0,
        "SynthEngine::process() allocated {allocs} time(s) on the audio thread; \
         the RT path must be allocation-free"
    );
}

#[test]
fn common_control_commands_do_not_allocate_during_process() {
    let (mut engine, mut handle, context, mut out) = warmed_engine_with_voice();

    assert!(handle.set_master_volume(Gain::new(0.5)));
    let master_volume_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert!(handle.note_off(MidiNote::C4));
    let note_off_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert!(handle.note_on(MidiNote::new(64), Velocity::new(0.7)));
    let note_on_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert_eq!(
        (master_volume_allocs, note_off_allocs, note_on_allocs),
        (0, 0, 0),
        "processing common control commands allocated on the audio thread"
    );
}

#[test]
fn guard_actually_detects_an_allocation() {
    // Sanity check on the guard itself: a deliberate heap allocation inside
    // the armed region must be observed, otherwise the guard above is inert.
    let allocs = count_allocs(|| {
        let v: Vec<u8> = Vec::with_capacity(1024);
        std::hint::black_box(&v);
    });
    assert!(
        allocs >= 1,
        "the allocation guard failed to observe a deliberate Vec allocation"
    );
}
