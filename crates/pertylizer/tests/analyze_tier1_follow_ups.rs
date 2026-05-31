//! Integration tests for the Tier-1 follow-ups to the v0.276.0 music-tools
//! release:
//!
//! 1. `analyze_harmony` drum-track filtering — notes from tracks whose
//!    instrument has category `Drums` no longer pollute chord identification.
//! 2. `analyze_section` per-track contribution breakdown — opting in returns
//!    one `TrackContribution` per audible track that overlaps the section.
//!
//! Both exercises follow the same recipe as
//! `arrangement_render_integration.rs`: build a real `SynthEngine` +
//! `SynthSession`, install a sustaining patch on N instruments, construct a
//! `Song` with multiple tracks, then call the bridge implementation directly.

use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{
    AudioCallbackContext, AudioProcessor, DistortionParam, ModuleType, NormalizedValue, Param,
};
use synth_engine::instrument::InstrumentId;
use synth_engine::{EngineCommand, InstrumentCategory, ModuleId, SynthEngine};
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SeqInstrumentId, Song, Tick, Velocity,
};

use pertylizer::mcp_bridge::{
    analyze_masking_matrix_impl, analyze_master_chain_impl, analyze_mix_bus_impl,
    analyze_section_impl, analyze_song_harmony,
};
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

fn sustain_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.005)
            .param_f("decay", 0.0)
            .param_f("sustain", 1.0)
            .param_f("release", 0.05)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

/// Patch with a noise source + percussive envelope — the shape the
/// instrument-profile inference looks for when the user has *not* manually
/// tagged the instrument as drums. Connections mirror `sustain_patch` so the
/// engine accepts the graph.
fn kick_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Noise)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.001)
            .param_f("decay", 0.05)
            .param_f("sustain", 0.0)
            .param_f("release", 0.05)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    patch.add_connection("nse-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

struct TwoInstrumentRig {
    _engine: SynthEngine,
    _handle: synth_engine::EngineHandle,
    session: SynthSession,
    sample_library: pertylizer::audio::preview::SharedSampleLibrary,
}

fn setup_two_instruments(drum_category: InstrumentCategory) -> TwoInstrumentRig {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    // Instrument 0 → melodic, instrument 1 → drums.
    let melodic = InstrumentId::new(0);
    let percussion = InstrumentId::new(1);
    session
        .add_instrument_with_id(melodic, "Pad")
        .expect("add melodic instrument");
    session
        .add_instrument_with_id(percussion, "Drums")
        .expect("add drum instrument");
    session
        .set_instrument_category(percussion, drum_category)
        .expect("set drum category");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(melodic, &sustain_patch("Pad"));
    let _ = session.apply_patch(percussion, &sustain_patch("Drums"));

    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    TwoInstrumentRig {
        _engine: engine,
        _handle: handle,
        session,
        sample_library: Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        )),
    }
}

/// Sample-based drum patch: just Sampler (PlayMode=OneShot) → Amp → Output.
/// No envelope module, no noise — exercises the `pure_oneshot_sampler` arm
/// of the drum gate added in 74d18da. Connections mirror `kick_patch` so the
/// engine accepts the graph.
fn sampler_kick_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Sampler)
            .param_choice("play_mode", "one_shot")
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    patch.add_connection("sam-1", "out", "amp-1", "in");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

/// Variant of `setup_two_instruments` that does *not* call
/// `set_instrument_category` on the drum instrument and gives it a true
/// noise+percussive-envelope patch. Used to verify the auto-inference path
/// in `analyze_harmony` (§8.2b regression target).
fn setup_pad_and_uncategorized_kick() -> TwoInstrumentRig {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let melodic = InstrumentId::new(0);
    let percussion = InstrumentId::new(1);
    session
        .add_instrument_with_id(melodic, "Pad")
        .expect("add melodic instrument");
    // Note: percussion deliberately keeps the default Uncategorized category
    // so the inference layer is the only thing that can classify it.
    session
        .add_instrument_with_id(percussion, "Track 5")
        .expect("add drum instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(melodic, &sustain_patch("Pad"));
    let _ = session.apply_patch(percussion, &kick_patch("Track 5"));

    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    TwoInstrumentRig {
        _engine: engine,
        _handle: handle,
        session,
        sample_library: Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        )),
    }
}

