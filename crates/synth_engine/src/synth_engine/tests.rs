//! Tests for `tests`.

use super::*;
use crate::voice_allocator::{AllocationMode, AllocatorConfig, VoiceAllocator};
use synth_core::{ModuleType, VoiceCount};

/// Create a default instrument and add it to the engine via command.
fn add_default_instrument(engine: &mut SynthEngine, handle: &mut EngineHandle) {
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
    engine.process_commands();
}

fn add_instrument_with_config(
    engine: &mut SynthEngine,
    handle: &mut EngineHandle,
    config: AllocatorConfig,
) {
    let mut instrument = Instrument::with_config(InstrumentId::FIRST, "Default", config.clone());
    instrument.set_midi_channel(MidiChannelSelection::CH1);
    SynthEngine::populate_default_voice_graph(instrument.voice_graph_mut());
    *instrument.allocator_mut() = VoiceAllocator::with_graph_template(
        instrument.allocator().config().clone(),
        instrument.voice_graph(),
    );
    handle.send(EngineCommand::AddInstrument {
        instrument: Box::new(instrument),
    });
    engine.process_commands();
}

#[test]
fn test_engine_creation() {
    let (engine, handle) = SynthEngine::new();
    assert_eq!(engine.instruments.len(), 0);
    assert_eq!(handle.voice_count(), 0);
    assert!((handle.master_volume() - 1.0).abs() < 0.001);
    assert_eq!(handle.command_capacity(), CommandCapacity::DEFAULT);
    drop(engine);
}

#[test]
fn processor_error_channel_does_not_mislabel_device_loss_as_an_underrun() {
    let (mut engine, mut handle) = SynthEngine::new();

    AudioProcessor::on_error(&mut engine, synth_core::AudioError::DeviceDisconnected);
    assert!(handle.poll_event().is_none());

    AudioProcessor::on_error(&mut engine, synth_core::AudioError::BufferUnderrun);
    assert!(matches!(
        handle.poll_event(),
        Some(EngineEvent::BufferUnderrun)
    ));
}

#[test]
fn blocking_send_reports_timeout_when_configured_ring_stays_full() {
    let Some(capacity) = CommandCapacity::new(1) else {
        panic!("test command capacity must be non-zero");
    };
    let (_engine, mut handle) = SynthEngine::with_command_capacity(capacity);
    assert_eq!(handle.command_capacity(), capacity);
    assert!(handle.send(EngineCommand::ResetDsp));

    let sender = handle.command_sender();
    let result = sender.send_with_timeout(EngineCommand::ResetDsp, Duration::from_millis(5));

    assert_eq!(result, Err(CommandSendError::Timeout));
}

#[test]
fn command_capacity_rejects_an_empty_ring() {
    assert_eq!(CommandCapacity::new(0), None);
}

/// A command that does not fit is lost, not deferred, and `enqueued` cannot
/// record it — counting it there would leave `processed` permanently behind and
/// hang every barrier. So the loss has to be visible somewhere else, or a
/// caller waiting on `processed >= enqueued` concludes the engine applied
/// everything it was sent.
#[test]
fn a_command_that_does_not_fit_is_counted_as_dropped() {
    let Some(capacity) = CommandCapacity::new(1) else {
        panic!("test command capacity must be non-zero");
    };
    let (_engine, mut handle) = SynthEngine::with_command_capacity(capacity);
    let sync = Arc::clone(&handle.state.command_sync);

    assert!(handle.send(EngineCommand::ResetDsp));
    assert_eq!(sync.enqueued(), 1);
    assert_eq!(sync.dropped(), 0, "an accepted command is not a drop");

    assert!(!handle.send(EngineCommand::ResetDsp), "the ring is full");
    assert_eq!(sync.dropped(), 1);
    assert_eq!(
        sync.enqueued(),
        1,
        "a dropped command must not advance the enqueue counter"
    );
}

/// The blocking sender gives up with an error the caller may well log and move
/// past. The command is just as lost as in the non-blocking case, so it counts
/// the same.
#[test]
fn a_blocking_send_that_times_out_is_counted_as_dropped() {
    let Some(capacity) = CommandCapacity::new(1) else {
        panic!("test command capacity must be non-zero");
    };
    let (_engine, mut handle) = SynthEngine::with_command_capacity(capacity);
    let sync = Arc::clone(&handle.state.command_sync);
    assert!(handle.send(EngineCommand::ResetDsp));

    let sender = handle.command_sender();
    assert_eq!(
        sender.send_with_timeout(EngineCommand::ResetDsp, Duration::from_millis(5)),
        Err(CommandSendError::Timeout)
    );
    assert_eq!(sync.dropped(), 1);
}

/// The other way a push fails: a pointer-swap command is refused for want of a
/// return slot even though the ring itself has room. That path returns early,
/// so it needs its own count.
#[test]
fn a_pointer_swap_refused_for_want_of_a_return_slot_is_counted_as_dropped() {
    // One slot, so the first pointer swap takes it and does not give it back
    // until `cleanup_dropped_modules` runs.
    let Some(capacity) = CommandCapacity::new(1) else {
        panic!("test command capacity must be non-zero");
    };
    let (mut engine, mut handle) = SynthEngine::with_command_capacity(capacity);
    let sync = Arc::clone(&handle.state.command_sync);
    let replacement = || {
        Arc::new(synth_sequencer::SharedSong::new(
            synth_sequencer::Song::new("replacement"),
        ))
    };

    assert!(handle.send(EngineCommand::SetSong {
        song: replacement(),
    }));
    engine.process_commands();
    assert_eq!(sync.dropped(), 0);

    assert!(
        !handle.send(EngineCommand::SetSong {
            song: replacement(),
        }),
        "the ring has room, but there is no return slot"
    );
    assert_eq!(sync.dropped(), 1);
}

#[test]
fn pointer_swap_commands_wait_for_deferred_drop_capacity() {
    let Some(capacity) = CommandCapacity::new(1) else {
        panic!("test command capacity must be non-zero");
    };
    let (mut engine, mut handle) = SynthEngine::with_command_capacity(capacity);
    let replacement = || {
        Arc::new(synth_sequencer::SharedSong::new(
            synth_sequencer::Song::new("replacement"),
        ))
    };

    assert!(handle.send(EngineCommand::SetSong {
        song: replacement(),
    }));
    engine.process_commands();

    assert!(
        !handle.send(EngineCommand::SetSong {
            song: replacement(),
        }),
        "a pointer swap must not enter the audio queue without a return slot"
    );

    handle.cleanup_dropped_modules();
    assert!(handle.send(EngineCommand::SetSong {
        song: replacement(),
    }));
    engine.process_commands();
    handle.cleanup_dropped_modules();
}

