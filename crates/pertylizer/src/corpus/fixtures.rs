//! The projects the reference corpus renders.
//!
//! Each fixture is built in code rather than hand-edited on disk, for three
//! reasons. A builder is reviewable — a diff shows "the filter cutoff moved",
//! not eight hundred lines of re-serialized JSON. It is regenerable, so a format
//! change reaches the corpus by re-running the generator instead of by hand
//! editing. And it is checkable: a test rebuilds every fixture and compares it
//! to the committed bytes, so the files on disk cannot drift from the code that
//! is supposed to describe them.
//!
//! The fixtures are deliberately small. A corpus case exists to isolate one
//! behaviour of the render core, and a four-minute demo song with thirty
//! instruments isolates nothing — when its render changes, nothing says which
//! subsystem moved. The bundled example projects under `assets/examples` remain
//! the realistic end-to-end material; these are the controlled ones.
//!
//! # Determinism
//!
//! Every fixture avoids the random-family modules. Their state is reproducible
//! (it derives from the voice index and module instance number, not from a
//! clock), so a noise source would still render identically twice — but it would
//! make each case's audio depend on the voice-allocation order as well as on the
//! behaviour under test, which is one variable too many for a baseline.

use std::path::{Path, PathBuf};

use synth_core::{BipolarValue, Cents, Gain, ModuleType, NormalizedValue, Semitones, VoiceCount};
use synth_engine::voice_allocator::{AllocationMode, StealingStrategy};
use synth_sequencer::{
    Duration as SeqDuration, InstrumentId, PatternTick, Pitch, Song, Tick, TrackId, TrackSend,
    Velocity,
};

use crate::patch::{InstrumentState, ModuleBuilder, ModuleState, Patch, PatchError};
use crate::project::{GlobalProjectState, ProjectFile, ReturnBusEffectsState};

use super::CorpusCaseId;

/// One buildable corpus input.
pub struct Fixture {
    /// The case this project belongs to.
    ///
    /// Held as text rather than as a [`CorpusCaseId`] because [`FIXTURES`] is a
    /// `const`, which a `String`-backed newtype cannot be. Use
    /// [`Fixture::case_id`] to get the typed form.
    pub case_id: &'static str,
    /// File name inside the corpus `projects/` directory.
    pub file_name: &'static str,
    /// Builds the project from nothing. Deterministic by construction: it reads
    /// no clock, no environment, and no file.
    pub build: fn() -> ProjectFile,
}

/// Every fixture, in case order.
pub const FIXTURES: &[Fixture] = &[
    Fixture {
        case_id: "CORPUS-0001",
        file_name: "subtractive-voice.ptz",
        build: subtractive_voice,
    },
    Fixture {
        case_id: "CORPUS-0002",
        file_name: "polyphonic-voice-stealing.ptz",
        build: polyphonic_voice_stealing,
    },
    Fixture {
        case_id: "CORPUS-0003",
        file_name: "mod-matrix.ptz",
        build: mod_matrix,
    },
    Fixture {
        case_id: "CORPUS-0004",
        file_name: "sends-returns-master.ptz",
        build: sends_returns_master,
    },
];

impl Fixture {
    /// The case this project belongs to, as an identifier rather than as text.
    #[must_use]
    pub fn case_id(&self) -> CorpusCaseId {
        CorpusCaseId::new_unchecked(self.case_id)
    }
}

