//! Undo/redo system for sequencer, module, and connection operations.
//!
//! The `UndoManager` stores actions on undo/redo stacks and returns them
//! to the caller for execution. The caller (GUI code) is responsible for
//! actually applying the inverse action via the existing session/song APIs.

use eframe::egui;
use synth_core::{BipolarValue, Bpm, NormalizedValue};
use synth_engine::graph::Connection;
use synth_engine::instrument::InstrumentId;
use synth_sampler::{Sample, SampleId, SampleMeta};
use synth_sequencer::{
    Duration as SeqDuration, Glide, NoteExpression, NoteId, NoteLane, Ornament, PatternId,
    PatternTick, Pitch, ReturnBus, ReturnBusId, ReturnSend, Tick, TrackId, TrackSend, Velocity,
};

/// Maximum number of undo actions to keep in history.
const MAX_UNDO_HISTORY: usize = 100;

/// Ceiling on the sample audio the undo history may keep alive, in bytes.
///
/// A destructive sample edit (normalize, reverse) replaces the buffer, and the
/// undo entry holds the old one — so history pins audio the library has already
/// let go of. Without a cap, a hundred edits on a long stereo sample would hold
/// gigabytes hostage for the life of the session. 256 MiB is generous enough
/// that ordinary editing never trims, and small enough to stay a footnote in a
/// modern process's memory.
///
/// The count is a bound on *retained* audio, not on any single edit: one
/// oversized sample is still undoable exactly once.
const MAX_UNDO_AUDIO_BYTES: usize = 256 * 1024 * 1024;

/// How long after an edit a further edit to the *same* control still counts as
/// the same gesture.
///
/// Comfortably longer than a frame, so a continuous drag always merges, and
/// short enough that two deliberate visits to one knob stay separate undo
/// steps.
const PARAMETER_MERGE_WINDOW: std::time::Duration = std::time::Duration::from_millis(600);