/// Like `setup_pad_and_uncategorized_kick`, but the drum instrument is a
/// pure sampler patch — the shape projects loaded from ZIP bundles produce.
/// Exercises the `pure_oneshot_sampler` drum-gate branch (74d18da).
fn setup_pad_and_uncategorized_sampler_kick() -> TwoInstrumentRig {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

    let melodic = InstrumentId::new(0);
    let percussion = InstrumentId::new(1);
    session
        .add_instrument_with_id(melodic, "Pad")
        .expect("add melodic instrument");
    session
        .add_instrument_with_id(percussion, "Track 5")
        .expect("add drum instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(melodic, &sustain_patch("Pad"));
    let _ = session.apply_patch(percussion, &sampler_kick_patch("Track 5"));

    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    TwoInstrumentRig {
        _engine: engine,
        _handle: handle,
        session,
        sample_library: Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        )),
    }
}

/// Build a song with:
/// - Track 0 holding an A minor pad triad (A3, C4, E4) for one bar.
/// - Track 1 holding a percussion MIDI note F#2 (42) for the same bar.
///
/// Without filtering, the analyzer should see {A, C, E, F#} and identify
/// the bar as `F#m7b5` (the bug from live testing). With the drum filter
/// active, the percussion track drops out and the bar becomes `Am`.
fn build_drum_pollution_song() -> Arc<RwLock<Song>> {
    let mut song = Song::new("DrumPollution");
    let bar = 3840u32; // 4 quarter notes at 960 PPQN

    let pad_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(pad_pattern_id).expect("pad pattern");
        for midi in [57u8, 60, 64] {
            // A3, C4, E4
            let nid = pat.add_note(PatternTick(0), Pitch::new(midi).unwrap(), Velocity::MF);
            if let Some(n) = pat.note_mut(nid) {
                n.duration = Some(SeqDuration(bar - 60));
            }
        }
    }

    let drum_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(drum_pattern_id).expect("drum pattern");
        // Deliberately set the note's instrument to a value that does NOT
        // match the drum track's instrument (SeqInstrumentId(1)). Real
        // projects routinely leave `Note.instrument` at a default that
        // doesn't correspond to the placement's track — routing is decided
        // by the track, not the note. The inference path must not rely on
        // `note.instrument` matching, or it sees an empty note pool for
        // every instrument and silently classifies everything as Unknown.
        let nid = pat.add_note(
            PatternTick(0),
            Pitch::new(42).unwrap(), // F#2 — classic GM hi-hat
            Velocity::MF,
        );
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }

    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = SeqInstrumentId(0);
    }
    let drum_track = song.create_track("Drums");
    if let Some(t) = song.track_mut(drum_track) {
        t.instrument = SeqInstrumentId(1);
    }

    song.place_pattern(pad_pattern_id, pad_track, Tick(0));
    song.place_pattern(drum_pattern_id, drum_track, Tick(0));

    Arc::new(RwLock::new(song))
}

#[test]
fn analyze_harmony_default_excludes_drum_tracks() {
    let rig = setup_two_instruments(InstrumentCategory::Drums);
    let song = build_drum_pollution_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_song_harmony(
        &rig.session,
        &shared,
        None,       // arrangement scope
        Some(0),    // start
        Some(3840), // end
        Some(3840), // single grouping window across the whole bar
        None,       // exclude_drums defaults to true
        None,       // no explicit exclude_track_ids
    )
    .expect("harmony analysis should succeed");

    let chord_symbols: Vec<&str> = result
        .chords
        .iter()
        .filter_map(|c| c.symbol.as_deref())
        .collect();
    assert!(
        chord_symbols.iter().any(|s| s == &"Am"),
        "default drum-filtering should yield Am, got {chord_symbols:?}"
    );
    assert!(
        chord_symbols
            .iter()
            .all(|s| !s.starts_with("F#") && !s.contains("m7b5")),
        "default drum-filtering should not produce F#m7b5, got {chord_symbols:?}"
    );
    let drum_track_excluded = result
        .warnings
        .iter()
        .any(|w| w.contains("Excluded") && w.contains("Drums"));
    assert!(
        drum_track_excluded,
        "expected an 'Excluded ... Drums' warning, got {:?}",
        result.warnings
    );
}