#[test]
fn test_polyphonic_notes() {
    let config = AllocatorConfig {
        max_voices: VoiceCount::QUAD,
        mode: AllocationMode::Polyphonic,
        ..Default::default()
    };
    let (mut engine, mut handle) = SynthEngine::new();
    add_instrument_with_config(&mut engine, &mut handle, config);

    // Send multiple notes
    handle.note_on(MidiNote::C4, Velocity::new(0.8));
    handle.note_on(MidiNote::new(64), Velocity::new(0.8));
    handle.note_on(MidiNote::new(67), Velocity::new(0.8));

    // Process commands
    engine.process_commands();

    // Should have 3 active voices across all instruments
    let total_active: usize = engine
        .instruments
        .iter()
        .map(|p| p.active_voice_count())
        .sum();
    assert_eq!(total_active, 3);
}

#[test]
fn test_engine_starts_empty() {
    let (engine, _handle) = SynthEngine::new();
    assert_eq!(engine.instruments.len(), 0);
}

#[test]
fn test_add_instrument_via_command() {
    let (mut engine, mut handle) = SynthEngine::new();
    add_default_instrument(&mut engine, &mut handle);

    assert_eq!(engine.instruments.len(), 1);
    assert_eq!(engine.instruments[0].id(), InstrumentId::FIRST);
    assert_eq!(engine.instruments[0].name(), "Default");
    assert_eq!(
        engine.instruments[0].midi_channel(),
        MidiChannelSelection::CH1
    );
}

#[test]
fn test_part_channel_routing() {
    let (mut engine, mut handle) = SynthEngine::new();
    add_default_instrument(&mut engine, &mut handle);

    // Send note on channel 1 - should be received
    handle.note_on_channel(
        MidiNote::C4,
        Velocity::new(0.8),
        crate::instrument::MidiChannelSelection::CH1,
    );
    engine.process_commands();
    assert_eq!(engine.instruments[0].active_voice_count(), 1);

    // Send note on channel 2 - should NOT be received
    let ch2 = crate::instrument::MidiChannelSelection::from_one_indexed(2).unwrap();
    handle.note_on_channel(MidiNote::new(64), Velocity::new(0.8), ch2);
    engine.process_commands();
    assert_eq!(engine.instruments[0].active_voice_count(), 1); // Still 1
}

#[test]
fn explicit_instrument_note_target_bypasses_channel_routing() {
    let (mut engine, mut handle) = SynthEngine::new();
    add_default_instrument(&mut engine, &mut handle);
    let second_id = InstrumentId::new(2);
    let mut second = Instrument::with_config(second_id, "Second", AllocatorConfig::default());
    second.set_midi_channel(MidiChannelSelection::from_one_indexed(2).unwrap());
    SynthEngine::populate_default_voice_graph(second.voice_graph_mut());
    *second.allocator_mut() = VoiceAllocator::with_graph_template(
        second.allocator().config().clone(),
        second.voice_graph(),
    );
    handle.send(EngineCommand::AddInstrument {
        instrument: Box::new(second),
    });
    engine.process_commands();

    handle.send(EngineCommand::NoteOn {
        note: MidiNote::C4,
        velocity: Velocity::MF,
        channel: MidiChannelSelection::CH1,
        instrument_id: Some(second_id),
    });
    engine.process_commands();
    assert_eq!(engine.instruments[0].active_voice_count(), 0);
    assert_eq!(engine.instruments[1].active_voice_count(), 1);

    handle.send(EngineCommand::NoteOff {
        note: MidiNote::C4,
        channel: MidiChannelSelection::CH1,
        instrument_id: Some(second_id),
    });
    engine.process_commands();
    assert_eq!(engine.instruments[0].active_voice_count(), 0);
}

/// A `SetModScript` for a non-existent instrument (stale id) must NOT drop the
/// unused script `Arc` on the audio thread — it must be parked in the trash
/// channel and freed on the main thread by `cleanup_dropped_modules`.
#[test]
fn set_mod_script_for_missing_instrument_routes_to_trash_not_audio_thread() {
    use synth_core::script::{BoundScript, CompiledScript, Op};

    let (mut engine, mut handle) = SynthEngine::new();
    // No instruments exist, so the instrument lookup in handle_set_mod_script
    // misses and the script is never installed.
    let script = std::sync::Arc::new(BoundScript::new(
        CompiledScript::new(vec![Op::PushConst(0)], vec![0.5], 0, 0),
        Vec::new(),
        "out = 0.5".to_string(),
    ));
    assert!(handle.send(EngineCommand::SetModScript {
        instrument_id: Some(InstrumentId::FIRST), // no such instrument
        module_id: ModuleId::new(ModuleType::ModMatrix, 1),
        slot: 0,
        script: Some(std::sync::Arc::clone(&script)),
        descriptor: None,
    }));
    // Test holds one ref, the queued command holds the other.
    assert_eq!(std::sync::Arc::strong_count(&script), 2);

    // Drain commands as the audio thread does. With the bug, the unused
    // script would drop here (count → 1); the fix parks it in the trash ring.
    engine.process_commands();
    assert_eq!(
        std::sync::Arc::strong_count(&script),
        2,
        "unused script must be parked in the trash channel, not freed on the audio thread"
    );

    // The main thread drains the trash and drops it here.
    handle.cleanup_dropped_modules();
    assert_eq!(std::sync::Arc::strong_count(&script), 1);
}

/// Regression tests for dynamic routing.
///
/// These tests verify that modules are correctly routed to either:
/// - instrument.voice_graph (for polyphonic voice modules like Oscillator, Filter, etc.)
/// - module_graph (for global effects like Reverb, Delay, etc.)
mod dynamic_routing {
    use super::*;
    use crate::commands::{EngineCommand, ModuleId, PortId};
    use crate::instrument::InstrumentId;
    use synth_modules::{Filter, Oscillator};