/// A single undoable action.
///
/// Each variant captures enough state to reverse the operation.
/// `Composite` groups multiple actions into a single undo step.
#[derive(Debug, Clone)]
pub(crate) enum UndoAction {
    // ── Sequencer note operations ──
    /// A note was added to a pattern.
    AddNote {
        pattern_id: PatternId,
        note: NoteSnapshot,
    },
    /// A note was removed from a pattern.
    RemoveNote {
        pattern_id: PatternId,
        note: NoteSnapshot,
    },
    /// A note was moved to a new start tick.
    MoveNote {
        pattern_id: PatternId,
        note_id: NoteId,
        old_start: PatternTick,
        new_start: PatternTick,
    },
    /// A note was resized to a new duration.
    ResizeNote {
        pattern_id: PatternId,
        note_id: NoteId,
        old_duration: Option<SeqDuration>,
        new_duration: Option<SeqDuration>,
    },
    /// A note was transposed to a new pitch.
    TransposeNote {
        pattern_id: PatternId,
        note_id: NoteId,
        old_pitch: Pitch,
        new_pitch: Pitch,
    },
    /// A note's velocity was changed.
    SetNoteVelocity {
        pattern_id: PatternId,
        note_id: NoteId,
        old_velocity: Velocity,
        new_velocity: Velocity,
    },
    /// Batch velocity change (scale velocities, multi-edit).
    /// Each tuple is `(note_id, old_velocity, new_velocity)`.
    SetVelocitiesBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, Velocity, Velocity)>,
    },
    /// Batch per-note legato/tie change (multi-edit).
    /// Each tuple is `(note_id, old_legato, new_legato)`.
    SetLegatoBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, bool, bool)>,
    },
    /// Batch per-note glide change (multi-edit).
    /// Each tuple is `(note_id, old_glide, new_glide)`.
    SetGlideBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, Option<Glide>, Option<Glide>)>,
    },
    /// Batch per-note expression-block change (multi-edit).
    /// Each tuple is `(note_id, old_expression, new_expression)`.
    SetExpressionBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, Option<NoteExpression>, Option<NoteExpression>)>,
    },
    /// Batch per-note voice-lane change (tracker column assignment / migration).
    /// Each tuple is `(note_id, old_lane, new_lane)`.
    SetLaneBatch {
        pattern_id: PatternId,
        changes: Vec<(NoteId, NoteLane, NoteLane)>,
    },
    /// Batch per-note note-scope graph binding change (multi-edit, plan §2.1).
    /// Each tuple is `(note_id, old_graph, new_graph)`.
    SetNoteGraphBindingBatch {
        pattern_id: PatternId,
        changes: Vec<(
            NoteId,
            Option<synth_sequencer::NoteGraphId>,
            Option<synth_sequencer::NoteGraphId>,
        )>,
    },

    /// A pattern's note graph (or legacy per-note ornaments) was frozen — baked
    /// into plain notes and the source cleared. Carries the full pre-freeze
    /// pattern so
    /// the inverse restores notes + rack losslessly; applying it re-runs the
    /// (deterministic) freeze for redo.
    FreezePattern {
        pattern_id: PatternId,
        before: synth_sequencer::Pattern,
    },
    /// Restore a full pattern snapshot — the inverse of [`Self::FreezePattern`].
    RestorePattern {
        pattern_id: PatternId,
        snapshot: synth_sequencer::Pattern,
    },
    /// A note's per-note ornament was set, changed, or cleared.
    SetNoteOrnament {
        pattern_id: PatternId,
        note_id: NoteId,
        old: Option<Ornament>,
        new: Option<Ornament>,
    },

    // ── Pattern + track metadata ──
    /// A pattern was renamed.
    RenamePattern {
        pattern_id: PatternId,
        old_name: String,
        new_name: String,
    },
    /// A track was renamed.
    RenameTrack {
        track_id: TrackId,
        old_name: String,
        new_name: String,
    },
    /// A pattern's whole contents were replaced — a recording take committed
    /// over it, for instance.
    ///
    /// Unlike [`Self::RestorePattern`], whose inverse re-runs a freeze, this is
    /// a plain symmetric swap: applying it writes `new`, and the inverse writes
    /// `old`.
    SwapPattern {
        pattern_id: PatternId,
        old: Box<synth_sequencer::Pattern>,
        new: Box<synth_sequencer::Pattern>,
    },
    /// The song's default time signature was changed.
    SetTimeSignature {
        old: synth_sequencer::TimeSignature,
        new: synth_sequencer::TimeSignature,
    },
    /// A pattern's length was changed.
    SetPatternLength {
        pattern_id: PatternId,
        old_length: SeqDuration,
        new_length: SeqDuration,
    },

    // ── Tempo curve ──
    /// A tempo change at a specific tick was set, replaced, or removed. Each
    /// side carries `(bpm, ramp)` where `ramp` selects a linear ramp toward the
    /// next point vs a step change.
    ///
    /// `old = None` means there was no explicit change at this tick; `new = None`
    /// means the change was removed. The two together cover "Set tempo here…"
    /// (None → Some), edit / ramp toggle (Some(a) → Some(b)) and "Remove tempo
    /// change here" (Some → None).
    SetTempo {
        tick: Tick,
        old: Option<(Bpm, bool)>,
        new: Option<(Bpm, bool)>,
    },

    /// A tempo point was dragged in the lane: it moved from `old` to `new`, each
    /// `(tick, bpm, ramp)`. Applying removes the point at `old.0` and writes
    /// `new`; the inverse does the reverse.
    MoveTempo {
        old: (Tick, Bpm, bool),
        new: (Tick, Bpm, bool),
    },

    // ── Arrangement ──
    /// A placement was moved or transferred between tracks.
    MovePlacement {
        pattern_id: PatternId,
        old_track_id: TrackId,
        old_start: Tick,
        new_track_id: TrackId,
        new_start: Tick,
    },

    // ── Track / pattern delete (full state) ──
    /// A track was deleted. Restoring it brings back the full track plus
    /// every placement that referenced it.
    DeleteTrack {
        track: synth_sequencer::SequencerTrack,
        track_index: usize,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A track was restored from undo (inverse of DeleteTrack).
    AddTrack {
        track: synth_sequencer::SequencerTrack,
        track_index: usize,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A pattern was deleted. Restoring it brings back the full pattern
    /// (notes + automation) plus every placement that referenced it.
    DeletePattern {
        pattern: synth_sequencer::Pattern,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A pattern was restored from undo (inverse of DeletePattern).
    AddPattern {
        pattern: synth_sequencer::Pattern,
        placements: Vec<synth_sequencer::PatternPlacement>,
    },
    /// A placement was removed from the arrangement.
    RemovePlacement {
        placement: synth_sequencer::PatternPlacement,
    },
    /// A placement was inserted (inverse of RemovePlacement).
    InsertPlacement {
        placement: synth_sequencer::PatternPlacement,
    },
    /// A placement's length override changed (resize handle).
    SetPlacementLength {
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
        old_length: Option<SeqDuration>,
        new_length: Option<SeqDuration>,
    },
    /// A placement switched between one-shot Clip and looping Repeat playback.
    SetPlacementLoopMode {
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
        old_mode: synth_sequencer::PlacementLoopMode,
        new_mode: synth_sequencer::PlacementLoopMode,
    },
    /// Arrangement sections were edited as one atomic operation.
    SetArrangementSections {
        old: Vec<synth_sequencer::ArrangementSection>,
        new: Vec<synth_sequencer::ArrangementSection>,
    },

    // ── Automation ──
    /// An automation point was added.
    AddAutomationPoint {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        tick: PatternTick,
        value: synth_core::NormalizedValue,
        curve: synth_sequencer::CurveType,
    },
    /// An automation point was removed.
    RemoveAutomationPoint {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        tick: PatternTick,
        value: synth_core::NormalizedValue,
        curve: synth_sequencer::CurveType,
    },
    /// An automation point was moved.
    MoveAutomationPoint {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        old_tick: PatternTick,
        old_value: synth_core::NormalizedValue,
        new_tick: PatternTick,
        new_value: synth_core::NormalizedValue,
        curve: synth_sequencer::CurveType,
    },
    /// An automation point's curve type was changed in place (same tick/value).
    SetAutomationPointCurve {
        pattern_id: PatternId,
        target: synth_sequencer::AutomationTarget,
        tick: PatternTick,
        value: synth_core::NormalizedValue,
        old_curve: synth_sequencer::CurveType,
        new_curve: synth_sequencer::CurveType,
    },
    /// A whole automation lane was added (e.g. a new empty tracker column).
    AddAutomationLane {
        pattern_id: PatternId,
        lane: synth_sequencer::AutomationLane,
    },
    /// A whole automation lane was removed (e.g. pruning empty tracker columns).
    /// Carries the full lane so undo restores its points too.
    RemoveAutomationLane {
        pattern_id: PatternId,
        lane: synth_sequencer::AutomationLane,
    },

    // ── Connection operations ──
    /// A connection was added.
    AddConnection {
        instrument_id: InstrumentId,
        connection: Connection,
    },
    /// A connection was removed.
    RemoveConnection {
        instrument_id: InstrumentId,
        connection: Connection,
    },

    // ── Note Grid (pooled note graphs) ──
    /// A pooled note graph was created (`old: None`), edited, or deleted
    /// (`new: None`) — full-snapshot on each side, like [`Self::FreezePattern`].
    /// Applying writes the `new` side into the `Song` pool. NB: like every
    /// snapshot action here, applying overwrites concurrent external (MCP)
    /// edits to the same object — the app-wide undo trade-off.
    SetNoteGraph {
        graph_id: synth_sequencer::NoteGraphId,
        old: Option<synth_sequencer::NoteGraph>,
        new: Option<synth_sequencer::NoteGraph>,
    },
    /// A Mod Grid graph was created (`old: None`), edited, or deleted
    /// (`new: None`) — full-snapshot on each side, mirroring [`Self::SetNoteGraph`].
    /// Applying writes the `new` side into the `Song` mod-grid pool.
    SetModGraph {
        graph_id: synth_sequencer::ModGraphId,
        old: Option<synth_sequencer::ModGraph>,
        new: Option<synth_sequencer::ModGraph>,
    },
    /// A pattern's note-graph binding changed.
    SetPatternNoteGraph {
        pattern_id: PatternId,
        old: Option<synth_sequencer::NoteGraphId>,
        new: Option<synth_sequencer::NoteGraphId>,
    },

    // ── Mixer ──
    /// A track's channel-strip control changed.
    ///
    /// One variant for all four rather than one each: they share an inverse
    /// (swap old and new) and an apply path (look up the track, write the
    /// field), so splitting them would triple the code for no added meaning.
    SetTrackMixer {
        track_id: TrackId,
        param: TrackMixerParam,
        old: MixerValue,
        new: MixerValue,
    },
    /// A track's send to one return bus was added, changed, or removed.
    ///
    /// `None` on either side means "no send" — the mixer deletes a send when
    /// its level reaches zero, so add, adjust and remove are the same edit seen
    /// at different levels.
    SetTrackSend {
        track_id: TrackId,
        return_bus: ReturnBusId,
        old: Option<TrackSend>,
        new: Option<TrackSend>,
    },
    /// A return bus's own fader/mute/solo changed.
    SetReturnBusMixer {
        bus_id: ReturnBusId,
        param: TrackMixerParam,
        old: MixerValue,
        new: MixerValue,
    },
    /// The master fader moved.
    ///
    /// Its own action rather than a `param` on one of the others because master
    /// volume is not a `Song` field: it is an engine atomic set by command, and
    /// undoing it means re-sending that command rather than writing a struct.
    /// It is still saved state, so it belongs in the history like any other
    /// mixer control.
    SetMasterVolume { old: MixerValue, new: MixerValue },
    /// A bus-to-bus send (return feeding another return) was added, changed,
    /// or removed.
    ///
    /// The return-side twin of [`Self::SetTrackSend`]. Separate rather than
    /// shared because the payloads genuinely differ: a `TrackSend` carries a
    /// pre/post-fader tap point, which is meaningless between two return buses.
    SetReturnSend {
        from: ReturnBusId,
        target: ReturnBusId,
        old: Option<ReturnSend>,
        new: Option<ReturnSend>,
    },
    /// A return bus was created (`old: None`) or deleted (`new: None`).
    ///
    /// Carries the whole bus so deleting and undoing restores its name, colour,
    /// fader and bus-to-bus sends — not just an empty bus with the same id.
    /// `index` is its position in the strip order, so undoing the deletion of a
    /// middle bus does not move it to the end.
    ///
    /// `effects` carries the bus's engine-side insert chain, which the song
    /// does not own: deleting a bus tears the chain down with it, so restoring
    /// the song-side definition alone would bring back a bus with no reverb on
    /// it.
    SetReturnBus {
        effects: Vec<EffectSnapshot>,
        bus_id: ReturnBusId,
        index: usize,
        old: Option<Box<ReturnBus>>,
        new: Option<Box<ReturnBus>>,
    },

    // ── Instrument ──
    /// An instrument's properties or performance settings changed.
    ///
    /// Carries a full snapshot rather than one variant per property: the patch
    /// bar edits around fifteen of them and they all share an inverse and an
    /// apply path. See
    /// [`InstrumentSettings`](crate::gui::instrument_rack::InstrumentSettings).
    SetInstrumentSettings {
        instrument_id: InstrumentId,
        old: Box<crate::gui::instrument_rack::InstrumentSettings>,
        new: Box<crate::gui::instrument_rack::InstrumentSettings>,
    },

    /// A module parameter was edited (a knob, a slider, a dropdown).
    ///
    /// Consecutive edits to the *same* parameter merge into one entry while the
    /// user is still moving the control — see [`UndoManager::push`]. A knob has
    /// no gesture signal at this layer (parameter changes arrive as a plain
    /// list, with no `Response` to ask about `drag_stopped`), so merging is how
    /// a drag becomes one undo step rather than one per frame.
    SetModuleParameter {
        instrument_id: InstrumentId,
        module_id: synth_engine::ModuleId,
        old: synth_core::Param,
        new: synth_core::Param,
    },

    /// Modules were added to or removed from a rack.
    ///
    /// Carries the modules' full persisted state plus the cables between them,
    /// because removing a module also removes its cables — restoring only the
    /// module would leave it disconnected, which is not what the user undid.
    ///
    /// `severed` holds cables that existed *before* the addition and were cut
    /// by it: adding a module inline on a cable replaces that cable with two.
    /// They are the mirror image of `connections` — laid back down when the
    /// modules go away, taken up again when they come back — without which
    /// undoing an inline insert would leave the two modules it sat between
    /// silently disconnected.
    ///
    /// `restore` says which direction *applying* this action goes; the inverse
    /// simply flips it. One variant rather than an Add/Remove pair because the
    /// payload is identical and the two would be pure duplication.
    SetRackModules {
        instrument_id: InstrumentId,
        modules: Vec<crate::patch::ModuleState>,
        connections: Vec<Connection>,
        severed: Vec<Connection>,
        restore: bool,
    },

    // ── Return / master effect chains ──
    /// An effect was added to (`old: None`) or removed from (`new: None`) a
    /// return-bus or master chain.
    ///
    /// Carries the effect's type and parameters rather than the instance —
    /// `Box<dyn AudioEffect>` cannot be cloned, so restoring builds a fresh one
    /// of the same type and replays its settings onto it.
    SetChainEffect {
        chain: EffectChain,
        old: Option<Box<EffectSnapshot>>,
        new: Option<Box<EffectSnapshot>>,
    },
    /// A return-bus or master effect's parameter was edited.
    ///
    /// Merges consecutive edits to the same control, like
    /// [`Self::SetModuleParameter`].
    SetChainEffectParameter {
        chain: EffectChain,
        module_id: synth_engine::ModuleId,
        old: synth_core::Param,
        new: synth_core::Param,
    },
    /// A return-bus or master effect was bypassed or re-enabled.
    SetChainEffectBypass {
        chain: EffectChain,
        module_id: synth_engine::ModuleId,
        old: bool,
        new: bool,
    },
    /// An effect chain's slot order changed — the ▲/▼ buttons on an effect
    /// module's header in the patch editor.
    ///
    /// Addressed the way the engine addresses a chain (`None` is the master
    /// bus) rather than by [`EffectChain`], because what this reaches is the
    /// *instrument* chains, which `EffectChain` deliberately does not name: its
    /// add/remove/parameter commands do not apply to patch modules. Return
    /// buses have no reorder surface today; when they get one they get their
    /// own variant, the way [`Self::SetTrackSend`] and [`Self::SetReturnSend`]
    /// are separate twins.
    ///
    /// Both sides carry the whole order rather than the direction of the move.
    /// Replaying "one slot up" would swap whichever pair happens to sit at that
    /// index by then; an explicit order restores the arrangement that was
    /// actually there.
    SetEffectChainOrder {
        instrument_id: Option<InstrumentId>,
        old: Vec<synth_engine::ModuleId>,
        new: Vec<synth_engine::ModuleId>,
    },

    // ── Sample library ──
    /// A sample was imported (`old: None`) or deleted (`new: None`).
    ///
    /// Carries the whole `Sample`, but that is cheap: its audio is an
    /// `Arc<[f32]>`, so restoring a deleted sample shares the original buffer
    /// rather than copying it. See [`UndoManager::push`] for the budget that
    /// bounds how much audio the history can pin alive.
    SetSample {
        id: SampleId,
        old: Option<Box<Sample>>,
        new: Option<Box<Sample>>,
    },
    /// A sample's metadata changed — name, description, root note, loop region
    /// or crop region.
    ///
    /// One variant for all of them because they share an inverse and an apply
    /// path, and because the sample editor writes metadata by read-modify-write
    /// of the whole `SampleMeta` anyway.
    SetSampleMeta {
        id: SampleId,
        old: Box<SampleMeta>,
        new: Box<SampleMeta>,
    },
    /// A destructive DSP edit replaced a sample's audio (normalize, reverse).
    ///
    /// Both sides are `Arc<[f32]>`, so the entry shares the buffers rather than
    /// cloning them — but holding the old one keeps it alive after the edit
    /// dropped the library's reference, which is exactly what the audio budget
    /// in [`UndoManager::push`] bounds.
    SetSampleData {
        id: SampleId,
        old: std::sync::Arc<[f32]>,
        new: std::sync::Arc<[f32]>,
    },

    // ── Composite ──
    /// Multiple actions grouped as a single undo step.
    Composite(Vec<UndoAction>),
}

impl UndoAction {
    /// What this entry edits, when consecutive edits to it should merge into
    /// one undo step.
    ///
    /// `None` means "never merge" — most edits are discrete and each deserves
    /// its own step. Only continuous controls with no gesture signal opt in.
    fn merge_key(&self) -> Option<MergeKey> {
        match self {
            Self::SetModuleParameter {
                instrument_id,
                module_id,
                new,
                ..
            } => Some(MergeKey::InstrumentModule(
                *instrument_id,
                *module_id,
                synth_core::ModuleParam::name(new),
            )),
            Self::SetChainEffectParameter {
                chain,
                module_id,
                new,
                ..
            } => Some(MergeKey::ChainEffect(
                *chain,
                *module_id,
                synth_core::ModuleParam::name(new),
            )),
            // The patch bar and the instrument window both edit a whole
            // settings snapshot, and their `Vol`/`Pan` drag values report a
            // change every frame of a drag — without merging, one fader sweep
            // would push a hundred entries and evict the rest of the history.
            Self::SetInstrumentSettings { instrument_id, .. } => {
                Some(MergeKey::Instrument(*instrument_id))
            }
            _ => None,
        }
    }

    /// Absorb a later edit to the same control, keeping this entry's original
    /// `old` and adopting the newer destination.
    ///
    /// Returns whether the two were the same kind of edit. A mismatch can only
    /// mean two variants shared a [`MergeKey`], so the caller keeps them as
    /// separate entries rather than dropping one.
    fn merge_from(&mut self, latest: &Self) -> bool {
        match (self, latest) {
            (
                Self::SetModuleParameter { new: target, .. },
                Self::SetModuleParameter { new: latest, .. },
            )
            | (
                Self::SetChainEffectParameter { new: target, .. },
                Self::SetChainEffectParameter { new: latest, .. },
            ) => {
                *target = *latest;
                true
            }
            (
                Self::SetInstrumentSettings { new: target, .. },
                Self::SetInstrumentSettings { new: latest, .. },
            ) => {
                target.clone_from(latest);
                true
            }
            _ => false,
        }
    }

    /// How many bytes of sample audio this entry keeps alive.
    ///
    /// Only the sample variants hold audio; everything else is metadata-sized
    /// and reports zero. Both sides of a data edit are counted even when they
    /// happen to share an `Arc` — over-counting only makes the budget trim
    /// sooner, which is the safe direction.
    fn audio_bytes(&self) -> usize {
        const BYTES_PER_SAMPLE: usize = std::mem::size_of::<f32>();
        match self {
            Self::SetSample { old, new, .. } => [old, new]
                .into_iter()
                .flatten()
                .map(|sample| sample.data.len() * BYTES_PER_SAMPLE)
                .sum(),
            Self::SetSampleData { old, new, .. } => (old.len() + new.len()) * BYTES_PER_SAMPLE,
            Self::Composite(actions) => actions.iter().map(Self::audio_bytes).sum(),
            _ => 0,
        }
    }
}

/// Identifies the control an entry edits, for merging consecutive edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeKey {
    /// A parameter on a module inside an instrument's voice graph.
    InstrumentModule(InstrumentId, synth_engine::ModuleId, &'static str),
    /// A parameter on an effect in a return-bus or master chain.
    ChainEffect(EffectChain, synth_engine::ModuleId, &'static str),
    /// An instrument's property snapshot, edited by the patch bar or the
    /// instrument window.
    Instrument(InstrumentId),
}

/// Which effect chain an entry refers to.
///
/// The master chain is the engine's `instrument_id: None` chain; a return bus
/// has its own. Instrument insert chains are *not* here — those are ordinary
/// modules in the patch editor, covered by
/// [`UndoAction::SetRackModules`](UndoAction::SetRackModules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EffectChain {
    /// A return bus's insert chain.
    Return(ReturnBusId),
    /// The master bus's insert chain.
    Master,
}

impl EffectChain {
    /// A widget-id seed unique per chain (master is distinct from any return).
    pub(crate) fn id_seed(self) -> u32 {
        match self {
            Self::Return(id) => u32::from(id.0),
            Self::Master => u32::from(u16::MAX) + 1,
        }
    }

    /// Command to bypass or re-enable an effect on this chain.
    pub(crate) fn set_enabled(
        self,
        module_id: synth_engine::ModuleId,
        enabled: bool,
    ) -> synth_engine::EngineCommand {
        match self {
            Self::Return(return_id) => synth_engine::EngineCommand::SetReturnEffectEnabled {
                return_id,
                module_id,
                enabled,
            },
            Self::Master => synth_engine::EngineCommand::SetEffectEnabled {
                instrument_id: None,
                module_id,
                enabled,
            },
        }
    }

    /// Command to remove an effect from this chain.
    pub(crate) fn remove(self, module_id: synth_engine::ModuleId) -> synth_engine::EngineCommand {
        match self {
            Self::Return(return_id) => synth_engine::EngineCommand::RemoveReturnEffect {
                return_id,
                id: module_id,
            },
            Self::Master => synth_engine::EngineCommand::RemoveEffect {
                instrument_id: None,
                id: module_id,
            },
        }
    }

    /// Command to set one of an effect's parameters.
    pub(crate) fn set_param(
        self,
        module_id: synth_engine::ModuleId,
        param: synth_core::Param,
    ) -> synth_engine::EngineCommand {
        match self {
            Self::Return(return_id) => synth_engine::EngineCommand::SetReturnEffectParameter {
                return_id,
                module_id,
                param,
            },
            Self::Master => synth_engine::EngineCommand::SetEffectParameter {
                instrument_id: None,
                module_id,
                param,
            },
        }
    }

    /// Command to set this chain's whole slot order.
    ///
    /// Used to put a restored effect back where it was: `add` only appends, so
    /// without this, undoing the removal of a mid-chain effect would silently
    /// move it to the end and change what the signal passes through in what
    /// order.
    pub(crate) fn set_order(
        self,
        order: Vec<synth_engine::ModuleId>,
    ) -> synth_engine::EngineCommand {
        match self {
            Self::Return(return_id) => {
                synth_engine::EngineCommand::SetReturnEffectChainOrder { return_id, order }
            }
            Self::Master => synth_engine::EngineCommand::SetEffectChainOrder {
                instrument_id: None,
                order,
            },
        }
    }

    /// Command to append an effect instance to this chain.
    pub(crate) fn add(
        self,
        id: synth_engine::ModuleId,
        effect: Box<dyn synth_core::AudioEffect>,
    ) -> synth_engine::EngineCommand {
        match self {
            Self::Return(return_id) => synth_engine::EngineCommand::AddReturnEffect {
                return_id,
                id,
                effect,
            },
            Self::Master => synth_engine::EngineCommand::AddEffectInstance {
                instrument_id: None,
                id,
                effect,
            },
        }
    }
}

/// The chain order that results from moving `module_id` one slot in
/// `direction`, or `None` when nothing would move.
///
/// Mirrors the swap the engine's `ReorderEffect` performs, so the order an
/// undo entry records is the one the engine actually ends up in. `None` covers
/// both a module that is not on the chain and one already at the end it is
/// being moved towards — neither is an edit, and recording one would leave a
/// history step that reverses something that never happened.
pub(crate) fn reordered_chain(
    order: &[synth_engine::ModuleId],
    module_id: synth_engine::ModuleId,
    direction: synth_engine::ReorderDirection,
) -> Option<Vec<synth_engine::ModuleId>> {
    let index = order.iter().position(|id| *id == module_id)?;
    let target = match direction {
        synth_engine::ReorderDirection::Up => index.checked_sub(1)?,
        synth_engine::ReorderDirection::Down => (index + 1 < order.len()).then_some(index + 1)?,
    };
    let mut reordered = order.to_vec();
    reordered.swap(index, target);
    Some(reordered)
}

impl EffectSnapshot {
    /// The commands that put this effect back on `chain`, in order.
    ///
    /// The instance is rebuilt from its type rather than restored, because
    /// `AudioEffect` is a trait object and cannot be cloned; the settings are
    /// then replayed onto the fresh instance. Returns nothing if the effect
    /// type is unknown to the factory, which would mean a project referencing a
    /// module this build does not have.
    pub(crate) fn restore_commands(&self, chain: EffectChain) -> Vec<synth_engine::EngineCommand> {
        let Some((effect, _descriptor)) = crate::module_factory::create_effect(self.module_type)
        else {
            tracing::warn!(
                target: "pertylizer::undo",
                module_type = ?self.module_type,
                "cannot restore an effect this build does not know",
            );
            return Vec::new();
        };
        let mut commands = vec![chain.add(self.module_id, effect)];
        commands.extend(
            self.params
                .iter()
                .map(|param| chain.set_param(self.module_id, *param)),
        );
        // Bypass last, so it is not overwritten by the parameter replay.
        commands.push(chain.set_enabled(self.module_id, !self.bypassed));
        if !self.chain_order.is_empty() {
            commands.push(chain.set_order(self.chain_order.clone()));
        }
        commands
    }
}

/// Everything needed to recreate one effect on a chain.
///
/// Deliberately not the effect instance: `Box<dyn AudioEffect>` is not
/// cloneable, and rebuilding from type plus parameters is what the mixer's own
/// add path already does.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EffectSnapshot {
    /// The chain's full slot order at capture time, this effect included.
    ///
    /// Restoring can only append, so the order is replayed afterwards to put
    /// the effect back in the slot it held — otherwise undoing the removal of a
    /// compressor that sat before a reverb would silently move it after.
    pub(crate) chain_order: Vec<synth_engine::ModuleId>,
    /// The id the effect occupied, preserved so undo does not renumber a chain.
    pub(crate) module_id: synth_engine::ModuleId,
    /// Which effect to build.
    pub(crate) module_type: synth_core::ModuleType,
    /// Its settings at capture time.
    pub(crate) params: Vec<synth_core::Param>,
    /// Whether it was bypassed.
    pub(crate) bypassed: bool,
}

/// Which channel-strip control an undo entry refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackMixerParam {
    /// Output level.
    Volume,
    /// Stereo position.
    Pan,
    /// Muted state.
    Mute,
    /// Soloed state.
    Solo,
}