/// A project that holds exactly `voices` notes at once, for cost measurement.
///
/// **This is not a corpus case, and deliberately so.** A corpus case pins a
/// *behaviour* that V2 must preserve or change; this pins nothing. It exists so
/// that P00A-T003 can measure cost as a function of polyphony without adding
/// eleven manifest entries whose behaviour claims nobody would ever check. It is
/// absent from [`FIXTURES`], so it is never written into the corpus directory
/// and never digested by the manifest test.
///
/// The patch is CORPUS-0001's — one sawtooth into one filter, one envelope —
/// so a cost difference between two voice counts is the cost of a voice and not
/// of a different sound. Every note starts at tick 0 and holds for two beats, so
/// the whole rendered window is spent at full polyphony rather than ramping into
/// it, and `max_voices` equals the note count so nothing is stolen.
///
/// Pitches walk up in semitones from C3. They differ because identical pitches
/// would let a future voice-deduplication optimization make this measure
/// something other than what it claims to.
#[must_use]
pub fn polyphony_probe(voices: u8) -> ProjectFile {
    let voices = voices.clamp(1, 128);
    let mut patch = Patch::new("Polyphony Probe");
    add_env_amp_out(&mut patch, 0.01, 0.10, 0.90, 0.30);
    add_saw_into_filter(&mut patch, 1_200.0, 0.10);

    let mut state = instrument(InstrumentId::FIRST, "Polyphony Probe", patch);
    state.max_voices = VoiceCount::new(voices);

    let notes: Vec<(u32, u8, SeqDuration)> = (0..voices)
        .map(|i| (0_u32, 48_u8.saturating_add(i), SeqDuration(1_920)))
        .collect();
    let (song, _) = one_pattern_song(
        "Polyphony Probe",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(vec![state], song, unity_global())
}

/// The fixture belonging to `case_id`.
#[must_use]
pub fn fixture(case_id: &CorpusCaseId) -> Option<&'static Fixture> {
    FIXTURES.iter().find(|f| f.case_id == case_id.as_str())
}

/// Directory holding the fixture projects inside the corpus directory.
pub const PROJECTS_SUBDIR: &str = "projects";

/// Build and write every fixture under `corpus_dir/projects/`, returning the
/// paths written in fixture order.
///
/// # Errors
///
/// Returns [`PatchError`] if the directory cannot be created or a project cannot
/// be serialized or written.
pub fn write_all(corpus_dir: &Path) -> Result<Vec<PathBuf>, PatchError> {
    let dir = corpus_dir.join(PROJECTS_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|e| PatchError::Io(e.to_string()))?;
    let mut written = Vec::with_capacity(FIXTURES.len());
    for fixture in FIXTURES {
        let path = dir.join(fixture.file_name);
        (fixture.build)().save(&path)?;
        written.push(path);
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

/// Envelope, amplifier, and stereo output, plus the two connections that gate
/// the amplifier and reach the output.
///
/// The caller adds the sound source and connects it to `amp-1.in`.
fn add_env_amp_out(patch: &mut Patch, attack: f32, decay: f32, sustain: f32, release: f32) {
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", attack)
            .param_f("decay", decay)
            .param_f("sustain", sustain)
            .param_f("release", release)
            .position(320.0, 200.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .position(480.0, 32.0)
            .build(),
    );
    patch.add_module(
        // Unity, not the 0.8 default: every stage of a reference patch is at
        // unity so the render's level is a property of the source, and a case
        // that wants headroom asks for it explicitly.
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .position(640.0, 32.0)
            .build(),
    );
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out", "out-1", "in");
}

/// A sawtooth oscillator into a low-pass filter, with the filter's cutoff CV
/// left unconnected so a case can drive it however it needs to.
fn add_saw_into_filter(patch: &mut Patch, cutoff_hz: f32, resonance: f32) {
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 1.0)
            .position(32.0, 32.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Filter)
            .filter_mode("lowpass")
            .param_f("cutoff", cutoff_hz)
            .param_f("resonance", resonance)
            // The envelope reaches the amplifier, not the cutoff: a case that
            // wants a filter sweep routes one explicitly, so this stage is a
            // fixed colour rather than a second envelope in disguise.
            .param_f("env_amt", 0.0)
            .position(192.0, 32.0)
            .build(),
    );
    patch.add_connection("osc-1", "out", "flt-1", "in");
    patch.add_connection("flt-1", "out", "amp-1", "in");
}

/// An instrument carrying `patch`, with every field of
/// [`crate::project::default_instrument_state`] except the ones a corpus case
/// has a reason to set.
fn instrument(id: InstrumentId, name: &str, patch: Patch) -> InstrumentState {
    InstrumentState {
        id,
        name: name.to_string(),
        channel: 1,
        volume: Gain::UNITY,
        pan: BipolarValue::CENTER,
        muted: false,
        solo: false,
        key_range: (0, 127),
        transpose: Semitones::ZERO,
        oversampling: 1,
        category: 0,
        description: String::new(),
        color: None,
        allocation_mode: AllocationMode::default(),
        stealing_strategy: StealingStrategy::default(),
        unison_detune: Cents::new(10.0),
        unison_spread: NormalizedValue::MIN,
        max_voices: VoiceCount::OCTO,
        velocity_amp_sensitivity: NormalizedValue::MAX,
        velocity_filter_sensitivity: NormalizedValue::MIN,
        sidechain_source_id: None,
        patch,
    }
}