    /// Test A: Polyphonic Allocation
    /// An Oscillator should be added to instrument's voice_graph, NOT to module_graph.
    #[test]
    fn test_oscillator_routed_to_voice_graph() {
        let (mut engine, mut handle) = SynthEngine::new();
        add_default_instrument(&mut engine, &mut handle);

        // Count existing oscillators in default instrument's voice graph (there are 2 by default)
        let initial_osc_count = engine.instruments[0]
            .voice_graph()
            .module_ids()
            .filter(|id| id.module_type == ModuleType::Oscillator)
            .count();

        // Create a new oscillator
        let osc_id = ModuleId::new(ModuleType::Oscillator, 10);
        let osc = Box::new(Oscillator::new());

        // Send command to add module to the default instrument
        handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(InstrumentId::FIRST),
            id: osc_id,
            module: osc,
        });
        engine.process_commands();

        // Verify: Oscillator should be in instrument's voice_graph
        assert!(
            engine.instruments[0]
                .voice_graph()
                .get_module(osc_id)
                .is_some(),
            "Oscillator should be in instrument's voice_graph"
        );

        // Verify: Oscillator should NOT be in module_graph
        assert!(
            engine.module_graph.get_module(osc_id).is_none(),
            "Oscillator should NOT be in module_graph"
        );

        // Verify: voice_graph oscillator count increased
        let final_osc_count = engine.instruments[0]
            .voice_graph()
            .module_ids()
            .filter(|id| id.module_type == ModuleType::Oscillator)
            .count();
        assert_eq!(final_osc_count, initial_osc_count + 1);
    }

    // Note: Effects (Reverb, Delay, etc.) don't implement PolyModule,
    // so they can't be added via AddModuleInstance. They use the separate
    // effect chain mechanism instead.

    /// Test B: Voice Propagation
    /// Adding a module to instrument's voice_graph should propagate to all its voices.
    #[test]
    fn test_voice_module_propagates_to_voices() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::QUAD,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let (mut engine, mut handle) = SynthEngine::new();
        add_instrument_with_config(&mut engine, &mut handle, config);

        // Create a new filter with a unique ID
        let filter_id = ModuleId::new(ModuleType::Filter, 10);
        let filter = Box::new(Filter::new());

        // First, verify that voices don't have this filter yet
        for voice in engine.instruments[0].allocator().voices() {
            assert!(
                voice.graph.get_module(filter_id).is_none(),
                "Voice should not have filter_id before AddModuleInstance"
            );
        }

        // Send command to add module to default instrument
        handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(InstrumentId::FIRST),
            id: filter_id,
            module: filter,
        });
        engine.process_commands();

        // Verify: All voices in the default instrument should have the new filter
        for (i, voice) in engine.instruments[0]
            .allocator()
            .voices()
            .iter()
            .enumerate()
        {
            assert!(
                voice.graph.get_module(filter_id).is_some(),
                "Voice {} should have filter_id after AddModuleInstance",
                i
            );
        }
    }

    /// Test D: Voice module connections propagate to all voices
    #[test]
    fn test_voice_connection_propagates_to_voices() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::DUAL,
            mode: AllocationMode::Polyphonic,
            ..Default::default()
        };
        let (mut engine, mut handle) = SynthEngine::new();
        add_instrument_with_config(&mut engine, &mut handle, config);

        // Add a new oscillator and amplifier to default instrument's voice graph
        let new_osc_id = ModuleId::new(ModuleType::Oscillator, 10);
        let new_amp_id = ModuleId::new(ModuleType::Amplifier, 10);

        handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(InstrumentId::FIRST),
            id: new_osc_id,
            module: Box::new(Oscillator::new()),
        });
        handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(InstrumentId::FIRST),
            id: new_amp_id,
            module: Box::new(synth_modules::Amplifier::new()),
        });
        engine.process_commands();

        // Connect new osc -> new amp in instrument's voice graph
        handle.send(EngineCommand::Connect {
            instrument_id: Some(InstrumentId::FIRST),
            from: PortId::new(new_osc_id, "out"),
            to: PortId::new(new_amp_id, "in"),
        });
        engine.process_commands();

        // Verify: instrument's voice_graph has the connection
        let template_connections: Vec<_> =
            engine.instruments[0].voice_graph().connections().collect();
        let has_connection = template_connections
            .iter()
            .any(|c| c.from_module == new_osc_id && c.to_module == new_amp_id);
        assert!(
            has_connection,
            "instrument's voice_graph should have the connection"
        );

        // Verify: All voices have the connection
        for (i, voice) in engine.instruments[0]
            .allocator()
            .voices()
            .iter()
            .enumerate()
        {
            let voice_connections: Vec<_> = voice.graph.connections().collect();
            let has_connection = voice_connections
                .iter()
                .any(|c| c.from_module == new_osc_id && c.to_module == new_amp_id);
            assert!(has_connection, "Voice {} should have the connection", i);
        }
    }

    /// Test E: ModuleType classification methods work correctly
    /// (Tests moved to src/engine/params/mod.rs - ModuleType now owns this logic)
    #[test]
    fn test_module_type_classification() {
        // Voice modules (should be true)
        assert!(ModuleType::Oscillator.is_voice_module());
        assert!(ModuleType::Filter.is_voice_module());
        assert!(ModuleType::StereoOutput.is_voice_module());

        // Effects (should be true for is_effect)
        assert!(ModuleType::Delay.is_effect());
        assert!(ModuleType::Reverb.is_effect());

        // Visualizers
        assert!(ModuleType::Oscilloscope.is_visualizer());
        assert!(ModuleType::LevelMeter.is_visualizer());

        // Global = !is_voice_module
        assert!(ModuleType::Delay.is_global());
        assert!(!ModuleType::Oscillator.is_global());
    }

    /// Test F: Remove voice module propagates to voices
    #[test]
    fn test_remove_voice_module_propagates() {
        let config = AllocatorConfig {
            max_voices: VoiceCount::DUAL,
            ..Default::default()
        };
        let (mut engine, mut handle) = SynthEngine::new();
        add_instrument_with_config(&mut engine, &mut handle, config);

        // Add a filter to default instrument
        let filter_id = ModuleId::new(ModuleType::Filter, 10);
        handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(InstrumentId::FIRST),
            id: filter_id,
            module: Box::new(Filter::new()),
        });
        engine.process_commands();

        // Verify it exists
        assert!(
            engine.instruments[0]
                .voice_graph()
                .get_module(filter_id)
                .is_some()
        );
        for voice in engine.instruments[0].allocator().voices() {
            assert!(voice.graph.get_module(filter_id).is_some());
        }

        // Remove it
        handle.send(EngineCommand::RemoveModule {
            instrument_id: Some(InstrumentId::FIRST),
            id: filter_id,
        });
        engine.process_commands();

        // Verify it's gone from instrument's voice_graph
        assert!(
            engine.instruments[0]
                .voice_graph()
                .get_module(filter_id)
                .is_none()
        );

        // Verify it's gone from all voices
        for voice in engine.instruments[0].allocator().voices() {
            assert!(voice.graph.get_module(filter_id).is_none());
        }
    }
}

// --- Orphan-preview lifecycle (channel-strip plan §5) ------------------
//
// The preview target must be cleared on every transport reset; stale
// preview surviving Stop/SetSong/solo was the v0.290.0 bug class. These
// pin the command-handler teardown (previously untested).

fn enter_preview(engine: &mut SynthEngine, handle: &mut EngineHandle) {
    handle.send(EngineCommand::SetPreviewPattern(Some((
        synth_sequencer::PatternId(0),
        synth_sequencer::InstrumentId::new(0),
    ))));
    engine.process_commands();
    assert_eq!(
        engine.sequencer.preview_pattern(),
        Some(synth_sequencer::PatternId(0)),
        "precondition: preview should be active"
    );
}

#[test]
fn stop_clears_orphan_preview() {
    let (mut engine, mut handle) = SynthEngine::new();
    enter_preview(&mut engine, &mut handle);
    handle.send(EngineCommand::Stop);
    engine.process_commands();
    assert_eq!(engine.sequencer.preview_pattern(), None);
}

#[test]
fn set_song_clears_orphan_preview() {
    let (mut engine, mut handle) = SynthEngine::new();
    enter_preview(&mut engine, &mut handle);
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(
            synth_sequencer::Song::default(),
        )),
    });
    engine.process_commands();
    assert_eq!(engine.sequencer.preview_pattern(), None);
}

