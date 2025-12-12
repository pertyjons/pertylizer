//! Integration tests for debug tools.
//!
//! These tests demonstrate real-world usage of the debug system
//! with actual modules and voice graphs.

#![allow(clippy::unwrap_used)]

use crate::debug::{GraphDebugger, SequencerDebugger, SignalProbe, VoiceDebugger};
use crate::engine::typed_params::{EnvelopeParam, ModuleType, OscillatorParam, Param};
use crate::engine::{ModuleGraph, ModuleId};
use crate::modules::{
    Amplifier, AudioBuffer, Envelope, InputPorts, Oscillator, PolyModule, ProcessContext,
    StereoOutput,
};
use crate::sequencer::{Duration, PatternTick, Pitch, SeqInstrumentId, Song, Tick, Velocity};
use crate::types::{BeatPosition, Bpm, Hertz, MidiNote, PortName, SampleCount, SampleRate};
use std::collections::HashMap;

fn make_context(samples: usize) -> ProcessContext {
    ProcessContext {
        sample_rate: SampleRate::DVD_QUALITY,
        samples: SampleCount::new(samples),
        tempo: Bpm::DEFAULT,
        is_playing: false,
        position_beats: BeatPosition::ZERO,
    }
}

/// Create a simple voice graph: Oscillator → Amplifier → StereoOutput
fn create_simple_voice_graph() -> ModuleGraph {
    let mut graph = ModuleGraph::new();

    let osc = Oscillator::new();
    let amp = Amplifier::new();
    let output = StereoOutput::new();

    let osc_id = graph.add_module(Box::new(osc));
    let amp_id = graph.add_module(Box::new(amp));
    let out_id = graph.add_module(Box::new(output));

    // Connect: osc.out → amp.in, amp.out → output.in (mono input)
    graph.connect(osc_id, "out", amp_id, "in").unwrap();
    graph.connect(amp_id, "out", out_id, "in").unwrap();

    graph
}

/// Create a voice graph with envelope: Oscillator → Amplifier ← Envelope → StereoOutput
fn create_voice_graph_with_envelope() -> ModuleGraph {
    let mut graph = ModuleGraph::new();

    let osc = Oscillator::new();
    let env = Envelope::new();
    let amp = Amplifier::new();
    let output = StereoOutput::new();

    let osc_id = graph.add_module(Box::new(osc));
    let env_id = graph.add_module(Box::new(env));
    let amp_id = graph.add_module(Box::new(amp));
    let out_id = graph.add_module(Box::new(output));

    // Audio path: osc → amp → output (mono input)
    graph.connect(osc_id, "out", amp_id, "in").unwrap();
    graph.connect(amp_id, "out", out_id, "in").unwrap();

    // CV path: env → amp.cv
    graph.connect(env_id, "out", amp_id, "cv").unwrap();

    graph
}

// =============================================================================
// GraphDebugger Tests
// =============================================================================

#[test]
fn test_graph_debugger_path_verification() {
    let graph = create_simple_voice_graph();
    let dbg = GraphDebugger::new(&graph);

    // Find modules
    // Note: StereoOutput returns ModuleType::Mixer from module_type() for compatibility
    let osc_id = graph.find_module_by_type(ModuleType::Oscillator).unwrap();
    let amp_id = graph.find_module_by_type(ModuleType::Amplifier).unwrap();
    let out_id = graph.find_module_by_type(ModuleType::Mixer).unwrap();

    // Verify paths exist
    assert!(
        dbg.path_exists(osc_id, amp_id),
        "osc → amp path should exist"
    );
    assert!(
        dbg.path_exists(amp_id, out_id),
        "amp → out path should exist"
    );
    assert!(
        dbg.path_exists(osc_id, out_id),
        "osc → out path should exist"
    );

    // Verify reverse path doesn't exist
    assert!(
        !dbg.path_exists(out_id, osc_id),
        "reverse path should not exist"
    );
}

#[test]
fn test_graph_debugger_diagnose_missing_cv() {
    let graph = create_simple_voice_graph();
    let dbg = GraphDebugger::new(&graph);

    let issues = dbg.diagnose();

    // Should warn about amplifier without CV input
    let has_cv_warning = issues
        .iter()
        .any(|issue| issue.contains("Amplifier") && issue.contains("CV"));
    assert!(has_cv_warning, "Should warn about missing CV connection");
}

#[test]
fn test_graph_debugger_diagnose_with_envelope() {
    let graph = create_voice_graph_with_envelope();
    let dbg = GraphDebugger::new(&graph);

    let issues = dbg.diagnose();

    // Should NOT warn about missing CV (envelope is connected)
    let has_cv_warning = issues
        .iter()
        .any(|issue| issue.contains("Amplifier") && issue.contains("CV"));
    assert!(
        !has_cv_warning,
        "Should not warn about CV when envelope is connected"
    );

    // Should warn about envelope without gate
    let has_gate_warning = issues.iter().any(|issue| issue.contains("gate"));
    assert!(
        has_gate_warning,
        "Should warn about missing gate connection"
    );
}

