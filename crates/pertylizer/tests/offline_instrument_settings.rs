//! The offline renderer must honour every per-instrument setting the project
//! carries.
//!
//! It did not. `OfflineEngineSession` built each instrument with no
//! `AllocatorConfig` and replayed only volume, pan, and solo, so `max_voices`,
//! `allocation_mode`, `stealing_strategy`, the unison pair, `transpose`,
//! `key_range`, `oversampling`, and both velocity sensitivities were left at
//! their engine defaults. Nothing warned, because nothing was missing — the
//! values were simply never sent.
//!
//! That is the same shape as the `analyze_*` snapshot bug: an offline reader
//! disagreeing with the live engine while looking entirely healthy. It is worse
//! here, because `pertylizer render` is what produces the Sound Core V2
//! reference corpus, and a baseline of audio the live engine never plays is
//! worse than no baseline.
//!
//! Each test below changes exactly one field in a saved project and asserts the
//! rendered bytes change. They are deliberately blunt: the point is not what the
//! field does to the sound, only that it reaches the renderer at all.
//!
//! Two of them are the other way round, and say so in their own doc comments:
//! `velocity_filter_sensitivity` is inert in V1, so the assertion is that it
//! makes *no* difference, and it will fail the day someone implements it.
//!
//! # Checking that these tests still bite
//!
//! Keep this file and revert only the production code, then run it. `git stash`
//! does not do that on a committed branch — it reverts the tests along with the
//! fix, and everything passes for the wrong reason:
//!
//! ```bash
//! git checkout main -- crates/pertylizer/src/audio/arrangement_render.rs \
//!     crates/synth_engine/src/synth_engine.rs crates/synth_engine/src/state.rs
//! cargo test -p pertylizer --test offline_instrument_settings   # 11 of 13 fail
//! git checkout HEAD -- crates/pertylizer/src/audio/arrangement_render.rs \
//!     crates/synth_engine/src/synth_engine.rs crates/synth_engine/src/state.rs
//! ```
//!
//! Only with the production files committed — `git checkout HEAD --` restores
//! what is in the commit, not what is in the working tree.
//!
//! Covered: `max_voices`, `allocation_mode`, `stealing_strategy`,
//! `unison_detune`, `unison_spread`, `transpose`, `key_range`, `oversampling`,
//! `velocity_amp_sensitivity`, the project's global `glide_time`, and the
//! sidechain source. Not covered by a render-difference test:
//! `velocity_filter_sensitivity` (inert, see above).
//!
//! The engine side of glide — that an instrument created *after* the global
//! value was set inherits it — is tested in `synth_engine`, because it is a
//! live-engine bug that this file's offline fix would otherwise mask.

mod common;

use std::path::Path;
use std::sync::Arc;

use synth_core::{InstrumentId, NormalizedValue, Seconds, Semitones, VoiceCount};
use synth_engine::voice_allocator::{AllocationMode, StealingStrategy};
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SharedSong, Song, Tick, Velocity,
};

use pertylizer::patch::{InstrumentState, ModuleBuilder, Patch};
use pertylizer::project::ProjectFile;
use pertylizer::project_apply::{ProjectBuildOptions, save_project_to};
use pertylizer::render::{MixSelection, RenderCommand, WavFormat, run_render_command};
use synth_core::ModuleType;

use common::{TEST_SR, setup_with_patch, sustain_patch};

/// Six overlapping sustained notes, spread across three octaves.
///
/// Overlapping so polyphony settings have something to do; spread so a transpose
/// or a key range has somewhere to move notes to and out of.
fn overlapping_song() -> Arc<SharedSong> {
    let mut song = Song::new("OfflineInstrumentSettings");
    let pattern_id = song.create_pattern(SeqDuration::WHOLE);
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        for (step, midi) in [36_u8, 48, 55, 60, 67, 72].iter().enumerate() {
            let note = pattern.add_note(
                PatternTick(step as u32 * 240),
                Pitch::new(*midi).expect("valid MIDI note"),
                Velocity::MF,
            );
            if let Some(note) = pattern.note_mut(note) {
                note.duration = Some(SeqDuration(1_920));
            }
        }
    }
    let track_id = song.create_track("T1");
    if let Some(track) = song.track_mut(track_id) {
        track.instrument = InstrumentId::new(0);
    }
    assert!(song.place_pattern(pattern_id, track_id, Tick(0)));
    Arc::new(SharedSong::new(song))
}