#[test]
fn solo_pattern_and_preview_are_mutually_exclusive() {
    let (mut engine, mut handle) = SynthEngine::new();
    enter_preview(&mut engine, &mut handle);

    // Entering solo clears preview.
    handle.send(EngineCommand::SetSoloPattern(Some(
        synth_sequencer::PatternId(1),
    )));
    engine.process_commands();
    assert_eq!(engine.sequencer.preview_pattern(), None);
    assert_eq!(
        engine.sequencer.solo_pattern(),
        Some(synth_sequencer::PatternId(1))
    );

    // Entering preview clears solo.
    enter_preview(&mut engine, &mut handle);
    assert_eq!(engine.sequencer.solo_pattern(), None);
}

#[test]
fn mod_grid_track_volume_offset_accumulates() {
    use crate::mod_grid::{ModGridInstance, ModGridRuntime, ModSource, ResolvedTarget};
    use synth_core::{NormalizedValue, SampleCount, SampleRate};
    use synth_sequencer::{
        AutomationTarget, CombineMode, InstrumentId, ModGraphId, Song, TrackId, TrackParam,
    };

    let (mut engine, mut handle) = SynthEngine::new();
    // An instrument on track 0 (InstrumentId::new(0) ↔ InstrumentId::FIRST).
    handle.send(EngineCommand::AddInstrument {
        instrument: Box::new(Instrument::new(InstrumentId::FIRST, "d")),
    });
    // A song with one track at base volume 0.4.
    let mut song = Song::new("t");
    let tid = song.create_track("t");
    song.track_mut(tid).unwrap().volume = NormalizedValue::new(0.4);
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
    });

    // A Constant(1.0) source → this-track Volume (already resolved to track 0)
    // at amount 0.5 → the pre-pass adds 0.5 to track 0's volume offset, which
    // update_track_controls folds onto the 0.4 base (clamped to 0.9).
    let target = ResolvedTarget {
        source: Some(ModSource::Constant(1.0)),
        target: AutomationTarget::Track {
            track: Some(TrackId(0)),
            param: TrackParam::Volume,
        },
        amount: 0.5,
        combine: CombineMode::Add,
        smooth: 0.0,
        dest_addr: None,
    };
    let mut runtime = ModGridRuntime {
        instances: vec![ModGridInstance {
            graph_id: ModGraphId::new(0),
            host_track: Some(TrackId(0)),
            dsp: crate::graph::ModuleGraph::new(),
            injections: Vec::new(),
            targets: vec![target],
        }],
        ..Default::default()
    };
    runtime.prekey_offsets();
    handle.send(EngineCommand::SetModGrid {
        runtime: Box::new(runtime),
    });
    engine.process_commands();

    let ctx = ProcessContext {
        samples: SampleCount::new(256),
        sample_rate: SampleRate::DVD_QUALITY,
        ..ProcessContext::default()
    };
    engine.process_mod_grid(&ctx);

    // Write path: the offset accumulated.
    let off = engine
        .mod_grid
        .track_offsets
        .get(&TrackId(0))
        .copied()
        .unwrap_or_default();
    assert!(
        (off.volume.as_f32() - 0.5).abs() < 1e-6,
        "expected track-0 volume offset 0.5, got {}",
        off.volume
    );

    // Composition path: update_track_controls folds it onto the base fader.
    engine.update_track_controls();
    let ctrl = TrackControlSnapshot {
        slots: &engine.track_controls,
        generation: engine.track_control_generation,
    }
    .get(Some(TrackId(0)));
    assert!(
        (ctrl.volume.as_f32() - 0.9).abs() < 1e-4,
        "expected composed track fader 0.9 (0.4 + 0.5), got {}",
        ctrl.volume.as_f32()
    );

    // Full-flow path: drive the real process() and re-check the fader. This
    // is what the live engine / offline render actually run.
    let context = AudioCallbackContext {
        sample_rate: synth_core::audio::DeviceSampleRate::new(48000),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: Seconds::ZERO,
    };
    let mut out = vec![0.0f32; 256 * 2];
    engine.process(&mut out, &context);
    let ctrl2 = TrackControlSnapshot {
        slots: &engine.track_controls,
        generation: engine.track_control_generation,
    }
    .get(Some(TrackId(0)));
    assert!(
        (ctrl2.volume.as_f32() - 0.9).abs() < 1e-4,
        "after process(): expected composed track fader 0.9, got {}",
        ctrl2.volume.as_f32()
    );
}

#[test]
fn mod_grid_instrument_volume_offset_is_order_independent() {
    use crate::mod_grid::{ModGridInstance, ModGridRuntime, ModSource, ResolvedTarget};
    use synth_core::{SampleCount, SampleRate};
    use synth_sequencer::{
        AutoInstrumentParam, AutomationTarget, CombineMode, InstrumentId, ModGraphId,
    };

    // A Constant(1.0) → Instrument 0 Volume runtime at amount 0.5, with its
    // offset slot pre-keyed by InstrumentId off the audio thread (as the
    // builder does). Because the slot needs no engine mapping, SetModGrid works
    // regardless of whether the instrument is loaded first — the old
    // offline-export ordering trap is gone.
    let make_runtime = || {
        let mut rt = ModGridRuntime {
            instances: vec![ModGridInstance {
                graph_id: ModGraphId::new(0),
                host_track: None,
                dsp: crate::graph::ModuleGraph::new(),
                injections: Vec::new(),
                targets: vec![ResolvedTarget {
                    source: Some(ModSource::Constant(1.0)),
                    target: AutomationTarget::Instrument {
                        instrument: InstrumentId::new(0),
                        param: AutoInstrumentParam::Volume,
                    },
                    amount: 0.5,
                    combine: CombineMode::Add,
                    smooth: 0.0,
                    dest_addr: None,
                }],
            }],
            ..Default::default()
        };
        rt.prekey_offsets();
        Box::new(rt)
    };
    let ctx = ProcessContext {
        samples: SampleCount::new(256),
        sample_rate: SampleRate::DVD_QUALITY,
        ..ProcessContext::default()
    };
    let written = |engine: &SynthEngine| -> f32 {
        engine
            .mod_grid
            .instrument_offsets
            .get(&InstrumentId::new(0))
            .copied()
            .unwrap_or_default()
            .volume
            .as_f32()
    };

    // SetModGrid *before* the instrument (the former trap) — still writes 0.5.
    let (mut e1, mut h1) = SynthEngine::new();
    h1.send(EngineCommand::SetModGrid {
        runtime: make_runtime(),
    });
    h1.send(EngineCommand::AddInstrument {
        instrument: Box::new(Instrument::new(InstrumentId::FIRST, "d")),
    });
    e1.process_commands();
    e1.process_mod_grid(&ctx);
    assert!(
        (written(&e1) - 0.5).abs() < 1e-6,
        "SetModGrid-before-instrument must still write the offset, got {}",
        written(&e1)
    );

    // Instrument first (what export.rs now does) — also writes 0.5.
    let (mut e2, mut h2) = SynthEngine::new();
    h2.send(EngineCommand::AddInstrument {
        instrument: Box::new(Instrument::new(InstrumentId::FIRST, "d")),
    });
    h2.send(EngineCommand::SetModGrid {
        runtime: make_runtime(),
    });
    e2.process_commands();
    e2.process_mod_grid(&ctx);
    assert!(
        (written(&e2) - 0.5).abs() < 1e-6,
        "instrument-first must write the offset, got {}",
        written(&e2)
    );
}