#[test]
fn test_graph_debugger_connections_list() {
    let graph = create_simple_voice_graph();
    let dbg = GraphDebugger::new(&graph);

    let connections = dbg.connections();

    // Should have 2 connections: osc→amp, amp→output
    assert_eq!(connections.len(), 2, "Should have 2 connections");

    // Verify connection details
    let has_osc_to_amp = connections
        .iter()
        .any(|c| c.from_port == "out" && c.to_port == "in");
    assert!(has_osc_to_amp, "Should have osc→amp connection");
}

#[test]
fn test_graph_debugger_describe() {
    let graph = create_simple_voice_graph();
    let dbg = GraphDebugger::new(&graph);

    let description = dbg.describe();

    // Should contain module types (using their prefixes from ModuleType::prefix())
    assert!(description.contains("osc-"), "Should mention oscillator");
    assert!(description.contains("amp-"), "Should mention amplifier");
    // StereoOutput returns ModuleType::Mixer, so it uses "mix-" prefix
    assert!(description.contains("mix-"), "Should mention mixer/output");

    // Should contain connections section
    assert!(
        description.contains("Connections"),
        "Should have connections section"
    );
}

// =============================================================================
// SignalProbe Tests with Real Modules
// =============================================================================

#[test]
fn test_signal_probe_oscillator_output() {
    let mut osc = Oscillator::new();
    let context = make_context(256);

    // Set frequency
    osc.set_param(Param::Oscillator(OscillatorParam::Frequency(Hertz::A4)));

    // Setup output buffer
    let mut outputs = HashMap::new();
    outputs.insert("out".to_string(), AudioBuffer::new(256));

    // Process
    osc.process(InputPorts::empty(), &mut outputs, &context);

    // Use SignalProbe to analyze
    let mut probe = SignalProbe::new();
    let osc_id = ModuleId::new(ModuleType::Oscillator, 1);
    probe.add_probe(osc_id, "out");
    probe.record(osc_id, "out", outputs.get("out").unwrap().as_slice());

    let stats = probe.stats(osc_id, "out").unwrap();

    // Oscillator should produce signal
    assert!(!stats.is_silent, "Oscillator should produce signal");
    assert!(
        stats.peak > 0.5,
        "Oscillator should have significant amplitude"
    );
    assert!(
        stats.dc_offset.abs() < 0.1,
        "Sine wave should have near-zero DC offset"
    );
}

#[test]
fn test_signal_probe_envelope_attack() {
    let mut env = Envelope::new();
    let context = make_context(256);

    // Set fast attack (10ms = 0.01s)
    env.set_param(Param::Envelope(EnvelopeParam::Attack(
        crate::types::Seconds::new(0.01),
    )));

    // Trigger envelope
    env.note_on(MidiNote::C4, crate::types::Velocity::MAX);

    // Process and record
    let mut outputs = HashMap::new();
    outputs.insert("out".to_string(), AudioBuffer::new(256));
    env.process(InputPorts::empty(), &mut outputs, &context);

    let mut probe = SignalProbe::new();
    let env_id = ModuleId::new(ModuleType::Envelope, 1);
    probe.add_probe(env_id, "out");
    probe.record(env_id, "out", outputs.get("out").unwrap().as_slice());

    let stats = probe.stats(env_id, "out").unwrap();

    // Envelope should ramp up during attack
    assert!(
        !stats.is_silent,
        "Envelope should produce output after trigger"
    );
    assert!(stats.max > 0.0, "Envelope should rise above zero");
}

