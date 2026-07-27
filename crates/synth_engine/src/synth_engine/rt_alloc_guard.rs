//! Tests for `rt_alloc_guard`.
#![allow(
    unsafe_code,
    reason = "the allocation regression harness must implement and forward the global allocator"
)]

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
use ringbuf::HeapRb;
use ringbuf::traits::{Producer, Split};

/// Build an engine with one default instrument holding a single sounding
/// voice, warmed up so all lazy initialization is already done.
fn warmed_engine_with_voice() -> (SynthEngine, EngineHandle, AudioCallbackContext, Vec<f32>) {
    let (mut engine, mut handle) = SynthEngine::new();

    let mut instrument =
        Instrument::with_config(InstrumentId::FIRST, "Default", AllocatorConfig::default());
    instrument.set_midi_channel(MidiChannelSelection::CH1);
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
        sample_rate: synth_core::audio::DeviceSampleRate::new(48000),
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

fn callback_context(frames: usize) -> AudioCallbackContext {
    AudioCallbackContext {
        sample_rate: synth_core::audio::DeviceSampleRate::new(48000),
        frames,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: Seconds::ZERO,
    }
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
fn steady_state_block_sizes_do_not_allocate() {
    let (mut engine, _handle, _context, _out) = warmed_engine_with_voice();

    for frames in [1, 64, 256, 1024] {
        let context = callback_context(frames);
        let mut out = vec![0.0f32; frames * 2];

        // Buffer growth for a newly observed device block size is warmed
        // outside the guarded region; subsequent callbacks at that size must
        // remain allocation-free.
        engine.process(&mut out, &context);
        let allocs = count_allocs(|| {
            for _ in 0..4 {
                out.fill(0.0);
                engine.process(&mut out, &context);
            }
        });

        assert_eq!(
            allocs, 0,
            "{frames}-frame steady-state processing allocated {allocs} time(s)"
        );
    }
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

    assert!(handle.send(EngineCommand::Stop));
    let stop_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert_eq!(
        (
            master_volume_allocs,
            note_off_allocs,
            note_on_allocs,
            stop_allocs
        ),
        (0, 0, 0, 0),
        "processing common control commands allocated on the audio thread"
    );
}

#[test]
fn instrument_control_matrix_does_not_allocate_during_process() {
    let (mut engine, mut handle, context, mut out) = warmed_engine_with_voice();
    let instrument_id = InstrumentId::FIRST;

    assert!(handle.send(EngineCommand::SetInstrumentEnabled {
        instrument_id,
        enabled: false,
    }));
    assert!(handle.send(EngineCommand::SetInstrumentSolo {
        instrument_id,
        solo: true,
    }));
    assert!(handle.send(EngineCommand::SetInstrumentMidiChannel {
        instrument_id,
        channel: MidiChannelSelection::OMNI,
    }));
    assert!(handle.send(EngineCommand::ControlChange {
        channel: MidiChannelSelection::CH1,
        cc: 74,
        value: synth_core::NormalizedValue::new(0.6),
    }));
    assert!(handle.send(EngineCommand::PitchBend {
        value: synth_core::BipolarValue::new(0.25),
        channel: MidiChannelSelection::CH1,
    }));
    assert!(handle.send(EngineCommand::Aftertouch {
        value: synth_core::NormalizedValue::new(0.4),
        channel: MidiChannelSelection::CH1,
    }));

    let allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert_eq!(
        allocs, 0,
        "processing the instrument control matrix allocated {allocs} time(s)"
    );
}

#[test]
fn pointer_swap_commands_do_not_allocate_during_process() {
    let (mut engine, mut handle, context, mut out) = warmed_engine_with_voice();
    assert!(handle.send(EngineCommand::SetSong {
        song: Arc::new(synth_sequencer::SharedSong::new(
            synth_sequencer::Song::new("replacement"),
        )),
    }));

    let song_swap_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    let ring = HeapRb::<synth_core::StereoSample>::new(8);
    let (_producer, consumer) = ring.split();
    assert!(handle.send(EngineCommand::SetAudioInputConsumer {
        consumer,
        sample_rate: synth_core::audio::DeviceSampleRate::DVD_QUALITY,
    }));
    out.fill(0.0);
    engine.process(&mut out, &context);

    assert!(handle.send(EngineCommand::ClearAudioInputConsumer));
    let input_clear_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert_eq!(
        (song_swap_allocs, input_clear_allocs),
        (0, 0),
        "pointer-swap commands allocated or deallocated on the audio thread"
    );
}

#[test]
fn prepared_mod_grid_swap_and_processing_do_not_allocate() {
    let (mut engine, mut handle, context, mut out) = warmed_engine_with_voice();
    let mut dsp = crate::graph::ModuleGraph::new();
    dsp.add_module(Box::new(synth_modules::Lfo::new()));
    let mut runtime = crate::mod_grid::ModGridRuntime {
        instances: vec![crate::mod_grid::ModGridInstance {
            graph_id: synth_sequencer::ModGraphId::new(1),
            host_track: None,
            dsp,
            injections: Vec::new(),
            targets: Vec::new(),
        }],
        ..Default::default()
    };
    runtime.prepare_realtime();

    assert!(handle.send(EngineCommand::SetModGrid {
        runtime: Box::new(runtime),
    }));
    let allocs = count_allocs(|| {
        for _ in 0..4 {
            out.fill(0.0);
            engine.process(&mut out, &context);
        }
    });

    assert_eq!(
        allocs, 0,
        "prepared Mod Grid swap or processing allocated {allocs} time(s)"
    );
}

#[test]
fn instrument_lifecycle_does_not_allocate_during_process() {
    let (mut engine, mut handle, context, mut out) = warmed_engine_with_voice();
    let instrument_id = InstrumentId::new(2);
    let instrument = Box::new(Instrument::new(instrument_id, "Aux"));
    assert!(handle.send(EngineCommand::AddInstrument { instrument }));

    let add_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert!(handle.send(EngineCommand::RemoveInstrument { instrument_id }));
    let remove_allocs = count_allocs(|| {
        out.fill(0.0);
        engine.process(&mut out, &context);
    });

    assert_eq!(
        (add_allocs, remove_allocs),
        (0, 0),
        "instrument lifecycle allocated or deallocated on the audio thread"
    );
}

#[test]
fn live_input_resampling_does_not_allocate_during_process() {
    let (mut engine, mut handle, context, mut out) = warmed_engine_with_voice();
    let ring = HeapRb::<synth_core::StereoSample>::new(2048);
    let (mut producer, consumer) = ring.split();
    for _ in 0..1536 {
        assert!(
            producer
                .try_push(synth_core::StereoSample::new(0.25, -0.25))
                .is_ok()
        );
    }
    assert!(handle.send(EngineCommand::SetAudioInputConsumer {
        consumer,
        sample_rate: synth_core::audio::DeviceSampleRate::CD_QUALITY,
    }));

    let allocs = count_allocs(|| {
        for _ in 0..4 {
            out.fill(0.0);
            engine.process(&mut out, &context);
        }
    });

    assert_eq!(
        allocs, 0,
        "live-input sample-rate conversion allocated {allocs} time(s) on the audio thread"
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