#[test]
fn mod_grid_midi_cc_source_reads_live_cc_state() {
    use crate::instrument::MidiChannelSelection;
    use crate::mod_grid::{ModGridInstance, ModGridRuntime, ModSource, ResolvedTarget};
    use synth_core::{NormalizedValue, SampleCount, SampleRate};
    use synth_sequencer::{AutomationTarget, CombineMode, GlobalParam, ModGraphId};

    let (mut engine, mut handle) = SynthEngine::new();
    // A grid MidiCc source (CC74, channel 0) → master volume at amount 0.5.
    let target = ResolvedTarget {
        source: Some(ModSource::MidiCc {
            cc: 74,
            channel: Some(0),
        }),
        target: AutomationTarget::Global(GlobalParam::MasterVolume),
        amount: 0.5,
        combine: CombineMode::Add,
        smooth: 0.0,
        dest_addr: None,
    };
    handle.send(EngineCommand::SetModGrid {
        runtime: Box::new(ModGridRuntime {
            instances: vec![ModGridInstance {
                graph_id: ModGraphId::new(0),
                host_track: None,
                dsp: crate::graph::ModuleGraph::new(),
                injections: Vec::new(),
                targets: vec![target],
            }],
            ..Default::default()
        }),
    });
    // A live CC message on channel 1 (zero-indexed 0), full value.
    handle.send(EngineCommand::ControlChange {
        channel: MidiChannelSelection::from_zero_indexed(0).expect("valid channel"),
        cc: 74,
        value: NormalizedValue::new(1.0),
    });
    engine.process_commands();

    let ctx = ProcessContext {
        samples: SampleCount::new(256),
        sample_rate: SampleRate::DVD_QUALITY,
        ..ProcessContext::default()
    };
    engine.process_mod_grid(&ctx);
    // cv (1.0) × amount (0.5) → the master-volume offset.
    assert!(
        (engine.grid_master_volume_offset - 0.5).abs() < 1e-6,
        "expected master offset 0.5 from CC74=1.0, got {}",
        engine.grid_master_volume_offset
    );
}

#[test]
fn sustain_pedal_holds_note_off_and_releases_on_lift() {
    use crate::instrument::MidiChannelSelection;
    use synth_core::{MidiNote, NormalizedValue, Velocity};

    let (mut engine, mut handle) = SynthEngine::new();
    let ch = MidiChannelSelection::from_zero_indexed(0).expect("valid channel");
    let note = MidiNote::new(60);
    let cc64 = |v: f32| EngineCommand::ControlChange {
        channel: ch,
        cc: 64,
        value: NormalizedValue::new(v),
    };

    // Pedal down, then play + release the key.
    handle.send(cc64(1.0));
    handle.send(EngineCommand::NoteOn {
        note,
        velocity: Velocity::MF,
        channel: ch,
        instrument_id: None,
    });
    handle.send(EngineCommand::NoteOff {
        note,
        channel: ch,
        instrument_id: None,
    });
    engine.process_commands();
    assert!(engine.sustain_pedal_down[0], "pedal recorded as down");
    assert!(
        engine.sustained_notes[0][usize::from(note.as_u8())],
        "the NoteOff is deferred while the pedal is held"
    );

    // Lifting the pedal releases the held note.
    handle.send(cc64(0.0));
    engine.process_commands();
    assert!(!engine.sustain_pedal_down[0]);
    assert!(
        !engine.sustained_notes[0][usize::from(note.as_u8())],
        "lifting the pedal releases every held note"
    );
}

#[test]
fn sustain_pedal_repress_reclaims_the_held_note() {
    use crate::instrument::MidiChannelSelection;
    use synth_core::{MidiNote, NormalizedValue, Velocity};

    let (mut engine, mut handle) = SynthEngine::new();
    let ch = MidiChannelSelection::from_zero_indexed(0).expect("valid channel");
    let note = MidiNote::new(60);

    handle.send(EngineCommand::ControlChange {
        channel: ch,
        cc: 64,
        value: NormalizedValue::new(1.0),
    });
    handle.send(EngineCommand::NoteOn {
        note,
        velocity: Velocity::MF,
        channel: ch,
        instrument_id: None,
    });
    handle.send(EngineCommand::NoteOff {
        note,
        channel: ch,
        instrument_id: None,
    });
    engine.process_commands();
    assert!(engine.sustained_notes[0][usize::from(note.as_u8())]);

    // Re-striking the same key reclaims it, so a later pedal-up won't cut it.
    handle.send(EngineCommand::NoteOn {
        note,
        velocity: Velocity::MF,
        channel: ch,
        instrument_id: None,
    });
    engine.process_commands();
    assert!(
        !engine.sustained_notes[0][usize::from(note.as_u8())],
        "a re-press must reclaim the pedal-held note"
    );
}

#[test]
fn filter_cutoff_automation_dispatches_through_override() {
    use synth_core::{SampleCount, SampleRate};
    use synth_modules::{Filter, Oscillator};
    use synth_sequencer::{InstrumentId, Tick};

    // Instrument graph: Osc -> Filter (sink). No voices allocated, so the
    // override lands on the template graph that we process directly.
    let mut instrument = Box::new(Instrument::new(InstrumentId::new(1), "test"));
    let g = instrument.voice_graph_mut();
    let osc_id = g.add_module(Box::new(Oscillator::new()));
    let flt_id = g.add_module(Box::new(Filter::new()));
    g.connect(osc_id, "out", flt_id, "in").unwrap();
    let mut instruments = vec![instrument];

    let note_rb = HeapRb::<NoteEvent>::new(16);
    let (mut note_prod, _note_cons) = note_rb.split();
    let drops = std::sync::atomic::AtomicU32::new(0);

    let ctx = ProcessContext {
        samples: SampleCount::new(256),
        sample_rate: SampleRate::DVD_QUALITY,
        ..ProcessContext::default()
    };
    // Warm-up blocks first so the filter reaches steady state before
    // measuring (avoids start/retune transients dominating the energy).
    fn settled_energy(graph: &mut ModuleGraph, ctx: &ProcessContext<'_>) -> f32 {
        let mut out = AudioBuffer::new(256);
        for _ in 0..16 {
            graph.process(&mut out, ctx);
        }
        graph.process(&mut out, ctx);
        (0..256).map(|i| out[i] * out[i]).sum()
    }

    let base = settled_energy(instruments[0].voice_graph_mut(), &ctx);
    assert!(base > 1e-3, "expected audible base output, got {base}");

    // FilterCutoff automation at 0.0 -> 20 Hz cutoff via the dispatch path.
    let events = vec![SequencerEvent::Parameter {
        tick: Tick(0),
        target: AutomationTarget::Instrument {
            instrument: InstrumentId::new(1),
            param: AutoInstrumentParam::FilterCutoff,
        },
        value: NormalizedValue::MIN,
    }];
    route_sequencer_events(&events, &mut instruments, &mut note_prod, &drops);

    let low = settled_energy(instruments[0].voice_graph_mut(), &ctx);
    assert!(
        low < base * 0.25,
        "automation should have lowered the cutoff: {low} vs base {base}"
    );
}