#[test]
fn test_signal_probe_compare_before_after_filter() {
    use crate::modules::Filter;

    let mut osc = Oscillator::new();
    let mut filter = Filter::new();
    let context = make_context(512);

    // Generate oscillator signal
    let mut osc_outputs = HashMap::new();
    osc_outputs.insert("out".to_string(), AudioBuffer::new(512));
    osc.process(InputPorts::empty(), &mut osc_outputs, &context);

    // Filter the signal
    let osc_buffer = osc_outputs.get("out").unwrap();
    let input_slice: [(PortName, &AudioBuffer); 1] = [(PortName::intern("in"), osc_buffer)];
    let inputs = InputPorts::new(&input_slice);

    let mut filter_outputs = HashMap::new();
    filter_outputs.insert("out".to_string(), AudioBuffer::new(512));
    filter.process(inputs, &mut filter_outputs, &context);

    // Compare using SignalProbe
    let mut probe = SignalProbe::new();
    let osc_id = ModuleId::new(ModuleType::Oscillator, 1);
    let filter_id = ModuleId::new(ModuleType::Filter, 1);

    probe.add_probe(osc_id, "out");
    probe.add_probe(filter_id, "out");
    probe.record(osc_id, "out", osc_buffer.as_slice());
    probe.record(
        filter_id,
        "out",
        filter_outputs.get("out").unwrap().as_slice(),
    );

    let osc_stats = probe.stats(osc_id, "out").unwrap();
    let filter_stats = probe.stats(filter_id, "out").unwrap();

    // Both should have signal
    assert!(!osc_stats.is_silent);
    assert!(!filter_stats.is_silent);

    // Compare
    let osc_point = crate::debug::ProbePoint::new(osc_id, "out");
    let filter_point = crate::debug::ProbePoint::new(filter_id, "out");
    let comparison = probe.compare(&osc_point, &filter_point).unwrap();

    // Signals should be correlated (filter doesn't invert phase)
    assert!(comparison.correlation > 0.0, "Signals should be correlated");
    // But not identical (filter changes spectrum)
    assert!(!comparison.are_identical, "Filter should modify signal");
}

// =============================================================================
// SequencerDebugger Tests
// =============================================================================

fn create_test_song_with_notes() -> Song {
    let mut song = Song::new("Test Song");

    // Create a pattern with multiple notes
    let pattern_id = song.create_pattern(Duration(384)); // 4 beats

    if let Some(pattern) = song.pattern_mut(pattern_id) {
        // Note at tick 0
        pattern.add_note(
            PatternTick(0),
            Pitch::new(60).unwrap(), // C4
            Velocity::MF,
            SeqInstrumentId(0),
        );

        // Note at tick 96 (1 beat later)
        pattern.add_note(
            PatternTick(96),
            Pitch::new(64).unwrap(), // E4
            Velocity::MF,
            SeqInstrumentId(0),
        );

        // Note at tick 192 (2 beats later)
        pattern.add_note(
            PatternTick(192),
            Pitch::new(67).unwrap(), // G4
            Velocity::MF,
            SeqInstrumentId(0),
        );
    }

    let track_id = song.create_track("Track 1");
    song.place_pattern(pattern_id, track_id, Tick(0));

    song
}

#[test]
fn test_sequencer_debugger_step_to_first_note() {
    let song = create_test_song_with_notes();
    let mut dbg = SequencerDebugger::new(song);

    // Step to tick 1 (should capture note at tick 0)
    let events = dbg.step_to(Tick(1));

    // Should have at least one NoteOn
    let note_ons: Vec<_> = events.iter().filter(|e| e.event.is_note_on()).collect();
    assert!(!note_ons.is_empty(), "Should have NoteOn at tick 0");

    // Verify it's C4
    if let crate::sequencer::SequencerEvent::NoteOn { pitch, .. } = &note_ons[0].event {
        assert_eq!(pitch.as_midi(), 60, "First note should be C4");
    }
}

#[test]
fn test_sequencer_debugger_step_through_pattern() {
    let song = create_test_song_with_notes();
    let mut dbg = SequencerDebugger::new(song);

    // Step through entire pattern
    let events = dbg.step_to(Tick(300));

    // Should have 3 NoteOn events
    let note_ons: Vec<_> = events.iter().filter(|e| e.event.is_note_on()).collect();
    assert_eq!(note_ons.len(), 3, "Should have 3 NoteOn events");

    // Check pitches
    let pitches: Vec<u8> = note_ons
        .iter()
        .filter_map(|e| {
            if let crate::sequencer::SequencerEvent::NoteOn { pitch, .. } = &e.event {
                Some(pitch.as_midi())
            } else {
                None
            }
        })
        .collect();

    assert!(pitches.contains(&60), "Should have C4");
    assert!(pitches.contains(&64), "Should have E4");
    assert!(pitches.contains(&67), "Should have G4");
}

#[test]
fn test_sequencer_debugger_snapshot() {
    let song = create_test_song_with_notes();
    let mut dbg = SequencerDebugger::new(song);

    // Get initial snapshot
    let snap1 = dbg.snapshot();
    assert_eq!(snap1.current_tick, Tick::ZERO);

    // Step forward
    dbg.step_to(Tick(100));

    // Get new snapshot
    let snap2 = dbg.snapshot();
    assert!(snap2.current_tick.0 >= 100);
}

#[test]
fn test_sequencer_debugger_event_filtering() {
    let song = create_test_song_with_notes();
    let mut dbg = SequencerDebugger::new(song);

    dbg.step_to(Tick(300));

    // Test filtering by instrument
    let inst0_events = dbg.events_for_instrument(SeqInstrumentId(0));
    assert!(
        !inst0_events.is_empty(),
        "Should have events for instrument 0"
    );

    let inst1_events = dbg.events_for_instrument(SeqInstrumentId(1));
    assert!(
        inst1_events.is_empty(),
        "Should have no events for instrument 1"
    );
}