/// Global project state at unity master gain.
///
/// The shipped default is 0.8, which is a sensible starting point for a person
/// but a silent 2 dB in a baseline: it scales every case by a constant that has
/// nothing to do with what the case tests, and it would have to be undone by
/// hand before comparing a corpus render against anything measured elsewhere.
fn unity_global() -> GlobalProjectState {
    GlobalProjectState {
        master_volume: Gain::UNITY,
        ..GlobalProjectState::default()
    }
}

/// A MIDI note number as a [`Pitch`], falling back to middle C.
///
/// Every call site passes a literal inside 0..=127, so the fallback is
/// unreachable; it exists because this is library code and a panicking
/// conversion has no place in it.
fn pitch(midi: u8) -> Pitch {
    Pitch::new(midi).unwrap_or(Pitch::MIDDLE_C)
}

/// Add `notes` — `(start, midi, duration)` — to a fresh pattern on one track
/// playing `instrument`, and place it at tick 0.
///
/// Returns the song with exactly one pattern, one track, and one placement, so
/// a case's arrangement is as small as the thing it is testing, plus that
/// track's id. The id is returned rather than assumed: a caller that guessed
/// [`TrackId(0)`](TrackId) and then reached for the track through
/// `track_mut(..)` would silently build a song without whatever it meant to add
/// the moment the sequencer's numbering changed, and every test here would still
/// pass.
fn one_pattern_song(
    name: &str,
    instrument: InstrumentId,
    length: SeqDuration,
    notes: &[(u32, u8, SeqDuration)],
) -> (Song, TrackId) {
    let mut song = Song::new(name);
    let pattern_id = song.create_pattern(length);
    if let Some(pattern) = song.pattern_mut(pattern_id) {
        for (start, midi, duration) in notes {
            let note_id = pattern.add_note(PatternTick(*start), pitch(*midi), Velocity::F);
            if let Some(note) = pattern.note_mut(note_id) {
                note.duration = Some(*duration);
            }
        }
    }
    let track_id = song.create_track("T1");
    if let Some(track) = song.track_mut(track_id) {
        track.instrument = instrument;
    }
    song.place_pattern(pattern_id, track_id, Tick(0));
    (song, track_id)
}