impl TrackMixerParam {
    /// The control's name, for a status message or a coalescing key.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Pan => "pan",
            Self::Mute => "mute",
            Self::Solo => "solo",
        }
    }
}

/// The value of a channel-strip control.
///
/// A small sum type rather than a bare `f32`, so the newtype each control
/// actually uses survives the round trip: writing a pan back as a
/// `NormalizedValue` would silently clamp the left half of the range away.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MixerValue {
    /// A `0.0..=1.0` fader position.
    Level(NormalizedValue),
    /// A `-1.0..=1.0` pan position.
    Balance(BipolarValue),
    /// A mute or solo state.
    Flag(bool),
}

/// Snapshot of a note for undo/redo purposes.
///
/// Captures all fields needed to fully reconstruct a note.
#[derive(Debug, Clone)]
pub(crate) struct NoteSnapshot {
    pub(crate) id: NoteId,
    pub(crate) start: PatternTick,
    pub(crate) duration: Option<SeqDuration>,
    pub(crate) pitch: Pitch,
    pub(crate) velocity: Velocity,
    pub(crate) track: Option<synth_sequencer::TrackId>,
    pub(crate) lane: NoteLane,
}

impl From<&synth_sequencer::Note> for NoteSnapshot {
    fn from(note: &synth_sequencer::Note) -> Self {
        Self {
            id: note.id,
            start: note.start,
            duration: note.duration,
            pitch: note.pitch,
            velocity: note.velocity,
            track: note.track,
            lane: note.lane,
        }
    }
}

/// Collapses a continuous drag into a single undo entry.
///
/// A slider or knob fires `changed()` on every frame of a drag, so pushing an
/// undo action per change would bury the history under hundreds of one-pixel
/// steps and make a single fader move take hundreds of Ctrl+Z presses to
/// reverse.
///
/// The fix is to remember the value from *before* the gesture started and push
/// one action when it ends:
///
/// ```ignore
/// let response = ui.add(slider);
/// if response.changed() {
///     coalescer.begin(response.id, MixerValue::Level(previous));
///     write_the_new_value();
/// }
/// if let Some(old) = coalescer.end_if_finished(&response) {
///     undo.push(UndoAction::SetTrackMixer { old, new, .. });
/// }
/// ```
///
/// Keying on the widget's `egui::Id` means two faders dragged in sequence keep
/// separate entries, and a gesture interrupted by clicking elsewhere still
/// resolves — `end_if_finished` also fires on lost focus and lost interaction,
/// not just on `drag_stopped`, so a keyboard or click edit is not left pending
/// forever.
#[derive(Default)]
pub(crate) struct DragCoalescer {
    /// Pre-gesture state for the widgets currently mid-edit. At most a handful
    /// at a time — one per pointer, in practice one.
    in_flight: Vec<(egui::Id, GestureStart)>,
}

/// What a control looked like before the current gesture began.
///
/// One enum rather than a parallel `Vec` per control kind: they all answer the
/// same question and differ only in payload, and a fourth parallel collection
/// was the point at which the duplication stopped paying for itself.
///
/// The payloads are whole values, not scalars, wherever a gesture can destroy
/// structure: dragging a send's level through zero *deletes* the send, so
/// restoring it needs its tap point and enabled flag, which a bare level cannot
/// carry.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GestureStart {
    /// A channel-strip fader, pan slider, or mute/solo button.
    Mixer(MixerValue),
    /// A track's send to a return bus, or its absence.
    TrackSend(Option<TrackSend>),
    /// A return bus's send to another return bus, or its absence.
    ReturnSend(Option<ReturnSend>),
    /// A sample's metadata, for the loop and crop drag handles.
    SampleMeta(Box<SampleMeta>),
}

impl DragCoalescer {
    /// Record the state a gesture started from.
    ///
    /// Only the first call for a widget takes effect; the rest of the drag
    /// re-reports the same widget, and the *first* value is the one undo has to
    /// return to.
    pub(crate) fn begin(&mut self, id: egui::Id, before: GestureStart) {
        if !self.is_tracking(id) {
            self.in_flight.push((id, before));
        }
    }

    /// Whether a gesture is being tracked for `id`.
    pub(crate) fn is_tracking(&self, id: egui::Id) -> bool {
        self.in_flight.iter().any(|(known, _)| *known == id)
    }

    /// End a gesture explicitly, for edits that are instant rather than dragged
    /// (a mute button, a value typed into a field).
    pub(crate) fn take(&mut self, id: egui::Id) -> Option<GestureStart> {
        let index = self.in_flight.iter().position(|(known, _)| *known == id)?;
        Some(self.in_flight.remove(index).1)
    }

    /// If `response`'s gesture has ended, return the state it started from.
    ///
    /// Returns `None` while the gesture is still in progress, and for a widget
    /// that was never begun. Ends on lost focus as well as `drag_stopped`, so a
    /// keyboard or click edit is not left pending forever.
    pub(crate) fn end_if_finished(&mut self, response: &egui::Response) -> Option<GestureStart> {
        if !(response.drag_stopped() || response.lost_focus()) {
            return None;
        }
        self.take(response.id)
    }
}

/// The undo plumbing a mixer control needs, bundled so the strip drawing code
/// takes one parameter instead of two and the begin/push dance stays in one
/// place.
pub(crate) struct MixerUndo<'a> {
    /// Where finished edits are pushed.
    pub(crate) undo: &'a mut UndoManager,
    /// Gesture tracking, so a fader drag is one entry rather than hundreds.
    pub(crate) coalescer: &'a mut DragCoalescer,
}

impl MixerUndo<'_> {
    /// Record an edit to a continuous control (fader, pan slider).
    ///
    /// Call unconditionally every frame with the widget's response. `before` is
    /// the value as of the start of this frame — for a mixer strip that is the
    /// snapshot field, which has not yet seen this frame's edit. `after` is the
    /// widget's live value.
    ///
    /// Nothing is pushed until the gesture ends, and nothing is pushed at all
    /// if the value came back to where it started — dragging a fader and
    /// putting it back is not an edit worth a history entry.
    pub(crate) fn record_drag(
        &mut self,
        response: &egui::Response,
        before: MixerValue,
        after: MixerValue,
        build: impl FnOnce(MixerValue, MixerValue) -> UndoAction,
    ) {
        if response.changed() {
            self.coalescer
                .begin(response.id, GestureStart::Mixer(before));
        }
        if let Some(GestureStart::Mixer(gesture_start)) = self.coalescer.end_if_finished(response)
            && gesture_start != after
        {
            self.undo.push(build(gesture_start, after));
        }
    }

    /// Record one click of a control that changes in a single step (a mute or
    /// solo button), where there is no gesture to coalesce.
    ///
    /// Takes every field the click touched, because mute and solo clear each
    /// other: one click can move both, and the two have to come back together.
    /// Pushed as a single entry — as two, one Ctrl+Z would restore the mute and
    /// leave the solo, a state the user was never in.
    pub(crate) fn record_click(&mut self, changes: impl IntoIterator<Item = UndoAction>) {
        let mut changes: Vec<UndoAction> = changes.into_iter().collect();
        match changes.len() {
            0 => {}
            1 => self.undo.push(changes.remove(0)),
            _ => self.undo.push(UndoAction::Composite(changes)),
        }
    }
}