// =============================================================================
// VoiceDebugger Tests
// =============================================================================

#[test]
fn test_voice_debugger_allocation_tracking() {
    use crate::debug::ReleaseReason;

    let mut dbg = VoiceDebugger::new();

    // Simulate voice allocation sequence
    dbg.record_allocation(0, MidiNote::C4, 0.8, Some(Tick(0)));
    dbg.advance(1000);

    dbg.record_allocation(1, MidiNote::new(64), 0.7, Some(Tick(96)));
    dbg.advance(1000);

    dbg.record_release(0, ReleaseReason::NoteOff);
    dbg.advance(500);

    // Check tracking
    assert_eq!(dbg.unique_voices_used(), 2, "Should have used 2 voices");
    assert_eq!(dbg.allocations().len(), 2, "Should have 2 allocations");
    assert_eq!(dbg.releases().len(), 1, "Should have 1 release");
}

#[test]
fn test_voice_debugger_steal_detection() {
    use crate::debug::ReleaseReason;

    let mut dbg = VoiceDebugger::new();

    // Allocate voice
    dbg.record_allocation(0, MidiNote::C4, 0.8, None);
    dbg.advance(500);

    // Steal it
    dbg.record_steal(0, MidiNote::new(64));
    dbg.advance(500);

    // Release
    dbg.record_release(0, ReleaseReason::NoteOff);

    // Check
    assert_eq!(dbg.steals().len(), 1, "Should detect 1 steal");
    assert_eq!(dbg.unique_voices_used(), 1, "Still only 1 unique voice");
}

#[test]
fn test_voice_debugger_summary() {
    use crate::debug::ReleaseReason;

    let mut dbg = VoiceDebugger::new();

    // Create some activity
    dbg.record_allocation(0, MidiNote::C4, 0.8, None);
    dbg.advance(1000);
    dbg.record_release(0, ReleaseReason::NoteOff);
    dbg.advance(500);
    dbg.record_allocation(0, MidiNote::new(64), 0.7, None);

    let summary = dbg.summarize();

    // Summary should contain key info
    assert!(summary.contains("Voice Allocation Summary"));
    assert!(summary.contains("allocations"));
    assert!(summary.contains("releases"));
}

// =============================================================================
// Combined Debug Workflow Test
// =============================================================================

#[test]
fn test_complete_debug_workflow() {
    // This test demonstrates a complete debugging workflow:
    // 1. Create a voice graph
    // 2. Verify connections with GraphDebugger
    // 3. Process audio and analyze with SignalProbe
    // 4. Track voice allocation with VoiceDebugger

    use crate::debug::ReleaseReason;

    // 1. Create and verify graph
    let graph = create_voice_graph_with_envelope();
    let graph_dbg = GraphDebugger::new(&graph);

    let osc_id = graph.find_module_by_type(ModuleType::Oscillator).unwrap();
    let env_id = graph.find_module_by_type(ModuleType::Envelope).unwrap();
    let amp_id = graph.find_module_by_type(ModuleType::Amplifier).unwrap();
    // Note: StereoOutput returns ModuleType::Mixer from module_type() for compatibility
    let out_id = graph.find_module_by_type(ModuleType::Mixer).unwrap();

    // Verify signal paths
    assert!(
        graph_dbg.path_exists(osc_id, out_id),
        "Audio path should exist"
    );
    assert!(
        graph_dbg.path_exists(env_id, amp_id),
        "CV path should exist"
    );

    // 2. Setup signal probe
    let mut probe = SignalProbe::new();
    probe.add_probe(osc_id, "out");
    probe.add_probe(env_id, "out");
    probe.add_probe(amp_id, "out");

    // 3. Setup voice debugger
    let mut voice_dbg = VoiceDebugger::new();

    // 4. Simulate note playback
    voice_dbg.record_allocation(0, MidiNote::C4, 0.8, None);

    // (In real code, we would process the graph here and record samples)
    // For this test, we just verify the setup is correct

    voice_dbg.advance(10000);
    voice_dbg.record_release(0, ReleaseReason::NoteOff);

    // 5. Verify results
    assert_eq!(voice_dbg.unique_voices_used(), 1);
    assert_eq!(voice_dbg.allocations().len(), 1);
    assert_eq!(voice_dbg.releases().len(), 1);

    // Graph should have proper structure
    let description = graph_dbg.describe();
    assert!(description.contains("osc-"));
    assert!(description.contains("env-"));
    assert!(description.contains("amp-"));
}