/// The shared `sustain_patch` with a low-pass filter spliced in between the
/// oscillator and the amplifier.
///
/// A filter is not decoration here: `velocity_filter_sensitivity` modulates a
/// cutoff, so on a patch with no filter that setting is inert and its test would
/// pass whether or not the value ever reached the renderer. The cutoff sits well
/// inside the sawtooth's harmonic series so the modulation has room to move.
fn filtered_sustain_patch() -> Patch {
    let mut patch = sustain_patch();
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .filter_mode("lowpass")
            .param_f("cutoff", 1_200.0)
            .param_f("resonance", 0.3)
            .build(),
    );
    // `add_connection` does not replace, so the direct oscillator-to-amplifier
    // cable the shared patch built has to go first.
    patch
        .connections
        .retain(|c| !(c.from.0 == "osc-1" && c.to.0 == "amp-1"));
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
    patch
}

/// Save the fixture project to `path`, with `adjust` applied to it first.
///
/// Written through the real save path and then edited on disk, so the test
/// exercises exactly what a user's project file would carry rather than a
/// hand-built in-memory state the loader never sees.
fn write_project(path: &Path, adjust: impl FnOnce(&mut ProjectFile)) {
    let rig = setup_with_patch(&filtered_sustain_patch());
    save_project_to(
        path,
        &rig.session,
        &overlapping_song(),
        &rig.sample_library,
        ProjectBuildOptions::default(),
    )
    .expect("saving the fixture project should succeed");

    let mut project = ProjectFile::load(path).expect("reload the saved project");
    adjust(&mut project);
    project.save(path).expect("save the adjusted project");
}

/// Lift an instrument-level adjustment to a whole-project one.
fn on_instrument(adjust: impl FnOnce(&mut InstrumentState)) -> impl FnOnce(&mut ProjectFile) {
    move |project| {
        adjust(
            project
                .instruments
                .first_mut()
                .expect("the fixture has one instrument"),
        );
    }
}

/// Render `path` and return the WAV's bytes.
fn render(path: &Path, output: &Path) -> Vec<u8> {
    run_render_command(&RenderCommand {
        input: path.to_path_buf(),
        output: output.to_path_buf(),
        sample_rate: TEST_SR,
        format: WavFormat::Float32,
        seconds: Seconds::new(1.5),
        tail: Seconds::new(0.5),
        result_json: None,
        mix: MixSelection::default(),
        argv: Vec::new(),
    })
    .expect("render succeeds");
    std::fs::read(output).unwrap_or_else(|e| panic!("read {}: {e}", output.display()))
}

/// Render the fixture twice, once with each whole-project adjustment, and return
/// whether the two sets of bytes differ.
fn project_renders_differ(
    left: impl FnOnce(&mut ProjectFile),
    right: impl FnOnce(&mut ProjectFile),
) -> bool {
    let dir = tempfile::tempdir().expect("tempdir");
    let left_project = dir.path().join("left.ptz");
    let right_project = dir.path().join("right.ptz");
    write_project(&left_project, left);
    write_project(&right_project, right);
    render(&left_project, &dir.path().join("left.wav"))
        != render(&right_project, &dir.path().join("right.wav"))
}

/// [`project_renders_differ`] for two adjustments to the one instrument.
fn renders_differ(
    left: impl FnOnce(&mut InstrumentState),
    right: impl FnOnce(&mut InstrumentState),
) -> bool {
    project_renders_differ(on_instrument(left), on_instrument(right))
}

/// The control. If two renders of the *same* settings differed, every assertion
/// below would pass for the wrong reason — and the corpus's determinism claims
/// would be false besides.
#[test]
fn identical_settings_render_identically() {
    assert!(
        !renders_differ(|_| {}, |_| {}),
        "two renders of one project must be byte-identical"
    );
}

/// Voice ceiling. This is the one the V2 corpus depends on: its
/// voice-stealing case is eight notes against four voices, and before the fix it
/// rendered as eight notes against the default eight — no stealing at all, with
/// every preserve claim about stealing satisfied vacuously.
#[test]
fn max_voices_reaches_the_offline_renderer() {
    assert!(
        renders_differ(
            |i| i.max_voices = VoiceCount::new(2),
            |i| i.max_voices = VoiceCount::new(8),
        ),
        "a two-voice instrument must not render like an eight-voice one"
    );
}