/// §8.2b regression target — when no instrument has been manually tagged
/// `Drums`, the auto-inference must still classify a Noise+percussive-
/// envelope instrument as drums and drop its track from harmony analysis.
/// Before §8.2b the drum filter was a silent no-op in this case and the
/// F#m7b5 bug returned.
#[test]
fn analyze_harmony_default_excludes_uncategorized_inferred_drums() {
    let rig = setup_pad_and_uncategorized_kick();
    let song = build_drum_pollution_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_song_harmony(
        &rig.session,
        &shared,
        None,
        Some(0),
        Some(3840),
        Some(3840),
        None, // exclude_drums defaults to true
        None,
    )
    .expect("harmony analysis should succeed");

    let chord_symbols: Vec<&str> = result
        .chords
        .iter()
        .filter_map(|c| c.symbol.as_deref())
        .collect();
    assert!(
        chord_symbols.iter().any(|s| s == &"Am"),
        "auto-inference should produce Am, got {chord_symbols:?}"
    );
    assert!(
        chord_symbols
            .iter()
            .all(|s| !s.starts_with("F#") && !s.contains("m7b5")),
        "auto-inference should not produce F#m7b5, got {chord_symbols:?}"
    );
    // Warning must mention the inferred-drums signal trail so a user can
    // see *why* a track was dropped without checking the inference output
    // separately.
    let warning_mentions_inference = result.warnings.iter().any(|w| {
        w.contains("Excluded")
            && w.contains("drums conf=")
            && (w.contains("graph:noise-no-osc") || w.contains("envelope:percussive"))
    });
    assert!(
        warning_mentions_inference,
        "expected an inference-tagged drum warning, got {:?}",
        result.warnings
    );
}

/// End-to-end cover for the sample-based drum-gate branch added in 74d18da:
/// a generic-named track running a pure Sampler (no envelope, no noise) with
/// PlayMode=OneShot must be auto-excluded from harmony, exactly the shape
/// that real ZIP-bundle projects load with.
#[test]
fn analyze_harmony_default_excludes_uncategorized_sampler_drums() {
    let rig = setup_pad_and_uncategorized_sampler_kick();
    let song = build_drum_pollution_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_song_harmony(
        &rig.session,
        &shared,
        None,
        Some(0),
        Some(3840),
        Some(3840),
        None, // exclude_drums defaults to true
        None,
    )
    .expect("harmony analysis should succeed");

    let chord_symbols: Vec<&str> = result
        .chords
        .iter()
        .filter_map(|c| c.symbol.as_deref())
        .collect();
    assert!(
        chord_symbols.iter().any(|s| s == &"Am"),
        "sampler-drum inference should produce Am, got {chord_symbols:?}"
    );
    assert!(
        chord_symbols
            .iter()
            .all(|s| !s.starts_with("F#") && !s.contains("m7b5")),
        "sampler-drum inference should not produce F#m7b5, got {chord_symbols:?}"
    );
    let warning_mentions_oneshot = result.warnings.iter().any(|w| {
        w.contains("Excluded") && w.contains("drums conf=") && w.contains("graph:oneshot-sampler")
    });
    assert!(
        warning_mentions_oneshot,
        "expected a `graph:oneshot-sampler` drum warning, got {:?}",
        result.warnings
    );
}

#[test]
fn analyze_harmony_explicit_disable_lets_drums_pollute() {
    let rig = setup_two_instruments(InstrumentCategory::Drums);
    let song = build_drum_pollution_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_song_harmony(
        &rig.session,
        &shared,
        None,
        Some(0),
        Some(3840),
        Some(3840),
        Some(false), // explicitly disable drum filter
        None,
    )
    .expect("harmony analysis should succeed");

    let chord_symbols: Vec<String> = result
        .chords
        .iter()
        .filter_map(|c| c.symbol.clone())
        .collect();
    assert!(
        chord_symbols.iter().any(|s| s.contains("m7b5")),
        "with drum filter off, F# from MIDI 42 should pollute the chord to ...m7b5, got {chord_symbols:?}"
    );
}

#[test]
fn analyze_harmony_excludes_by_track_id() {
    // Same song but the drum instrument is *not* tagged Drums. The
    // explicit `exclude_track_ids` parameter must still drop it.
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_drum_pollution_song();
    let shared = McpSharedState::with_song(song);

    // The percussion track is the second one created in build_drum_pollution_song,
    // so its TrackId is 1.
    let drum_track_id = 1u16;

    let result = analyze_song_harmony(
        &rig.session,
        &shared,
        None,
        Some(0),
        Some(3840),
        Some(3840),
        Some(false), // category-based filter off
        Some(vec![drum_track_id]),
    )
    .expect("harmony analysis should succeed");

    let chord_symbols: Vec<String> = result
        .chords
        .iter()
        .filter_map(|c| c.symbol.clone())
        .collect();
    assert!(
        chord_symbols.iter().any(|s| s == "Am"),
        "explicit track exclusion should yield Am, got {chord_symbols:?}"
    );
}