#[test]
fn module_automation_target_dispatches_through_override() {
    use synth_core::{SampleCount, SampleRate};
    use synth_modules::{Filter, Oscillator};
    use synth_sequencer::{InstrumentId, Tick};

    // Osc -> Filter (sink); the filter is ModuleId(Filter, instance 1).
    let mut instrument = Box::new(Instrument::new(InstrumentId::new(1), "test"));
    let g = instrument.voice_graph_mut();
    let osc_id = g.add_module(Box::new(Oscillator::new()));
    let flt_id = g.add_module(Box::new(Filter::new()));
    g.connect(osc_id, "out", flt_id, "in").unwrap();
    let mut instruments = vec![instrument];

    let note_rb = HeapRb::<NoteEvent>::new(16);
    let (mut note_prod, _note_cons) = note_rb.split();
    let drops = std::sync::atomic::AtomicU32::new(0);

    let ctx = ProcessContext {
        samples: SampleCount::new(256),
        sample_rate: SampleRate::DVD_QUALITY,
        ..ProcessContext::default()
    };
    fn settled_energy(graph: &mut ModuleGraph, ctx: &ProcessContext<'_>) -> f32 {
        let mut out = AudioBuffer::new(256);
        for _ in 0..16 {
            graph.process(&mut out, ctx);
        }
        graph.process(&mut out, ctx);
        (0..256).map(|i| out[i] * out[i]).sum()
    }

    let base = settled_energy(instruments[0].voice_graph_mut(), &ctx);
    assert!(base > 1e-3, "expected audible base output, got {base}");

    // Generic Module target: first Filter, "cutoff" param, 0.0 -> 20 Hz.
    let events = vec![SequencerEvent::Parameter {
        tick: Tick(0),
        target: AutomationTarget::Module {
            instrument: InstrumentId::new(1),
            module_type: ModuleType::Filter,
            instance: 1,
            param_id: "cutoff".into(),
        },
        value: NormalizedValue::MIN,
    }];
    route_sequencer_events(&events, &mut instruments, &mut note_prod, &drops);

    let low = settled_energy(instruments[0].voice_graph_mut(), &ctx);
    assert!(
        low < base * 0.25,
        "module automation should have lowered the cutoff: {low} vs base {base}"
    );
}

#[test]
fn resolve_instrument_index_matches_full_u64_id() {
    // Ids past u16::MAX must resolve exactly. The old SeqInstrumentId(u16)
    // truncation would have aliased 65536 onto 0 and misrouted the note.
    let big = InstrumentId::new(u64::from(u16::MAX) + 1); // 65536
    let instruments = vec![
        Box::new(Instrument::new(InstrumentId::new(5), "a")),
        Box::new(Instrument::new(big, "b")),
    ];
    assert_eq!(resolve_instrument_index(&big, &instruments), Some(1));
    assert_eq!(
        resolve_instrument_index(&InstrumentId::new(5), &instruments),
        Some(0)
    );
}

#[test]
fn resolve_instrument_index_falls_back_for_orphan_reference() {
    // A note naming a removed / never-existent instrument routes to the
    // first instrument (orphaned-note fallback) rather than being dropped.
    let instruments = vec![Box::new(Instrument::new(InstrumentId::new(3), "only"))];
    assert_eq!(
        resolve_instrument_index(&InstrumentId::new(999), &instruments),
        Some(0)
    );
    // With no instruments at all there is nothing to route to.
    let empty: Vec<Box<Instrument>> = Vec::new();
    assert_eq!(
        resolve_instrument_index(&InstrumentId::new(0), &empty),
        None
    );
}

// --- Sends / returns (Phase 7) ------------------------------------------

use synth_sequencer::{ReturnBusId, Song, TrackSend};

#[test]
fn apply_send_tap_post_fader_scales_by_channel_and_send() {
    // 2 frames; post-fader multiplies by the channel gains AND the level.
    let src = [0.2, 0.4, 0.2, 0.4];
    let send = ChannelSend {
        return_index: 0,
        level: 0.5,
        pre_fader: false,
    };
    let mut dst = [0.0f32; 4];
    apply_send_tap(&src, 0.5, 0.25, send, &mut dst);
    assert!((dst[0] - 0.2 * 0.5 * 0.5).abs() < 1e-6);
    assert!((dst[1] - 0.4 * 0.25 * 0.5).abs() < 1e-6);
    assert!((dst[2] - 0.2 * 0.5 * 0.5).abs() < 1e-6);
    assert!((dst[3] - 0.4 * 0.25 * 0.5).abs() < 1e-6);
}

#[test]
fn apply_send_tap_pre_fader_ignores_channel_gains_and_accumulates() {
    let src = [0.2, 0.4, 0.2, 0.4];
    let send = ChannelSend {
        return_index: 0,
        level: 0.5,
        pre_fader: true,
    };
    let mut dst = [0.0f32; 4];
    // Pre-fader: the 0.5/0.25 channel gains must be ignored.
    apply_send_tap(&src, 0.5, 0.25, send, &mut dst);
    assert!((dst[0] - 0.2 * 0.5).abs() < 1e-6);
    assert!((dst[1] - 0.4 * 0.5).abs() < 1e-6);
    // A second tap accumulates (+=) into the same return buffer.
    apply_send_tap(&src, 1.0, 1.0, send, &mut dst);
    assert!((dst[0] - 2.0 * 0.2 * 0.5).abs() < 1e-6);
}

#[test]
fn apply_send_tap_clamps_to_shorter_buffer() {
    let src = [0.2, 0.4, 0.2, 0.4];
    let send = ChannelSend {
        return_index: 0,
        level: 1.0,
        pre_fader: true,
    };
    let mut dst = [0.0f32; 2]; // shorter than src — must not panic
    apply_send_tap(&src, 1.0, 1.0, send, &mut dst);
    assert!((dst[0] - 0.2).abs() < 1e-6);
    assert!((dst[1] - 0.4).abs() < 1e-6);
}

#[test]
fn return_bus_create_remove_and_fader_synced_from_song() {
    let (mut engine, mut handle) = SynthEngine::new();
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(0) });
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(1) });
    engine.process_commands();
    assert_eq!(engine.return_busses.len(), 2);

    // Re-using an id is a no-op (load idempotence).
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(0) });
    engine.process_commands();
    assert_eq!(engine.return_busses.len(), 2);

    // The fader is owned by the song; the engine snapshots it each block.
    let mut song = Song::default();
    let id = song.create_return_bus("Reverb"); // → ReturnBusId(0)
    assert_eq!(id, ReturnBusId(0));
    let def = song.return_bus_mut(id).unwrap();
    def.volume = NormalizedValue::new(0.3);
    def.pan = BipolarValue::new(-0.5);
    def.mute = true;
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
    });
    engine.process_commands();
    engine.update_track_controls();
    let bus = engine
        .return_busses
        .iter()
        .find(|b| b.id() == ReturnBusId(0))
        .unwrap();
    assert!((bus.volume().as_f32() - 0.3).abs() < 1e-6);
    assert!((bus.pan().as_f32() - (-0.5)).abs() < 1e-6);
    assert!(bus.is_muted());

    handle.send(EngineCommand::RemoveReturnBus { id: ReturnBusId(0) });
    engine.process_commands();
    assert_eq!(engine.return_busses.len(), 1);
    assert_eq!(engine.return_busses[0].id(), ReturnBusId(1));
}