/// Allocation mode. A monophonic instrument playing a six-note chord sounds
/// nothing like a polyphonic one.
#[test]
fn allocation_mode_reaches_the_offline_renderer() {
    assert!(
        renders_differ(
            |i| i.allocation_mode = AllocationMode::Mono,
            |i| i.allocation_mode = AllocationMode::Polyphonic,
        ),
        "a monophonic instrument must not render like a polyphonic one"
    );
}

/// Stealing strategy, which only shows up once the ceiling is low enough to
/// force a steal — hence the shared two-voice setting on both sides.
///
/// `None` against `Oldest` rather than a subtler pair: V1's `Quietest` is
/// currently "oldest releasing voice, then oldest active"
/// (`voice_allocator.rs`'s own "For now" comment), so it renders identically to
/// `Oldest` on material with nothing in release. Asserting those two differ
/// would be asserting behaviour V1 does not have, and the assertion here is
/// about the setting reaching the renderer.
#[test]
fn stealing_strategy_reaches_the_offline_renderer() {
    assert!(
        renders_differ(
            |i| {
                i.max_voices = VoiceCount::new(2);
                i.stealing_strategy = StealingStrategy::None;
            },
            |i| {
                i.max_voices = VoiceCount::new(2);
                i.stealing_strategy = StealingStrategy::Oldest;
            },
        ),
        "refusing to steal must not render the same as stealing the oldest voice"
    );
}

/// Transpose. An octave of it is unmissable in the audio and was invisible in
/// the render.
#[test]
fn transpose_reaches_the_offline_renderer() {
    assert!(
        renders_differ(
            |i| i.transpose = Semitones::new(0.0),
            |i| i.transpose = Semitones::new(12.0),
        ),
        "an octave of transpose must change the render"
    );
}

/// Key range. Narrowing it to a single note silences five of the six.
#[test]
fn key_range_reaches_the_offline_renderer() {
    assert!(
        renders_differ(|i| i.key_range = (0, 127), |i| i.key_range = (60, 60)),
        "a one-note key range must silence the notes outside it"
    );
}

/// Velocity to amplitude. The fixture plays at mezzo-forte, so a velocity-
/// insensitive instrument is audibly louder than a fully sensitive one.
#[test]
fn velocity_amp_sensitivity_reaches_the_offline_renderer() {
    assert!(
        renders_differ(
            |i| i.velocity_amp_sensitivity = NormalizedValue::MIN,
            |i| i.velocity_amp_sensitivity = NormalizedValue::MAX,
        ),
        "velocity sensitivity must change the level of a mezzo-forte note"
    );
}

/// Velocity to filter cutoff is **inert in V1**, and this test says so rather
/// than pretending otherwise.
///
/// The value is stored on the instrument, mirrored into the snapshot, persisted
/// in the project, and exposed by the GUI and MCP — and read by nothing that
/// produces audio. Grepping the workspace finds the setter, the getter, the
/// snapshot field, and no consumer. So it is inert live as well as offline, and
/// a render-difference assertion would be asserting a feature that does not
/// exist.
///
/// Written as a characterization test so it fails the day someone implements it.
/// That day this file needs a real assertion, the offline path needs checking,
/// and the corpus needs a `change` claim — because a V2 that implements it
/// correctly will differ from V1 by design.
#[test]
fn velocity_filter_sensitivity_is_inert_in_v1() {
    assert!(
        !renders_differ(
            |i| i.velocity_filter_sensitivity = NormalizedValue::MIN,
            |i| i.velocity_filter_sensitivity = NormalizedValue::MAX,
        ),
        "velocity-to-cutoff now changes the render — see this test's doc comment"
    );
}

/// Oversampling, which changes the anti-aliasing of everything the voice does.
#[test]
fn oversampling_reaches_the_offline_renderer() {
    assert!(
        renders_differ(|i| i.oversampling = 1, |i| i.oversampling = 4),
        "4x oversampling must not render identically to none"
    );
}

/// Unison detune, which needs unison mode to mean anything — so both sides are
/// in it and only the detune differs.
#[test]
fn unison_settings_reach_the_offline_renderer() {
    assert!(
        renders_differ(
            |i| {
                i.allocation_mode = AllocationMode::Unison;
                i.unison_detune = synth_core::Cents::new(0.0);
            },
            |i| {
                i.allocation_mode = AllocationMode::Unison;
                i.unison_detune = synth_core::Cents::new(50.0);
            },
        ),
        "unison detune must change the render of a unison instrument"
    );
}