/// Same two-instrument rig but both tracks play melodic pad chords so the
/// per-track render produces audible output for both. Used for the section
/// breakdown test.
fn build_two_track_song() -> Arc<RwLock<Song>> {
    let mut song = Song::new("TwoTrack");
    let bar = 3840u32;

    let pad_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(pad_pattern_id).expect("pad pattern");
        for midi in [60u8, 64, 67] {
            // C major triad
            let nid = pat.add_note(PatternTick(0), Pitch::new(midi).unwrap(), Velocity::MF);
            if let Some(n) = pat.note_mut(nid) {
                n.duration = Some(SeqDuration(bar - 60));
            }
        }
    }

    let bass_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(bass_pattern_id).expect("bass pattern");
        let nid = pat.add_note(
            PatternTick(0),
            Pitch::new(36).unwrap(), // C2 — strong sub
            Velocity::MF,
        );
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }

    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = SeqInstrumentId(0);
    }
    let bass_track = song.create_track("Bass");
    if let Some(t) = song.track_mut(bass_track) {
        t.instrument = SeqInstrumentId(1);
    }
    song.place_pattern(pad_pattern_id, pad_track, Tick(0));
    song.place_pattern(bass_pattern_id, bass_track, Tick(0));

    Arc::new(RwLock::new(song))
}

#[test]
fn analyze_section_per_track_breakdown_emits_one_entry_per_track() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(true),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("section analysis should succeed");

    assert_eq!(
        result.per_track.len(),
        2,
        "expected one contribution per audible track, got {}",
        result.per_track.len()
    );

    // Each track should produce audible content on its own.
    for tc in &result.per_track {
        assert!(
            tc.metrics.rms > 0.005,
            "track {}({}) should be audible when soloed, got RMS {}",
            tc.track_name,
            tc.track_id,
            tc.metrics.rms
        );
        assert!(
            tc.rms_share > 0.0 && tc.rms_share <= 1.0,
            "rms_share for {} should be in (0, 1], got {}",
            tc.track_name,
            tc.rms_share
        );
    }

    // Shares should sum to ~1.0 since each contribution is a partition of the
    // total summed-track RMS.
    let total_share: f32 = result.per_track.iter().map(|t| t.rms_share).sum();
    assert!(
        (total_share - 1.0).abs() < 1e-4,
        "rms_share should sum to ~1.0, got {total_share}"
    );
}

/// `analyze_mix_bus` reuses the same per-track path as `analyze_section`, keyed
/// off a duration window. With `include_per_track = true` it should emit one
/// contribution per audible track; without the flag the breakdown stays empty.
#[test]
fn analyze_mix_bus_per_track_breakdown_emits_one_entry_per_track() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_mix_bus_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        10.0,
        Some(0),
        Some(true),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("mix-bus analysis should succeed");

    assert_eq!(
        result.per_track.len(),
        2,
        "expected one contribution per audible track, got {}",
        result.per_track.len()
    );
    let total_share: f32 = result.per_track.iter().map(|t| t.rms_share).sum();
    assert!(
        (total_share - 1.0).abs() < 1e-4,
        "rms_share should sum to ~1.0, got {total_share}"
    );

    // Without the flag, no breakdown is produced.
    let no_breakdown = analyze_mix_bus_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        10.0,
        Some(0),
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("mix-bus analysis should succeed");
    assert!(
        no_breakdown.per_track.is_empty(),
        "per_track should be empty when include_per_track is not set"
    );
}

/// With no master effects loaded, `analyze_master_chain` returns no stages and
/// the output mix equals the input mix (plus an explanatory warning).
#[test]
fn analyze_master_chain_empty_chain_has_no_stages() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_master_chain_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        10.0,
        Some(0),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("master-chain analysis should succeed");

    assert!(
        result.stages.is_empty(),
        "no master effects → no stages, got {}",
        result.stages.len()
    );
    // Input and output describe the same render, so the metrics must match.
    assert!(
        (result.input_metrics.rms - result.output_metrics.rms).abs() < 1e-6,
        "empty chain: output RMS ({}) should equal input RMS ({})",
        result.output_metrics.rms,
        result.input_metrics.rms
    );
    assert!(
        result.warnings.iter().any(|w| w.contains("no effects")),
        "expected an empty-chain warning, got {:?}",
        result.warnings
    );
}