#[test]
fn bus_to_bus_send_resolves_and_orders_source_before_target() {
    let (mut engine, mut handle) = SynthEngine::new();
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(0) });
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(1) });
    engine.process_commands();

    // Song: return 0 ("Delay") feeds return 1 ("Reverb").
    let mut song = Song::default();
    let a = song.create_return_bus("Delay"); // → ReturnBusId(0)
    let b = song.create_return_bus("Reverb"); // → ReturnBusId(1)
    // Before the edge exists, b -> a is not yet a cycle.
    assert!(!song.return_send_would_cycle(b, a));
    song.return_bus_mut(a)
        .unwrap()
        .sends
        .push(synth_sequencer::ReturnSend::new(
            b,
            NormalizedValue::new(0.5),
        ));
    // Now b -> a would close a loop.
    assert!(song.return_send_would_cycle(b, a));
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
    });
    engine.process_commands();
    engine.update_track_controls();
    engine.resolve_return_routing();

    let ia = engine.return_index[&a];
    let ib = engine.return_index[&b];
    assert_eq!(engine.return_sends[ia].len(), 1, "source has one send");
    assert_eq!(engine.return_sends[ia][0].target_index, ib);
    assert!(engine.return_sends[ib].is_empty(), "target has no send");
    let pos = |idx: usize| engine.return_order.iter().position(|&x| x == idx).unwrap();
    assert!(
        pos(ia) < pos(ib),
        "the source return must be processed before its target"
    );

    // Disabling the send drops it from the resolved routing.
    let mut song = Song::default();
    let _ = song.create_return_bus("Delay");
    let _ = song.create_return_bus("Reverb");
    song.return_bus_mut(a)
        .unwrap()
        .sends
        .push(synth_sequencer::ReturnSend {
            target: b,
            level: NormalizedValue::new(0.5),
            enabled: false,
        });
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
    });
    engine.process_commands();
    engine.update_track_controls();
    engine.resolve_return_routing();
    assert!(
        engine.return_sends[ia].is_empty(),
        "a disabled bus-to-bus send must not resolve"
    );
}

#[test]
fn update_track_controls_resolves_sends_and_drops_missing() {
    let (mut engine, mut handle) = SynthEngine::new();
    add_default_instrument(&mut engine, &mut handle); // FIRST ↔ InstrumentId::new(0)
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(5) });
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(9) });

    let mut song = Song::default();
    let tid = song.create_track("t"); // default instrument = InstrumentId::new(0)
    let track = song.track_mut(tid).unwrap();
    track.sends.push(TrackSend {
        target: ReturnBusId(9),
        level: NormalizedValue::new(0.5),
        pre_fader: true,
        enabled: true,
    });
    // A send to a non-existent return bus must be dropped, not panic.
    track.sends.push(TrackSend {
        target: ReturnBusId(99),
        level: NormalizedValue::MAX,
        pre_fader: false,
        enabled: true,
    });
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
    });
    engine.process_commands();
    engine.update_track_controls();

    let sends = &engine.channel_sends[&InstrumentId::FIRST];
    assert_eq!(sends.len(), 1, "the missing-target send must be dropped");
    // ReturnBusId(9) was created second → index 1.
    assert_eq!(sends[0].return_index, 1);
    assert!((sends[0].level - 0.5).abs() < 1e-6);
    assert!(sends[0].pre_fader);
}

#[test]
fn shared_instrument_send_list_does_not_carry_over_between_tracks() {
    // Two tracks share instrument 0; the first sends to a return, the second
    // (which wins for the block) has none. The shared channel must end with
    // NO sends — the first track's send must not leak into it.
    let (mut engine, mut handle) = SynthEngine::new();
    add_default_instrument(&mut engine, &mut handle); // FIRST ↔ InstrumentId::new(0)
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(0) });

    let mut song = Song::default();
    let with_send = song.create_track("with_send"); // instrument 0
    song.track_mut(with_send)
        .unwrap()
        .sends
        .push(TrackSend::new(ReturnBusId(0), NormalizedValue::MAX));
    let _no_send = song.create_track("no_send"); // also instrument 0, later → wins
    handle.send(EngineCommand::SetSong {
        song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
    });
    engine.process_commands();
    engine.update_track_controls();

    assert!(
        engine.channel_sends[&InstrumentId::FIRST].is_empty(),
        "a no-send track sharing an instrument must clear the carried-over send"
    );
}

/// Render `blocks` callbacks of a sustained C4 and return the total output
/// energy. `with_send` adds a unity return bus and routes a full post-fader
/// send to it via the song.
fn render_send_energy(with_send: bool) -> f32 {
    let (mut engine, mut handle) = SynthEngine::new();
    add_default_instrument(&mut engine, &mut handle);
    if with_send {
        handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(0) });
        let mut song = Song::default();
        let tid = song.create_track("t");
        let track = song.track_mut(tid).unwrap();
        track.sends.push(TrackSend {
            target: ReturnBusId(0),
            level: NormalizedValue::MAX,
            pre_fader: false,
            enabled: true,
        });
        handle.send(EngineCommand::SetSong {
            song: std::sync::Arc::new(synth_sequencer::SharedSong::new(song)),
        });
    }
    // Per-module RNG state makes oscillator start phase deterministic.
    handle.note_on(MidiNote::C4, Velocity::new(0.8));
    engine.process_commands();

    let context = AudioCallbackContext {
        sample_rate: synth_core::audio::DeviceSampleRate::new(48000),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: Seconds::ZERO,
    };
    let mut out = vec![0.0f32; 256 * 2];
    let mut energy = 0.0f32;
    for _ in 0..32 {
        out.fill(0.0);
        engine.process(&mut out, &context);
        energy += out.iter().map(|s| s * s).sum::<f32>();
    }
    energy
}

#[test]
fn return_effect_commands_add_and_remove_from_chain() {
    let (mut engine, mut handle) = SynthEngine::new();
    handle.send(EngineCommand::CreateReturnBus { id: ReturnBusId(0) });
    engine.process_commands();

    let effect_slots = |engine: &SynthEngine| -> usize {
        engine.return_busses[0]
            .effect_chain()
            .slots()
            .iter()
            .filter(|s| matches!(s, crate::effect_chain::ChainSlot::Effect(_)))
            .count()
    };
    assert_eq!(effect_slots(&engine), 0);

    handle.send(EngineCommand::AddReturnEffect {
        return_id: ReturnBusId(0),
        id: ModuleId::new(ModuleType::Distortion, 1),
        effect: Box::new(synth_modules::Distortion::new()),
    });
    engine.process_commands();
    assert_eq!(
        effect_slots(&engine),
        1,
        "effect should be added to the return chain"
    );

    handle.send(EngineCommand::RemoveReturnEffect {
        return_id: ReturnBusId(0),
        id: ModuleId::new(ModuleType::Distortion, 1),
    });
    engine.process_commands();
    assert_eq!(
        effect_slots(&engine),
        0,
        "effect should be removed from the return chain"
    );
}