/// Unison stereo spread, which is a separate field from the detune above and
/// travels the same way — through the allocator config rather than a parameter.
#[test]
fn unison_spread_reaches_the_offline_renderer() {
    assert!(
        project_renders_differ(
            on_instrument(|i| {
                i.allocation_mode = AllocationMode::Unison;
                i.unison_detune = synth_core::Cents::new(25.0);
                i.unison_spread = NormalizedValue::MIN;
            }),
            on_instrument(|i| {
                i.allocation_mode = AllocationMode::Unison;
                i.unison_detune = synth_core::Cents::new(25.0);
                i.unison_spread = NormalizedValue::MAX;
            }),
        ),
        "unison stereo spread must change the render of a unison instrument"
    );
}

/// Give the project a second instrument and a track for it, and put a
/// sidechained compressor on the first.
///
/// The compressor ducks on whatever the sidechain source plays, so with a source
/// set the render differs from one without — provided the source is actually
/// audible, which the second track makes it.
fn add_sidechain_rig(project: &mut ProjectFile) {
    let source_id = InstrumentId::new(1);

    // The ducked instrument: a compressor at the end of the chain, sidechain
    // enabled, threshold well under the fixture's level so it engages.
    if let Some(target) = project.instruments.first_mut() {
        target.patch.add_module(
            ModuleBuilder::new(1, ModuleType::Compressor)
                .param_f("sidechain", 1.0)
                .param_f("threshold", -40.0)
                .param_f("ratio", 12.0)
                .param_f("attack", 1.0)
                .param_f("release", 200.0)
                .build(),
        );
        target.patch.settings.effect_chain_order = vec!["cmp-1".to_string()];
    }

    // The source: a copy of the same instrument under a new id, so it makes
    // sound without needing a second patch.
    let mut source = project
        .instruments
        .first()
        .expect("the fixture has one instrument")
        .clone();
    source.id = source_id;
    source.name = "Sidechain Source".to_string();
    source.patch.settings.effect_chain_order.clear();
    source
        .patch
        .modules
        .retain(|m| m.module_type != ModuleType::Compressor);
    project.instruments.push(source);

    // A track for it, playing the pattern the fixture already has.
    let pattern_id = project
        .song
        .patterns()
        .next()
        .expect("the fixture has a pattern")
        .id;
    let track_id = project.song.create_track("Source");
    if let Some(track) = project.song.track_mut(track_id) {
        track.instrument = source_id;
    }
    assert!(project.song.place_pattern(pattern_id, track_id, Tick(0)));
}

/// The sidechain source travels through its own `EngineCommand`, not through the
/// instrument-parameter list, so it is the one setting the parameter sweep could
/// not have covered by construction.
#[test]
fn sidechain_source_reaches_the_offline_renderer() {
    assert!(
        project_renders_differ(
            |p| {
                add_sidechain_rig(p);
                if let Some(i) = p.instruments.first_mut() {
                    i.sidechain_source_id = None;
                }
            },
            |p| {
                add_sidechain_rig(p);
                if let Some(i) = p.instruments.first_mut() {
                    i.sidechain_source_id = Some(InstrumentId::new(1).as_u64());
                }
            },
        ),
        "a sidechained compressor with a source must not render like one without"
    );
}

/// Glide is the setting the first fix missed. It is global in the project and
/// per-instrument in the engine, so it reaches no instrument snapshot — which is
/// how it survived a sweep that went field by field through one.
///
/// The GUI's Export WAV path sent it all along (`export.rs`), so before this the
/// same project exported one way and rendered another.
#[test]
fn global_glide_time_reaches_the_offline_renderer() {
    assert!(
        project_renders_differ(
            |p| {
                p.global.glide_time = Seconds::ZERO;
                on_instrument(|i| i.allocation_mode = AllocationMode::Legato)(p);
            },
            |p| {
                p.global.glide_time = Seconds::new(0.4);
                on_instrument(|i| i.allocation_mode = AllocationMode::Legato)(p);
            },
        ),
        "a 400 ms glide must not render like no glide at all"
    );
}