/// Regression: per-track contributions include `pre_master_peak`, the
/// pan/volume-compensated patch peak. For a center-panned, full-volume track
/// the unattenuated peak should be ≈ sqrt(2) × the master-contribution peak.
#[test]
fn analyze_section_per_track_pre_master_peak_compensates_for_pan_law() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(true),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("section analysis should succeed");

    assert_eq!(result.per_track.len(), 2);
    for tc in &result.per_track {
        // Instruments default to pan = CENTER and volume = MAX. Constant-power
        // pan-law puts √2/2 on each channel, so per-channel peaks should be
        // ≈ internal_peak × 0.7071. Dividing by that gain in the analytical
        // path recovers √2 × the master-contribution peak.
        let ratio = tc.pre_master_peak / tc.metrics.peak.max(1e-9);
        assert!(
            (ratio - std::f32::consts::SQRT_2).abs() < 0.05,
            "track {}({}) pre_master_peak / metrics.peak = {} (expected ≈ {})",
            tc.track_name,
            tc.track_id,
            ratio,
            std::f32::consts::SQRT_2
        );
        assert!(
            tc.pre_master_peak >= tc.metrics.peak,
            "pre_master_peak ({}) should be ≥ master-contribution peak ({}) for {}",
            tc.pre_master_peak,
            tc.metrics.peak,
            tc.track_name
        );
        assert!(
            (tc.pre_master_peak_dbfs - 20.0 * tc.pre_master_peak.log10()).abs() < 1e-3,
            "pre_master_peak_dbfs ({}) inconsistent with pre_master_peak ({})",
            tc.pre_master_peak_dbfs,
            tc.pre_master_peak
        );
    }
}

#[test]
fn analyze_section_without_per_track_flag_returns_empty_breakdown() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("section analysis should succeed");
    assert!(
        result.per_track.is_empty(),
        "per_track must default to empty when not requested"
    );

    let opt_off = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(false),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("section analysis should succeed");
    assert!(opt_off.per_track.is_empty());
}

/// §7.2 regression: the parallel per-track render path must be deterministic.
/// Two back-to-back `analyze_section` calls with `include_per_track = true`
/// must return bit-exact metrics for every track. With rayon the work order
/// across worker threads is non-deterministic; this guards against a hashmap
/// / RNG / iteration-order regression sneaking back into the offline engine
/// when multiple sessions are alive concurrently.
#[test]
fn analyze_section_per_track_is_bit_exact_across_calls() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let a = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(true),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("first analyze_section");
    let b = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(true),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("second analyze_section");

    assert_eq!(a.per_track.len(), b.per_track.len());
    for (ai, bi) in a.per_track.iter().zip(b.per_track.iter()) {
        assert_eq!(ai.track_id, bi.track_id, "per_track order must match");
        assert_eq!(
            ai.metrics.peak.to_bits(),
            bi.metrics.peak.to_bits(),
            "peak for track {} drifted ({} vs {})",
            ai.track_id,
            ai.metrics.peak,
            bi.metrics.peak
        );
        assert_eq!(
            ai.metrics.rms.to_bits(),
            bi.metrics.rms.to_bits(),
            "rms for track {} drifted",
            ai.track_id
        );
        assert_eq!(
            ai.metrics.lufs_integrated.to_bits(),
            bi.metrics.lufs_integrated.to_bits(),
            "lufs_integrated for track {} drifted",
            ai.track_id
        );
        assert_eq!(
            ai.pre_master_peak.to_bits(),
            bi.pre_master_peak.to_bits(),
            "pre_master_peak for track {} drifted",
            ai.track_id
        );
        assert_eq!(
            ai.metrics.clipped_samples, bi.metrics.clipped_samples,
            "clipped_samples for track {} drifted",
            ai.track_id
        );
    }
}

