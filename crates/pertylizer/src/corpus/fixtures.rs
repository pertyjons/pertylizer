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

use synth_core::{
    BipolarValue, Bpm, Cents, Gain, ModuleType, NormalizedValue, Semitones, VoiceCount,
};
use synth_engine::voice_allocator::{AllocationMode, StealingStrategy};
use synth_sequencer::{
    Duration as SeqDuration, InstrumentId, PatternTick, Pitch, Song, Tick, TrackId, TrackSend,
    Velocity,
};

use crate::patch::{InstrumentState, ModuleBuilder, ModuleState, ParamValue, Patch, PatchError};
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
    Fixture {
        case_id: "CORPUS-0005",
        file_name: "instrument-inserts.ptz",
        build: instrument_inserts,
    },
    Fixture {
        case_id: "CORPUS-0006",
        file_name: "keyboard-panner-stereo.ptz",
        build: keyboard_panner_stereo,
    },
    Fixture {
        case_id: "CORPUS-0007",
        file_name: "yams-control.ptz",
        build: yams_control,
    },
    Fixture {
        case_id: "CORPUS-0008",
        file_name: "yams-audio-script.ptz",
        build: yams_audio_script,
    },
    Fixture {
        case_id: "CORPUS-0009",
        file_name: "tempo-map-arrangement.ptz",
        build: tempo_map_arrangement,
    },
    Fixture {
        case_id: "CORPUS-0010",
        file_name: "shared-instrument-tracks.ptz",
        build: shared_instrument_tracks,
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

/// Turn off an oscillator's note-on phase randomization.
///
/// **The parameter defaults to full randomization** (`uni_phase`, descriptor
/// default 1.0, and `Oscillator::unison_phase_random` is initialized to
/// `NormalizedValue::MAX`), and `Oscillator::set_voice_index` seeds the
/// generator from the voice index. The phase a note starts at is therefore a
/// function of which voice the allocator handed it, deterministic but
/// allocation-order dependent — the same class of variable this module's own
/// header says a fixture avoids by staying away from the random-family modules.
/// It is on in every oscillator unless a patch says otherwise.
///
/// A case that needs its audio to depend only on the behaviour under test calls
/// this. It is not applied through [`add_saw_into_filter`], because the four
/// fixtures that predate it are committed with their digests and pinned by
/// EVD-0001 through EVD-0003; changing them is a decision for P00A-T001 rather
/// than a side effect of adding a case.
///
/// Does nothing if `module_id` is not in the patch. That cannot happen from the
/// call sites here, which pass an id they just added, and a panicking lookup has
/// no place in library code.
fn silence_phase_randomization(patch: &mut Patch, module_id: &str) {
    if let Some(module) = patch.modules.iter_mut().find(|m| m.id == module_id) {
        module
            .parameters
            .insert("uni_phase".to_string(), ParamValue::Float(0.0));
    }
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
    silence_phase_randomization(&mut patch, "osc-1");

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
    silence_phase_randomization(&mut patch, "osc-1");

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
    silence_phase_randomization(&mut patch, "osc-1");
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
    silence_phase_randomization(&mut patch, "osc-1");

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

// ---------------------------------------------------------------------------
// CORPUS-0005 — instrument insert effects
// ---------------------------------------------------------------------------

/// An instrument carrying its own insert chain: distortion into delay.
///
/// # What this case pins that CORPUS-0004 does not
///
/// CORPUS-0004's effects live on a return bus and on the master, so they see a
/// signal that has already left the instrument. An insert chain sits *inside*
/// the instrument, between voice summing and the mixer. Four properties follow
/// from that position rather than from either effect's DSP, and each is
/// measured against a counterfactual in `EVD-0004` under
/// `plans/v2/evidence/phase-00a/` — a repository path rather than a link,
/// because rustdoc's output cannot reach outside the generated docs:
///
/// - **The chain runs on the summed voices, not per voice.** Rendering the dyad
///   through the chain differs from summing two single-note renders of the same
///   chain, which is what a per-voice chain would have produced. With only the
///   clipper in the chain that difference is 3.1 dB below the case's own RMS,
///   against a null control — the same construction with an empty chain — at
///   −147 dB relative, which is floating-point rounding.
/// - **Chain state is shared across voices.** With only the delay in the chain
///   the difference is still 35 dB below the case's RMS, and 112 dB above the
///   null control. Two notes can only interact inside a delay whose line they
///   share, through the soft clip on its feedback write (`effects/delay.rs`);
///   per-voice delay lines would land at the control.
/// - **Chain state outlives the notes that produced it.** The delay's repeats
///   fill the gap between the dyad and the isolated note, when every voice has
///   been released, and run 2.3 s past the final note-off — from −14.8 dBFS at
///   1.2 s down to −70.7 dBFS at 3.4 s, against a control with both inserts
///   removed that is digital silence from 1.7 s on.
/// - **The authored order is load-bearing.** Distortion into delay gives clean
///   repeats of a clipped signal; delay into distortion clips the sum of the
///   repeats. Reversing the two moves the render's RMS by 5.97 dB. The order
///   lives in `patch.settings.effect_chain_order` rather than in the module
///   list, so this case is also the one that fails if that field stops being
///   honoured.
///
/// # Why this fixture turns off phase randomization
///
/// The first three claims are measured by comparing a chord rendered whole
/// against its notes rendered separately and summed. That construction is only
/// valid if the voice path itself is additive, and with the oscillator's shipped
/// default it is not: `uni_phase` defaults to 1.0 and the generator behind it is
/// seeded from the voice index, so the same note starts at a different phase
/// depending on which voice the allocator handed it. A first attempt at these
/// measurements read that as a sequencer defect and withdrew two claims over it.
/// The null control is what exposed the mistake, and
/// [`silence_phase_randomization`] is what removes the variable. With it off,
/// reversing the two notes in the pattern's note list renders bit-identically.
///
/// Each figure above is a V1 measurement taken when the case was authored, not
/// a tolerance: the manifest's claims are what a comparison is judged against,
/// and these numbers exist so that a reader can tell a claim with margin from
/// one that would survive on rounding. The recipe that produced them is in the
/// corpus README under *Checking that a case tests what it claims*.
///
/// # Why the order is written out rather than left to fall out
///
/// V1 appends any chain module missing from `effect_chain_order` after the ones
/// that are listed, with a warning. That recovery is fine for a user's project
/// and wrong for a reference: the case would then pin whatever order the
/// append happened to produce, and it would keep passing if the field were
/// ignored entirely. Both effects are named explicitly so the order is authored
/// data that a comparison can hold V2 to.
fn instrument_inserts() -> ProjectFile {
    let mut patch = Patch::new("Insert Chain");
    // A fast, fully-decaying envelope: the sustain stage is what would otherwise
    // mask the delay's repeats under the note that produced them.
    add_env_amp_out(&mut patch, 0.005, 0.18, 0.0, 0.10);
    // Brighter than CORPUS-0001's 1.2 kHz, because the clipper needs harmonics
    // above the filter's corner to have anything to fold back down.
    add_saw_into_filter(&mut patch, 2_400.0, 0.15);
    silence_phase_randomization(&mut patch, "osc-1");
    patch.add_module(distortion_insert());
    patch.add_module(delay_insert());
    // Distortion first. See the note above on why this is written out.
    patch.settings.effect_chain_order = vec!["dst-1".to_string(), "dly-1".to_string()];

    // A fifth held together, then one isolated short note. The dyad is what
    // makes the chain's position and its shared state measurable — a chord is
    // the only thing a per-voice chain would process differently — and the
    // isolated note leaves the delay ringing in silence. With phase
    // randomization off, the two notes' order in this list does not affect the
    // render.
    let notes = [
        (0, 48, SeqDuration(720)),
        (0, 55, SeqDuration(720)),
        (1_920, 60, SeqDuration(240)),
    ];
    let (song, _) = one_pattern_song(
        "Insert Chain",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(
        vec![instrument(InstrumentId::FIRST, "Insert Chain", patch)],
        song,
        unity_global(),
    )
}

/// The first insert: a soft clipper at full wet.
///
/// Soft clip rather than bitcrush or foldback: it is the one mode whose output
/// is a continuous function of its input, so a small V2 numeric difference stays
/// a small audible one instead of landing on the far side of a step and turning
/// a rounding difference into a waveform difference.
fn distortion_insert() -> ModuleState {
    ModuleBuilder::new(1, ModuleType::Distortion)
        .param_choice("type", "soft_clip")
        .param_f("drive", 0.7)
        .param_f("tone", 0.8)
        .param_f("mix", 1.0)
        .position(800.0, 32.0)
        .build()
}

/// The second insert: a mono delay whose repeats are separately visible.
///
/// `time_left` and `time_right` carry the delay time rather than the `time`
/// macro, which is the parameter the module actually persists and emits — the
/// macro exists to set both at once and is hidden from the GUI. `tempo_sync` is
/// written out at its default of off so that the case's repeat interval is a
/// property of the patch rather than of the song tempo; a tempo-synced delay
/// would silently make this a tempo-map case as well.
///
/// 0.25 s at 120 BPM puts a repeat on every eighth note, so a repeat never lands
/// on a note onset and the two are never confused in an envelope comparison.
fn delay_insert() -> ModuleState {
    ModuleBuilder::new(1, ModuleType::Delay)
        .param_choice("mode", "mono")
        .param_f("time_left", 0.25)
        .param_f("time_right", 0.25)
        .param_f("feedback", 0.45)
        .param_f("mix", 0.5)
        .param_f("tone", 0.4)
        .param_f("tempo_sync", 0.0)
        .position(960.0, 32.0)
        .build()
}

// ---------------------------------------------------------------------------
// CORPUS-0006 — stereo voice
// ---------------------------------------------------------------------------

/// Three separated notes spanning the Keyboard Panner's center, so the same
/// mono oscillator appears left, centered, and right without another spatial
/// mechanism in the patch.
fn keyboard_panner_stereo() -> ProjectFile {
    let mut patch = Patch::new("Keyboard Panner Stereo");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.7)
            .build(),
    );
    silence_phase_randomization(&mut patch, "osc-1");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.002)
            .param_f("decay", 0.0)
            .param_f("sustain", 1.0)
            .param_f("release", 0.02)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::KeyboardPanner)
            .param_f("spread", 0.8)
            .param_f("center", 60.0)
            .param_f("curve", 0.5)
            .param_f("invert", 0.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out", "kbp-1", "in");
    patch.add_connection("kbp-1", "out_l", "out-1", "in_l");
    patch.add_connection("kbp-1", "out_r", "out-1", "in_r");

    let notes = [
        (0, 36, SeqDuration(720)),
        (960, 60, SeqDuration(720)),
        (1_920, 84, SeqDuration(720)),
    ];
    let (song, _) = one_pattern_song(
        "Keyboard Panner Stereo",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(
        vec![instrument(
            InstrumentId::FIRST,
            "Keyboard Panner Stereo",
            patch,
        )],
        song,
        unity_global(),
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0007 — YAMS control-rate Script
// ---------------------------------------------------------------------------

/// A constant control-rate program is the amplifier's only CV source. The
/// render is therefore audible only when the Script program is installed and
/// evaluated, while the constant makes the expected result attributable.
fn yams_control() -> ProjectFile {
    let mut patch = Patch::new("YAMS Control");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.7)
            .build(),
    );
    silence_phase_randomization(&mut patch, "osc-1");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.01)
            .param_f("decay", 0.05)
            .param_f("sustain", 0.8)
            .param_f("release", 0.1)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(2, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    let mut script = ModuleBuilder::new(1, ModuleType::Script).build();
    script
        .scripts
        .insert("1".to_string(), "out1 = 0.65".to_string());
    patch.add_module(script);
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("scr-1", "out1", "amp-1", "cv");
    patch.add_connection("amp-1", "out_l", "amp-2", "in_l");
    patch.add_connection("amp-1", "out_r", "amp-2", "in_r");
    patch.add_connection("env-1", "out", "amp-2", "cv");
    patch.add_connection("amp-2", "out_l", "out-1", "in_l");
    patch.add_connection("amp-2", "out_r", "out-1", "in_r");

    let notes = [(0, 60, SeqDuration(1_920))];
    let (song, _) = one_pattern_song(
        "YAMS Control",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(
        vec![instrument(InstrumentId::FIRST, "YAMS Control", patch)],
        song,
        unity_global(),
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0008 — YAMS audio-rate Script
// ---------------------------------------------------------------------------

/// A per-sample program applies different signed gains to the same mono source
/// on its two channels. This isolates AudioScript installation, stereo input
/// fallback, and audio-rate evaluation without state, randomness, or a stock
/// effect that could produce the same output.
fn yams_audio_script() -> ProjectFile {
    let mut patch = Patch::new("YAMS Audio Script");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sine")
            .param_f("level", 0.7)
            .build(),
    );
    silence_phase_randomization(&mut patch, "osc-1");
    let mut script = ModuleBuilder::new(1, ModuleType::AudioScript).build();
    script.scripts.insert(
        "1".to_string(),
        "out.left = in_l * 0.75\nout.right = in_r * -0.5".to_string(),
    );
    patch.add_module(script);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.01)
            .param_f("decay", 0.05)
            .param_f("sustain", 0.8)
            .param_f("release", 0.1)
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
    patch.add_connection("osc-1", "out", "asc-1", "in_l");
    patch.add_connection("osc-1", "out", "asc-1", "in_r");
    patch.add_connection("asc-1", "out_l", "amp-1", "in_l");
    patch.add_connection("asc-1", "out_r", "amp-1", "in_r");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "out_l", "out-1", "in_l");
    patch.add_connection("amp-1", "out_r", "out-1", "in_r");

    let notes = [(0, 57, SeqDuration(1_920))];
    let (song, _) = one_pattern_song(
        "YAMS Audio Script",
        InstrumentId::FIRST,
        SeqDuration::WHOLE,
        &notes,
    );
    project(
        vec![instrument(InstrumentId::FIRST, "YAMS Audio Script", patch)],
        song,
        unity_global(),
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0009 — tempo-map arrangement
// ---------------------------------------------------------------------------

/// Notes at equal tick intervals cross a 90-to-180 BPM ramp and a later 120 BPM
/// step. Their unequal sample intervals make tempo-map use observable without
/// asking this Phase 0A fixture to choose V2's later ramp-position law.
fn tempo_map_arrangement() -> ProjectFile {
    let mut patch = Patch::new("Tempo Map Arrangement");
    add_env_amp_out(&mut patch, 0.002, 0.08, 0.0, 0.05);
    add_saw_into_filter(&mut patch, 2_000.0, 0.1);
    silence_phase_randomization(&mut patch, "osc-1");

    let notes = [
        (0, 60, SeqDuration(240)),
        (960, 62, SeqDuration(240)),
        (1_920, 64, SeqDuration(240)),
        (2_880, 65, SeqDuration(240)),
        (3_840, 67, SeqDuration(240)),
        (4_800, 69, SeqDuration(240)),
    ];
    let (mut song, _) = one_pattern_song(
        "Tempo Map Arrangement",
        InstrumentId::FIRST,
        SeqDuration(5_760),
        &notes,
    );
    song.set_tempo_ramp_at(Tick(0), Bpm::new(90.0), true);
    song.set_tempo_at(Tick(1_920), Bpm::new(180.0));
    song.set_tempo_at(Tick(3_840), Bpm::new(120.0));
    project(
        vec![instrument(
            InstrumentId::FIRST,
            "Tempo Map Arrangement",
            patch,
        )],
        song,
        unity_global(),
    )
}

// ---------------------------------------------------------------------------
// CORPUS-0010 — two tracks sharing one instrument
// ---------------------------------------------------------------------------

/// Two independent patterns and tracks address one persisted instrument id.
/// The patch contains no random source, so the case observes V1 sharing
/// directly without making its audio depend on the open script-seed policy.
fn shared_instrument_tracks() -> ProjectFile {
    let mut patch = Patch::new("Shared Instrument Tracks");
    add_env_amp_out(&mut patch, 0.002, 0.02, 0.8, 0.05);
    add_saw_into_filter(&mut patch, 1_500.0, 0.1);
    silence_phase_randomization(&mut patch, "osc-1");

    let mut song = Song::new("Shared Instrument Tracks");
    // The two patterns are deliberately identical — the render test's 0.5 RMS
    // ratio between the tracks holds only while both play the same note — so
    // the construction lives in one place rather than as a copy a later edit
    // could vary.
    let identical_pattern = |song: &mut Song| {
        let pattern_id = song.create_pattern(SeqDuration::WHOLE);
        if let Some(pattern) = song.pattern_mut(pattern_id) {
            let note = pattern.add_note(PatternTick::ZERO, pitch(48), Velocity::F);
            if let Some(note) = pattern.note_mut(note) {
                note.duration = Some(SeqDuration(1_920));
            }
        }
        pattern_id
    };
    let unity_pattern = identical_pattern(&mut song);
    let half_pattern = identical_pattern(&mut song);

    let unity_track = song.create_track("Low — unity");
    if let Some(track) = song.track_mut(unity_track) {
        track.instrument = InstrumentId::FIRST;
        track.volume = NormalizedValue::MAX;
    }
    let half_track = song.create_track("Second — half gain");
    if let Some(track) = song.track_mut(half_track) {
        track.instrument = InstrumentId::FIRST;
        track.volume = NormalizedValue::new(0.5);
    }
    // Asserted rather than discarded: a refused placement would generate a
    // fixture whose second track plays nothing, and this builder runs in
    // generation tooling (`gen_corpus`), where aborting loudly beats writing a
    // silently broken corpus. Both ids were created lines above, so a refusal
    // is impossible by construction.
    assert!(
        song.place_pattern(unity_pattern, unity_track, Tick::ZERO),
        "new unity pattern and track must accept their first placement"
    );
    assert!(
        song.place_pattern(half_pattern, half_track, Tick::ZERO),
        "new half-gain pattern and track must accept their first placement"
    );

    project(
        vec![instrument(
            InstrumentId::FIRST,
            "Shared Instrument Tracks",
            patch,
        )],
        song,
        unity_global(),
    )
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

    /// `effect_chain_order` is matched against module ids by string, and an
    /// entry that matches nothing is dropped with a warning rather than
    /// refused. A mistyped id would therefore leave the chain in whatever order
    /// the append fallback produced, and every other test here would still
    /// pass.
    #[test]
    fn every_fixture_chain_order_entry_names_an_effect_in_the_patch() {
        for fixture in FIXTURES {
            let built = (fixture.build)();
            for instrument in &built.instruments {
                for entry in &instrument.patch.settings.effect_chain_order {
                    let module = instrument
                        .patch
                        .modules
                        .iter()
                        .find(|m| m.id.as_str() == entry.as_str());
                    let Some(module) = module else {
                        panic!(
                            "{}: effect_chain_order names {entry:?}, which is not a module in the patch",
                            fixture.case_id
                        );
                    };
                    assert!(
                        module.module_type.is_effect(),
                        "{}: effect_chain_order names {entry:?}, which is a {:?} rather than an effect",
                        fixture.case_id,
                        module.module_type
                    );
                }
            }
        }
    }

    /// Every effect in a patch must be named in the chain order. V1 appends the
    /// unnamed ones, so a fixture that relied on that would pin an order it did
    /// not author — and would keep passing if `effect_chain_order` were ignored.
    #[test]
    fn every_fixture_names_all_of_its_effects_in_chain_order() {
        for fixture in FIXTURES {
            let built = (fixture.build)();
            for instrument in &built.instruments {
                for module in &instrument.patch.modules {
                    if !module.module_type.is_effect() {
                        continue;
                    }
                    assert!(
                        instrument
                            .patch
                            .settings
                            .effect_chain_order
                            .iter()
                            .any(|entry| entry.as_str() == module.id.as_str()),
                        "{}: effect {} is absent from effect_chain_order, so its position \
                         would come from V1's append fallback rather than from the fixture",
                        fixture.case_id,
                        module.id
                    );
                }
            }
        }
    }

    /// The insert case's three claims each depend on a property of the fixture
    /// rather than of the effects: two inserts in an authored order, a dyad to
    /// make the summing position observable, and an isolated note for the
    /// delay to ring out after.
    #[test]
    fn the_insert_fixture_carries_an_ordered_chain_a_dyad_and_an_isolated_note() {
        let built = instrument_inserts();
        let patch = &built.instruments[0].patch;

        assert_eq!(
            patch.settings.effect_chain_order,
            vec!["dst-1".to_string(), "dly-1".to_string()],
            "the chain order is the thing this case pins"
        );

        let starts: Vec<u32> = built
            .song
            .patterns()
            .flat_map(|pattern| pattern.notes().iter().map(|note| note.start.0))
            .collect();
        assert_eq!(
            starts.iter().filter(|start| **start == 0).count(),
            2,
            "the dyad is what makes summing-before-the-chain observable"
        );
        let last = starts.iter().copied().max().unwrap_or(0);
        assert_eq!(
            starts.iter().filter(|start| **start == last).count(),
            1,
            "the final note must be alone for the delay to ring out into silence"
        );
    }

    /// Three of CORPUS-0005's four claims are measured by comparing a chord
    /// rendered whole against its notes rendered separately and summed, and that
    /// construction is only valid while the voice path is additive. With the
    /// oscillator's shipped `uni_phase` default it is not, and the null control
    /// that would catch it lives in an evidence record rather than in CI — so
    /// dropping this parameter would silently invalidate the claims and every
    /// other test here would still pass.
    #[test]
    fn the_insert_fixture_disables_oscillator_phase_randomization() {
        let built = instrument_inserts();
        let osc = built.instruments[0]
            .patch
            .modules
            .iter()
            .find(|m| m.id == "osc-1")
            .expect("the insert fixture has an oscillator");
        assert_eq!(
            osc.parameters.get("uni_phase"),
            Some(&ParamValue::Float(0.0)),
            "uni_phase must be pinned to 0; the shipped default randomizes phase per voice index"
        );
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