#[test]
fn send_routes_channel_signal_into_return_and_back_to_master() {
    let baseline = render_send_energy(false);
    let with_send = render_send_energy(true);
    assert!(baseline > 1e-4, "baseline render should be audible");
    assert!(
        with_send > baseline * 1.3,
        "a unity post-fader send through an (empty-chain) return bus must add \
         wet energy to the master mix (baseline={baseline}, with_send={with_send})"
    );
}

/// Glide is set globally and applied per instrument, so an instrument created
/// *after* the global value was set has to inherit it.
///
/// It did not. `SetGlideTime` walked the instruments that existed at the time
/// and stopped there, so an instrument added later kept the zero its allocator
/// config was built with — while the engine's published global still read the
/// value the user chose. Nothing reconciled the two, and every offline render
/// reads the published global, so the same instrument glided offline and jumped
/// live.
///
/// Project loading never showed it: that path sends the glide after the
/// instruments. It takes an instrument created during a session — from the GUI,
/// from MCP, from anything that adds one to a running engine.
#[test]
fn an_instrument_added_after_a_global_glide_inherits_it() {
    let (mut engine, mut handle) = SynthEngine::new();

    handle.send(EngineCommand::SetGlideTime(Seconds::new(0.75)));
    engine.process_commands();
    add_default_instrument(&mut engine, &mut handle);

    let published = engine.state.glide_time.load();
    let on_instrument = engine.instruments[0]
        .allocator()
        .config()
        .glide_time
        .as_f32();
    assert!(
        (published - 0.75).abs() < 1e-6,
        "the engine should publish the glide it was given, got {published}"
    );
    assert!(
        (on_instrument - published).abs() < 1e-6,
        "an instrument added after the global glide was set holds {on_instrument} \
         while the engine publishes {published} — the offline renderer reads the \
         published value, so the two would disagree"
    );
}

/// The other order, which already worked, kept as the pair: setting the glide
/// after the instrument exists must still reach it.
#[test]
fn a_global_glide_reaches_an_instrument_that_already_exists() {
    let (mut engine, mut handle) = SynthEngine::new();

    add_default_instrument(&mut engine, &mut handle);
    handle.send(EngineCommand::SetGlideTime(Seconds::new(0.4)));
    engine.process_commands();

    let on_instrument = engine.instruments[0]
        .allocator()
        .config()
        .glide_time
        .as_f32();
    assert!((on_instrument - 0.4).abs() < 1e-6, "got {on_instrument}");
}

/// Fill the engine-to-GUI event ring so the next `try_push` fails.
fn fill_event_ring(engine: &mut SynthEngine) {
    // `EVENT_BUFFER_SIZE` slots; push until the producer refuses.
    for _ in 0..(EVENT_BUFFER_SIZE + 8) {
        if engine
            .event_producer
            .try_push(EngineEvent::NoteReleased {
                note: MidiNote::new(60),
                channel: MidiChannelSelection::CH1,
            })
            .is_err()
        {
            return;
        }
    }
    panic!("event ring never filled");
}

fn one_recorded_note() -> Vec<crate::recording::RecordedNote> {
    vec![crate::recording::RecordedNote {
        pitch: synth_sequencer::Pitch::MIDDLE_C,
        velocity: synth_sequencer::Velocity::new(0.8),
        start: synth_sequencer::PatternTick(0),
        duration: synth_sequencer::Duration::QUARTER,
    }]
}

/// With the event ring full, the first failed flush parks in the retry slot.
#[test]
fn a_failed_recorded_note_flush_parks_for_retry() {
    let (mut engine, _handle) = SynthEngine::new();
    fill_event_ring(&mut engine);

    engine.flush_recorded_notes(
        synth_sequencer::PatternId::new(1),
        one_recorded_note(),
        false,
    );

    assert!(
        engine.pending_recorded_notes.is_some(),
        "the first failure must be kept for retry, not lost"
    );
    assert_eq!(
        engine
            .state
            .refused_recorded_note_flushes
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
}

/// A second failed flush is **refused and counted**, never written over the
/// first. Overwriting would destroy notes the user already played and drop
/// their `Vec` on the audio thread; this is the regression test for that.
#[test]
fn a_second_failed_flush_is_refused_not_overwritten() {
    let (mut engine, _handle) = SynthEngine::new();
    fill_event_ring(&mut engine);

    engine.flush_recorded_notes(
        synth_sequencer::PatternId::new(1),
        one_recorded_note(),
        false,
    );
    let first = engine
        .pending_recorded_notes
        .as_ref()
        .map(|(id, notes, _)| (*id, notes.len()))
        .expect("first flush parked");

    engine.flush_recorded_notes(
        synth_sequencer::PatternId::new(1),
        one_recorded_note(),
        true,
    );

    let still_there = engine
        .pending_recorded_notes
        .as_ref()
        .map(|(id, notes, overdub)| (*id, notes.len(), *overdub))
        .expect("the parked flush must survive a second failure");
    assert_eq!(
        (still_there.0, still_there.1),
        first,
        "the earlier take must be the one kept"
    );
    assert!(
        !still_there.2,
        "the slot must still hold the FIRST flush, not the second's overdub flag"
    );
    assert_eq!(
        engine
            .state
            .refused_recorded_note_flushes
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "the refusal must be counted in shared state, where the UI can read it"
    );
}

/// A device reporting more than two channels gets silence past the second, not
/// whatever was in the buffer.
///
/// The mix is stereo. Before this was fixed the output loop wrote frame[0] and
/// frame[1] and left the rest of each frame alone, so on a 6-channel device the
/// surplus channels could carry undefined content. The engine still silences
/// them explicitly; CPAL 0.18.2's pre-filled-silence guarantee is additional
/// protection. The test seeds the buffer with a value no render would produce,
/// so an untouched channel is unmistakable.
#[test]
fn surplus_output_channels_are_silenced() {
    let (mut engine, _handle) = SynthEngine::new();

    const CHANNELS: usize = 6;
    const FRAMES: usize = 64;
    let context = AudioCallbackContext {
        sample_rate: synth_core::audio::DeviceSampleRate::new(48000),
        frames: FRAMES,
        channels: CHANNELS as u16,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: Seconds::ZERO,
    };

    let mut out = vec![f32::from(-7i8); FRAMES * CHANNELS];
    engine.process(&mut out, &context);

    for (index, frame) in out.chunks(CHANNELS).enumerate() {
        for (channel, sample) in frame.iter().enumerate().skip(2) {
            assert_eq!(
                *sample, 0.0,
                "frame {index} channel {channel} was left at {sample}, not silenced"
            );
        }
    }
}