/// `render_quality` selects the offline render sample rate, and the per-track
/// (engine-reuse) renders inherit the same rate. Also guards that the reused
/// engine still produces one contribution per audible track.
#[test]
fn analyze_section_render_quality_controls_sample_rate() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let full = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(true),
        synth_mcp::AnalysisScope::from_flags(
            None,
            None,
            None,
            None,
            synth_mcp::RenderQuality::Full,
        ),
    )
    .expect("full-quality section");
    assert_eq!(full.metrics.sample_rate, 44_100);
    assert_eq!(
        full.per_track.len(),
        2,
        "one contribution per audible track"
    );
    assert!(
        full.per_track
            .iter()
            .all(|t| t.metrics.sample_rate == 44_100),
        "per-track renders must inherit the full sample rate"
    );

    let draft = analyze_section_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        0,
        3840,
        Some(true),
        synth_mcp::AnalysisScope::from_flags(
            None,
            None,
            None,
            None,
            synth_mcp::RenderQuality::Draft,
        ),
    )
    .expect("draft-quality section");
    assert_eq!(draft.metrics.sample_rate, 22_050);
    assert_eq!(
        draft.per_track.len(),
        2,
        "engine-reuse per-track path must still yield one entry per audible track"
    );
    assert!(
        draft
            .per_track
            .iter()
            .all(|t| t.metrics.sample_rate == 22_050),
        "per-track renders must inherit the draft sample rate"
    );

    // Both resolutions must still detect the real signal.
    assert!(
        full.metrics.rms > 1e-4 && draft.metrics.rms > 1e-4,
        "both renders should contain audible signal (full={}, draft={})",
        full.metrics.rms,
        draft.metrics.rms
    );
}

#[test]
fn analyze_masking_matrix_emits_pair_with_well_formed_bands() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_masking_matrix_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        Some(0),
        Some(3840),
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("masking matrix should succeed");

    assert_eq!(result.track_count, 2);
    assert_eq!(
        result.pairs.len(),
        1,
        "expected exactly one pair for 2 tracks"
    );
    let pair = &result.pairs[0];

    // Pair ordering invariant: track_a_id < track_b_id.
    assert!(
        pair.track_a_id < pair.track_b_id,
        "track_a_id ({}) must be < track_b_id ({})",
        pair.track_a_id,
        pair.track_b_id
    );

    // Four bands in the canonical order.
    assert_eq!(pair.bands.len(), 4, "expected 4 bands per pair");
    let names: Vec<&str> = pair.bands.iter().map(|b| b.band.as_str()).collect();
    assert_eq!(names, vec!["sub", "low", "mid", "high"]);

    // conflict_score is a probability.
    assert!(
        (0.0..=1.0).contains(&pair.conflict_score),
        "conflict_score {} out of [0, 1]",
        pair.conflict_score
    );

    // Per-band invariants.
    for band in &pair.bands {
        assert!(
            band.overlap_energy <= band.track_a_energy.max(band.track_b_energy) + 1e-6,
            "overlap_energy ({}) exceeds max(a, b) ({})",
            band.overlap_energy,
            band.track_a_energy.max(band.track_b_energy)
        );
        assert!(
            band.dominance_db >= 0.0,
            "dominance_db ({}) must be ≥ 0",
            band.dominance_db
        );
        assert!(
            band.freq_high_hz > band.freq_low_hz,
            "band {} has malformed freq range {}-{}",
            band.band,
            band.freq_low_hz,
            band.freq_high_hz
        );
    }
}

#[test]
fn analyze_masking_matrix_is_deterministic_across_calls() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let a = analyze_masking_matrix_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        Some(0),
        Some(3840),
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("first call");
    let b = analyze_masking_matrix_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        Some(0),
        Some(3840),
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("second call");

    assert_eq!(a.pairs.len(), b.pairs.len());
    for (ap, bp) in a.pairs.iter().zip(b.pairs.iter()) {
        assert_eq!(ap.track_a_id, bp.track_a_id);
        assert_eq!(ap.track_b_id, bp.track_b_id);
        assert_eq!(
            ap.conflict_score.to_bits(),
            bp.conflict_score.to_bits(),
            "conflict_score drifted for pair ({}, {})",
            ap.track_a_id,
            ap.track_b_id
        );
        for (ab, bb) in ap.bands.iter().zip(bp.bands.iter()) {
            assert_eq!(
                ab.overlap_energy.to_bits(),
                bb.overlap_energy.to_bits(),
                "{} band overlap drifted",
                ab.band
            );
            assert_eq!(
                ab.dominance_db.to_bits(),
                bb.dominance_db.to_bits(),
                "{} band dominance drifted",
                ab.band
            );
        }
        assert_eq!(ap.hint, bp.hint);
    }
}

#[test]
fn analyze_masking_matrix_rejects_inverted_range() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let err = analyze_masking_matrix_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        Some(3840),
        Some(1920),
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect_err("inverted range must error");
    let msg = err.to_string();
    assert!(
        msg.contains("Arrangement range invalid"),
        "error message should describe the invalid arrangement range: {msg}"
    );
}