/// One channel-strip toggle change, or `None` if the click left it alone.
///
/// A small builder so the mute/solo call sites can hand
/// [`MixerUndo::record_click`] a list without repeating the "did it actually
/// move?" test four times.
pub(crate) fn toggle_change(
    before: bool,
    after: bool,
    build: impl FnOnce(MixerValue, MixerValue) -> UndoAction,
) -> Option<UndoAction> {
    (before != after).then(|| build(MixerValue::Flag(before), MixerValue::Flag(after)))
}

/// The undo plumbing the sample editor needs.
///
/// Its helpers take the mutation as a closure and capture before/after around
/// it, so a call site cannot record the wrong "before" or forget to record at
/// all — the failure mode that left the whole sample editor non-undoable.
pub(crate) struct SampleUndo<'a> {
    /// Where finished edits are pushed.
    pub(crate) undo: &'a mut UndoManager,
    /// Gesture tracking for the loop and crop drag handles.
    pub(crate) coalescer: &'a mut DragCoalescer,
}

/// The sample library behind its lock.
///
/// Deliberately the bare `RwLock` rather than the `Arc<RwLock<_>>` the app
/// stores, so callers holding either can pass it — `commit_recording_as_sample`
/// takes the inner reference to keep its borrows disjoint from `self`.
pub(crate) type SharedLibrary = std::sync::RwLock<synth_sampler::SampleLibrary>;

impl SampleUndo<'_> {
    /// Run a metadata edit and record it as one undo entry.
    ///
    /// A no-op edit records nothing, so clicking a value and leaving it alone
    /// does not fill the history.
    pub(crate) fn record_meta(
        &mut self,
        library: &SharedLibrary,
        id: SampleId,
        edit: impl FnOnce(&mut synth_sampler::SampleLibrary),
    ) {
        let before = read_meta(library, id);
        apply(library, edit);
        self.push_meta_change(library, id, before);
    }

    /// Run a metadata edit that is part of a drag, recording one entry for the
    /// whole gesture rather than one per frame.
    ///
    /// `response` is the dragged widget; the entry lands when its gesture ends.
    pub(crate) fn record_meta_drag(
        &mut self,
        response: &egui::Response,
        library: &SharedLibrary,
        id: SampleId,
        edit: impl FnOnce(&mut synth_sampler::SampleLibrary),
    ) {
        if let Some(before) = read_meta(library, id) {
            self.coalescer
                .begin(response.id, GestureStart::SampleMeta(before));
        }
        apply(library, edit);
        if let Some(GestureStart::SampleMeta(before)) = self.coalescer.end_if_finished(response) {
            self.push_meta_change(library, id, Some(before));
        }
    }

    /// Close a metadata gesture that ended without a drag — a value typed into
    /// a field, or a slider released outside the widget.
    pub(crate) fn finish_meta_drag(
        &mut self,
        response: &egui::Response,
        library: &SharedLibrary,
        id: SampleId,
    ) {
        if let Some(GestureStart::SampleMeta(before)) = self.coalescer.end_if_finished(response) {
            self.push_meta_change(library, id, Some(before));
        }
    }

    /// Compare the metadata against `before` and push an entry if it moved.
    fn push_meta_change(
        &mut self,
        library: &SharedLibrary,
        id: SampleId,
        before: Option<Box<SampleMeta>>,
    ) {
        let (Some(before), Some(after)) = (before, read_meta(library, id)) else {
            return;
        };
        if before != after {
            self.undo.push(UndoAction::SetSampleMeta {
                id,
                old: before,
                new: after,
            });
        }
    }

    /// Run a destructive DSP edit (normalize, reverse) and record it.
    ///
    /// Both buffers are `Arc`, so the entry shares them; see
    /// [`MAX_UNDO_AUDIO_BYTES`] for the budget that bounds how many such
    /// entries the history keeps alive.
    pub(crate) fn record_data(
        &mut self,
        library: &SharedLibrary,
        id: SampleId,
        edit: impl FnOnce(&mut synth_sampler::SampleLibrary),
    ) {
        let before = read_data(library, id);
        apply(library, edit);
        let (Some(before), Some(after)) = (before, read_data(library, id)) else {
            return;
        };
        if !std::sync::Arc::ptr_eq(&before, &after) {
            self.undo.push(UndoAction::SetSampleData {
                id,
                old: before,
                new: after,
            });
        }
    }

    /// Record the removal of a sample, capturing it so undo can restore it.
    ///
    /// Deleting is the one sample edit that destroys everything at once, so it
    /// is captured whole — metadata and the `Arc`-shared audio together.
    pub(crate) fn record_delete(&mut self, library: &SharedLibrary, id: SampleId) {
        let removed = apply_with(library, |lib| lib.remove(id)).flatten();
        if let Some(sample) = removed {
            self.undo.push(UndoAction::SetSample {
                id,
                old: Some(Box::new(sample)),
                new: None,
            });
        }
    }

    /// Record a sample import so it can be undone.
    pub(crate) fn record_import(&mut self, library: &SharedLibrary, id: SampleId) {
        let Some(sample) = library.read().ok().and_then(|lib| lib.get(id).cloned()) else {
            return;
        };
        self.undo.push(UndoAction::SetSample {
            id,
            old: None,
            new: Some(Box::new(sample)),
        });
    }
}

/// A sample's metadata, or `None` if the library is locked or the id is gone.
fn read_meta(library: &SharedLibrary, id: SampleId) -> Option<Box<SampleMeta>> {
    library
        .read()
        .ok()?
        .get_meta(id)
        .map(|meta| Box::new(meta.clone()))
}

/// A sample's audio handle. Cloning the `Arc` does not copy the audio.
fn read_data(library: &SharedLibrary, id: SampleId) -> Option<std::sync::Arc<[f32]>> {
    library.read().ok()?.get_data(id)
}

/// Run `edit` under the library's write lock, ignoring a poisoned lock — the
/// sample editor's existing convention.
fn apply(library: &SharedLibrary, edit: impl FnOnce(&mut synth_sampler::SampleLibrary)) {
    if let Ok(mut lib) = library.write() {
        edit(&mut lib);
    }
}

/// As [`apply`], for an edit that returns something.
fn apply_with<T>(
    library: &SharedLibrary,
    edit: impl FnOnce(&mut synth_sampler::SampleLibrary) -> T,
) -> Option<T> {
    library.write().ok().map(|mut lib| edit(&mut lib))
}

/// One entry on a history stack: the action plus an id identifying the state
/// it produced.
///
/// The id exists because stack *depth* alone cannot identify a point in the
/// history: undoing one edit and then making a different one returns to the
/// same depth by a different route, and the project is not back where it was.
/// See [`UndoManager::position`].
struct HistoryEntry {
    /// Unique per push, and re-issued when an entry is merged into (a merge
    /// moves the project without changing the depth).
    id: u64,
    action: UndoAction,
}

/// Where the history currently stands: `(depth, id of the newest entry)`.
///
/// Comparable for equality only — two equal positions mean the same sequence of
/// edits has been applied, which is what "back at the saved state" means.
pub(crate) type HistoryPosition = (usize, u64);

/// Stack-based undo/redo manager.
///
/// Stores actions and returns them on undo/redo. The caller is responsible
/// for actually executing the returned (inverse) action.
pub(crate) struct UndoManager {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    /// Source of the per-entry ids in [`HistoryEntry`]. Never reused within a
    /// session, so a position can never be mistaken for an earlier one.
    next_entry_id: u64,
    /// When the last entry was pushed, for the parameter merge window. `None`
    /// until the first push, so the first edit of a session never merges.
    last_push: Option<std::time::Instant>,
    /// Monotonic count of everything that passed through this manager: pushes
    /// (including merged ones, which change the project without changing the
    /// stack depth) and undo/redo applications.
    ///
    /// The unsaved-changes check uses it to notice a project mutation that did
    /// *not* come through here — see `SynthApp::observe_untracked_mutation`.
    mutations: u64,
}

