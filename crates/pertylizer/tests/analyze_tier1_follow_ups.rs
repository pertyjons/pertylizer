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
use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
use synth_engine::instrument::InstrumentId;
use synth_engine::{InstrumentCategory, SynthEngine};
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SeqInstrumentId, Song, Tick, Velocity,
};

use pertylizer::mcp_bridge::{analyze_section_impl, analyze_song_harmony};
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
    }
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
            let nid = pat.add_note(
                PatternTick(0),
                Pitch::new(midi).unwrap(),
                Velocity::MF,
                SeqInstrumentId(0),
            );
            if let Some(n) = pat.note_mut(nid) {
                n.duration = Some(SeqDuration(bar - 60));
            }
        }
    }

    let drum_pattern_id = song.create_pattern(SeqDuration(bar));
    {
        let pat = song.pattern_mut(drum_pattern_id).expect("drum pattern");
        let nid = pat.add_note(
            PatternTick(0),
            Pitch::new(42).unwrap(), // F#2 — classic GM hi-hat
            Velocity::MF,
            SeqInstrumentId(1),
        );
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }

    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = Some(SeqInstrumentId(0));
    }
    let drum_track = song.create_track("Drums");
    if let Some(t) = song.track_mut(drum_track) {
        t.instrument = Some(SeqInstrumentId(1));
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
            let nid = pat.add_note(
                PatternTick(0),
                Pitch::new(midi).unwrap(),
                Velocity::MF,
                SeqInstrumentId(0),
            );
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
            SeqInstrumentId(1),
        );
        if let Some(n) = pat.note_mut(nid) {
            n.duration = Some(SeqDuration(bar - 60));
        }
    }

    let pad_track = song.create_track("Pad");
    if let Some(t) = song.track_mut(pad_track) {
        t.instrument = Some(SeqInstrumentId(0));
    }
    let bass_track = song.create_track("Bass");
    if let Some(t) = song.track_mut(bass_track) {
        t.instrument = Some(SeqInstrumentId(1));
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

    let result = analyze_section_impl(&rig.session, &shared, 0, 3840, Some(true))
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
            tc.rms > 0.005,
            "track {}({}) should be audible when soloed, got RMS {}",
            tc.track_name,
            tc.track_id,
            tc.rms
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

#[test]
fn analyze_section_without_per_track_flag_returns_empty_breakdown() {
    let rig = setup_two_instruments(InstrumentCategory::Uncategorized);
    let song = build_two_track_song();
    let shared = McpSharedState::with_song(song);

    let result = analyze_section_impl(&rig.session, &shared, 0, 3840, None)
        .expect("section analysis should succeed");
    assert!(
        result.per_track.is_empty(),
        "per_track must default to empty when not requested"
    );

    let opt_off = analyze_section_impl(&rig.session, &shared, 0, 3840, Some(false))
        .expect("section analysis should succeed");
    assert!(opt_off.per_track.is_empty());
}