#[test]
fn analyze_masking_matrix_with_single_track_returns_no_pairs() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    // Build a one-track song using the same instruments rig (only pad placed).
    let mut song = Song::new("OneTrack");
    let bar = 3840u32;
    let pad_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(pad_pattern_id).expect("pad pattern");
        let nid = pat.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }
    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = SeqInstrumentId(0);
    }
    song.place_pattern(pad_pattern_id, pad_track, Tick(0));
    let shared = McpSharedState::with_song(Arc::new(RwLock::new(song)));

    let result = analyze_masking_matrix_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        Some(0),
        Some(3840),
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("single-track masking matrix should succeed (just empty)");
    assert_eq!(result.track_count, 1);
    assert!(result.pairs.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("at least 2 audible tracks")),
        "expected an explanatory warning, got {:?}",
        result.warnings
    );
}

#[test]
fn analyze_masking_matrix_defaults_to_full_arrangement_when_range_omitted() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_masking_matrix_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        None,
        None,
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("default range should succeed");
    // The two-track fixture spans [0, 3840) — explicit and default ranges agree.
    assert_eq!(result.start_tick, 0);
    assert_eq!(result.end_tick, 3840);
    assert_eq!(result.track_count, 2);
}

/// `AnalysisScope { master_effects: true, .. }` regression: the offline render
/// behind `analyze_section` must reconstruct the *live* engine's master effect
/// chain, not just the dry instrument sum. We render the same section twice —
/// once with the default DRY scope and once with `master_effects: true` after
/// loading a heavy Distortion onto the live master bus — and assert the WET
/// metrics differ meaningfully from the DRY metrics. A no-op scope (the old
/// behaviour) would yield identical numbers.
///
/// Self-contained setup: unlike `setup_two_instruments` the engine is kept
/// local and mutable so we can pump it after issuing the AddEffectInstance /
/// SetEffectParameter commands. That pumping is the critical step: the audio
/// thread only republishes `state.master_effects` (via
/// `update_shared_master_effects`) once it has drained and processed those
/// commands, and the offline render reads that published snapshot.
#[test]
fn analyze_section_master_effects_scope_reconstructs_live_master_chain() {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    let sample_library: pertylizer::audio::preview::SharedSampleLibrary = Arc::new(
        std::sync::RwLock::new(synth_sampler::SampleLibrary::default()),
    );

    // One sustaining melodic instrument.
    let pad = InstrumentId::new(0);
    session
        .add_instrument_with_id(pad, "Pad")
        .expect("add pad instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(pad, &sustain_patch("Pad"));
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    // One-track song: a sustained C4 for the whole bar.
    let mut song = Song::new("MasterScope");
    let bar = 3840u32;
    let pad_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(pad_pattern_id).expect("pad pattern");
        let nid = pat.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }
    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = SeqInstrumentId(0);
    }
    song.place_pattern(pad_pattern_id, pad_track, Tick(0));
    let shared = McpSharedState::with_song(Arc::new(RwLock::new(song)));

    // --- DRY render: instruments + their own effects only. ---
    let dry = analyze_section_impl(
        &session,
        &sample_library,
        &shared,
        0,
        3840,
        None,
        synth_mcp::AnalysisScope::default(),
    )
    .expect("dry section analysis should succeed");

    // Sanity: the instrument actually produced sound.
    assert!(
        dry.metrics.rms > 1e-4 && dry.metrics.peak > 1e-4,
        "dry render must be audible, got rms={} peak={}",
        dry.metrics.rms,
        dry.metrics.peak
    );

    // --- Add a heavy Distortion to the LIVE engine's master chain. ---
    let dist_id = ModuleId::new(ModuleType::Distortion, 1);
    let (effect, _descriptor) = pertylizer::module_factory::create_effect(ModuleType::Distortion)
        .expect("Distortion effect should be constructible");
    let sender = session.command_sender();
    assert!(
        sender.send(EngineCommand::AddEffectInstance {
            instrument_id: None,
            id: dist_id,
            effect,
        }),
        "AddEffectInstance command should enqueue"
    );
    // Crank the drive so the master chain audibly reshapes the signal.
    assert!(
        sender.send(EngineCommand::SetEffectParameter {
            instrument_id: None,
            module_id: dist_id,
            param: Param::Distortion(DistortionParam::Drive(NormalizedValue::MAX)),
        }),
        "SetEffectParameter command should enqueue"
    );

    // Pump the live engine so both commands drain AND the audio thread
    // republishes `state.master_effects`. Without this the shared snapshot
    // stays empty and the WET render sees no master effect.
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    // --- WET render: now also load the live master effect chain. ---
    let wet = analyze_section_impl(
        &session,
        &sample_library,
        &shared,
        0,
        3840,
        None,
        synth_mcp::AnalysisScope {
            master_effects: true,
            ..Default::default()
        },
    )
    .expect("wet section analysis should succeed");

    // The maxed-drive Distortion is a hard, near-square-wave saturator: it
    // pushes the peak toward full scale and lifts overall RMS substantially.
    // Assert on RMS (the most robust mover) with a comfortable margin.
    let rms_delta = (wet.metrics.rms - dry.metrics.rms).abs();
    assert!(
        rms_delta > 1e-3,
        "master_effects scope must change the rendered RMS: dry={} wet={} (delta={})",
        dry.metrics.rms,
        wet.metrics.rms,
        rms_delta
    );
    assert!(
        wet.metrics.rms > dry.metrics.rms,
        "maxed-drive distortion should raise RMS: dry={} wet={}",
        dry.metrics.rms,
        wet.metrics.rms
    );
}

