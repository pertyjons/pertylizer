//! Every major editor's mutations must reach the unsaved-changes mechanism.
//!
//! Dirty state is derived from three edit counters (`SharedSong::revision`,
//! `SharedGraph::version`, `SampleLibrary::revision`) plus a fingerprint of the
//! patch-canvas layout. The point of that design is that an editor cannot
//! silently bypass it — but only as long as each editor really does route its
//! mutations through the shared state those counters watch.
//!
//! These tests perform one representative mutation per view and assert the
//! corresponding counter moved. A future editor that starts writing somewhere
//! else — a private cache, a side-channel — will fail here rather than shipping
//! a project that closes without a prompt after real work.

use std::sync::Arc;

use synth_core::{Bpm, ContentRevision, NormalizedValue};
use synth_engine::SynthEngine;
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SharedSong, Song, Tick, Velocity,
};

/// Shared song, as the application holds it.
fn shared_song() -> Arc<SharedSong> {
    Arc::new(SharedSong::new(Song::new("Coverage")))
}

/// Assert that `mutate` advances the song revision — i.e. that an edit made
/// this way would mark the project unsaved.
fn assert_marks_dirty(view: &str, mutate: impl FnOnce(&SharedSong)) {
    let song = shared_song();
    let before = song.revision();

    mutate(&song);

    assert_ne!(
        song.revision(),
        before,
        "an edit from the {view} must advance the song revision",
    );
}

#[test]
fn piano_roll_and_tracker_note_edits_mark_the_project_dirty() {
    assert_marks_dirty("piano roll / tracker", |song| {
        let pattern_id = song.write().create_pattern(SeqDuration::WHOLE);
        let _ = song
            .write()
            .pattern_mut(pattern_id)
            .expect("pattern")
            .add_note(
                PatternTick::ZERO,
                Pitch::new(60).expect("middle C"),
                Velocity::new(NormalizedValue::MAX.as_f32()),
            );
    });
}

#[test]
fn arrangement_track_edits_mark_the_project_dirty() {
    assert_marks_dirty("arrangement view", |song| {
        let _ = song.write().create_track("Lead");
    });
}

#[test]
fn transport_tempo_edits_mark_the_project_dirty() {
    assert_marks_dirty("transport", |song| {
        song.write().set_tempo_at(Tick::ZERO, Bpm::new(140.0));
    });
}

#[test]
fn note_grid_edits_mark_the_project_dirty() {
    assert_marks_dirty("Note Grid", |song| {
        let _ = song.write().create_note_graph("Arp");
    });
}

#[test]
fn mod_grid_edits_mark_the_project_dirty() {
    assert_marks_dirty("Mod Grid", |song| {
        let _ = song.write().create_mod_graph("Wobble");
    });
}

#[test]
fn mixer_fader_edits_mark_the_project_dirty() {
    assert_marks_dirty("mixer", |song| {
        let track_id = song.write().create_track("Drums");
        song.write().track_mut(track_id).expect("track").volume = NormalizedValue::new(0.25);
    });
}

#[test]
fn return_bus_edits_mark_the_project_dirty() {
    assert_marks_dirty("mixer return bus", |song| {
        let _ = song.write().create_return_bus("Reverb");
    });
}

/// The rack and patch editor mutate the engine graph rather than the song, so
/// they are covered by the graph version instead.
#[test]
fn rack_module_edits_mark_the_project_dirty() {
    let (_engine, handle) = SynthEngine::new();
    let before = handle.state.shared_graph.version();

    handle
        .command_sender()
        .send(synth_engine::EngineCommand::AddInstrument {
            instrument: Box::new(synth_engine::instrument::Instrument::new(
                synth_core::InstrumentId::FIRST,
                "Lead",
            )),
        });

    assert_ne!(
        handle.state.shared_graph.version(),
        before,
        "an edit from the rack must advance the engine graph version",
    );
}

/// Sample edits live in the library, the third counter.
#[test]
fn sample_edits_mark_the_project_dirty() {
    use synth_sampler::{Sample, SampleLibrary, SampleMeta, SampleSource};

    let mut library = SampleLibrary::new();
    let before = library.revision();

    let _ = library.add(Sample::new(
        SampleMeta {
            id: synth_sampler::SampleId::new(0),
            name: "kick".to_string(),
            description: String::new(),
            sample_rate: synth_core::audio::DeviceSampleRate::new(44_100),
            channels: synth_core::ChannelCount::Mono,
            frame_count: synth_core::SampleCount::new(4),
            root_note: None,
            loop_region: None,
            crop: None,
            source: SampleSource::Generated,
        },
        vec![0.0_f32; 4].into(),
    ));

    assert_ne!(
        library.revision(),
        before,
        "an edit from the sample view must advance the library revision",
    );
}

/// Two independent edits must be distinguishable, so saving between them
/// establishes a baseline the second edit can be measured against.
#[test]
fn a_save_baseline_taken_between_edits_sees_only_the_later_edit() {
    let song = shared_song();

    let _ = song.write().create_track("First");
    let saved_baseline = song.revision();
    assert_eq!(
        song.revision(),
        saved_baseline,
        "nothing happened between the edit and the baseline",
    );

    let _ = song.write().create_track("Second");

    assert_ne!(
        song.revision(),
        saved_baseline,
        "the edit after the save must show as unsaved",
    );
}

/// Rendering, playback and analysis read the song without editing it. If any of
/// those paths advanced the revision, the project would look unsaved the moment
/// it was opened.
#[test]
fn reading_the_song_never_marks_the_project_dirty() {
    let song = shared_song();
    let _ = song.write().create_track("Lead");
    let baseline = song.revision();

    let _ = song.read().tracks().count();
    let _ = song.snapshot().tracks().count();
    let _ = song.try_read().map(|s| s.tracks().count());

    assert_eq!(song.revision(), baseline);
}

/// The counters must start from a defined point so a freshly created project is
/// clean rather than immediately dirty.
#[test]
fn fresh_state_starts_clean() {
    assert_eq!(shared_song().revision(), ContentRevision::INITIAL);
    assert_eq!(
        synth_sampler::SampleLibrary::new().revision(),
        ContentRevision::INITIAL,
    );
}