impl UndoManager {
    /// Create a new empty undo manager.
    pub(crate) fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            next_entry_id: 1,
            last_push: None,
            mutations: 0,
        }
    }

    /// Allocate an id for a new (or newly merged-into) history entry.
    fn issue_entry_id(&mut self) -> u64 {
        let id = self.next_entry_id;
        self.next_entry_id = self.next_entry_id.wrapping_add(1);
        id
    }

    /// Push a new action onto the undo stack and clear the redo stack.
    ///
    /// History is bounded two ways: by entry count ([`MAX_UNDO_HISTORY`]) and by
    /// the sample audio it keeps alive ([`MAX_UNDO_AUDIO_BYTES`]). Both trim the
    /// oldest entries first, which is what a user expects — the far end of the
    /// history is the part they are least likely to reach for.
    pub(crate) fn push(&mut self, action: UndoAction) {
        self.mutations = self.mutations.wrapping_add(1);
        self.redo_stack.clear();
        if self.merge_into_previous(&action) {
            return;
        }
        let id = self.issue_entry_id();
        self.undo_stack.push(HistoryEntry { id, action });
        // Trim history if it exceeds the maximum.
        if self.undo_stack.len() > MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.trim_to_audio_budget();
    }

    /// Fold `action` into the newest entry when both edit the same control.
    ///
    /// Turning a knob emits one change per frame with no signal for when the
    /// gesture ends, so without this a single sweep would leave a hundred undo
    /// steps. Merging keeps the *original* `old` and adopts the latest `new`,
    /// so one Ctrl+Z returns to where the knob started.
    ///
    /// Bounded by [`PARAMETER_MERGE_WINDOW`]: two deliberate edits to the same
    /// knob a few seconds apart stay separate steps, which is what a user
    /// expects when they come back to a control.
    ///
    /// Returns whether the action was absorbed.
    fn merge_into_previous(&mut self, action: &UndoAction) -> bool {
        let Some(key) = action.merge_key() else {
            return false;
        };
        let now = std::time::Instant::now();
        let within_window = self
            .last_push
            .is_some_and(|last| now.duration_since(last) < PARAMETER_MERGE_WINDOW);
        self.last_push = Some(now);
        if !within_window {
            return false;
        }

        if self
            .undo_stack
            .last()
            .is_none_or(|previous| previous.action.merge_key() != Some(key))
        {
            return false;
        }
        // Issued before the borrow below, and simply unused if the merge turns
        // out not to apply — ids only have to be unique, not contiguous.
        let id = self.issue_entry_id();
        let Some(previous) = self.undo_stack.last_mut() else {
            return false;
        };
        if !previous.action.merge_from(action) {
            return false;
        }
        // A merged entry describes a different state than it did a moment ago,
        // so it gets a fresh identity. Without that, saving mid-gesture and
        // then dragging further would leave the project reading as clean.
        previous.id = id;
        true
    }

    /// Drop the oldest entries until the retained sample audio is within
    /// budget.
    ///
    /// The newest entry is never dropped: an edit the user just made must stay
    /// undoable even if it alone exceeds the budget. Otherwise normalizing one
    /// enormous sample would be silently irreversible, which is worse than
    /// briefly exceeding a memory target.
    fn trim_to_audio_budget(&mut self) {
        while self.retained_audio_bytes() > MAX_UNDO_AUDIO_BYTES && self.undo_stack.len() > 1 {
            self.undo_stack.remove(0);
        }
    }

    /// Total sample audio held alive by both stacks.
    ///
    /// Recomputed rather than tracked incrementally: the stacks are capped at
    /// [`MAX_UNDO_HISTORY`] entries and this runs once per edit, so the walk is
    /// far cheaper than the bookkeeping errors an incremental counter invites
    /// when entries move between the stacks on every undo and redo.
    fn retained_audio_bytes(&self) -> usize {
        self.undo_stack
            .iter()
            .chain(self.redo_stack.iter())
            .map(|entry| entry.action.audio_bytes())
            .sum()
    }

    /// Undo the most recent action.
    ///
    /// Pops from undo stack, pushes to redo stack, and returns the
    /// **inverse** action that should be executed to reverse the change.
    pub(crate) fn undo(&mut self) -> Option<UndoAction> {
        let entry = self.undo_stack.pop()?;
        self.mutations = self.mutations.wrapping_add(1);
        let inverse = Self::inverse(&entry.action);
        self.redo_stack.push(entry);
        Some(inverse)
    }

    /// Redo the most recently undone action.
    ///
    /// Pops from redo stack, pushes back to undo stack, and returns
    /// the **original** action that should be re-executed.
    pub(crate) fn redo(&mut self) -> Option<UndoAction> {
        let entry = self.redo_stack.pop()?;
        self.mutations = self.mutations.wrapping_add(1);
        let action = entry.action.clone();
        self.undo_stack.push(entry);
        Some(action)
    }

    /// The action the next [`Self::undo`] will reverse.
    ///
    /// For callers that must bring a captured snapshot up to date against live
    /// state before the entry is applied and moves to the redo stack — see the
    /// effect-addition refresh in `undo_flow`. Amending a payload this way
    /// deliberately leaves the entry's id alone: it still describes the same
    /// edit, so the saved-position comparison is unaffected.
    pub(crate) fn peek_undo_mut(&mut self) -> Option<&mut UndoAction> {
        self.undo_stack.last_mut().map(|entry| &mut entry.action)
    }

    /// Where the history stands right now.
    ///
    /// The unsaved-changes check compares this against the position at save
    /// time: coming back to the same position means the same edits have been
    /// undone as were made, so the project is back at the saved state.
    ///
    /// Depth alone is not enough. Undoing one edit and then making a different
    /// one returns to the depth the save was taken at while leaving the project
    /// two edits away from the file on disk — reading that as clean drops the
    /// `*`, skips the quit prompt, and stops autosave. The newest entry's id
    /// distinguishes the two routes.
    pub(crate) fn position(&self) -> HistoryPosition {
        (
            self.undo_stack.len(),
            self.undo_stack.last().map_or(0, |entry| entry.id),
        )
    }

    /// Monotonic count of mutations this manager has seen. See [`Self::mutations`].
    pub(crate) fn mutation_count(&self) -> u64 {
        self.mutations
    }

    /// Whether there are actions available to undo.
    pub(crate) fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether there are actions available to redo.
    pub(crate) fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clear all undo and redo history.
    pub(crate) fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_push = None;
    }

    /// Create the inverse of an action (swaps old/new, add/remove).
    fn inverse(action: &UndoAction) -> UndoAction {
        match action {
            UndoAction::AddNote { pattern_id, note } => UndoAction::RemoveNote {
                pattern_id: *pattern_id,
                note: note.clone(),
            },
            UndoAction::RemoveNote { pattern_id, note } => UndoAction::AddNote {
                pattern_id: *pattern_id,
                note: note.clone(),
            },
            UndoAction::SwapPattern {
                pattern_id,
                old,
                new,
            } => UndoAction::SwapPattern {
                pattern_id: *pattern_id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetTimeSignature { old, new } => UndoAction::SetTimeSignature {
                old: *new,
                new: *old,
            },
            UndoAction::FreezePattern { pattern_id, before } => UndoAction::RestorePattern {
                pattern_id: *pattern_id,
                snapshot: before.clone(),
            },
            UndoAction::RestorePattern {
                pattern_id,
                snapshot,
            } => UndoAction::FreezePattern {
                pattern_id: *pattern_id,
                before: snapshot.clone(),
            },
            UndoAction::SetNoteOrnament {
                pattern_id,
                note_id,
                old,
                new,
            } => UndoAction::SetNoteOrnament {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old: *new,
                new: *old,
            },
            UndoAction::MoveNote {
                pattern_id,
                note_id,
                old_start,
                new_start,
            } => UndoAction::MoveNote {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_start: *new_start,
                new_start: *old_start,
            },
            UndoAction::ResizeNote {
                pattern_id,
                note_id,
                old_duration,
                new_duration,
            } => UndoAction::ResizeNote {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_duration: *new_duration,
                new_duration: *old_duration,
            },
            UndoAction::TransposeNote {
                pattern_id,
                note_id,
                old_pitch,
                new_pitch,
            } => UndoAction::TransposeNote {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_pitch: *new_pitch,
                new_pitch: *old_pitch,
            },
            UndoAction::SetNoteVelocity {
                pattern_id,
                note_id,
                old_velocity,
                new_velocity,
            } => UndoAction::SetNoteVelocity {
                pattern_id: *pattern_id,
                note_id: *note_id,
                old_velocity: *new_velocity,
                new_velocity: *old_velocity,
            },
            UndoAction::SetVelocitiesBatch {
                pattern_id,
                changes,
            } => UndoAction::SetVelocitiesBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetLegatoBatch {
                pattern_id,
                changes,
            } => UndoAction::SetLegatoBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetGlideBatch {
                pattern_id,
                changes,
            } => UndoAction::SetGlideBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetExpressionBatch {
                pattern_id,
                changes,
            } => UndoAction::SetExpressionBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetLaneBatch {
                pattern_id,
                changes,
            } => UndoAction::SetLaneBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::SetNoteGraphBindingBatch {
                pattern_id,
                changes,
            } => UndoAction::SetNoteGraphBindingBatch {
                pattern_id: *pattern_id,
                changes: changes
                    .iter()
                    .map(|(id, old, new)| (*id, *new, *old))
                    .collect(),
            },
            UndoAction::RenamePattern {
                pattern_id,
                old_name,
                new_name,
            } => UndoAction::RenamePattern {
                pattern_id: *pattern_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            },
            UndoAction::RenameTrack {
                track_id,
                old_name,
                new_name,
            } => UndoAction::RenameTrack {
                track_id: *track_id,
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            },
            UndoAction::SetPatternLength {
                pattern_id,
                old_length,
                new_length,
            } => UndoAction::SetPatternLength {
                pattern_id: *pattern_id,
                old_length: *new_length,
                new_length: *old_length,
            },
            UndoAction::SetTempo { tick, old, new } => UndoAction::SetTempo {
                tick: *tick,
                old: *new,
                new: *old,
            },
            UndoAction::MoveTempo { old, new } => UndoAction::MoveTempo {
                old: *new,
                new: *old,
            },
            UndoAction::MovePlacement {
                pattern_id,
                old_track_id,
                old_start,
                new_track_id,
                new_start,
            } => UndoAction::MovePlacement {
                pattern_id: *pattern_id,
                old_track_id: *new_track_id,
                old_start: *new_start,
                new_track_id: *old_track_id,
                new_start: *old_start,
            },
            UndoAction::AddAutomationPoint {
                pattern_id,
                target,
                tick,
                value,
                curve,
            } => UndoAction::RemoveAutomationPoint {
                pattern_id: *pattern_id,
                target: target.clone(),
                tick: *tick,
                value: *value,
                curve: *curve,
            },
            UndoAction::RemoveAutomationPoint {
                pattern_id,
                target,
                tick,
                value,
                curve,
            } => UndoAction::AddAutomationPoint {
                pattern_id: *pattern_id,
                target: target.clone(),
                tick: *tick,
                value: *value,
                curve: *curve,
            },
            UndoAction::DeleteTrack {
                track,
                track_index,
                placements,
            } => UndoAction::AddTrack {
                track: track.clone(),
                track_index: *track_index,
                placements: placements.clone(),
            },
            UndoAction::AddTrack {
                track,
                track_index,
                placements,
            } => UndoAction::DeleteTrack {
                track: track.clone(),
                track_index: *track_index,
                placements: placements.clone(),
            },
            UndoAction::DeletePattern {
                pattern,
                placements,
            } => UndoAction::AddPattern {
                pattern: pattern.clone(),
                placements: placements.clone(),
            },
            UndoAction::AddPattern {
                pattern,
                placements,
            } => UndoAction::DeletePattern {
                pattern: pattern.clone(),
                placements: placements.clone(),
            },
            UndoAction::RemovePlacement { placement } => UndoAction::InsertPlacement {
                placement: placement.clone(),
            },
            UndoAction::InsertPlacement { placement } => UndoAction::RemovePlacement {
                placement: placement.clone(),
            },
            UndoAction::SetPlacementLength {
                pattern_id,
                track_id,
                start,
                old_length,
                new_length,
            } => UndoAction::SetPlacementLength {
                pattern_id: *pattern_id,
                track_id: *track_id,
                start: *start,
                old_length: *new_length,
                new_length: *old_length,
            },
            UndoAction::SetPlacementLoopMode {
                pattern_id,
                track_id,
                start,
                old_mode,
                new_mode,
            } => UndoAction::SetPlacementLoopMode {
                pattern_id: *pattern_id,
                track_id: *track_id,
                start: *start,
                old_mode: *new_mode,
                new_mode: *old_mode,
            },
            UndoAction::SetArrangementSections { old, new } => UndoAction::SetArrangementSections {
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::MoveAutomationPoint {
                pattern_id,
                target,
                old_tick,
                old_value,
                new_tick,
                new_value,
                curve,
            } => UndoAction::MoveAutomationPoint {
                pattern_id: *pattern_id,
                target: target.clone(),
                old_tick: *new_tick,
                old_value: *new_value,
                new_tick: *old_tick,
                new_value: *old_value,
                curve: *curve,
            },
            UndoAction::SetAutomationPointCurve {
                pattern_id,
                target,
                tick,
                value,
                old_curve,
                new_curve,
            } => UndoAction::SetAutomationPointCurve {
                pattern_id: *pattern_id,
                target: target.clone(),
                tick: *tick,
                value: *value,
                old_curve: *new_curve,
                new_curve: *old_curve,
            },
            UndoAction::AddAutomationLane { pattern_id, lane } => {
                UndoAction::RemoveAutomationLane {
                    pattern_id: *pattern_id,
                    lane: lane.clone(),
                }
            }
            UndoAction::RemoveAutomationLane { pattern_id, lane } => {
                UndoAction::AddAutomationLane {
                    pattern_id: *pattern_id,
                    lane: lane.clone(),
                }
            }
            UndoAction::AddConnection {
                instrument_id,
                connection,
            } => UndoAction::RemoveConnection {
                instrument_id: *instrument_id,
                connection: *connection,
            },
            UndoAction::RemoveConnection {
                instrument_id,
                connection,
            } => UndoAction::AddConnection {
                instrument_id: *instrument_id,
                connection: *connection,
            },
            UndoAction::SetNoteGraph { graph_id, old, new } => UndoAction::SetNoteGraph {
                graph_id: *graph_id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetModGraph { graph_id, old, new } => UndoAction::SetModGraph {
                graph_id: *graph_id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetPatternNoteGraph {
                pattern_id,
                old,
                new,
            } => UndoAction::SetPatternNoteGraph {
                pattern_id: *pattern_id,
                old: *new,
                new: *old,
            },
            // ── Mixer ──
            // All four are plain old/new swaps; the value types differ but the
            // shape does not.
            UndoAction::SetTrackMixer {
                track_id,
                param,
                old,
                new,
            } => UndoAction::SetTrackMixer {
                track_id: *track_id,
                param: *param,
                old: *new,
                new: *old,
            },
            UndoAction::SetTrackSend {
                track_id,
                return_bus,
                old,
                new,
            } => UndoAction::SetTrackSend {
                track_id: *track_id,
                return_bus: *return_bus,
                old: *new,
                new: *old,
            },
            UndoAction::SetReturnBusMixer {
                bus_id,
                param,
                old,
                new,
            } => UndoAction::SetReturnBusMixer {
                bus_id: *bus_id,
                param: *param,
                old: *new,
                new: *old,
            },
            UndoAction::SetMasterVolume { old, new } => UndoAction::SetMasterVolume {
                old: *new,
                new: *old,
            },
            UndoAction::SetReturnSend {
                from,
                target,
                old,
                new,
            } => UndoAction::SetReturnSend {
                from: *from,
                target: *target,
                old: *new,
                new: *old,
            },
            UndoAction::SetReturnBus {
                effects,
                bus_id,
                index,
                old,
                new,
            } => UndoAction::SetReturnBus {
                // The chain belongs to the bus, not to a direction: the same
                // effects are torn down on delete and rebuilt on restore.
                effects: effects.clone(),
                bus_id: *bus_id,
                index: *index,
                old: new.clone(),
                new: old.clone(),
            },

            // ── Instrument ──
            UndoAction::SetInstrumentSettings {
                instrument_id,
                old,
                new,
            } => UndoAction::SetInstrumentSettings {
                instrument_id: *instrument_id,
                old: new.clone(),
                new: old.clone(),
            },

            UndoAction::SetModuleParameter {
                instrument_id,
                module_id,
                old,
                new,
            } => UndoAction::SetModuleParameter {
                instrument_id: *instrument_id,
                module_id: *module_id,
                old: *new,
                new: *old,
            },

            UndoAction::SetRackModules {
                instrument_id,
                modules,
                connections,
                severed,
                restore,
            } => UndoAction::SetRackModules {
                instrument_id: *instrument_id,
                modules: modules.clone(),
                connections: connections.clone(),
                severed: severed.clone(),
                restore: !restore,
            },

            // ── Return / master effect chains ──
            UndoAction::SetChainEffect { chain, old, new } => UndoAction::SetChainEffect {
                chain: *chain,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetChainEffectParameter {
                chain,
                module_id,
                old,
                new,
            } => UndoAction::SetChainEffectParameter {
                chain: *chain,
                module_id: *module_id,
                old: *new,
                new: *old,
            },
            UndoAction::SetChainEffectBypass {
                chain,
                module_id,
                old,
                new,
            } => UndoAction::SetChainEffectBypass {
                chain: *chain,
                module_id: *module_id,
                old: *new,
                new: *old,
            },
            UndoAction::SetEffectChainOrder {
                instrument_id,
                old,
                new,
            } => UndoAction::SetEffectChainOrder {
                instrument_id: *instrument_id,
                old: new.clone(),
                new: old.clone(),
            },

            // ── Sample library ──
            UndoAction::SetSample { id, old, new } => UndoAction::SetSample {
                id: *id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetSampleMeta { id, old, new } => UndoAction::SetSampleMeta {
                id: *id,
                old: new.clone(),
                new: old.clone(),
            },
            UndoAction::SetSampleData { id, old, new } => UndoAction::SetSampleData {
                id: *id,
                old: std::sync::Arc::clone(new),
                new: std::sync::Arc::clone(old),
            },

            UndoAction::Composite(actions) => {
                // Reverse the order and invert each action.
                UndoAction::Composite(actions.iter().rev().map(Self::inverse).collect())
            }
        }
    }
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::assert_matches;

    fn test_pattern_id() -> PatternId {
        PatternId(1)
    }

    fn test_note_snapshot() -> NoteSnapshot {
        NoteSnapshot {
            id: NoteId(42),
            start: PatternTick(0),
            duration: Some(SeqDuration(960)),
            pitch: Pitch::MIDDLE_C,
            velocity: Velocity::MF,
            track: None,
            lane: NoteLane::ZERO,
        }
    }

    #[test]
    fn test_push_clears_redo_stack() {
        let mut mgr = UndoManager::new();
        let action = UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        };
        mgr.push(action.clone());
        assert!(mgr.can_undo());

        // Undo puts inverse on redo stack.
        mgr.undo();
        assert!(mgr.can_redo());

        // Push new action clears redo.
        mgr.push(action);
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_undo_returns_action_and_pushes_inverse_to_redo() {
        let mut mgr = UndoManager::new();
        mgr.push(UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        });

        let undone = mgr.undo().unwrap();
        // Undo returns the inverse: RemoveNote (to reverse AddNote).
        assert_matches!(undone, UndoAction::RemoveNote { .. });
        assert!(mgr.can_redo());
        assert!(!mgr.can_undo());
    }

    #[test]
    fn test_redo_returns_action_and_pushes_inverse_to_undo() {
        let mut mgr = UndoManager::new();
        mgr.push(UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        });
        mgr.undo();

        let redone = mgr.redo().unwrap();
        // Redo returns the original action (AddNote) to re-execute.
        assert_matches!(redone, UndoAction::AddNote { .. });
        assert!(mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_undo_empty_returns_none() {
        let mut mgr = UndoManager::new();
        assert!(mgr.undo().is_none());
    }

    #[test]
    fn test_redo_empty_returns_none() {
        let mut mgr = UndoManager::new();
        assert!(mgr.redo().is_none());
    }

    #[test]
    fn test_clear_empties_both_stacks() {
        let mut mgr = UndoManager::new();
        mgr.push(UndoAction::AddNote {
            pattern_id: test_pattern_id(),
            note: test_note_snapshot(),
        });
        mgr.undo();
        assert!(mgr.can_redo());

        mgr.clear();
        assert!(!mgr.can_undo());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn test_max_history_trims_oldest() {
        let mut mgr = UndoManager::new();
        for i in 0..150 {
            mgr.push(UndoAction::MoveNote {
                pattern_id: test_pattern_id(),
                note_id: NoteId(i),
                old_start: PatternTick(0),
                new_start: PatternTick(i as u32),
            });
        }
        // Should be capped at MAX_UNDO_HISTORY.
        assert_eq!(mgr.undo_stack.len(), MAX_UNDO_HISTORY);
    }

    #[test]
    fn test_inverse_of_move_note_swaps_positions() {
        let action = UndoAction::MoveNote {
            pattern_id: test_pattern_id(),
            note_id: NoteId(1),
            old_start: PatternTick(0),
            new_start: PatternTick(100),
        };
        let inv = UndoManager::inverse(&action);
        if let UndoAction::MoveNote {
            old_start,
            new_start,
            ..
        } = inv
        {
            assert_eq!(old_start, PatternTick(100));
            assert_eq!(new_start, PatternTick(0));
        } else {
            panic!("Expected MoveNote inverse");
        }
    }

    #[test]
    fn test_inverse_of_set_automation_point_curve_swaps_curves() {
        use synth_sequencer::{AutomationTarget, CurveType, GlobalParam, PatternTick};
        let apply = UndoAction::SetAutomationPointCurve {
            pattern_id: PatternId(1),
            target: AutomationTarget::Global(GlobalParam::MasterVolume),
            tick: PatternTick(480),
            value: synth_core::NormalizedValue::new(0.5),
            old_curve: CurveType::Linear,
            new_curve: CurveType::Step,
        };
        let inv = UndoManager::inverse(&apply);
        if let UndoAction::SetAutomationPointCurve {
            old_curve,
            new_curve,
            tick,
            ..
        } = inv
        {
            // Inverse swaps the curves and keeps the same tick/value/target.
            assert_eq!(old_curve, CurveType::Step);
            assert_eq!(new_curve, CurveType::Linear);
            assert_eq!(tick, PatternTick(480));
        } else {
            panic!("Expected SetAutomationPointCurve inverse");
        }
    }

    #[test]
    fn test_inverse_of_set_tempo_apply_round_trips() {
        // Apply tempo at a tick that had no explicit change → inverse must be a remove.
        let apply = UndoAction::SetTempo {
            tick: Tick(960),
            old: None,
            new: Some((Bpm::new(140.0), false)),
        };
        let inv = UndoManager::inverse(&apply);
        if let UndoAction::SetTempo { tick, old, new } = inv {
            assert_eq!(tick, Tick(960));
            assert_eq!(old, Some((Bpm::new(140.0), false)));
            assert_eq!(new, None);
        } else {
            panic!("Expected SetTempo inverse");
        }
    }

    #[test]
    fn test_inverse_of_set_tempo_remove_round_trips() {
        // Remove an existing (ramp) tempo change → inverse must re-apply it verbatim.
        let remove = UndoAction::SetTempo {
            tick: Tick(1920),
            old: Some((Bpm::new(96.0), true)),
            new: None,
        };
        let inv = UndoManager::inverse(&remove);
        if let UndoAction::SetTempo { old, new, .. } = inv {
            assert_eq!(old, None);
            assert_eq!(new, Some((Bpm::new(96.0), true)));
        } else {
            panic!("Expected SetTempo inverse");
        }
    }

    #[test]
    fn test_inverse_of_move_tempo_swaps_endpoints() {
        // Dragging a point (tick + bpm change) → inverse must restore the origin.
        let mv = UndoAction::MoveTempo {
            old: (Tick(960), Bpm::new(120.0), false),
            new: (Tick(1440), Bpm::new(150.0), false),
        };
        let inv = UndoManager::inverse(&mv);
        if let UndoAction::MoveTempo { old, new } = inv {
            assert_eq!(old, (Tick(1440), Bpm::new(150.0), false));
            assert_eq!(new, (Tick(960), Bpm::new(120.0), false));
        } else {
            panic!("Expected MoveTempo inverse");
        }
    }

    #[test]
    fn test_inverse_of_section_snapshot_swaps_lists() {
        let old = vec![synth_sequencer::ArrangementSection::new(
            synth_sequencer::SectionId::new(1),
            "Verse",
            synth_sequencer::SectionKind::Verse,
            Tick(0),
            SeqDuration(3_840),
        )];
        let new = vec![synth_sequencer::ArrangementSection::new(
            synth_sequencer::SectionId::new(2),
            "Chorus",
            synth_sequencer::SectionKind::Chorus,
            Tick(3_840),
            SeqDuration(3_840),
        )];
        let inverse = UndoManager::inverse(&UndoAction::SetArrangementSections {
            old: old.clone(),
            new: new.clone(),
        });

        assert_matches!(
            inverse,
            UndoAction::SetArrangementSections {
                old: inverse_old,
                new: inverse_new
            } if inverse_old == new && inverse_new == old
        );
    }

    #[test]
    fn test_inverse_of_composite_reverses_order() {
        let action = UndoAction::Composite(vec![
            UndoAction::AddNote {
                pattern_id: test_pattern_id(),
                note: test_note_snapshot(),
            },
            UndoAction::MoveNote {
                pattern_id: test_pattern_id(),
                note_id: NoteId(1),
                old_start: PatternTick(0),
                new_start: PatternTick(100),
            },
        ]);
        let inv = UndoManager::inverse(&action);
        if let UndoAction::Composite(actions) = inv {
            assert_eq!(actions.len(), 2);
            // First in inverse should be inverse of MoveNote (was second).
            assert_matches!(actions[0], UndoAction::MoveNote { .. });
            // Second should be inverse of AddNote (was first) => RemoveNote.
            assert_matches!(actions[1], UndoAction::RemoveNote { .. });
        } else {
            panic!("Expected Composite inverse");
        }
    }

    // ── Mixer ──

    #[test]
    fn inverse_of_a_track_mixer_change_swaps_the_values() {
        let action = UndoAction::SetTrackMixer {
            track_id: TrackId(1),
            param: TrackMixerParam::Volume,
            old: MixerValue::Level(NormalizedValue::new(0.8)),
            new: MixerValue::Level(NormalizedValue::new(0.2)),
        };

        let UndoAction::SetTrackMixer {
            param, old, new, ..
        } = UndoManager::inverse(&action)
        else {
            panic!("inverse must stay a SetTrackMixer");
        };
        assert_eq!(param, TrackMixerParam::Volume);
        assert_eq!(old, MixerValue::Level(NormalizedValue::new(0.2)));
        assert_eq!(new, MixerValue::Level(NormalizedValue::new(0.8)));
    }

    /// Undoing twice must land back where it started — the property that makes
    /// redo work.
    #[test]
    fn a_track_mixer_change_round_trips_through_two_inversions() {
        let action = UndoAction::SetTrackMixer {
            track_id: TrackId(1),
            param: TrackMixerParam::Pan,
            old: MixerValue::Balance(BipolarValue::new(-1.0)),
            new: MixerValue::Balance(BipolarValue::new(0.5)),
        };

        let back = UndoManager::inverse(&UndoManager::inverse(&action));

        let UndoAction::SetTrackMixer { old, new, .. } = back else {
            panic!("expected SetTrackMixer");
        };
        assert_eq!(old, MixerValue::Balance(BipolarValue::new(-1.0)));
        assert_eq!(new, MixerValue::Balance(BipolarValue::new(0.5)));
    }

    /// The master fader is the one mixer control that is not a `Song` field, so
    /// it carries its own action — but it has to invert like every other one.
    #[test]
    fn inverse_of_a_master_volume_change_swaps_the_values() {
        let action = UndoAction::SetMasterVolume {
            old: MixerValue::Level(NormalizedValue::new(1.0)),
            new: MixerValue::Level(NormalizedValue::new(0.32)),
        };

        let UndoAction::SetMasterVolume { old, new } = UndoManager::inverse(&action) else {
            panic!("inverse must stay a SetMasterVolume");
        };
        assert_eq!(old, MixerValue::Level(NormalizedValue::new(0.32)));
        assert_eq!(new, MixerValue::Level(NormalizedValue::new(1.0)));
    }

    #[test]
    fn a_master_volume_change_round_trips_through_two_inversions() {
        let action = UndoAction::SetMasterVolume {
            old: MixerValue::Level(NormalizedValue::new(1.0)),
            new: MixerValue::Level(NormalizedValue::new(0.32)),
        };

        let back = UndoManager::inverse(&UndoManager::inverse(&action));

        let UndoAction::SetMasterVolume { old, new } = back else {
            panic!("expected SetMasterVolume");
        };
        assert_eq!(old, MixerValue::Level(NormalizedValue::new(1.0)));
        assert_eq!(new, MixerValue::Level(NormalizedValue::new(0.32)));
    }

    /// Creating a send and deleting one are the same action seen from opposite
    /// sides, so one inverse covers both.
    #[test]
    fn inverse_of_creating_a_send_removes_it() {
        let send = TrackSend {
            target: ReturnBusId(2),
            level: NormalizedValue::new(0.4),
            pre_fader: true,
            enabled: true,
        };
        let action = UndoAction::SetTrackSend {
            track_id: TrackId(1),
            return_bus: ReturnBusId(2),
            old: None,
            new: Some(send),
        };

        let UndoAction::SetTrackSend { old, new, .. } = UndoManager::inverse(&action) else {
            panic!("expected SetTrackSend");
        };
        assert_eq!(old, Some(send));
        assert_eq!(new, None, "undoing a created send must remove it");
    }

    /// Deleting a return bus has to restore everything about it, not just its
    /// id — otherwise undo silently loses the name and fader.
    #[test]
    fn inverse_of_deleting_a_return_bus_restores_the_whole_bus() {
        let mut bus = ReturnBus::new(ReturnBusId(3), "Plate");
        bus.volume = NormalizedValue::new(0.33);
        let action = UndoAction::SetReturnBus {
            effects: Vec::new(),
            bus_id: ReturnBusId(3),
            index: 1,
            old: Some(Box::new(bus.clone())),
            new: None,
        };

        let UndoAction::SetReturnBus {
            index, old, new, ..
        } = UndoManager::inverse(&action)
        else {
            panic!("expected SetReturnBus");
        };
        assert_eq!(index, 1, "the strip position must survive the inversion");
        assert!(old.is_none());
        let restored = new.expect("undo must restore the bus");
        assert_eq!(restored.name, "Plate");
        assert_eq!(restored.volume, NormalizedValue::new(0.33));
    }

    // ── Drag coalescing ──

    /// The whole point: a drag that fires `changed()` on every frame must leave
    /// one entry carrying the value from *before* the gesture.
    #[test]
    fn a_drag_coalesces_to_the_value_it_started_from() {
        let mut coalescer = DragCoalescer::default();
        let id = egui::Id::new("fader");

        // Frame 1 of the drag.
        coalescer.begin(
            id,
            GestureStart::Mixer(MixerValue::Level(NormalizedValue::new(0.5))),
        );
        // Frames 2..n report the intermediate values, which must be ignored.
        for step in 1_u8..20 {
            let intermediate = 0.5 - f32::from(step) * 0.02_f32;
            coalescer.begin(
                id,
                GestureStart::Mixer(MixerValue::Level(NormalizedValue::new(intermediate))),
            );
        }

        assert_eq!(
            coalescer.take(id),
            Some(GestureStart::Mixer(MixerValue::Level(
                NormalizedValue::new(0.5)
            ))),
            "the pre-gesture value must survive the whole drag",
        );
    }

    #[test]
    fn a_finished_gesture_stops_being_tracked() {
        let mut coalescer = DragCoalescer::default();
        let id = egui::Id::new("fader");
        coalescer.begin(id, GestureStart::Mixer(MixerValue::Flag(false)));

        assert!(coalescer.is_tracking(id));
        assert!(coalescer.take(id).is_some());
        assert!(!coalescer.is_tracking(id));
        assert!(coalescer.take(id).is_none(), "a gesture ends only once");
    }

    /// Two faders dragged one after another must not share a gesture.
    #[test]
    fn separate_widgets_keep_separate_gestures() {
        let mut coalescer = DragCoalescer::default();
        let volume = egui::Id::new("volume");
        let pan = egui::Id::new("pan");

        coalescer.begin(
            volume,
            GestureStart::Mixer(MixerValue::Level(NormalizedValue::new(0.9))),
        );
        coalescer.begin(
            pan,
            GestureStart::Mixer(MixerValue::Balance(BipolarValue::new(-0.5))),
        );

        assert_eq!(
            coalescer.take(volume),
            Some(GestureStart::Mixer(MixerValue::Level(
                NormalizedValue::new(0.9)
            ))),
        );
        assert_eq!(
            coalescer.take(pan),
            Some(GestureStart::Mixer(MixerValue::Balance(BipolarValue::new(
                -0.5
            )))),
        );
    }

    /// A send gesture carries the whole send, so dragging the level through
    /// zero (which deletes it) can still be undone back to a working send.
    #[test]
    fn a_send_gesture_remembers_the_send_it_started_from() {
        let mut coalescer = DragCoalescer::default();
        let id = egui::Id::new("send");
        let before = Some(TrackSend {
            target: ReturnBusId(1),
            level: NormalizedValue::new(0.7),
            pre_fader: true,
            enabled: false,
        });

        coalescer.begin(id, GestureStart::TrackSend(before));
        // Dragging to zero deletes the send; the gesture must not adopt that.
        coalescer.begin(id, GestureStart::TrackSend(None));

        let Some(GestureStart::TrackSend(Some(restored))) = coalescer.take(id) else {
            panic!("the gesture must yield the send it started from");
        };
        assert_eq!(restored.level, NormalizedValue::new(0.7));
        assert!(restored.pre_fader, "the tap point must survive");
        assert!(!restored.enabled, "the bypass flag must survive");
    }

    // ── Sample library ──

    fn sample_of(frames: usize) -> Sample {
        Sample::new(
            SampleMeta {
                id: SampleId::new(1),
                name: "kick".to_string(),
                description: String::new(),
                sample_rate: synth_core::audio::DeviceSampleRate::new(44_100),
                channels: synth_core::ChannelCount::Mono,
                frame_count: synth_core::SampleCount::new(frames),
                root_note: None,
                loop_region: None,
                crop: None,
                source: synth_sampler::SampleSource::Generated,
            },
            vec![0.0_f32; frames].into(),
        )
    }

    /// Deleting a sample must be reversible, and the inverse must carry the
    /// audio back — this is the one sample edit that destroys everything.
    #[test]
    fn inverse_of_deleting_a_sample_restores_it() {
        let sample = sample_of(64);
        let action = UndoAction::SetSample {
            id: SampleId::new(1),
            old: Some(Box::new(sample)),
            new: None,
        };

        let UndoAction::SetSample { old, new, .. } = UndoManager::inverse(&action) else {
            panic!("expected SetSample");
        };
        assert!(old.is_none());
        assert_eq!(new.expect("restored").data.len(), 64);
    }

    #[test]
    fn inverse_of_a_metadata_edit_swaps_the_two_sides() {
        let mut before = sample_of(4).meta;
        before.name = "before".to_string();
        let mut after = before.clone();
        after.name = "after".to_string();

        let action = UndoAction::SetSampleMeta {
            id: SampleId::new(1),
            old: Box::new(before),
            new: Box::new(after),
        };

        let UndoAction::SetSampleMeta { old, new, .. } = UndoManager::inverse(&action) else {
            panic!("expected SetSampleMeta");
        };
        assert_eq!(old.name, "after");
        assert_eq!(new.name, "before");
    }

    /// A destructive DSP edit must be undoable back to the original audio.
    #[test]
    fn inverse_of_a_destructive_edit_restores_the_original_audio() {
        let original: std::sync::Arc<[f32]> = vec![0.25_f32; 8].into();
        let edited: std::sync::Arc<[f32]> = vec![1.0_f32; 8].into();
        let action = UndoAction::SetSampleData {
            id: SampleId::new(1),
            old: std::sync::Arc::clone(&original),
            new: std::sync::Arc::clone(&edited),
        };

        let UndoAction::SetSampleData { new, .. } = UndoManager::inverse(&action) else {
            panic!("expected SetSampleData");
        };
        assert!(
            std::sync::Arc::ptr_eq(&new, &original),
            "undo must hand back the original buffer, not a copy",
        );
    }

    // ── Audio budget ──

    /// Only the sample variants pin audio; everything else must report zero, or
    /// the budget would trim history for edits that cost nothing.
    #[test]
    fn non_sample_entries_hold_no_audio() {
        let action = UndoAction::SetTrackMixer {
            track_id: TrackId(1),
            param: TrackMixerParam::Volume,
            old: MixerValue::Level(NormalizedValue::MIN),
            new: MixerValue::Level(NormalizedValue::MAX),
        };
        assert_eq!(action.audio_bytes(), 0);
    }

    #[test]
    fn a_destructive_edit_reports_both_buffers() {
        let action = UndoAction::SetSampleData {
            id: SampleId::new(1),
            old: vec![0.0_f32; 100].into(),
            new: vec![0.0_f32; 100].into(),
        };
        assert_eq!(action.audio_bytes(), 200 * std::mem::size_of::<f32>());
    }

    /// The budget must actually evict: without it, a long editing session on a
    /// large sample would pin gigabytes for the life of the process.
    #[test]
    fn the_audio_budget_evicts_the_oldest_entries() {
        let mut manager = UndoManager::new();
        // Each entry pins ~32 MiB (two 16 MiB buffers), so nine of them exceed
        // the 256 MiB budget.
        let frames = 4 * 1024 * 1024;
        for _ in 0..9 {
            manager.push(UndoAction::SetSampleData {
                id: SampleId::new(1),
                old: vec![0.0_f32; frames].into(),
                new: vec![0.0_f32; frames].into(),
            });
        }

        assert!(
            manager.retained_audio_bytes() <= MAX_UNDO_AUDIO_BYTES,
            "retained {} bytes, budget is {MAX_UNDO_AUDIO_BYTES}",
            manager.retained_audio_bytes(),
        );
        assert!(
            manager.can_undo(),
            "trimming must never empty the stack — the newest edit stays undoable",
        );
    }

    /// One edit larger than the whole budget must still be undoable: silently
    /// making a destructive operation irreversible is worse than exceeding a
    /// memory target for one entry.
    #[test]
    fn an_oversized_edit_is_still_undoable() {
        let mut manager = UndoManager::new();
        let frames = MAX_UNDO_AUDIO_BYTES / std::mem::size_of::<f32>();
        manager.push(UndoAction::SetSampleData {
            id: SampleId::new(1),
            old: vec![0.0_f32; frames].into(),
            new: vec![0.0_f32; frames].into(),
        });

        assert!(manager.can_undo());
    }

    /// Ordinary editing must never trip the budget — a session of metadata
    /// tweaks should keep its full history.
    #[test]
    fn metadata_editing_never_trims_for_audio() {
        let mut manager = UndoManager::new();
        for i in 0..MAX_UNDO_HISTORY {
            let mut before = sample_of(4).meta;
            before.name = format!("name {i}");
            let after = before.clone();
            manager.push(UndoAction::SetSampleMeta {
                id: SampleId::new(1),
                old: Box::new(before),
                new: Box::new(after),
            });
        }
        assert_eq!(manager.retained_audio_bytes(), 0);
        assert!(manager.can_undo());
    }

    // ── Depth and mutation counting (the unsaved-changes shortcut) ──

    fn note_edit() -> UndoAction {
        UndoAction::RenameTrack {
            track_id: TrackId(1),
            old_name: "a".to_string(),
            new_name: "b".to_string(),
        }
    }

    /// The property the clean-state check rests on: undoing every edit made
    /// since a save returns the history to the position it held then.
    #[test]
    fn undoing_every_edit_returns_to_the_saved_position() {
        let mut manager = UndoManager::new();
        manager.push(note_edit());
        let saved = manager.position();

        manager.push(note_edit());
        manager.push(note_edit());
        assert_ne!(manager.position(), saved, "edits move the position");

        manager.undo();
        manager.undo();
        assert_eq!(manager.position(), saved, "undoing them returns it");
    }

    /// Redoing past the save point must not read as clean.
    #[test]
    fn redoing_past_the_saved_position_moves_away_from_it() {
        let mut manager = UndoManager::new();
        let saved = manager.position();
        manager.push(note_edit());
        manager.undo();
        assert_eq!(manager.position(), saved);

        manager.redo();
        assert_ne!(manager.position(), saved);
    }

    /// Undoing a pre-save edit and then making a different one lands on the
    /// saved *depth* by another route — the project is two edits away from the
    /// file, and reading that as clean would drop the unsaved marker, skip the
    /// quit prompt and stop autosave.
    #[test]
    fn a_new_edit_after_undoing_past_the_save_is_not_the_saved_position() {
        let mut manager = UndoManager::new();
        manager.push(note_edit());
        manager.push(note_edit());
        let saved = manager.position();

        manager.undo();
        manager.push(note_edit());

        assert_eq!(
            manager.position().0,
            saved.0,
            "same depth, by another route"
        );
        assert_ne!(manager.position(), saved, "but not the saved state");
    }

    /// Redo, on the other hand, really does return to the saved state, so it
    /// must compare equal — otherwise undo-then-redo would leave the project
    /// permanently marked unsaved.
    #[test]
    fn undo_then_redo_returns_to_the_saved_position() {
        let mut manager = UndoManager::new();
        manager.push(note_edit());
        let saved = manager.position();

        manager.undo();
        manager.redo();

        assert_eq!(manager.position(), saved);
    }

    /// Every kind of mutation must bump the counter, or a change that bypassed
    /// the manager would be indistinguishable from one it recorded — and the
    /// depth shortcut would wrongly call the project clean.
    #[test]
    fn every_mutation_bumps_the_counter() {
        let mut manager = UndoManager::new();
        let start = manager.mutation_count();

        manager.push(note_edit());
        let after_push = manager.mutation_count();
        assert!(after_push > start, "a push must count");

        manager.undo();
        let after_undo = manager.mutation_count();
        assert!(after_undo > after_push, "an undo must count");

        manager.redo();
        assert!(manager.mutation_count() > after_undo, "a redo must count");
    }

    /// A merged parameter edit changes the project without changing the stack
    /// depth. If it did not bump the counter, the untracked-mutation watcher
    /// would mistake it for a change that bypassed the manager.
    #[test]
    fn a_merged_edit_still_counts_as_a_mutation() {
        let mut manager = UndoManager::new();
        let param = |v: f32| UndoAction::SetModuleParameter {
            instrument_id: InstrumentId::FIRST,
            module_id: synth_engine::ModuleId::new(synth_core::ModuleType::Oscillator, 1),
            old: synth_core::Param::Oscillator(synth_core::OscillatorParam::Frequency(
                synth_core::Hertz::new(100.0),
            )),
            new: synth_core::Param::Oscillator(synth_core::OscillatorParam::Frequency(
                synth_core::Hertz::new(v),
            )),
        };

        manager.push(param(200.0));
        let position_after_first = manager.position();
        let count_after_first = manager.mutation_count();

        manager.push(param(300.0));

        assert_eq!(
            manager.position().0,
            position_after_first.0,
            "the second edit should merge, leaving the depth alone",
        );
        assert!(
            manager.mutation_count() > count_after_first,
            "but it must still register as a mutation",
        );
        assert_ne!(
            manager.position(),
            position_after_first,
            "a merged entry is a new state — a save taken mid-drag must not \
             keep reading as clean while the knob moves on",
        );
    }

    /// A merged run of edits must collapse to one step that returns to where
    /// the gesture began: the original `old`, the latest `new`.
    #[test]
    fn merging_keeps_the_first_value_and_the_last() {
        let mut manager = UndoManager::new();
        let param = |from: f32, to: f32| UndoAction::SetChainEffectParameter {
            chain: EffectChain::Master,
            module_id: synth_engine::ModuleId::new(synth_core::ModuleType::Delay, 1),
            old: synth_core::Param::Delay(synth_core::DelayParam::Feedback(
                synth_core::NormalizedValue::new(from),
            )),
            new: synth_core::Param::Delay(synth_core::DelayParam::Feedback(
                synth_core::NormalizedValue::new(to),
            )),
        };

        // One drag: each frame reports the step it made.
        manager.push(param(0.1, 0.2));
        manager.push(param(0.2, 0.3));
        manager.push(param(0.3, 0.4));

        use synth_core::ModuleParam as _;
        assert_eq!(manager.position().0, 1, "a drag is one undo step");
        let UndoAction::SetChainEffectParameter { old, new, .. } = manager
            .undo()
            .map(|inverse| UndoManager::inverse(&inverse))
            .expect("one entry")
        else {
            panic!("expected a chain-effect parameter entry");
        };
        assert_eq!(
            old.as_f32(),
            0.1,
            "undo must return to where the drag began"
        );
        assert_eq!(new.as_f32(), 0.4, "and forget the intermediate steps");
    }

    /// Instrument properties arrive as whole snapshots, once per frame while a
    /// `Vol`/`Pan` drag value is moving. Without merging, one sweep would push
    /// a hundred entries and evict the rest of the history.
    #[test]
    fn instrument_settings_edits_merge_into_one_step() {
        use crate::gui::instrument_rack::InstrumentUiState;

        let mut manager = UndoManager::new();
        let base = InstrumentUiState::new(InstrumentId::FIRST, "Lead").settings();
        let at = |volume: f32| {
            let mut settings = base.clone();
            settings.volume = synth_core::Gain::new(volume);
            settings
        };
        let edit = |from: f32, to: f32| UndoAction::SetInstrumentSettings {
            instrument_id: InstrumentId::FIRST,
            old: Box::new(at(from)),
            new: Box::new(at(to)),
        };

        manager.push(edit(1.0, 0.9));
        manager.push(edit(0.9, 0.8));
        manager.push(edit(0.8, 0.7));

        assert_eq!(manager.position().0, 1, "a fader sweep is one undo step");
    }

    // ── Effect-chain restoration ──

    fn effect_id(instance: u16) -> synth_engine::ModuleId {
        synth_engine::ModuleId::new(synth_core::ModuleType::Reverb, instance)
    }

    /// Restoring only appends, so a mid-chain effect would come back at the
    /// end and silently change what the signal passes through in what order.
    /// The captured chain order has to be replayed after the add.
    #[test]
    fn restoring_a_mid_chain_effect_replays_the_chain_order() {
        let order = vec![effect_id(1), effect_id(2), effect_id(3)];
        let snapshot = EffectSnapshot {
            chain_order: order.clone(),
            module_id: effect_id(2),
            module_type: synth_core::ModuleType::Reverb,
            params: Vec::new(),
            bypassed: false,
        };

        let commands = snapshot.restore_commands(EffectChain::Master);

        let last = commands.last().expect("restore must emit commands");
        match last {
            synth_engine::EngineCommand::SetEffectChainOrder {
                instrument_id,
                order: replayed,
            } => {
                assert_eq!(*instrument_id, None, "master chain");
                assert_eq!(replayed, &order, "the original order must be restored");
            }
            other => panic!("the last command must set the chain order, got {other:?}"),
        }
    }

    /// A return bus takes its own order command, not the master one.
    #[test]
    fn restoring_on_a_return_bus_targets_that_bus() {
        let snapshot = EffectSnapshot {
            chain_order: vec![effect_id(1)],
            module_id: effect_id(1),
            module_type: synth_core::ModuleType::Reverb,
            params: Vec::new(),
            bypassed: false,
        };

        let commands = snapshot.restore_commands(EffectChain::Return(ReturnBusId(4)));

        assert!(
            commands.iter().any(|c| matches!(
                c,
                synth_engine::EngineCommand::SetReturnEffectChainOrder { return_id, .. }
                    if *return_id == ReturnBusId(4)
            )),
            "a return-bus restore must reorder that bus's chain",
        );
    }

    /// A freshly added effect is appended, so there is no position to restore
    /// and no reorder should be emitted — issuing one would be a no-op at best
    /// and could disturb a chain built in order.
    #[test]
    fn restoring_an_appended_effect_emits_no_reorder() {
        let snapshot = EffectSnapshot {
            chain_order: Vec::new(),
            module_id: effect_id(1),
            module_type: synth_core::ModuleType::Reverb,
            params: Vec::new(),
            bypassed: false,
        };

        let commands = snapshot.restore_commands(EffectChain::Master);

        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, synth_engine::EngineCommand::SetEffectChainOrder { .. })),
            "an appended effect needs no reorder",
        );
    }

    // ── Effect-chain reordering ──

    /// Moving a slot is its own edit, and its inverse is the order that was
    /// there before — not a move in the opposite direction, which would swap
    /// whatever pair happens to sit at that index by the time it is replayed.
    #[test]
    fn inverse_of_a_chain_reorder_restores_the_previous_order() {
        let before = vec![effect_id(1), effect_id(2), effect_id(3)];
        let after = vec![effect_id(2), effect_id(1), effect_id(3)];
        let action = UndoAction::SetEffectChainOrder {
            instrument_id: Some(InstrumentId::FIRST),
            old: before.clone(),
            new: after.clone(),
        };

        let UndoAction::SetEffectChainOrder {
            instrument_id,
            old,
            new,
        } = UndoManager::inverse(&action)
        else {
            panic!("expected a chain-order entry");
        };

        assert_eq!(instrument_id, Some(InstrumentId::FIRST));
        assert_eq!(new, before, "undo must put the original order back");
        assert_eq!(old, after, "and redo must return to the reordered one");
    }

    /// The recorded order has to be the one the engine's own swap produces, or
    /// undo would restore a chain the user never saw.
    #[test]
    fn reordering_swaps_with_the_neighbour() {
        let order = vec![effect_id(1), effect_id(2), effect_id(3)];

        assert_eq!(
            reordered_chain(&order, effect_id(3), synth_engine::ReorderDirection::Up),
            Some(vec![effect_id(1), effect_id(3), effect_id(2)]),
            "moving up trades places with the slot before it",
        );
        assert_eq!(
            reordered_chain(&order, effect_id(1), synth_engine::ReorderDirection::Down),
            Some(vec![effect_id(2), effect_id(1), effect_id(3)]),
            "moving down trades places with the slot after it",
        );
    }

    /// A click at either end of the chain moves nothing, so it must not leave a
    /// history step — one that would reorder the chain when undone.
    #[test]
    fn reordering_off_either_end_is_not_an_edit() {
        let order = vec![effect_id(1), effect_id(2)];

        assert_eq!(
            reordered_chain(&order, effect_id(1), synth_engine::ReorderDirection::Up),
            None,
            "the first slot cannot move up",
        );
        assert_eq!(
            reordered_chain(&order, effect_id(2), synth_engine::ReorderDirection::Down),
            None,
            "the last slot cannot move down",
        );
    }

    /// Same reasoning for a module the chain does not hold at all — the engine
    /// ignores it, so nothing happened.
    #[test]
    fn reordering_a_module_off_the_chain_is_not_an_edit() {
        let order = vec![effect_id(1)];

        assert_eq!(
            reordered_chain(&order, effect_id(9), synth_engine::ReorderDirection::Up),
            None,
        );
    }

    // ── Refreshing a captured snapshot ──

    /// An addition's snapshot is refreshed against live state just before it is
    /// undone, so redo brings the effect back as it was when it went away
    /// rather than with the defaults it was created with. Amending the payload
    /// describes the same edit, so it must not move the history position — a
    /// project saved at this point would otherwise start reading as dirty
    /// merely because the user pressed Ctrl+Z.
    #[test]
    fn refreshing_the_top_entry_leaves_the_history_position_alone() {
        let mut manager = UndoManager::new();
        manager.push(UndoAction::SetChainEffect {
            chain: EffectChain::Master,
            old: None,
            new: Some(Box::new(EffectSnapshot {
                chain_order: Vec::new(),
                module_id: effect_id(1),
                module_type: synth_core::ModuleType::Reverb,
                params: Vec::new(),
                bypassed: false,
            })),
        });
        let position = manager.position();
        let mutations = manager.mutation_count();

        let Some(UndoAction::SetChainEffect {
            new: Some(snapshot),
            ..
        }) = manager.peek_undo_mut()
        else {
            panic!("the addition must be on top of the stack");
        };
        snapshot.bypassed = true;

        assert_eq!(
            manager.position(),
            position,
            "refreshing a payload is not a new edit",
        );
        assert_eq!(
            manager.mutation_count(),
            mutations,
            "and it must not register as a mutation",
        );

        let UndoAction::SetChainEffect { old: Some(old), .. } =
            manager.undo().expect("one entry to undo")
        else {
            panic!("the inverse of an addition removes the effect it captured");
        };
        assert!(
            old.bypassed,
            "the refreshed state is what redo will restore",
        );
    }

    /// Bypass has to be replayed after the parameters, or the parameter pass
    /// would leave a bypassed effect audible again.
    #[test]
    fn a_bypassed_effect_comes_back_bypassed() {
        let snapshot = EffectSnapshot {
            chain_order: Vec::new(),
            module_id: effect_id(1),
            module_type: synth_core::ModuleType::Reverb,
            params: Vec::new(),
            bypassed: true,
        };

        let commands = snapshot.restore_commands(EffectChain::Master);

        assert!(
            commands.iter().any(|c| matches!(
                c,
                synth_engine::EngineCommand::SetEffectEnabled { enabled: false, .. }
            )),
            "the bypass state must be restored",
        );
    }
}