/// Assemble a project from one instrument and a song, at the format version the
/// build writes.
fn project(
    instruments: Vec<InstrumentState>,
    song: Song,
    global: GlobalProjectState,
) -> ProjectFile {
    ProjectFile::new(
        instruments,
        InstrumentId::FIRST.as_u64(),
        None,
        song,
        global,
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0001 — basic subtractive voice
// ---------------------------------------------------------------------------

/// Sawtooth into a static low-pass, gated by one envelope; four separated
/// quarter notes so each note-on is its own measurable onset.
fn subtractive_voice() -> ProjectFile {
    let mut patch = Patch::new("Subtractive Voice");
    add_env_amp_out(&mut patch, 0.01, 0.20, 0.60, 0.25);
    add_saw_into_filter(&mut patch, 1_200.0, 0.30);

    // Separated rather than legato: a gap between notes makes the release tail
    // and the next attack independently visible in an envelope comparison.
    let notes = [
        (0, 57, SeqDuration(720)),
        (960, 60, SeqDuration(720)),
        (1920, 64, SeqDuration(720)),
        (2880, 69, SeqDuration(720)),
    ];
    let (song, _) = one_pattern_song(
        "Subtractive Voice",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(
        vec![instrument(InstrumentId::FIRST, "Subtractive Voice", patch)],
        song,
        unity_global(),
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0002 — polyphony and voice stealing
// ---------------------------------------------------------------------------

/// Eight overlapping sustained notes against a four-voice instrument, so the
/// allocator must steal four times inside the rendered window.
///
/// The long release matters: a stolen voice is only audible as a difference if
/// the note it was playing had a tail to cut off.
fn polyphonic_voice_stealing() -> ProjectFile {
    let mut patch = Patch::new("Stealing Pad");
    add_env_amp_out(&mut patch, 0.05, 0.30, 0.70, 1.20);
    add_saw_into_filter(&mut patch, 900.0, 0.20);

    let mut state = instrument(InstrumentId::FIRST, "Stealing Pad", patch);
    state.max_voices = VoiceCount::new(4);

    // Each note starts an eighth apart and holds for two whole beats, so from
    // the fifth onset there are always more sounding notes than voices.
    let notes: Vec<(u32, u8, SeqDuration)> = [48_u8, 55, 60, 64, 67, 72, 76, 79]
        .iter()
        .enumerate()
        .map(|(i, midi)| (i as u32 * 480, *midi, SeqDuration(1_920)))
        .collect();
    let (song, _) = one_pattern_song(
        "Stealing Pad",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(vec![state], song, unity_global())
}

// ---------------------------------------------------------------------------
// CORPUS-0003 — Mod Matrix
// ---------------------------------------------------------------------------

/// One Mod Matrix slot routing an LFO to the filter cutoff.
///
/// The LFO is not wired to the filter's `cutoff_cv` port: the point of the case
/// is the matrix path, and a patch cable would produce the same audible sweep
/// through a different mechanism.
///
/// # Why the LFO's phase is note-locked without a `retrigger` setting
///
/// The LFO lives in the voice graph, and V1 zeroes a voice's whole DSP state
/// when the allocator hands the voice out — `Lfo::reset` sets the phase to zero
/// — so every note starts its sweep from the same point. That is the property
/// CORPUS-0003-P2 claims, and it is a property of voice allocation.
///
/// The module's own `retrigger` parameter is deliberately *not* set here,
/// because setting it would say something untrue: `Lfo::process` only acts on
/// it while the module's `retrigger` **gate port** is connected, and nothing in
/// this patch drives that port. With the port unconnected the parameter is
/// inert — rendering this case with `retrigger` at 0.0 and at 1.0 produces
/// byte-identical WAVs — so carrying it would document a mechanism the case
/// does not use.
fn mod_matrix() -> ProjectFile {
    let mut patch = Patch::new("Mod Matrix Sweep");
    add_env_amp_out(&mut patch, 0.01, 0.15, 0.80, 0.30);
    add_saw_into_filter(&mut patch, 800.0, 0.45);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Lfo)
            .waveform("triangle")
            .param_f("rate", 2.0)
            .param_f("depth", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::ModMatrix)
            .param_choice("grid_size", "1x1")
            .param_choice("slot_1_source", "lfo-1.out")
            .param_choice("slot_1_dest", "flt-1.cutoff")
            .param_f("slot_1_amount", 0.7)
            .param_f("slot_1_enabled", 1.0)
            .build(),
    );

    let notes = [(0, 45, SeqDuration(1_800)), (1_920, 52, SeqDuration(1_800))];
    let (song, _) = one_pattern_song(
        "Mod Matrix Sweep",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(
        vec![instrument(InstrumentId::FIRST, "Mod Matrix Sweep", patch)],
        song,
        unity_global(),
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0004 — sends, returns, and master effects
// ---------------------------------------------------------------------------

/// A dry track sending into a reverb return, with a compressor on the master.
///
/// Three signal paths meet here that the voice cases never touch: the send tap,
/// the return bus's own effect chain, and the master chain applied to the summed
/// mix. The reverb's `mix` is full wet, so the return contributes only its own
/// output and the dry/wet balance is the send level alone.
fn sends_returns_master() -> ProjectFile {
    let mut patch = Patch::new("Send Source");
    add_env_amp_out(&mut patch, 0.002, 0.12, 0.0, 0.08);
    add_saw_into_filter(&mut patch, 2_000.0, 0.10);

    let notes = [
        (0, 60, SeqDuration(240)),
        (960, 60, SeqDuration(240)),
        (1_920, 60, SeqDuration(240)),
        (2_880, 60, SeqDuration(240)),
    ];
    let (mut song, track_id) = one_pattern_song(
        "Send Source",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    let return_id = song.create_return_bus("Reverb");
    if let Some(track) = song.track_mut(track_id) {
        track
            .sends
            .push(TrackSend::new(return_id, NormalizedValue::new(0.6)));
    }

    let global = GlobalProjectState {
        return_bus_effects: vec![ReturnBusEffectsState {
            id: return_id.0,
            effects: vec![reverb_effect()],
        }],
        master_effects: vec![master_compressor()],
        ..unity_global()
    };
    project(
        vec![instrument(InstrumentId::FIRST, "Send Source", patch)],
        song,
        global,
    )
}

/// The fully-wet plate on the return bus.
fn reverb_effect() -> ModuleState {
    ModuleBuilder::new(1, ModuleType::Reverb)
        .param_f("mix", 1.0)
        .param_f("room_size", 0.6)
        .param_f("decay", 0.5)
        .param_f("damping", 0.4)
        .param_f("pre_delay", 0.02)
        .build()
}

/// A compressor that actually engages on the fixture's level, so the master
/// chain is measurable rather than a pass-through.
fn master_compressor() -> ModuleState {
    ModuleBuilder::new(1, ModuleType::Compressor)
        .param_f("threshold", -24.0)
        .param_f("ratio", 4.0)
        .param_f("attack", 5.0)
        .param_f("release", 120.0)
        .param_f("makeup", 3.0)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two builds of the same fixture have to serialize to identical bytes, or
    /// the committed corpus would churn on every regeneration and its digests
    /// would mean nothing.
    #[test]
    fn every_fixture_builds_reproducibly() {
        for fixture in FIXTURES {
            let first = serde_json::to_vec_pretty(&(fixture.build)()).expect("serialize");
            let second = serde_json::to_vec_pretty(&(fixture.build)()).expect("serialize");
            assert_eq!(
                first, second,
                "{} does not build reproducibly",
                fixture.case_id
            );
        }
    }

    /// A fixture whose project cannot be read back is not an input to anything.
    #[test]
    fn every_fixture_round_trips_through_the_project_format() {
        for fixture in FIXTURES {
            let built = (fixture.build)();
            let json = serde_json::to_string(&built).expect("serialize");
            let reloaded: ProjectFile = serde_json::from_str(&json).expect("parse");
            assert_eq!(reloaded.file_type, "project", "{}", fixture.case_id);
            assert_eq!(
                reloaded.version,
                ProjectFile::FORMAT_VERSION,
                "{}",
                fixture.case_id
            );
            assert!(
                !reloaded.instruments.is_empty(),
                "{} has no instrument",
                fixture.case_id
            );
        }
    }

    /// Every fixture must actually play something. A corpus entry that renders
    /// silence passes every comparison and proves nothing.
    #[test]
    fn every_fixture_has_a_placed_pattern_with_notes() {
        for fixture in FIXTURES {
            let built = (fixture.build)();
            let notes: usize = built
                .song
                .patterns()
                .map(|pattern| pattern.notes().len())
                .sum();
            assert!(notes > 0, "{} has no notes", fixture.case_id);
            assert!(
                !built.song.arrangement().is_empty(),
                "{} places no pattern",
                fixture.case_id
            );
        }
    }

    /// [`Patch::add_connection`] validates nothing, so a mistyped module id
    /// produces a cable that goes nowhere and the case renders a quieter — or
    /// silent — signal that every other test here still accepts.
    #[test]
    fn every_fixture_connection_names_modules_that_exist() {
        for fixture in FIXTURES {
            let built = (fixture.build)();
            for instrument in &built.instruments {
                let ids: Vec<&str> = instrument
                    .patch
                    .modules
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect();
                for connection in &instrument.patch.connections {
                    for (id, end) in [(&connection.from.0, "source"), (&connection.to.0, "target")]
                    {
                        assert!(
                            ids.contains(&id.as_str()),
                            "{}: connection {end} {id:?} is not a module in the patch",
                            fixture.case_id
                        );
                    }
                }
            }
        }
    }

    /// File names are the corpus's on-disk identity; a collision would silently
    /// make two cases share one input.
    #[test]
    fn fixture_case_ids_and_file_names_are_unique() {
        for (i, a) in FIXTURES.iter().enumerate() {
            for b in &FIXTURES[i + 1..] {
                assert_ne!(a.case_id, b.case_id);
                assert_ne!(a.file_name, b.file_name);
            }
        }
    }

    /// The stealing case only tests stealing if the arrangement actually asks
    /// for more voices than the instrument has.
    #[test]
    fn the_stealing_fixture_oversubscribes_its_voices() {
        let built = polyphonic_voice_stealing();
        let voices = built.instruments[0].max_voices.as_usize();
        let notes: usize = built
            .song
            .patterns()
            .map(|pattern| pattern.notes().len())
            .sum();
        assert!(
            notes > voices,
            "{notes} notes against {voices} voices does not force a steal"
        );
    }
}