/// `analyze_master_chain` isolates each master effect's contribution: the chain
/// input (no master effect) vs. the output after a maxed-drive Distortion. The
/// single stage must report a positive RMS delta and its output metrics must
/// equal the result's `output_metrics`.
#[test]
fn analyze_master_chain_isolates_single_effect_contribution() {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    let sample_library: pertylizer::audio::preview::SharedSampleLibrary = Arc::new(
        std::sync::RwLock::new(synth_sampler::SampleLibrary::default()),
    );

    let pad = InstrumentId::new(0);
    session
        .add_instrument_with_id(pad, "Pad")
        .expect("add pad instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(pad, &sustain_patch("Pad"));
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    let mut song = Song::new("MasterChain");
    let bar = 3840u32;
    let pad_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(pad_pattern_id).expect("pad pattern");
        let nid = pat.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }
    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = SeqInstrumentId(0);
    }
    song.place_pattern(pad_pattern_id, pad_track, Tick(0));
    let shared = McpSharedState::with_song(Arc::new(RwLock::new(song)));

    // Add a maxed-drive Distortion to the live master chain and pump so the
    // shared `master_effects` snapshot republishes.
    let dist_id = ModuleId::new(ModuleType::Distortion, 1);
    let (effect, _descriptor) = pertylizer::module_factory::create_effect(ModuleType::Distortion)
        .expect("Distortion effect should be constructible");
    let sender = session.command_sender();
    assert!(sender.send(EngineCommand::AddEffectInstance {
        instrument_id: None,
        id: dist_id,
        effect,
    }));
    assert!(sender.send(EngineCommand::SetEffectParameter {
        instrument_id: None,
        module_id: dist_id,
        param: Param::Distortion(DistortionParam::Drive(NormalizedValue::MAX)),
    }));
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    let result = analyze_master_chain_impl(
        &session,
        &sample_library,
        &shared,
        1.0,
        Some(0),
        synth_mcp::AnalysisScope::default(),
    )
    .expect("master-chain analysis should succeed");

    assert_eq!(result.stages.len(), 1, "one master effect → one stage");
    let stage = &result.stages[0];
    assert_eq!(stage.effect_type, "dst");
    assert!(
        !stage.bypassed,
        "the distortion is enabled, so it should not be reported bypassed"
    );

    // The chain input has no master effect; the output went through maxed
    // distortion, which lifts RMS — so the stage's delta is positive and the
    // gain_reduction (its inverse) is negative.
    assert!(
        result.output_metrics.rms > result.input_metrics.rms,
        "distortion should raise output RMS: input={} output={}",
        result.input_metrics.rms,
        result.output_metrics.rms
    );
    assert!(
        stage.rms_delta_db > 0.0,
        "stage rms_delta_db should be positive, got {}",
        stage.rms_delta_db
    );
    assert!(
        (stage.gain_reduction_db + stage.rms_delta_db).abs() < 1e-3,
        "gain_reduction_db ({}) should be the inverse of rms_delta_db ({})",
        stage.gain_reduction_db,
        stage.rms_delta_db
    );
    // The only stage's output IS the master output.
    assert!(
        (stage.metrics.rms - result.output_metrics.rms).abs() < 1e-6,
        "last stage metrics.rms ({}) should equal output_metrics.rms ({})",
        stage.metrics.rms,
        result.output_metrics.rms
    );
}
