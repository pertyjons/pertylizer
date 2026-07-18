//! Song structure and arrangement.

use serde::{Deserialize, Serialize};

use super::ids::{
    InstrumentId, ModGraphId, NoteGraphId, NoteId, NoteModuleId, PatternId, ReturnBusId, TrackId,
};
use super::mod_grid::{ModGraph, ModGraphScope};
use super::note::Note;
use super::note_graph::{HostKey, NoteConnection, NoteGraph, NoteModuleConfig, NoteScopeCtx};
use super::note_processor::{ExpansionBuffer, NoteProcessor, note_scope_strum_tail};
use super::pattern::{Pattern, RowResolution};
use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
use super::track::{ReturnBus, SequencerTrack};
use synth_core::{Bpm, Gain, Semitones};

/// Tempo change event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TempoChange {
    /// Position of the tempo change.
    pub tick: Tick,
    /// New tempo in BPM.
    pub bpm: Bpm,
    /// When `true`, the tempo ramps linearly (in tick space) from `bpm` at
    /// `tick` toward the *next* change's bpm, reaching it exactly at that
    /// change (accelerando / ritardando). With no following change the tempo
    /// holds constant at `bpm`. When `false` (the default) the tempo steps to
    /// `bpm` at `tick` and holds until the next change.
    #[serde(default)]
    pub ramp: bool,
}

/// A persisted transport loop region: playback wraps from `end` back to `start`
/// while `enabled`. This is the serialized carrier for the loop the engine's
/// sequencer owns at runtime — captured at save time and restored (via a
/// `SetLoop` command) at load time, so a saved project keeps its loop instead of
/// silently dropping it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct LoopRegion {
    /// Loop start position.
    pub start: Tick,
    /// Loop end position (exclusive; playback wraps here back to `start`).
    pub end: Tick,
    /// Whether the loop is active. A region can be stored while disabled so the
    /// bounds survive a toggle.
    pub enabled: bool,
}

/// Seconds to advance from `seg_start` to tick `q` within a tempo segment whose
/// full extent is `[seg_start, seg_end]`, tempo `b0` at `seg_start` ramping to
/// `b_end` at `seg_end` when `ramp` (else constant `b0`). `q` must lie in
/// `[seg_start, seg_end]`. BPMs are assumed positive.
///
/// For a linear-in-tick ramp, time is the exact log-integral of `60/bpm`:
/// `K·ln(b_q/b0)/(b_end−b0)` with `K = full_beats·60`. `ln_1p` keeps it stable
/// as `b_end → b0`, and a `< 1e-5` branch falls back to the endpoint-average
/// constant tempo to avoid the `0/0` cancellation.
fn ramp_segment_seconds(
    seg_start: u64,
    seg_end: u64,
    b0: f64,
    b_end: f64,
    ramp: bool,
    q: u64,
) -> f64 {
    debug_assert!(b0 > 0.0 && b_end > 0.0, "tempo must be positive");
    let beats = (q - seg_start) as f64 / f64::from(TICKS_PER_QUARTER);
    if !ramp || seg_end <= seg_start {
        return beats * 60.0 / b0;
    }
    let diff = b_end - b0;
    if diff.abs() < 1e-5 {
        // Flat / near-flat ramp: constant tempo at the endpoint average.
        return beats * 60.0 / ((b0 + b_end) * 0.5);
    }
    let full_beats = (seg_end - seg_start) as f64 / f64::from(TICKS_PER_QUARTER);
    let k = full_beats * 60.0;
    let b_q = b0 + diff * ((q - seg_start) as f64 / (seg_end - seg_start) as f64);
    k * ((b_q - b0) / b0).ln_1p() / diff
}

/// Inverse of [`ramp_segment_seconds`]: the tick offset from a segment's start
/// reached after `s` seconds into a segment of length `seg_ticks` (tempo `b0`
/// ramping to `b_end` when `ramp`). Uses `exp_m1` for stability and clamps the
/// fraction to `[0, 1]`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn ramp_segment_tick_offset(seg_ticks: u64, b0: f64, b_end: f64, ramp: bool, s: f64) -> u64 {
    debug_assert!(b0 > 0.0 && b_end > 0.0, "tempo must be positive");
    if seg_ticks == 0 {
        return 0;
    }
    let k = (seg_ticks as f64 / f64::from(TICKS_PER_QUARTER)) * 60.0;
    let diff = b_end - b0;
    let u = if !ramp || diff.abs() < 1e-5 {
        b0 * s / k
    } else {
        // b_q = b0·exp(s·diff/K) ⇒ u = (b_q − b0)/diff, via exp_m1.
        b0 * (s * diff / k).exp_m1() / diff
    };
    (u.clamp(0.0, 1.0) * seg_ticks as f64) as u64
}

/// Time signature change event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TimeSignatureChange {
    /// Position of the time signature change.
    pub tick: Tick,
    /// New time signature.
    pub signature: TimeSignature,
}

/// A pattern placement in the arrangement.
#[must_use]
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PatternPlacement {
    /// Which pattern to play.
    pub pattern_id: PatternId,
    /// Which track this is on.
    pub track_id: TrackId,
    /// Start position in the song.
    pub start: Tick,
    /// Transposition in semitones.
    pub transpose: Semitones,
    /// Volume scaling (1.0 = normal).
    pub gain: Gain,
    /// Optional per-placement length override. When `None`, the placement
    /// occupies exactly `pattern.length` ticks. When `Some(d)`, the
    /// placement extends/clips to that length; the engine still walks the
    /// pattern's note timeline up to `min(d, pattern.length)`.
    #[serde(default)]
    pub length_override: Option<Duration>,
}

impl PatternPlacement {
    /// Create a new pattern placement.
    pub fn new(pattern_id: PatternId, track_id: TrackId, start: Tick) -> Self {
        Self {
            pattern_id,
            track_id,
            start,
            transpose: Semitones::ZERO,
            gain: Gain::UNITY,
            length_override: None,
        }
    }

    /// Set transposition (builder pattern).
    pub fn with_transpose(mut self, semitones: Semitones) -> Self {
        self.transpose = semitones;
        self
    }

    /// Set gain (builder pattern).
    pub fn with_gain(mut self, gain: Gain) -> Self {
        self.gain = gain;
        self
    }

    /// Calculate end position. Respects `length_override` when set,
    /// otherwise uses the pattern's own length.
    pub fn end(&self, pattern_length: Duration) -> Tick {
        let len = self.length_override.unwrap_or(pattern_length);
        Tick(self.start.0 + len.0 as u64)
    }

    /// Effective length (override if set, otherwise the pattern's length).
    pub fn effective_length(&self, pattern_length: Duration) -> Duration {
        self.length_override.unwrap_or(pattern_length)
    }
}

/// Outcome of [`Song::freeze_pattern`]: the pattern's note count after the bake,
/// plus how many events were **dropped** during it (a graph node hitting the
/// 128-event expansion cap — reported so an overflowing graph is surfaced, not
/// silently truncated, plan §7). The retired rack path reports `dropped: 0` (its
/// per-tick output is bounded).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FreezeStats {
    /// The pattern's note count after the bake.
    pub notes: usize,
    /// Events dropped during the bake (128-event cap overflow).
    pub dropped: u32,
}

/// A complete song with patterns, tracks, and arrangement.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Song {
    /// Song name.
    pub name: String,
    /// Song author.
    pub author: String,
    /// Free-text description capturing intent (what the song is for, its mood
    /// or production notes). Empty by default; readable/writable via MCP and GUI.
    #[serde(default)]
    pub description: String,

    patterns: Vec<Pattern>,
    next_pattern_id: u32,

    /// Tracks in display order (top → bottom).
    tracks: Vec<SequencerTrack>,
    next_track_id: u16,

    // Arrangement
    arrangement: Vec<PatternPlacement>,

    // Tempo and time signature
    tempo_changes: Vec<TempoChange>,
    time_signature_changes: Vec<TimeSignatureChange>,
    /// Default tempo (BPM).
    pub default_tempo: Bpm,
    /// Default time signature.
    pub default_time_signature: TimeSignature,
    /// Row resolution for pattern grid display and quantization.
    #[serde(default)]
    pub row_resolution: RowResolution,

    /// Return busses (effect-send destinations) referenced by `TrackSend`s.
    /// Source of truth for the busses' fader (read live by the engine).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    return_busses: Vec<ReturnBus>,
    #[serde(default)]
    next_return_bus_id: u16,

    /// Pooled Note Grid graphs — project assets referenced by [`NoteGraphId`].
    /// A pattern binds one; editing a graph affects every binder. Their derived
    /// `processing_order` is not serialized; [`Self::rebuild_note_graphs`]
    /// recomputes it after load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    note_graphs: Vec<NoteGraph>,
    #[serde(default)]
    next_note_graph_id: u32,

    /// Pooled Mod Grid graphs — control-rate modulator graphs referenced by
    /// [`ModGraphId`]. Global graphs run one always-on instance; Track graphs
    /// run one instance per assigned track. The engine (re)builds running
    /// instances off the audio thread whenever [`Self::mod_grid_generation`]
    /// changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mod_graphs: Vec<ModGraph>,
    #[serde(default)]
    next_mod_graph_id: u32,

    /// Persisted transport loop region. The runtime loop lives in the sequencer
    /// engine (which the audio thread owns); this is the save/load carrier only,
    /// synced engine→here at save and here→engine at load. `None` = no loop set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport_loop: Option<LoopRegion>,

    /// Monotonic counter bumped on every mutation that can change
    /// [`Self::calculate_length`] (arrangement placements or pattern lengths).
    /// Lets the audio engine cache the song length and recompute it only when
    /// this value changes, instead of rescanning the whole arrangement every
    /// tick. Not persisted — resets to 0 on load, where the engine refreshes
    /// its cache unconditionally (see `SequencerEngine::set_song`).
    #[serde(skip)]
    structure_generation: u64,

    /// Monotonic counter bumped on every Mod Grid mutation (graph/node/cable/
    /// target/assignment/scope edit). The audio engine compares this per block
    /// and rebuilds its running mod-grid instances — off the hot path — only
    /// when it changes. Not persisted — resets to 0 on load, where the engine
    /// refreshes its instances unconditionally (see `SequencerEngine::set_song`).
    #[serde(skip)]
    mod_grid_generation: u64,
}

impl Song {
    /// Create a new empty song.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: String::new(),
            description: String::new(),
            patterns: Vec::new(),
            next_pattern_id: 0,
            tracks: Vec::new(),
            next_track_id: 0,
            arrangement: Vec::new(),
            tempo_changes: Vec::new(),
            time_signature_changes: Vec::new(),
            default_tempo: Bpm::new(120.0),
            default_time_signature: TimeSignature::COMMON,
            row_resolution: RowResolution::default(),
            return_busses: Vec::new(),
            next_return_bus_id: 0,
            note_graphs: Vec::new(),
            next_note_graph_id: 0,
            mod_graphs: Vec::new(),
            next_mod_graph_id: 0,
            transport_loop: None,
            structure_generation: 0,
            mod_grid_generation: 0,
        }
    }

    /// The persisted transport loop region, if one was saved. See
    /// [`LoopRegion`]; the live loop is owned by the sequencer engine.
    #[must_use]
    pub fn transport_loop(&self) -> Option<LoopRegion> {
        self.transport_loop
    }

    /// Store the transport loop region to persist with the song (`None` clears
    /// it). Set from the engine's runtime loop state at save time.
    pub fn set_transport_loop(&mut self, region: Option<LoopRegion>) {
        self.transport_loop = region;
    }

    /// Current structural generation — bumped whenever a mutation may have
    /// changed [`Self::calculate_length`] (arrangement placements or pattern
    /// lengths). The audio engine caches the song length and recomputes it only
    /// when this value changes. See the field docs.
    #[must_use]
    pub fn structure_generation(&self) -> u64 {
        self.structure_generation
    }

    /// Mark the length-affecting structure as changed so cached derivations
    /// (the engine's song length) recompute on next read. Conservative: callers
    /// bump even when a change is merely possible (e.g. a `&mut Pattern` handout).
    fn bump_structure(&mut self) {
        self.structure_generation = self.structure_generation.wrapping_add(1);
    }

    /// Set the author (builder pattern).
    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Set the default tempo (builder pattern).
    #[must_use]
    pub fn with_tempo(mut self, bpm: Bpm) -> Self {
        self.default_tempo = bpm;
        self
    }

    /// Set the default time signature (builder pattern).
    #[must_use]
    pub fn with_time_signature(mut self, sig: TimeSignature) -> Self {
        self.default_time_signature = sig;
        self
    }

    // === Pattern management ===

    /// Create a new pattern and add it to the song.
    pub fn create_pattern(&mut self, length: Duration) -> PatternId {
        let id = PatternId(self.next_pattern_id);
        self.next_pattern_id = self.next_pattern_id.saturating_add(1);
        self.patterns.push(Pattern::new(id, length));
        self.bump_structure();
        id
    }

    // === Note Grid pool ===

    /// Create a new empty Note Grid graph in the pool, returning its id.
    pub fn create_note_graph(&mut self, name: impl Into<String>) -> NoteGraphId {
        let id = NoteGraphId(self.next_note_graph_id);
        self.next_note_graph_id = self.next_note_graph_id.saturating_add(1);
        self.note_graphs.push(NoteGraph::new(id, name));
        id
    }

    /// Insert a pre-built graph, keeping `next_note_graph_id` ahead of restored
    /// ids. Returns false if a graph with that id already exists.
    pub fn insert_note_graph(&mut self, graph: NoteGraph) -> bool {
        if self.note_graphs.iter().any(|g| g.id == graph.id) {
            return false;
        }
        if graph.id.0 >= self.next_note_graph_id {
            self.next_note_graph_id = graph.id.0.saturating_add(1);
        }
        self.note_graphs.push(graph);
        true
    }

    /// Get a pooled graph by id.
    #[must_use]
    pub fn note_graph(&self, id: NoteGraphId) -> Option<&NoteGraph> {
        self.note_graphs.iter().find(|g| g.id == id)
    }

    /// Get a pooled graph mutably by id. After mutating nodes/connections the
    /// caller must call [`NoteGraph::rebuild_derived`] (the `try_*` editors do).
    pub fn note_graph_mut(&mut self, id: NoteGraphId) -> Option<&mut NoteGraph> {
        self.note_graphs.iter_mut().find(|g| g.id == id)
    }

    /// All pooled graphs, in insertion order.
    pub fn note_graphs(&self) -> impl Iterator<Item = &NoteGraph> {
        self.note_graphs.iter()
    }

    /// The pooled graphs as a slice — the note-scope resolution pool threaded
    /// into per-tick source seeding ([`NoteScopeCtx`]) so each note's
    /// `Note::note_graph` can be resolved during expansion.
    #[must_use]
    pub fn note_graph_pool(&self) -> &[NoteGraph] {
        &self.note_graphs
    }

    /// All pooled graphs, mutably — for a non-RT resync step that rebuilds
    /// derived, non-serialized per-node state after a load (e.g. recompiling
    /// each [`NoteGraph`]'s `NoteScriptTransform` programs).
    pub fn note_graphs_mut(&mut self) -> impl Iterator<Item = &mut NoteGraph> {
        self.note_graphs.iter_mut()
    }

    /// How many patterns currently bind `id` — the pool-view usage count.
    #[must_use]
    pub fn note_graph_usage(&self, id: NoteGraphId) -> usize {
        self.patterns
            .iter()
            .filter(|p| p.note_graph() == Some(id))
            .count()
    }

    /// Remove a graph from the pool and clear every pattern reference to it
    /// (bound patterns fall back to their rack). Returns the removed graph, or
    /// `None` if absent.
    pub fn remove_note_graph(&mut self, id: NoteGraphId) -> Option<NoteGraph> {
        let idx = self.note_graphs.iter().position(|g| g.id == id)?;
        for pattern in &mut self.patterns {
            if pattern.note_graph() == Some(id) {
                pattern.set_note_graph(None);
            }
        }
        Some(self.note_graphs.remove(idx))
    }

    /// Recompute the derived `processing_order` of every pooled graph — call on
    /// every non-RT load path (the order is not serialized, §1.2). Connection
    /// sets are sanitized first so files saved before a validation rule was
    /// added (duplicate edges, double-fed value inputs) load playable instead
    /// of silently degrading to pass-through. A graph that still fails (e.g. a
    /// corrupt saved cycle) keeps an empty order and is treated as
    /// pass-through, never a load failure.
    pub fn rebuild_note_graphs(&mut self) {
        for graph in &mut self.note_graphs {
            graph.sanitize_connections();
            let _ = graph.rebuild_derived();
        }
    }

    // === Mod Grid pool ===

    /// Current Mod Grid generation — bumped on every mod-graph mutation. The
    /// audio engine rebuilds its running instances only when this changes.
    #[must_use]
    pub fn mod_grid_generation(&self) -> u64 {
        self.mod_grid_generation
    }

    /// Mark the Mod Grid as changed so the engine rebuilds running instances on
    /// the next block. Conservative: any structural or value edit bumps it.
    fn bump_mod_grid(&mut self) {
        self.mod_grid_generation = self.mod_grid_generation.wrapping_add(1);
    }

    /// Create a new empty Mod Grid graph in the pool, returning its id.
    pub fn create_mod_graph(&mut self, name: impl Into<String>) -> ModGraphId {
        let id = ModGraphId(self.next_mod_graph_id);
        self.next_mod_graph_id = self.next_mod_graph_id.saturating_add(1);
        self.mod_graphs.push(ModGraph::new(id, name));
        self.bump_mod_grid();
        id
    }

    /// Insert a pre-built graph, keeping `next_mod_graph_id` ahead of restored
    /// ids. Returns false if a graph with that id already exists.
    pub fn insert_mod_graph(&mut self, graph: ModGraph) -> bool {
        if self.mod_graphs.iter().any(|g| g.id == graph.id) {
            return false;
        }
        if graph.id.0 >= self.next_mod_graph_id {
            self.next_mod_graph_id = graph.id.0.saturating_add(1);
        }
        self.mod_graphs.push(graph);
        self.bump_mod_grid();
        true
    }

    /// Get a pooled mod graph by id.
    #[must_use]
    pub fn mod_graph(&self, id: ModGraphId) -> Option<&ModGraph> {
        self.mod_graphs.iter().find(|g| g.id == id)
    }

    /// Get a pooled mod graph mutably by id, bumping the generation so the engine
    /// rebuilds its instance (conservative — the handout may mutate anything).
    pub fn mod_graph_mut(&mut self, id: ModGraphId) -> Option<&mut ModGraph> {
        let found = self.mod_graphs.iter_mut().find(|g| g.id == id);
        if found.is_some() {
            self.mod_grid_generation = self.mod_grid_generation.wrapping_add(1);
        }
        found
    }

    /// All pooled mod graphs, in insertion order.
    pub fn mod_graphs(&self) -> impl Iterator<Item = &ModGraph> {
        self.mod_graphs.iter()
    }

    /// The pooled mod graphs as a slice — the engine's instance-build source.
    #[must_use]
    pub fn mod_graph_pool(&self) -> &[ModGraph] {
        &self.mod_graphs
    }

    /// All pooled mod graphs, mutably — for a non-RT resync step after load.
    /// Bumps the generation so the engine rebuilds all instances.
    pub fn mod_graphs_mut(&mut self) -> impl Iterator<Item = &mut ModGraph> {
        self.mod_grid_generation = self.mod_grid_generation.wrapping_add(1);
        self.mod_graphs.iter_mut()
    }

    /// Set a mod graph's scope. Returns false if the id is absent.
    pub fn set_mod_graph_scope(&mut self, id: ModGraphId, scope: ModGraphScope) -> bool {
        let Some(graph) = self.mod_graphs.iter_mut().find(|g| g.id == id) else {
            return false;
        };
        graph.scope = scope;
        // Global graphs have no per-track assignments.
        if scope == ModGraphScope::Global {
            graph.assigned_tracks.clear();
        }
        self.bump_mod_grid();
        true
    }

    /// Replace a mod graph's track assignments (deduplicated, only tracks that
    /// exist). Returns false if the id is absent.
    pub fn assign_mod_graph(&mut self, id: ModGraphId, tracks: &[TrackId]) -> bool {
        let existing: Vec<TrackId> = self.tracks.iter().map(|t| t.id).collect();
        let Some(graph) = self.mod_graphs.iter_mut().find(|g| g.id == id) else {
            return false;
        };
        let mut assigned: Vec<TrackId> = Vec::new();
        for &t in tracks {
            if existing.contains(&t) && !assigned.contains(&t) {
                assigned.push(t);
            }
        }
        graph.assigned_tracks = assigned;
        self.bump_mod_grid();
        true
    }

    /// How many tracks currently have `id` assigned — the pool-view usage count.
    /// Global graphs report 1 (the single always-on instance).
    #[must_use]
    pub fn mod_graph_usage(&self, id: ModGraphId) -> usize {
        match self.mod_graphs.iter().find(|g| g.id == id) {
            Some(g) if g.scope == ModGraphScope::Global => 1,
            Some(g) => g.assigned_tracks.len(),
            None => 0,
        }
    }

    /// Remove a mod graph from the pool. Returns the removed graph, or `None` if
    /// absent.
    pub fn remove_mod_graph(&mut self, id: ModGraphId) -> Option<ModGraph> {
        let idx = self.mod_graphs.iter().position(|g| g.id == id)?;
        let removed = self.mod_graphs.remove(idx);
        self.bump_mod_grid();
        Some(removed)
    }

    /// Duplicate a pooled mod graph (name suffixed `" copy"`), returning a clone
    /// of the new graph, or `None` if the source is absent.
    pub fn duplicate_mod_graph(&mut self, src: ModGraphId) -> Option<ModGraph> {
        let source = self.mod_graphs.iter().find(|g| g.id == src)?.clone();
        let id = ModGraphId(self.next_mod_graph_id);
        self.next_mod_graph_id = self.next_mod_graph_id.saturating_add(1);
        let mut graph = source;
        graph.id = id;
        graph.name = format!("{} copy", graph.name);
        self.mod_graphs.push(graph.clone());
        self.bump_mod_grid();
        Some(graph)
    }

    /// Sanitize every pooled mod graph on load: drop any cable that fails
    /// validation (unknown node, duplicate, cycle) so a corrupt save loads
    /// playable instead of failing. Also bumps the generation so the engine
    /// builds fresh instances.
    pub fn rebuild_mod_graphs(&mut self) {
        for graph in &mut self.mod_graphs {
            if graph.validate().is_err() {
                // Rebuild the connection set incrementally, dropping edges the
                // validator rejects — mirrors `rebuild_note_graphs`' tolerance.
                let candidates = std::mem::take(&mut graph.connections);
                for c in candidates {
                    let _ = graph.try_connect(c);
                }
            }
        }
        self.bump_mod_grid();
    }

    /// Convert a pattern's linear `NoteProcessor` rack into a new pooled Note
    /// Grid graph — one `Processor` node per rack stage, chained in the rack's
    /// (canonical) order — then bind the pattern to it and clear the rack.
    /// Returns the new graph id, or `None` if the pattern is missing, already
    /// graph-bound, or has an empty rack. The graph is a plain linear chain that
    /// reproduces the rack's output for this pattern — including seeded Humanize
    /// (its seed is pre-compensated for the host key); the only residual is a
    /// `Random`-mode arpeggiator, whose step order re-seeds per host. This is
    /// both the one-click UI action and the primitive the load migration reuses.
    pub fn convert_rack_to_graph(&mut self, pattern_id: PatternId) -> Option<NoteGraphId> {
        let pattern = self.pattern(pattern_id)?;
        if pattern.note_graph().is_some() || pattern.processors().is_empty() {
            return None;
        }
        let procs: Vec<NoteProcessor> = pattern.processors().to_vec();
        let name = if pattern.name.is_empty() {
            "Rack".to_owned()
        } else {
            format!("{} FX", pattern.name)
        };
        // The rack evaluated with host salt 0; the graph evaluates with this
        // pattern's host key. Pre-compensate seeded stages so the migrated graph
        // reproduces the rack's rendered output for the pattern it came from.
        let host_salt = HostKey::from(pattern_id).get();
        let id = self.create_note_graph(name);
        if let Some(graph) = self.note_graph_mut(id) {
            let mut prev: Option<NoteModuleId> = None;
            for (i, proc) in procs.iter().enumerate() {
                let nid = NoteModuleId::new((i + 1) as u32);
                // Humanize XORs host_salt into its seed at eval, so pre-XORing it
                // here cancels out and the draws match the rack byte-for-byte. Arp
                // Random has no seed field to compensate, so its step order
                // re-seeds per host (uncommon; the only residual difference).
                let proc = match *proc {
                    NoteProcessor::Humanize(mut h) => {
                        h.seed ^= host_salt;
                        NoteProcessor::Humanize(h)
                    }
                    other => other,
                };
                // Cap-guard: a rack longer than MAX_NOTE_GRID_NODES stops here,
                // silently dropping the excess stages (racks are effectively never
                // that long; a hard error would block the load).
                if graph
                    .try_insert_node(nid, NoteModuleConfig::Processor(proc))
                    .is_err()
                {
                    break;
                }
                if let Some(p) = prev {
                    let _ = graph.try_connect(NoteConnection::stream(p, nid));
                }
                prev = Some(nid);
            }
            let _ = graph.rebuild_derived();
        }
        if let Some(pattern) = self.pattern_mut(pattern_id) {
            pattern.set_note_graph(Some(id));
            pattern.clear_processors();
        }
        Some(id)
    }

    /// Migrate every pattern's legacy `NoteProcessor` rack into a pooled Note
    /// Grid graph (via [`Self::convert_rack_to_graph`]), so projects saved before
    /// the Note Grid load as graphs. Patterns already bound to a graph, or with
    /// an empty rack, are untouched. Returns the number of patterns migrated.
    /// Call on load **before** [`Self::rebuild_note_graphs`] — each conversion
    /// builds its own graph's order, and the later rebuild covers the rest.
    pub fn migrate_processor_racks_to_graphs(&mut self) -> usize {
        // Collect ids first: `convert_rack_to_graph` takes `&mut self` and
        // mutates both the pool and the pattern, so a live patterns borrow can't
        // span the calls.
        let ids: Vec<PatternId> = self.patterns.iter().map(|p| p.id).collect();
        ids.into_iter()
            .filter(|&pid| self.convert_rack_to_graph(pid).is_some())
            .count()
    }

    /// Clone a pooled graph into a fresh id (`"<name> copy"`), copying content
    /// and metadata — the "duplicate" / "make unique" primitive. Returns the
    /// new graph's snapshot (callers use it for undo), or `None` if `src`
    /// does not exist.
    pub fn duplicate_note_graph(&mut self, src: NoteGraphId) -> Option<NoteGraph> {
        let src = self.note_graph(src)?.clone();
        let id = self.create_note_graph(format!("{} copy", src.name));
        if let Some(dst) = self.note_graph_mut(id) {
            dst.description = src.description;
            dst.color = src.color;
            dst.nodes = src.nodes;
            dst.node_descriptions = src.node_descriptions;
            dst.connections = src.connections;
            dst.node_positions = src.node_positions;
            // A copy of a valid graph revalidates identically.
            let _ = dst.rebuild_derived();
        }
        self.note_graph(id).cloned()
    }

    /// Freeze a pattern's note processing into plain notes, honoring playback
    /// precedence (plan §7): a bound, resolvable note graph bakes; otherwise
    /// the processor rack + per-note ornaments bake. A dangling graph id is
    /// cleared by the graph path and falls back to the rack — matching the
    /// engine's dangling-id fallback, so freeze always bakes what playback
    /// plays. Returns the post-bake note count plus any dropped-event count
    /// ([`FreezeStats`]).
    pub fn freeze_pattern(&mut self, pattern_id: PatternId, bpm: synth_core::Bpm) -> FreezeStats {
        if let Some(dropped) = self.freeze_pattern_note_graph(pattern_id, bpm) {
            let notes = self.pattern(pattern_id).map_or(0, |p| p.notes().len());
            return FreezeStats { notes, dropped };
        }
        // Rack fallback. Disjoint field borrow: the note-scope pool
        // (`note_graphs`, shared) alongside the mutable pattern (`patterns`) —
        // `pattern_mut` would borrow all of `self` and conflict with the pool
        // borrow. The rack bake resolves each note's own note-scope graph so
        // freeze matches playback (plan §2.1, §7). The retired rack's per-tick
        // output is bounded, so its drop count is reported as 0.
        let graphs = &self.note_graphs;
        if let Some(pattern) = self.patterns.iter_mut().find(|p| p.id == pattern_id) {
            return FreezeStats {
                notes: pattern.freeze_processors(bpm, graphs),
                dropped: 0,
            };
        }
        FreezeStats {
            notes: self.pattern(pattern_id).map_or(0, |p| p.notes().len()),
            dropped: 0,
        }
    }

    /// Bake a pattern's bound Note Grid graph into plain notes and clear the
    /// binding — the pooled graph itself survives for other patterns. Purity
    /// makes the bake identical to playback (plan §7). Returns `Some(dropped)`
    /// with the events dropped during the bake if a graph was baked; a pattern
    /// with no binding is a no-op and a dangling id has its stale reference
    /// cleared, both returning `None`.
    pub fn freeze_pattern_note_graph(
        &mut self,
        pattern_id: PatternId,
        bpm: synth_core::Bpm,
    ) -> Option<u32> {
        let gid = self.pattern(pattern_id).and_then(Pattern::note_graph)?;
        let Some(graph) = self.note_graph(gid) else {
            // Dangling id — clear the stale reference, nothing to bake.
            if let Some(pattern) = self.pattern_mut(pattern_id) {
                pattern.set_note_graph(None);
            }
            return None;
        };
        let pattern = self.pattern(pattern_id)?;
        // Extend the walk past the source span by any strummed chord's tail and
        // any timing node's echo tail — the pattern-scope graph's own, plus the
        // widest note-scope articulation tail of any note bound to a graph — so a
        // strum or echo near the pattern end bakes in full (mirrors the rack's
        // `freeze_processors`; plan `note-grid.md` §11).
        let note_scope_tail = note_scope_strum_tail(pattern.notes(), &self.note_graphs);
        let walk_end = pattern
            .source_walk_end()
            .saturating_add(graph.max_walk_tail())
            .saturating_add(note_scope_tail);
        let mut ns_scratch = ExpansionBuffer::new();
        let mut ns_ctx = NoteScopeCtx {
            pool: &self.note_graphs,
            scratch: &mut ns_scratch,
        };
        let (baked, dropped) = graph.bake_counted(
            pattern.notes(),
            PatternTick(walk_end),
            HostKey::from(pattern_id),
            bpm,
            Some(&mut ns_ctx),
        );
        if let Some(pattern) = self.pattern_mut(pattern_id) {
            pattern.replace_with_baked(baked);
            pattern.set_note_graph(None);
            // The bound graph suppressed the rack at playback; leaving the
            // rack alive would re-process the baked notes, so playback would
            // change right after the freeze. Retire it with the binding.
            pattern.clear_processors();
        }
        Some(dropped)
    }

    /// The pattern's notes **as they play**: expanded through the bound Note Grid
    /// graph (and any per-note note-scope graphs) if one is set, else the raw
    /// authored notes. Non-mutating — offline note analyzers use this so they
    /// reflect the played stream, not just the source (plan §7; resolves the
    /// "analyzers read source not expansion" debt for the pattern-graph case).
    /// A dangling/absent graph binding falls back to the authored notes.
    #[must_use]
    pub fn expanded_pattern_notes(&self, pattern_id: PatternId, bpm: synth_core::Bpm) -> Vec<Note> {
        let Some(pattern) = self.pattern(pattern_id) else {
            return Vec::new();
        };
        let Some(graph) = pattern.note_graph().and_then(|gid| self.note_graph(gid)) else {
            return pattern.notes().to_vec();
        };
        // Walk the whole source span plus any strum/echo/note-scope tail, exactly
        // like freeze — so the analysis sees the full played figure.
        let note_scope_tail = note_scope_strum_tail(pattern.notes(), &self.note_graphs);
        let walk_end = pattern
            .source_walk_end()
            .saturating_add(graph.max_walk_tail())
            .saturating_add(note_scope_tail);
        let mut ns_scratch = ExpansionBuffer::new();
        let mut ns_ctx = NoteScopeCtx {
            pool: &self.note_graphs,
            scratch: &mut ns_scratch,
        };
        graph
            .bake(
                pattern.notes(),
                PatternTick(walk_end),
                HostKey::from(pattern_id),
                bpm,
                Some(&mut ns_ctx),
            )
            .into_iter()
            .enumerate()
            .map(|(i, (tick, en))| {
                // Synthesize a plain Note from the expanded event; ids are
                // positional (analyzers key on pitch/timing/velocity, not id).
                let mut note = Note::new(NoteId(i as u64), tick, en.pitch, en.velocity);
                if let Some(d) = en.duration {
                    note = note.with_duration(d);
                }
                note
            })
            .collect()
    }

    /// Get a pattern by ID.
    #[must_use]
    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    /// Get a mutable pattern by ID.
    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut Pattern> {
        // A caller with `&mut Pattern` may change `pattern.length`, which feeds
        // `calculate_length()`. We cannot observe whether they actually do, so
        // conservatively mark the structure dirty on every handout.
        self.bump_structure();
        self.patterns.iter_mut().find(|p| p.id == id)
    }

    /// Get all patterns.
    pub fn patterns(&self) -> impl Iterator<Item = &Pattern> {
        self.patterns.iter()
    }

    /// Get the number of patterns.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    // === Automation reference index ===

    /// Index of every module referenced by an [`AutomationTarget::Module`] lane:
    /// `(instrument, module_type, instance)` → the set of descriptor `param_id`s
    /// driven across all patterns. Only lanes with at least one point are
    /// counted (an empty lane is a transient picker artifact, not a reference).
    ///
    /// Module identity is positional (see [`AutomationTarget::Module`]). Built on
    /// demand (allocating); call once and query, e.g. for the Rack "automated"
    /// badge. Not real-time safe — UI/command thread only.
    ///
    /// [`AutomationTarget::Module`]: super::automation::AutomationTarget::Module
    #[must_use]
    pub fn automated_module_params(
        &self,
    ) -> std::collections::HashMap<
        (InstrumentId, synth_core::ModuleType, u16),
        std::collections::BTreeSet<String>,
    > {
        use super::automation::AutomationTarget;
        let mut index = std::collections::HashMap::new();
        for pattern in &self.patterns {
            for lane in &pattern.automation {
                if lane.is_empty() {
                    continue;
                }
                if let AutomationTarget::Module {
                    instrument,
                    module_type,
                    instance,
                    param_id,
                } = &lane.target
                {
                    index
                        .entry((*instrument, *module_type, *instance))
                        .or_insert_with(std::collections::BTreeSet::new)
                        .insert(param_id.as_str().to_owned());
                }
            }
        }
        index
    }

    /// Whether any (non-empty) automation lane references the given module by its
    /// positional identity. Cheaper than [`Self::automated_module_params`] for a
    /// single lookup (e.g. a module-delete guard). UI/command thread only.
    #[must_use]
    pub fn is_module_automated(
        &self,
        instrument: InstrumentId,
        module_type: synth_core::ModuleType,
        instance: u16,
    ) -> bool {
        use super::automation::AutomationTarget;
        self.patterns.iter().any(|pattern| {
            pattern.automation.iter().any(|lane| {
                !lane.is_empty()
                    && matches!(
                        &lane.target,
                        AutomationTarget::Module {
                            instrument: i,
                            module_type: mt,
                            instance: inst,
                            ..
                        } if *i == instrument && *mt == module_type && *inst == instance
                    )
            })
        })
    }

    /// Delete a pattern.
    pub fn delete_pattern(&mut self, id: PatternId) -> Option<Pattern> {
        // Also remove from arrangement
        self.arrangement.retain(|p| p.pattern_id != id);
        self.bump_structure();
        let pos = self.patterns.iter().position(|p| p.id == id)?;
        Some(self.patterns.remove(pos))
    }

    /// Insert a pre-built pattern under its existing id. Used by undo to
    /// restore a deleted pattern with its full notes and automation.
    /// Returns true if inserted, false if a pattern with that id already
    /// exists.
    pub fn insert_pattern(&mut self, pattern: Pattern) -> bool {
        if self.patterns.iter().any(|p| p.id == pattern.id) {
            return false;
        }
        // Keep `next_pattern_id` ahead of any restored id so later
        // create_pattern calls don't reuse the same slot.
        if pattern.id.0 >= self.next_pattern_id {
            self.next_pattern_id = pattern.id.0.saturating_add(1);
        }
        self.patterns.push(pattern);
        self.bump_structure();
        true
    }

    /// Duplicate a pattern.
    pub fn duplicate_pattern(&mut self, id: PatternId) -> Option<PatternId> {
        let source = self.pattern(id)?.clone();
        let new_id = PatternId(self.next_pattern_id);
        self.next_pattern_id = self.next_pattern_id.saturating_add(1);

        let mut new_pattern = Pattern::new(new_id, source.length);
        new_pattern.name = format!("{} (copy)", source.name);

        // Copy notes
        for note in source.notes() {
            let _ = new_pattern.insert_note(note.clone());
        }

        // Copy automation lanes and the (legacy) note-processor rack
        new_pattern.automation = source.automation.clone();
        new_pattern.processors = source.processors.clone();
        // Copy the Note Grid binding — a duplicate must play like its source
        // (the pooled graph is shared by reference, like an instrument).
        new_pattern.set_note_graph(source.note_graph());

        self.patterns.push(new_pattern);
        self.bump_structure();
        Some(new_id)
    }

    // === Track management ===

    /// Create a new track.
    pub fn create_track(&mut self, name: impl Into<String>) -> TrackId {
        let id = TrackId(self.next_track_id);
        self.next_track_id = self.next_track_id.saturating_add(1);
        self.tracks.push(SequencerTrack::new(id, name));
        id
    }

    /// Insert a pre-built track under its existing id. Used by undo to
    /// restore a deleted track. The track is appended to the display order
    /// (or its old position if provided via `at_index`). Returns true if
    /// inserted, false if the id already exists.
    pub fn insert_track(&mut self, track: SequencerTrack, at_index: Option<usize>) -> bool {
        if self.tracks.iter().any(|t| t.id == track.id) {
            return false;
        }
        if track.id.0 >= self.next_track_id {
            self.next_track_id = track.id.0.saturating_add(1);
        }
        let pos = at_index.map_or(self.tracks.len(), |i| i.min(self.tracks.len()));
        self.tracks.insert(pos, track);
        true
    }

    /// Insert a pre-built placement as-is. Used by undo to restore a
    /// removed placement with its transpose/gain/etc. preserved. Returns
    /// true if inserted, false if the pattern or track is missing.
    pub fn insert_placement(&mut self, placement: PatternPlacement) -> bool {
        if !self.patterns.iter().any(|p| p.id == placement.pattern_id)
            || !self.tracks.iter().any(|t| t.id == placement.track_id)
        {
            return false;
        }
        let start = placement.start;
        let pos = self.arrangement.partition_point(|p| p.start <= start);
        self.arrangement.insert(pos, placement);
        self.bump_structure();
        true
    }

    /// Move a track from one index to another in the display order.
    /// Returns true if the move happened.
    pub fn reorder_track(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() || from == to {
            return false;
        }
        let track = self.tracks.remove(from);
        self.tracks.insert(to, track);
        true
    }

    /// Get a track by ID.
    #[must_use]
    pub fn track(&self, id: TrackId) -> Option<&SequencerTrack> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Get a mutable track by ID.
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut SequencerTrack> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// Get all tracks in display order.
    pub fn tracks(&self) -> impl Iterator<Item = &SequencerTrack> {
        self.tracks.iter()
    }

    /// Get the number of tracks.
    #[must_use]
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Mutable iterator over all tracks, in display order.
    pub fn tracks_mut(&mut self) -> impl Iterator<Item = &mut SequencerTrack> {
        self.tracks.iter_mut()
    }

    /// Delete a track.
    pub fn delete_track(&mut self, id: TrackId) -> Option<SequencerTrack> {
        use super::automation::AutomationTarget;
        // Also remove placements on this track
        self.arrangement.retain(|p| p.track_id != id);
        // Strip automation lanes pinned to this track — a dangling
        // `Track { Some(deleted) }` lane would silently automate nothing and
        // re-serialize as invisible cruft. Host-track lanes (`track: None`)
        // stay: they follow whatever track their pattern is placed on.
        // Mirrors `delete_return_bus`'s send strip.
        for pattern in &mut self.patterns {
            pattern.automation.retain(|lane| {
                !matches!(
                    lane.target,
                    AutomationTarget::Track { track: Some(t), .. } if t == id
                )
            });
        }
        self.bump_structure();
        let pos = self.tracks.iter().position(|t| t.id == id)?;
        Some(self.tracks.remove(pos))
    }

    // --- Return busses (effect sends) --------------------------------------

    /// All return busses.
    #[must_use]
    pub fn return_busses(&self) -> &[ReturnBus] {
        &self.return_busses
    }

    /// Get a mutable return bus by id.
    pub fn return_bus_mut(&mut self, id: ReturnBusId) -> Option<&mut ReturnBus> {
        self.return_busses.iter_mut().find(|b| b.id == id)
    }

    /// Create a new return bus with an auto-assigned id and return that id.
    pub fn create_return_bus(&mut self, name: impl Into<String>) -> ReturnBusId {
        let id = ReturnBusId(self.next_return_bus_id);
        self.next_return_bus_id = self.next_return_bus_id.saturating_add(1);
        self.return_busses.push(ReturnBus::new(id, name));
        id
    }

    /// Delete a return bus and strip every track send and bus-to-bus send that
    /// targeted it. Returns the removed definition, if any.
    pub fn delete_return_bus(&mut self, id: ReturnBusId) -> Option<ReturnBus> {
        let pos = self.return_busses.iter().position(|b| b.id == id)?;
        for track in &mut self.tracks {
            track.sends.retain(|s| s.target != id);
        }
        for bus in &mut self.return_busses {
            bus.sends.retain(|s| s.target != id);
        }
        Some(self.return_busses.remove(pos))
    }

    /// Would adding a bus-to-bus send from `from` to `to` create a cycle in the
    /// return-routing graph? A self-send (`from == to`) is always a cycle. Other
    /// cases are rejected when `to` can already reach `from` through existing
    /// sends (so the new edge would close a loop). Used by callers (GUI / MCP) to
    /// refuse routings the engine cannot order acyclically.
    ///
    /// Traverses **all** sends, enabled or not: enabling/disabling a send does not
    /// re-run this check, so the stored send graph is kept acyclic regardless of
    /// the `enabled` flags. (The engine separately ignores disabled sends when it
    /// builds the per-block routing.)
    #[must_use]
    pub fn return_send_would_cycle(&self, from: ReturnBusId, to: ReturnBusId) -> bool {
        if from == to {
            return true;
        }
        // DFS from `to`: if we can reach `from`, the new `from -> to` edge closes
        // a cycle. Bounded by the number of return busses (no infinite loop even
        // if the existing graph is somehow already cyclic).
        let mut stack = vec![to];
        let mut seen = std::collections::HashSet::new();
        while let Some(node) = stack.pop() {
            if node == from {
                return true;
            }
            if !seen.insert(node) {
                continue;
            }
            if let Some(bus) = self.return_busses.iter().find(|b| b.id == node) {
                for send in &bus.sends {
                    stack.push(send.target);
                }
            }
        }
        false
    }

    /// Check if any track is soloed.
    #[must_use]
    pub fn any_solo(&self) -> bool {
        self.tracks.iter().any(|t| t.solo)
    }

    /// Set `solo = true` on `target` and `solo = false` on every other track.
    ///
    /// No-op for `target` if it does not exist; other tracks are still
    /// cleared.
    pub fn set_solo_only(&mut self, target: TrackId) {
        for track in &mut self.tracks {
            track.solo = track.id == target;
        }
    }

    /// Make exactly the tracks driving `instrument` audible and silence every
    /// other track, isolating one instrument's whole contribution (an instrument
    /// may be played by several tracks).
    ///
    /// Unlike [`set_solo_only`](Self::set_solo_only), this works by muting the
    /// non-matching tracks and clearing all solo flags, so the result is
    /// unambiguous regardless of any prior mute/solo state: if the instrument
    /// drives no track the return value is `0` and the whole song is silenced
    /// (the soloed-only approach would instead leave everything audible, since
    /// "no track soloed" reads as "no solo active"). Intended for use on a
    /// throwaway clone of the song.
    ///
    /// Returns the number of tracks left audible.
    pub fn isolate_instrument(&mut self, instrument: InstrumentId) -> usize {
        let mut audible = 0;
        for track in &mut self.tracks {
            let matches = track.instrument == instrument;
            track.solo = false;
            track.mute = !matches;
            if matches {
                audible += 1;
            }
        }
        audible
    }

    // === Arrangement ===

    /// Place a pattern in the arrangement.
    ///
    /// Returns `false` if the pattern or track does not exist.
    pub fn place_pattern(&mut self, pattern_id: PatternId, track_id: TrackId, start: Tick) -> bool {
        if !self.patterns.iter().any(|p| p.id == pattern_id)
            || !self.tracks.iter().any(|t| t.id == track_id)
        {
            return false;
        }

        // Check for duplicate placement at same position and track
        let already_exists = self
            .arrangement
            .iter()
            .any(|p| p.start == start && p.track_id == track_id);
        if already_exists {
            return false;
        }

        let placement = PatternPlacement::new(pattern_id, track_id, start);

        // Insert sorted by start time
        let pos = self.arrangement.partition_point(|p| p.start <= start);
        self.arrangement.insert(pos, placement);
        self.bump_structure();
        true
    }

    /// Remove a placement at a specific position.
    pub fn remove_placement(
        &mut self,
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
    ) -> bool {
        let pos = self
            .arrangement
            .iter()
            .position(|p| p.pattern_id == pattern_id && p.track_id == track_id && p.start == start);

        if let Some(idx) = pos {
            let _ = self.arrangement.remove(idx);
            self.bump_structure();
            true
        } else {
            false
        }
    }

    /// Move a placement to a new position and/or track.
    pub fn move_placement(
        &mut self,
        pattern_id: PatternId,
        from_track: TrackId,
        from_start: Tick,
        to_track: TrackId,
        to_start: Tick,
    ) -> bool {
        let pos = self.arrangement.iter().position(|p| {
            p.pattern_id == pattern_id && p.track_id == from_track && p.start == from_start
        });

        if let Some(idx) = pos {
            self.arrangement[idx].track_id = to_track;
            self.arrangement[idx].start = to_start;
            // Re-sort by start time
            self.arrangement.sort_by_key(|p| p.start);
            self.bump_structure();
            true
        } else {
            false
        }
    }

    /// Set a placement's length override. Identified by
    /// `(pattern_id, track_id, start)`. Returns true if found and updated.
    pub fn set_placement_length(
        &mut self,
        pattern_id: PatternId,
        track_id: TrackId,
        start: Tick,
        length: Option<Duration>,
    ) -> bool {
        let pos = self
            .arrangement
            .iter()
            .position(|p| p.pattern_id == pattern_id && p.track_id == track_id && p.start == start);
        if let Some(idx) = pos {
            self.arrangement[idx].length_override = length;
            self.bump_structure();
            true
        } else {
            false
        }
    }

    /// Get all placements.
    pub fn arrangement(&self) -> &[PatternPlacement] {
        &self.arrangement
    }

    /// If `pattern_id` is currently playing through some placement at
    /// `current_tick`, return the pattern-relative tick. Otherwise `None`.
    #[must_use]
    pub fn pattern_playhead_for(
        &self,
        pattern_id: PatternId,
        current_tick: Tick,
    ) -> Option<PatternTick> {
        let pattern_length = self.pattern(pattern_id)?.length;
        self.arrangement.iter().find_map(|p| {
            if p.pattern_id != pattern_id {
                return None;
            }
            let end = p.end(pattern_length);
            if current_tick >= p.start && current_tick < end {
                #[allow(clippy::cast_possible_truncation)]
                Some(PatternTick((current_tick.0 - p.start.0) as u32))
            } else {
                None
            }
        })
    }

    /// Get placements in a time range.
    pub fn placements_in_range(
        &self,
        start: Tick,
        end: Tick,
    ) -> impl Iterator<Item = &PatternPlacement> {
        self.arrangement.iter().filter(move |p| {
            let pattern_end = self
                .pattern(p.pattern_id)
                .map(|pat| p.end(pat.length))
                .unwrap_or(p.start);
            p.start < end && pattern_end > start
        })
    }

    /// Get placements on a specific track.
    pub fn placements_on_track(
        &self,
        track_id: TrackId,
    ) -> impl Iterator<Item = &PatternPlacement> {
        self.arrangement
            .iter()
            .filter(move |p| p.track_id == track_id)
    }

    // === Tempo ===

    /// Set a **step** tempo change at a position (replaces any existing change
    /// at `tick`). For a ramp, use [`Self::set_tempo_ramp_at`].
    pub fn set_tempo_at(&mut self, tick: Tick, bpm: Bpm) {
        self.set_tempo_ramp_at(tick, bpm, false);
    }

    /// Set a tempo change at a position with an explicit ramp mode. When `ramp`
    /// is `true` the tempo ramps linearly toward the next change (see
    /// [`TempoChange::ramp`]); when `false` it is a step. Replaces any existing
    /// change at `tick`.
    pub fn set_tempo_ramp_at(&mut self, tick: Tick, bpm: Bpm, ramp: bool) {
        // Remove existing at same tick
        self.tempo_changes.retain(|t| t.tick != tick);

        let change = TempoChange { tick, bpm, ramp };
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        self.tempo_changes.insert(pos, change);
    }

    /// Get the tempo at a position, interpolating across ramp segments.
    ///
    /// Returns the preceding change's bpm for a step, the linearly interpolated
    /// bpm when that change is a ramp and a following change exists, or
    /// [`Self::default_tempo`] before the first change.
    #[must_use]
    pub fn tempo_at(&self, tick: Tick) -> Bpm {
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        if pos == 0 {
            return self.default_tempo;
        }
        let prev = &self.tempo_changes[pos - 1];
        // Ramp only when the starting point is a ramp AND a following point
        // exists to ramp toward; otherwise the tempo holds at `prev.bpm`.
        if let Some(next) = self.tempo_changes.get(pos)
            && prev.ramp
            && next.tick.0 > prev.tick.0
        {
            let u = (tick.0 - prev.tick.0) as f32 / (next.tick.0 - prev.tick.0) as f32;
            let b0 = prev.bpm.as_f32();
            let b1 = next.bpm.as_f32();
            return Bpm::new(b0 + (b1 - b0) * u);
        }
        prev.bpm
    }

    /// Get all tempo changes.
    #[must_use]
    pub fn tempo_changes(&self) -> &[TempoChange] {
        &self.tempo_changes
    }

    /// Clear all tempo changes.
    pub fn clear_tempo_changes(&mut self) {
        self.tempo_changes.clear();
    }

    /// Remove the tempo change at exactly `tick`. Returns true if one was
    /// removed. No-op for ticks that don't carry an explicit change.
    pub fn remove_tempo_change(&mut self, tick: Tick) -> bool {
        let before = self.tempo_changes.len();
        self.tempo_changes.retain(|t| t.tick != tick);
        self.tempo_changes.len() != before
    }

    // === Time signature ===

    /// Set time signature at a position.
    pub fn set_time_signature_at(&mut self, tick: Tick, sig: TimeSignature) {
        self.time_signature_changes.retain(|t| t.tick != tick);

        let change = TimeSignatureChange {
            tick,
            signature: sig,
        };
        let pos = self
            .time_signature_changes
            .partition_point(|t| t.tick <= tick);
        self.time_signature_changes.insert(pos, change);
    }

    /// Get time signature at a position.
    pub fn time_signature_at(&self, tick: Tick) -> TimeSignature {
        let pos = self
            .time_signature_changes
            .partition_point(|t| t.tick <= tick);
        if pos > 0 {
            self.time_signature_changes[pos - 1].signature
        } else {
            self.default_time_signature
        }
    }

    // === Time conversion ===

    /// Convert tick to seconds (handles both step and ramp tempo changes).
    #[must_use]
    pub fn tick_to_seconds(&self, target: Tick) -> f64 {
        let mut seconds = 0.0;
        // Segment currently being walked: it starts at `seg_start` with tempo
        // `b0` and (when `ramp`) ramps toward the next change's bpm. The
        // pre-first-change segment on `default_tempo` is always constant.
        let mut seg_start = 0u64;
        let mut b0 = f64::from(self.default_tempo.as_f32());
        let mut ramp = false;

        for change in &self.tempo_changes {
            let b_end = f64::from(change.bpm.as_f32());
            if change.tick >= target {
                // `target` lies inside the segment ending at this change.
                return seconds
                    + ramp_segment_seconds(seg_start, change.tick.0, b0, b_end, ramp, target.0);
            }
            seconds +=
                ramp_segment_seconds(seg_start, change.tick.0, b0, b_end, ramp, change.tick.0);
            seg_start = change.tick.0;
            b0 = b_end;
            ramp = change.ramp;
        }

        // `target` is past all changes: the final segment is constant at `b0`.
        seconds + ramp_segment_seconds(seg_start, seg_start, b0, b0, false, target.0)
    }

    /// Convert seconds to tick (inverse of [`Self::tick_to_seconds`]; handles
    /// both step and ramp tempo changes).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn seconds_to_tick(&self, target_seconds: f64) -> Tick {
        let mut seconds = 0.0;
        let mut seg_start = 0u64;
        let mut b0 = f64::from(self.default_tempo.as_f32());
        let mut ramp = false;

        for change in &self.tempo_changes {
            let b_end = f64::from(change.bpm.as_f32());
            let seg_ticks = change.tick.0 - seg_start;
            let segment_seconds =
                ramp_segment_seconds(seg_start, change.tick.0, b0, b_end, ramp, change.tick.0);

            if seconds + segment_seconds >= target_seconds {
                let s = target_seconds - seconds;
                let offset = ramp_segment_tick_offset(seg_ticks, b0, b_end, ramp, s);
                return Tick(seg_start + offset);
            }

            seconds += segment_seconds;
            seg_start = change.tick.0;
            b0 = b_end;
            ramp = change.ramp;
        }

        // Target is after all tempo changes: constant tempo at `b0`.
        let remaining_beats = (target_seconds - seconds) * b0 / 60.0;
        let remaining_ticks = (remaining_beats * TICKS_PER_QUARTER as f64) as u64;
        Tick(seg_start + remaining_ticks)
    }

    /// Remove patterns and tracks not referenced by any arrangement placement.
    /// Returns `(removed_pattern_names, removed_track_names, used_instrument_ids)`.
    /// The caller is responsible for removing unused instruments from the engine.
    pub fn remove_unused(
        &mut self,
    ) -> (
        Vec<String>,
        Vec<String>,
        std::collections::HashSet<InstrumentId>,
    ) {
        use std::collections::HashSet;

        // Find used patterns and tracks from arrangement
        let mut used_patterns = HashSet::new();
        let mut used_tracks = HashSet::new();
        for p in &self.arrangement {
            used_patterns.insert(p.pattern_id);
            used_tracks.insert(p.track_id);
        }

        // Remove unused patterns
        let removed_patterns: Vec<String> = self
            .patterns
            .iter()
            .filter(|p| !used_patterns.contains(&p.id))
            .map(|p| p.name.clone())
            .collect();
        self.patterns.retain(|p| used_patterns.contains(&p.id));
        self.bump_structure();

        // Remove unused tracks
        let removed_tracks: Vec<String> = self
            .tracks
            .iter()
            .filter(|t| !used_tracks.contains(&t.id))
            .map(|t| t.name.clone())
            .collect();
        self.tracks.retain(|t| used_tracks.contains(&t.id));

        // Collect instrument IDs still in use. Notes route through their
        // track's instrument, so retained tracks cover every played instrument.
        let mut used_instruments = HashSet::new();
        for track in &self.tracks {
            used_instruments.insert(track.instrument);
        }

        (removed_patterns, removed_tracks, used_instruments)
    }

    /// Calculate total length based on arrangement.
    pub fn calculate_length(&self) -> Tick {
        self.arrangement
            .iter()
            .filter_map(|p| self.pattern(p.pattern_id).map(|pat| p.end(pat.length)))
            .max()
            .unwrap_or(Tick(0))
    }

    /// Get length in seconds.
    #[must_use]
    pub fn length_seconds(&self) -> f64 {
        self.tick_to_seconds(self.calculate_length())
    }
}

impl Default for Song {
    fn default() -> Self {
        Self::new("Untitled")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::track::TrackSend;
    use synth_core::{BipolarValue, NormalizedValue};

    #[test]
    fn return_busses_and_sends_round_trip_json() {
        let mut song = Song::new("rt");
        let rid = song.create_return_bus("Reverb");
        {
            let def = song.return_bus_mut(rid).unwrap();
            def.volume = NormalizedValue::new(0.7);
            def.pan = BipolarValue::new(-0.3);
            def.mute = true;
        }
        let tid = song.create_track("lead");
        song.track_mut(tid).unwrap().sends.push(TrackSend {
            target: rid,
            level: NormalizedValue::new(0.4),
            pre_fader: true,
            enabled: true,
        });

        let json = serde_json::to_string(&song).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();

        assert_eq!(back.return_busses().len(), 1);
        let rb = &back.return_busses()[0];
        assert_eq!(rb.id, rid);
        assert_eq!(rb.name, "Reverb");
        assert!((rb.volume.as_f32() - 0.7).abs() < 1e-6);
        assert!((rb.pan.as_f32() - (-0.3)).abs() < 1e-6);
        assert!(rb.mute);

        let t = back.tracks().next().unwrap();
        assert_eq!(t.sends.len(), 1);
        assert_eq!(t.sends[0].target, rid);
        assert!((t.sends[0].level.as_f32() - 0.4).abs() < 1e-6);
        assert!(t.sends[0].pre_fader);
    }

    #[test]
    fn delete_return_bus_strips_targeting_sends() {
        let mut song = Song::new("rt");
        let rid = song.create_return_bus("Reverb");
        let tid = song.create_track("lead");
        song.track_mut(tid)
            .unwrap()
            .sends
            .push(TrackSend::new(rid, NormalizedValue::MAX));
        assert_eq!(song.tracks().next().unwrap().sends.len(), 1);

        song.delete_return_bus(rid);
        assert!(song.return_busses().is_empty());
        assert_eq!(
            song.tracks().next().unwrap().sends.len(),
            0,
            "sends targeting a deleted return bus must be removed"
        );
    }

    #[test]
    fn delete_track_strips_pinned_lanes_keeps_host_lanes() {
        use crate::automation::{AutomationLane, AutomationTarget, TrackParam};

        let mut song = Song::new("gc");
        let keep = song.create_track("keep");
        let doomed = song.create_track("doomed");
        let pid = song.create_pattern(Duration(960));
        let pattern = song.pattern_mut(pid).unwrap();
        pattern
            .automation
            .push(AutomationLane::new(AutomationTarget::Track {
                track: Some(doomed),
                param: TrackParam::Volume,
            }));
        pattern
            .automation
            .push(AutomationLane::new(AutomationTarget::Track {
                track: Some(keep),
                param: TrackParam::Pan,
            }));
        pattern
            .automation
            .push(AutomationLane::new(AutomationTarget::Track {
                track: None,
                param: TrackParam::Volume,
            }));

        song.delete_track(doomed);

        let lanes = &song.pattern(pid).unwrap().automation;
        assert_eq!(
            lanes.len(),
            2,
            "only the lane pinned to the deleted track is stripped"
        );
        assert!(lanes.iter().all(|l| !matches!(
            l.target,
            AutomationTarget::Track { track: Some(t), .. } if t == doomed
        )));
    }

    #[test]
    fn test_song_creation() {
        let song = Song::new("Test Song")
            .with_author("Test Author")
            .with_tempo(Bpm::new(140.0));

        assert_eq!(song.name, "Test Song");
        assert_eq!(song.author, "Test Author");
        assert_eq!(song.default_tempo, Bpm::new(140.0));
    }

    #[test]
    fn test_pattern_management() {
        let mut song = Song::new("Test");

        let id = song.create_pattern(Duration(3840));
        assert!(song.pattern(id).is_some());
        assert_eq!(song.pattern_count(), 1);

        song.delete_pattern(id);
        assert!(song.pattern(id).is_none());
        assert_eq!(song.pattern_count(), 0);
    }

    #[test]
    fn test_track_management() {
        let mut song = Song::new("Test");

        let id = song.create_track("Lead");
        assert_eq!(song.track(id).unwrap().name, "Lead");
        assert_eq!(song.track_count(), 1);
    }

    #[test]
    fn test_arrangement() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(3840));
        let track_id = song.create_track("Track 1");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(3840));

        assert_eq!(song.arrangement().len(), 2);
    }

    #[test]
    fn test_tick_to_seconds_constant_tempo() {
        let song = Song::new("Test");
        // 120 BPM = 2 beats/sec = 1920 ticks/sec
        let seconds = song.tick_to_seconds(Tick(1920));
        assert!((seconds - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_tick_to_seconds_with_tempo_change() {
        let mut song = Song::new("Test");
        song.set_tempo_at(Tick(960), Bpm::new(240.0)); // Double tempo after 1 beat

        // First beat: 0.5 sec (120 BPM)
        // Second beat: 0.25 sec (240 BPM)
        let seconds = song.tick_to_seconds(Tick(1920));
        assert!((seconds - 0.75).abs() < 0.001);
    }

    #[test]
    fn test_tempo_at() {
        let mut song = Song::new("Test");
        assert_eq!(song.tempo_at(Tick(0)), Bpm::new(120.0));

        song.set_tempo_at(Tick(1000), Bpm::new(180.0));
        assert_eq!(song.tempo_at(Tick(500)), Bpm::new(120.0));
        assert_eq!(song.tempo_at(Tick(1500)), Bpm::new(180.0));
    }

    #[test]
    fn tempo_at_ramp_interpolates() {
        let mut song = Song::new("ramp");
        song.set_tempo_ramp_at(Tick(0), Bpm::new(120.0), true);
        song.set_tempo_at(Tick(960), Bpm::new(240.0));

        // Midpoint of a linear-in-tick ramp is the endpoint mean.
        assert!((song.tempo_at(Tick(480)).as_f32() - 180.0).abs() < 1e-3);
        // Endpoints are exact.
        assert!((song.tempo_at(Tick(0)).as_f32() - 120.0).abs() < 1e-3);
        assert!((song.tempo_at(Tick(960)).as_f32() - 240.0).abs() < 1e-3);
        // Past the last change: holds constant (nothing to ramp toward).
        assert!((song.tempo_at(Tick(1920)).as_f32() - 240.0).abs() < 1e-3);
    }

    #[test]
    fn tick_to_seconds_ramp_matches_closed_form() {
        let mut song = Song::new("ramp");
        song.set_tempo_ramp_at(Tick(0), Bpm::new(120.0), true);
        song.set_tempo_at(Tick(960), Bpm::new(240.0));

        // Exact log-integral over the ramp: K·ln(b1/b0)/(b1−b0), K = 1 beat · 60.
        let expected = 60.0 * f64::ln(2.0) / 120.0;
        assert!((song.tick_to_seconds(Tick(960)) - expected).abs() < 1e-6);
        // A step change with equal endpoints would give 0.5 s over the beat;
        // the ramp is faster on average, so strictly less.
        assert!(song.tick_to_seconds(Tick(960)) < 0.5);
    }

    #[test]
    fn seconds_to_tick_inverts_ramp() {
        let mut song = Song::new("ramp");
        song.set_tempo_ramp_at(Tick(0), Bpm::new(90.0), true); // seg 0: 90→160 ramp
        song.set_tempo_ramp_at(Tick(1920), Bpm::new(160.0), true); // seg 1: 160→120 ramp
        song.set_tempo_at(Tick(3840), Bpm::new(120.0)); // constant tail

        for &t in &[0_u64, 500, 960, 1920, 2500, 3840, 5000] {
            let seconds = song.tick_to_seconds(Tick(t));
            let back = song.seconds_to_tick(seconds);
            // Integer-tick truncation costs at most a couple of ticks.
            assert!(
                (back.0 as i64 - t as i64).abs() <= 2,
                "round-trip t={t} -> {seconds}s -> {}",
                back.0
            );
        }
    }

    #[test]
    fn tempo_map_round_trips_json() {
        let mut song = Song::new("rt");
        song.default_tempo = Bpm::new(100.0);
        song.set_tempo_at(Tick(960), Bpm::new(140.0));
        song.set_tempo_at(Tick(3840), Bpm::new(90.0));
        // Out-of-order insert must land sorted.
        song.set_tempo_at(Tick(1920), Bpm::new(160.0));

        let json = serde_json::to_string(&song).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();

        let changes = back.tempo_changes();
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].tick, Tick(960));
        assert_eq!(changes[1].tick, Tick(1920));
        assert_eq!(changes[2].tick, Tick(3840));
        assert_eq!(changes[0].bpm, Bpm::new(140.0));
        assert_eq!(changes[1].bpm, Bpm::new(160.0));
        assert_eq!(changes[2].bpm, Bpm::new(90.0));
        assert_eq!(back.default_tempo, Bpm::new(100.0));
        assert_eq!(back.tempo_at(Tick(2000)), Bpm::new(160.0));
    }

    #[test]
    fn test_calculate_length() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(3840));
        let track_id = song.create_track("Track");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(3840));

        assert_eq!(song.calculate_length().0, 7680);
    }

    #[test]
    fn structure_generation_tracks_length_affecting_mutations() {
        let mut song = Song::new("Test");

        // Read-only access never advances the generation, so the engine's
        // cached length stays valid across plain `calculate_length` reads.
        let g0 = song.structure_generation();
        let _ = song.calculate_length();
        assert_eq!(song.structure_generation(), g0, "reads must not bump");

        // Each mutation that can change `calculate_length` advances it.
        let pattern_id = song.create_pattern(Duration(960));
        let g_after_pattern = song.structure_generation();
        assert!(g_after_pattern > g0, "create_pattern must bump");

        let track_id = song.create_track("Track");

        assert!(song.place_pattern(pattern_id, track_id, Tick(0)));
        let g_after_place = song.structure_generation();
        assert!(g_after_place > g_after_pattern, "place_pattern must bump");

        assert!(song.set_placement_length(pattern_id, track_id, Tick(0), Some(Duration(1920)),));
        let g_after_len = song.structure_generation();
        assert!(
            g_after_len > g_after_place,
            "set_placement_length must bump"
        );

        // A `&mut Pattern` handout conservatively bumps even though we cannot
        // see whether the caller changed `pattern.length`.
        let _ = song.pattern_mut(pattern_id);
        assert!(
            song.structure_generation() > g_after_len,
            "pattern_mut must bump conservatively"
        );

        // A failed lookup that placed nothing must not change the result the
        // engine reads — calculate_length still reflects the override.
        assert!(!song.move_placement(pattern_id, track_id, Tick(9999), track_id, Tick(0)));
        assert_eq!(song.calculate_length().0, 1920);
    }

    #[test]
    fn test_move_placement() {
        let mut song = Song::new("Test");
        let pattern_id = song.create_pattern(Duration(960));
        let track1 = song.create_track("Track 1");
        let track2 = song.create_track("Track 2");

        song.place_pattern(pattern_id, track1, Tick(0));

        // Move to different track and position
        assert!(song.move_placement(pattern_id, track1, Tick(0), track2, Tick(1920)));
        assert_eq!(song.arrangement().len(), 1);
        assert_eq!(song.arrangement()[0].track_id, track2);
        assert_eq!(song.arrangement()[0].start, Tick(1920));

        // Move non-existent placement returns false
        assert!(!song.move_placement(pattern_id, track1, Tick(0), track2, Tick(0)));
    }

    #[test]
    fn test_placements_in_range() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(960));
        let track_id = song.create_track("Track");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(1000));
        song.place_pattern(pattern_id, track_id, Tick(2000));

        let placements: Vec<_> = song.placements_in_range(Tick(500), Tick(1500)).collect();
        assert_eq!(placements.len(), 2); // First and second overlap with range
    }

    #[test]
    fn tracks_mut_iterates_every_track() {
        let mut song = Song::new("Test");
        let a = song.create_track("A");
        let b = song.create_track("B");
        let c = song.create_track("C");

        let mut seen: Vec<TrackId> = song.tracks_mut().map(|t| t.id).collect();
        seen.sort_by_key(|id| id.0);
        assert_eq!(seen, vec![a, b, c]);

        for track in song.tracks_mut() {
            track.mute = true;
        }
        assert!(song.tracks().all(|t| t.mute));
    }

    #[test]
    fn set_solo_only_isolates_target_and_clears_others() {
        let mut song = Song::new("Test");
        let a = song.create_track("A");
        let b = song.create_track("B");
        let c = song.create_track("C");

        song.track_mut(a).unwrap().solo = true;
        song.track_mut(b).unwrap().solo = true;

        song.set_solo_only(c);

        assert!(!song.track(a).unwrap().solo);
        assert!(!song.track(b).unwrap().solo);
        assert!(song.track(c).unwrap().solo);
        assert!(song.any_solo());
    }

    #[test]
    fn set_solo_only_with_unknown_target_clears_all() {
        let mut song = Song::new("Test");
        let a = song.create_track("A");
        song.track_mut(a).unwrap().solo = true;

        // TrackId(9999) does not exist; helper must still clear existing solos.
        song.set_solo_only(TrackId(9999));

        assert!(!song.track(a).unwrap().solo);
        assert!(!song.any_solo());
    }

    #[test]
    fn test_automated_module_index() {
        use crate::automation::{AutomationPoint, AutomationTarget};
        use crate::time::PatternTick;
        use synth_core::{ModuleType, NormalizedValue};

        let mut song = Song::new("auto");
        let pid = song.create_pattern(Duration(3840));
        let target = AutomationTarget::Module {
            instrument: InstrumentId::new(2),
            module_type: ModuleType::Filter,
            instance: 1,
            param_id: "cutoff".into(),
        };

        // An empty lane (picker artifact) is not counted as a reference.
        song.pattern_mut(pid)
            .unwrap()
            .get_or_create_automation(target.clone());
        assert!(!song.is_module_automated(InstrumentId::new(2), ModuleType::Filter, 1));
        assert!(song.automated_module_params().is_empty());

        // A point makes it a real reference.
        song.pattern_mut(pid)
            .unwrap()
            .get_or_create_automation(target)
            .add_point(AutomationPoint::new(
                PatternTick(0),
                NormalizedValue::new(0.5),
            ));

        assert!(song.is_module_automated(InstrumentId::new(2), ModuleType::Filter, 1));
        // Different instance / instrument are not automated.
        assert!(!song.is_module_automated(InstrumentId::new(2), ModuleType::Filter, 2));
        assert!(!song.is_module_automated(InstrumentId::new(9), ModuleType::Filter, 1));

        let index = song.automated_module_params();
        let params = index
            .get(&(InstrumentId::new(2), ModuleType::Filter, 1))
            .expect("filter instance 1 must be indexed");
        assert!(params.contains("cutoff"));
    }

    // --- Note Grid pool -----------------------------------------------------

    use crate::ids::NoteModuleId;
    use crate::note_graph::{NoteConnection, NoteModuleConfig};
    use crate::note_processor::{Humanize, NoteProcessor};

    fn humanize_node() -> NoteModuleConfig {
        NoteModuleConfig::Processor(NoteProcessor::Humanize(Humanize::default()))
    }

    /// Add a 1->2 chain to the pooled graph `gid`.
    fn build_chain(song: &mut Song, gid: NoteGraphId) {
        let g = song.note_graph_mut(gid).expect("graph");
        g.try_insert_node(NoteModuleId::new(1), humanize_node())
            .unwrap();
        g.try_insert_node(NoteModuleId::new(2), humanize_node())
            .unwrap();
        g.try_connect(NoteConnection::stream(
            NoteModuleId::new(1),
            NoteModuleId::new(2),
        ))
        .unwrap();
    }

    #[test]
    fn note_graph_pool_crud_and_usage() {
        let mut song = Song::new("t");
        let gid = song.create_note_graph("arp");
        assert_eq!(song.note_graphs().count(), 1);
        assert!(song.note_graph(gid).is_some());
        build_chain(&mut song, gid);
        let pid = song.create_pattern(Duration(960));
        song.pattern_mut(pid).unwrap().set_note_graph(Some(gid));
        assert_eq!(song.note_graph_usage(gid), 1);

        // Removing the graph clears the pattern reference.
        let removed = song.remove_note_graph(gid).expect("removed");
        assert_eq!(removed.id, gid);
        assert_eq!(song.note_graph_usage(gid), 0);
        assert_eq!(song.pattern(pid).unwrap().note_graph(), None);
        assert!(song.note_graph(gid).is_none());
    }

    #[test]
    fn note_graph_survives_round_trip_and_rebuilds_order() {
        let mut song = Song::new("t");
        let gid = song.create_note_graph("chain");
        build_chain(&mut song, gid);
        song.note_graph_mut(gid)
            .unwrap()
            .node_descriptions
            .insert(NoteModuleId::new(1), "First transform".to_owned());
        let pid = song.create_pattern(Duration(960));
        song.pattern_mut(pid).unwrap().set_note_graph(Some(gid));

        let json = serde_json::to_string(&song).unwrap();
        let mut back: Song = serde_json::from_str(&json).unwrap();

        // processing_order is not serialized → empty until rebuilt.
        assert!(back.note_graph(gid).unwrap().processing_order.is_empty());
        back.rebuild_note_graphs();
        assert_eq!(
            back.note_graph(gid).unwrap().processing_order,
            vec![NoteModuleId::new(1), NoteModuleId::new(2)]
        );
        assert_eq!(
            back.note_graph(gid).unwrap().node_descriptions[&NoteModuleId::new(1)],
            "First transform"
        );
        // Binding survived, and next_note_graph_id stayed ahead of restored ids.
        assert_eq!(back.pattern(pid).unwrap().note_graph(), Some(gid));
        assert_ne!(back.create_note_graph("second"), gid);
    }

    #[test]
    fn mod_graph_survives_json_round_trip() {
        use crate::automation::{AutomationTarget, TrackParam};
        use crate::ids::{InstrumentId, ModNodeId};
        use crate::mod_grid::{
            AudioTapNode, AudioTapSource, CombineMode, MacroNode, MidiCcNode, ModConnection,
            ModNodeConfig, ModTarget, ModuleNode, TransportNode, TransportSource,
        };
        use crate::note_graph::NodePosition;
        use crate::track::TrackColor;
        use std::collections::BTreeMap;
        use synth_core::ModuleType;

        let mut song = Song::new("t");
        // A track the Track-scope graph is assigned to.
        let tid = song.create_track("lead");
        let gid = song.create_mod_graph("wobble");

        {
            let g = song.mod_graph_mut(gid).expect("graph exists");
            g.scope = ModGraphScope::Track;
            g.assigned_tracks = vec![tid];
            g.description = "test graph".into();
            g.color = Some(TrackColor {
                r: 10,
                g: 20,
                b: 30,
            });

            // A hosted LFO with a param map and an explicit seed (random-family
            // offline determinism).
            let mut params = BTreeMap::new();
            params.insert("rate".to_string(), 3.5);
            g.try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Module(ModuleNode {
                    module_type: ModuleType::Lfo,
                    params,
                    seed: Some(0xDEAD_BEEF),
                }),
            )
            .unwrap();
            // The cheap grid-native sources: MIDI CC, audio tap, transport, macro.
            g.try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::MidiCc(MidiCcNode {
                    cc: 74,
                    channel: Some(2),
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(2),
                ModNodeConfig::AudioTap(AudioTapNode {
                    source: AudioTapSource::Master,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(3),
                ModNodeConfig::Transport(TransportNode {
                    source: TransportSource::BarPhase,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(4),
                ModNodeConfig::Macro(MacroNode {
                    name: "depth".into(),
                    value: 0.6,
                }),
            )
            .unwrap();
            // Two routing sinks: a relative "this track" volume and a module param.
            g.try_insert_node(
                ModNodeId::new(5),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Track {
                        track: None,
                        param: TrackParam::Volume,
                    },
                    amount: 0.34,
                    combine: CombineMode::Add,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(6),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Module {
                        instrument: InstrumentId::new(0),
                        module_type: ModuleType::Filter,
                        instance: 1,
                        param_id: "cutoff".into(),
                    },
                    amount: 1.0,
                    combine: CombineMode::Add,
                }),
            )
            .unwrap();
            // Cables: LFO → track-volume target, MIDI CC → module-cutoff target.
            g.try_connect(ModConnection::new(
                ModNodeId::new(0),
                "out",
                ModNodeId::new(5),
                "in",
            ))
            .unwrap();
            g.try_connect(ModConnection::new(
                ModNodeId::new(1),
                "out",
                ModNodeId::new(6),
                "in",
            ))
            .unwrap();
            // Canvas layout metadata for one node.
            g.node_positions
                .insert(ModNodeId::new(0), NodePosition { x: 12.0, y: 34.0 });
            g.node_descriptions
                .insert(ModNodeId::new(0), "Slow movement source".to_owned());
        }

        let original = song.mod_graph(gid).unwrap().clone();

        let json = serde_json::to_string(&song).unwrap();
        let mut back: Song = serde_json::from_str(&json).unwrap();

        // The whole graph survives (nodes, cables, scope, assignment, description,
        // color, positions, per-node params + seed).
        assert_eq!(back.mod_graph(gid), Some(&original));
        // The restored pool keeps the id allocator ahead — a fresh graph gets a
        // new id, not a collision with the restored one.
        assert_ne!(back.create_mod_graph("second"), gid);
    }

    #[test]
    fn dangling_note_graph_reference_resolves_to_none() {
        let mut song = Song::new("t");
        let pid = song.create_pattern(Duration(960));
        // Bind a non-existent graph id (e.g. after an out-of-band removal).
        song.pattern_mut(pid)
            .unwrap()
            .set_note_graph(Some(NoteGraphId::new(99)));
        // The pool has no such graph (pass-through is decided at expansion)…
        assert!(song.note_graph(NoteGraphId::new(99)).is_none());
        // …but the binding itself persists.
        assert_eq!(
            song.pattern(pid).unwrap().note_graph(),
            Some(NoteGraphId::new(99))
        );
    }

    #[test]
    fn convert_rack_to_graph_matches_rack_playback() {
        use crate::note_processor::{
            Chord, ExpandedNote, ExpansionBuffer, Humanize, ScaleQuantize,
        };
        use crate::pitch::{Pitch, Velocity};
        use synth_core::NormalizedValue;

        let mut song = Song::new("t");
        let pid = song.create_pattern(Duration(960));
        {
            let p = song.pattern_mut(pid).unwrap();
            let _ = p.add_note(PatternTick(0), Pitch::new(61).unwrap(), Velocity::MF);
            let _ = p.add_note(PatternTick(240), Pitch::new(66).unwrap(), Velocity::MF);
            p.add_processor(NoteProcessor::ScaleQuantize(ScaleQuantize::default()));
            p.add_processor(NoteProcessor::Chord(Chord::major()));
            // A SEEDED Humanize: byte-identical only because convert pre-compensates
            // its seed for the host key (the fix — without it the graph re-seeds).
            p.add_processor(NoteProcessor::Humanize(Humanize {
                velocity: NormalizedValue::new(0.3),
                gate: NormalizedValue::new(0.2),
                seed: 0x9E37_79B9,
            }));
        }
        let bpm = Bpm::DEFAULT;
        let expand = |song: &Song, use_graph: bool| -> Vec<(u32, ExpandedNote)> {
            let p = song.pattern(pid).unwrap();
            let mut buf = ExpansionBuffer::new();
            let mut out = Vec::new();
            for t in 0..960 {
                if use_graph {
                    let g = song.note_graph(p.note_graph().unwrap()).unwrap();
                    g.expand_at_tick(
                        p.notes(),
                        PatternTick(t),
                        HostKey::from(pid),
                        bpm,
                        |_| true,
                        None,
                        None,
                        &mut buf,
                    );
                } else {
                    p.expand_at_tick(PatternTick(t), |_| true, bpm, None, &mut buf);
                }
                out.extend(buf.notes().iter().map(|n| (t, *n)));
            }
            out
        };
        let rack_out = expand(&song, false);
        let gid = song.convert_rack_to_graph(pid).expect("rack converts");
        assert_eq!(song.pattern(pid).unwrap().note_graph(), Some(gid));
        assert!(song.pattern(pid).unwrap().processors().is_empty());
        assert_eq!(rack_out, expand(&song, true), "graph plays like the rack");
        // Idempotent: an already-bound / empty-rack pattern does not re-convert.
        assert_eq!(song.convert_rack_to_graph(pid), None);
    }

    /// Full playback expansion of a pattern over `[0, walk_end)`, resolving each
    /// note's note-scope graph from the pool — the reference for freeze equality.
    fn note_scope_playback(
        song: &Song,
        pid: PatternId,
        bpm: Bpm,
    ) -> Vec<(u32, u8, u32, Option<u32>)> {
        use crate::note_processor::{ExpansionBuffer, note_scope_strum_tail};
        let p = song.pattern(pid).unwrap();
        let mut scratch = ExpansionBuffer::new();
        let mut buf = ExpansionBuffer::new();
        let mut lookback = crate::lookback_pool();
        let mut out = Vec::new();
        // Mirror `freeze_pattern`'s walk exactly (the bound pattern-scope graph's
        // own strum tail + echo tail, plus the widest note-scope articulation
        // tail) so this reference can't drift from freeze if a strumming or
        // echoing pattern-scope graph is used. (These tests carry no rack, so the
        // rack strum tail is 0.)
        let pattern_scope_tail = p
            .note_graph()
            .and_then(|gid| song.note_graph(gid))
            .map_or(0, NoteGraph::max_walk_tail);
        let walk_end = p
            .source_walk_end()
            .saturating_add(pattern_scope_tail)
            .saturating_add(note_scope_strum_tail(p.notes(), song.note_graph_pool()));
        for t in 0..walk_end {
            let mut ctx = NoteScopeCtx {
                pool: song.note_graph_pool(),
                scratch: &mut scratch,
            };
            match p.note_graph().and_then(|gid| song.note_graph(gid)) {
                Some(g) => g.expand_at_tick(
                    p.notes(),
                    PatternTick(t),
                    HostKey::from(pid),
                    bpm,
                    |_| true,
                    Some(&mut ctx),
                    Some(&mut lookback),
                    &mut buf,
                ),
                None => p.expand_at_tick(PatternTick(t), |_| true, bpm, Some(&mut ctx), &mut buf),
            }
            out.extend(buf.notes().iter().map(|n| {
                (
                    t,
                    n.pitch.as_midi(),
                    n.velocity.as_f32().to_bits(),
                    n.duration.map(|d| d.0),
                )
            }));
        }
        out.sort_unstable();
        out
    }

    #[test]
    fn note_scope_freeze_equals_playback_no_rack() {
        use crate::note_processor::{Chord, NoteProcessor};
        use crate::pitch::{Pitch, Velocity};
        // One note bound to a note-scope triad graph, no rack, no pattern graph:
        // freeze goes through `freeze_processors` (the `has_note_scope` guard).
        let mut song = Song::new("t");
        let pid = song.create_pattern(Duration(960));
        let gid = song.create_note_graph("triad");
        song.note_graph_mut(gid)
            .unwrap()
            .try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Processor(NoteProcessor::Chord(Chord::major())),
            )
            .unwrap();
        {
            let p = song.pattern_mut(pid).unwrap();
            let nid = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
            p.note_mut(nid).unwrap().note_graph = Some(gid);
        }
        song.rebuild_note_graphs();

        let bpm = Bpm::DEFAULT;
        let expected = note_scope_playback(&song, pid, bpm);
        assert_eq!(expected.len(), 3, "triad, three tones");

        song.freeze_pattern(pid, bpm);
        let p = song.pattern(pid).unwrap();
        let mut baked: Vec<(u32, u8, u32, Option<u32>)> = p
            .notes()
            .iter()
            .map(|n| {
                (
                    n.start.0,
                    n.pitch.as_midi(),
                    n.velocity.as_f32().to_bits(),
                    n.duration.map(|d| d.0),
                )
            })
            .collect();
        baked.sort_unstable();
        assert_eq!(baked, expected, "note-scope freeze must equal playback");
        assert!(
            p.notes().iter().all(|n| n.note_graph.is_none()),
            "baked notes are plain (the note-scope binding is retired)"
        );
    }

    #[test]
    fn note_scope_freeze_equals_playback_through_pattern_graph() {
        use crate::note_processor::{Chord, NoteProcessor, PitchClass, ScaleMask, ScaleQuantize};
        use crate::pitch::{Pitch, Velocity};
        // Note scope articulates a triad; a bound *pattern-scope* graph then
        // quantizes it — so freeze runs the bake path (`freeze_pattern_note_graph`)
        // and must still equal playback.
        let mut song = Song::new("t");
        let pid = song.create_pattern(Duration(960));
        let ns_gid = song.create_note_graph("triad");
        song.note_graph_mut(ns_gid)
            .unwrap()
            .try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Processor(NoteProcessor::Chord(Chord::major())),
            )
            .unwrap();
        let pat_gid = song.create_note_graph("quantize");
        song.note_graph_mut(pat_gid)
            .unwrap()
            .try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
                    root: PitchClass::new(0),
                    mask: ScaleMask::from_intervals(&[0, 2, 3, 5, 7, 9, 10]),
                })),
            )
            .unwrap();
        {
            let p = song.pattern_mut(pid).unwrap();
            let nid = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
            p.note_mut(nid).unwrap().note_graph = Some(ns_gid);
            p.set_note_graph(Some(pat_gid));
        }
        song.rebuild_note_graphs();

        let bpm = Bpm::DEFAULT;
        let expected = note_scope_playback(&song, pid, bpm);
        assert!(!expected.is_empty(), "the articulated, quantized triad");

        song.freeze_pattern(pid, bpm);
        let p = song.pattern(pid).unwrap();
        let mut baked: Vec<(u32, u8, u32, Option<u32>)> = p
            .notes()
            .iter()
            .map(|n| {
                (
                    n.start.0,
                    n.pitch.as_midi(),
                    n.velocity.as_f32().to_bits(),
                    n.duration.map(|d| d.0),
                )
            })
            .collect();
        baked.sort_unstable();
        assert_eq!(baked, expected, "bake-path freeze must equal playback");
        assert_eq!(
            p.note_graph(),
            None,
            "the pattern-scope binding is retired by the bake"
        );
    }

    #[test]
    fn migrate_processor_racks_converts_only_unbound_racks() {
        use crate::note_processor::ScaleQuantize;

        let mut song = Song::new("t");
        // Pattern A: a legacy rack, no binding → migrates.
        let a = song.create_pattern(Duration(960));
        song.pattern_mut(a)
            .unwrap()
            .add_processor(NoteProcessor::ScaleQuantize(ScaleQuantize::default()));
        // Pattern B: already graph-bound → untouched.
        let gid = song.create_note_graph("existing");
        let b = song.create_pattern(Duration(960));
        song.pattern_mut(b).unwrap().set_note_graph(Some(gid));
        // Pattern C: empty rack, no binding → untouched.
        let c = song.create_pattern(Duration(960));

        assert_eq!(song.migrate_processor_racks_to_graphs(), 1);

        // A converted: bound to a fresh graph, rack drained.
        assert!(song.pattern(a).unwrap().note_graph().is_some());
        assert!(song.pattern(a).unwrap().processors().is_empty());
        assert_ne!(song.pattern(a).unwrap().note_graph(), Some(gid));
        // B kept its existing binding; C stays unbound.
        assert_eq!(song.pattern(b).unwrap().note_graph(), Some(gid));
        assert_eq!(song.pattern(c).unwrap().note_graph(), None);
        // Idempotent: a second pass migrates nothing.
        assert_eq!(song.migrate_processor_racks_to_graphs(), 0);
    }

    #[test]
    fn freeze_pattern_note_graph_bakes_and_clears_binding() {
        use crate::note_processor::{PitchClass, ScaleMask, ScaleQuantize};
        use crate::pitch::{Pitch, Velocity};

        let mut song = Song::new("t");
        let gid = song.create_note_graph("snap");
        {
            let g = song.note_graph_mut(gid).unwrap();
            g.try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
                    root: PitchClass::new(0),
                    mask: ScaleMask::MAJOR,
                })),
            )
            .unwrap();
        }
        let pid = song.create_pattern(Duration(960));
        {
            let p = song.pattern_mut(pid).unwrap();
            let _ = p.add_note(PatternTick(0), Pitch::new(61).unwrap(), Velocity::MF);
            p.set_note_graph(Some(gid));
        }

        assert!(
            song.freeze_pattern_note_graph(pid, synth_core::Bpm::DEFAULT)
                .is_some()
        );
        // Binding cleared; the pooled graph survives for other patterns.
        assert_eq!(song.pattern(pid).unwrap().note_graph(), None);
        assert!(song.note_graph(gid).is_some());
        // The baked note is the graph output: C#4 (61) snapped to D4 (62).
        let pitches: Vec<u8> = song
            .pattern(pid)
            .unwrap()
            .notes()
            .iter()
            .map(|n| n.pitch.as_midi())
            .collect();
        assert_eq!(pitches, vec![62]);
    }

    /// Offline note analyzers expand a pattern through its bound graph (plan §7):
    /// a C4 through a `[0,4,7]` Chord node reads as the full triad, not one note —
    /// and the authored source is left untouched (unlike freeze). Regression for
    /// the "analyzers read source not expansion" gap.
    #[test]
    fn expanded_pattern_notes_expands_through_bound_graph() {
        use crate::note_processor::Chord;
        use crate::pitch::{Pitch, Velocity};

        let mut song = Song::new("t");
        let gid = song.create_note_graph("triad");
        {
            let g = song.note_graph_mut(gid).unwrap();
            g.try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Processor(NoteProcessor::Chord(Chord::new(&[0, 4, 7]))),
            )
            .unwrap();
        }
        let pid = song.create_pattern(Duration(960));
        {
            let p = song.pattern_mut(pid).unwrap();
            let _ = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
            p.set_note_graph(Some(gid));
        }

        let expanded = song.expanded_pattern_notes(pid, synth_core::Bpm::DEFAULT);
        let mut pitches: Vec<u8> = expanded.iter().map(|n| n.pitch.as_midi()).collect();
        pitches.sort_unstable();
        assert_eq!(
            pitches,
            vec![60, 64, 67],
            "the analyzer sees the played triad"
        );
        // Non-mutating: the authored source is untouched (unlike freeze).
        assert_eq!(song.pattern(pid).unwrap().notes().len(), 1);

        // No binding ⇒ the raw authored notes.
        let plain = song.create_pattern(Duration(960));
        let _ = song.pattern_mut(plain).unwrap().add_note(
            PatternTick(0),
            Pitch::new(72).unwrap(),
            Velocity::MF,
        );
        let unbound = song.expanded_pattern_notes(plain, synth_core::Bpm::DEFAULT);
        assert_eq!(unbound.len(), 1);
        assert_eq!(unbound[0].pitch.as_midi(), 72);
    }

    #[test]
    fn duplicate_pattern_keeps_the_note_graph_binding() {
        // Found in-app (2026-07-12): the duplicate dropped the pattern's Note
        // Grid binding, so a copy silently played dry. A duplicate must play
        // like its source — the pooled graph is shared by reference.
        let mut song = Song::new("t");
        let gid = song.create_note_graph("shared");
        let pid = song.create_pattern(Duration(960));
        song.pattern_mut(pid).unwrap().set_note_graph(Some(gid));
        let copy = song.duplicate_pattern(pid).expect("duplicates");
        assert_eq!(
            song.pattern(copy).unwrap().note_graph(),
            Some(gid),
            "the copy keeps the binding"
        );
        assert_eq!(song.note_graph_usage(gid), 2, "usage counts both patterns");
    }

    #[test]
    fn freeze_of_generator_headed_graph_bakes_nothing_past_the_pattern_end() {
        use crate::note_graph::{EuclideanGenerator, NoteConnection, NoteDelay};
        use crate::pitch::{Pitch, Velocity};
        use synth_core::NormalizedValue;

        // Euclid → Delay: the delay has real reach, but a generator-headed spine
        // must not extend the freeze walk — it would bake phantom generator hits
        // (and their echoes) past the pattern end, inert at the current length
        // and surfacing as stale material if the pattern is later lengthened.
        let mut song = Song::new("t");
        let gid = song.create_note_graph("euclid-delay");
        {
            let g = song.note_graph_mut(gid).unwrap();
            g.try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Euclidean(EuclideanGenerator {
                    steps: 4,
                    pulses: 4,
                    rotation: 0,
                    step_len: Duration(240),
                    pitch: Pitch::new(60).unwrap(),
                    velocity: Velocity::MF,
                }),
            )
            .unwrap();
            g.try_insert_node(
                NoteModuleId::new(2),
                NoteModuleConfig::NoteDelay(NoteDelay {
                    delay_ticks: Duration(480),
                    repeats: 3,
                    feedback: NormalizedValue::new(0.6),
                }),
            )
            .unwrap();
            g.try_connect(NoteConnection::stream(
                NoteModuleId::new(1),
                NoteModuleId::new(2),
            ))
            .unwrap();
        }
        let pid = song.create_pattern(Duration(960));
        song.pattern_mut(pid).unwrap().set_note_graph(Some(gid));

        assert!(
            song.freeze_pattern_note_graph(pid, synth_core::Bpm::DEFAULT)
                .is_some()
        );
        let notes = song.pattern(pid).unwrap().notes().to_vec();
        assert!(
            !notes.is_empty(),
            "the generator's in-range hits and echoes bake"
        );
        assert!(
            notes.iter().all(|n| n.start.0 < 960),
            "nothing baked at or past the pattern end, got starts {:?}",
            notes.iter().map(|n| n.start.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn freeze_pattern_honors_graph_precedence_and_retires_rack() {
        use crate::note_processor::{PitchClass, ScaleMask, ScaleQuantize};
        use crate::pitch::{Pitch, Velocity};

        let mut song = Song::new("t");
        let gid = song.create_note_graph("snap");
        song.note_graph_mut(gid)
            .unwrap()
            .try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(ScaleQuantize {
                    root: PitchClass::new(0),
                    mask: ScaleMask::MAJOR,
                })),
            )
            .unwrap();
        let pid = song.create_pattern(Duration(960));
        {
            let p = song.pattern_mut(pid).unwrap();
            let _ = p.add_note(PatternTick(0), Pitch::new(61).unwrap(), Velocity::MF);
            // A rack that playback IGNORES while the graph is bound: a chord
            // would fan the note out if it were (wrongly) applied.
            p.add_processor(NoteProcessor::Chord(crate::note_processor::Chord::new(&[
                0, 4, 7,
            ])));
            p.set_note_graph(Some(gid));
        }

        let bpm = song.tempo_at(crate::time::Tick(0));
        let count = song.freeze_pattern(pid, bpm).notes;

        // The GRAPH baked (one snapped note, no chord fan-out), and the
        // suppressed rack is retired — leaving it would re-process the baked
        // notes and change playback right after the freeze.
        assert_eq!(count, 1);
        let p = song.pattern(pid).unwrap();
        assert_eq!(p.note_graph(), None);
        assert!(p.processors().is_empty());
        assert_eq!(p.notes()[0].pitch.as_midi(), 62);
    }

    #[test]
    fn freeze_pattern_dangling_graph_falls_back_to_rack() {
        use crate::pitch::{Pitch, Velocity};

        let mut song = Song::new("t");
        let pid = song.create_pattern(Duration(960));
        {
            let p = song.pattern_mut(pid).unwrap();
            let _ = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
            p.add_processor(NoteProcessor::Chord(crate::note_processor::Chord::new(&[
                0, 4, 7,
            ])));
            // Dangling binding: no such graph in the pool — playback falls
            // back to the rack, so freeze must too.
            p.set_note_graph(Some(NoteGraphId::new(99)));
        }

        let bpm = song.tempo_at(crate::time::Tick(0));
        let count = song.freeze_pattern(pid, bpm).notes;

        // The RACK baked (chord fan-out), the stale binding is cleared.
        assert_eq!(count, 3);
        let p = song.pattern(pid).unwrap();
        assert_eq!(p.note_graph(), None);
        assert!(p.processors().is_empty());
    }

    #[test]
    fn duplicate_note_graph_copies_everything_but_identity() {
        let mut song = Song::new("t");
        let gid = song.create_note_graph("src");
        {
            let g = song.note_graph_mut(gid).unwrap();
            g.description = "desc".to_owned();
            g.color = Some(crate::track::TrackColor::CYAN);
            g.try_insert_node(
                NoteModuleId::new(1),
                NoteModuleConfig::Euclidean(crate::note_graph::EuclideanGenerator::default()),
            )
            .unwrap();
            g.try_insert_node(
                NoteModuleId::new(2),
                NoteModuleConfig::ProbabilityGate(crate::note_graph::ProbabilityGate::default()),
            )
            .unwrap();
            g.try_connect(crate::note_graph::NoteConnection::stream(
                NoteModuleId::new(1),
                NoteModuleId::new(2),
            ))
            .unwrap();
            g.node_positions.insert(
                NoteModuleId::new(1),
                crate::note_graph::NodePosition { x: 64.0, y: 32.0 },
            );
            g.node_descriptions
                .insert(NoteModuleId::new(1), "Pulse source".to_owned());
        }

        let clone = song.duplicate_note_graph(gid).expect("source exists");
        let src = song.note_graph(gid).unwrap();

        assert_ne!(clone.id, src.id);
        assert_eq!(clone.name, "src copy");
        assert_eq!(clone.description, src.description);
        assert_eq!(clone.color, src.color);
        assert_eq!(clone.nodes, src.nodes);
        assert_eq!(clone.connections, src.connections);
        assert_eq!(clone.node_positions, src.node_positions);
        assert_eq!(clone.node_descriptions, src.node_descriptions);
        assert_eq!(clone.processing_order, src.processing_order);
        assert!(song.duplicate_note_graph(NoteGraphId::new(999)).is_none());
    }

    #[test]
    fn mod_graph_pool_crud_bumps_generation() {
        use crate::automation::AutomationTarget;
        use crate::ids::ModNodeId;
        use crate::mod_grid::{ModGraphScope, ModNodeConfig, ModTarget, ModuleNode};
        let mut song = Song::new("t");
        assert_eq!(song.mod_grid_generation(), 0);

        let gid = song.create_mod_graph("wobble");
        assert!(song.mod_grid_generation() > 0);
        let after_create = song.mod_grid_generation();

        // A track to assign, plus a Track-scope switch.
        let t0 = song.create_track("lead");
        assert!(song.set_mod_graph_scope(gid, ModGraphScope::Track));
        assert!(song.assign_mod_graph(gid, &[t0]));
        assert!(song.mod_grid_generation() > after_create);
        assert_eq!(song.mod_graph_usage(gid), 1);

        // Assigning an unknown track id is filtered out.
        assert!(song.assign_mod_graph(gid, &[t0, TrackId(9999)]));
        assert_eq!(song.mod_graph(gid).unwrap().assigned_tracks, vec![t0]);

        // Switching back to Global clears assignments and reports one instance.
        assert!(song.set_mod_graph_scope(gid, ModGraphScope::Global));
        assert!(song.mod_graph(gid).unwrap().assigned_tracks.is_empty());
        assert_eq!(song.mod_graph_usage(gid), 1);

        // Populate a node so serde has content to carry.
        {
            let g = song.mod_graph_mut(gid).unwrap();
            g.try_insert_node(
                ModNodeId::new(0),
                ModNodeConfig::Module(ModuleNode {
                    module_type: synth_core::ModuleType::Lfo,
                    params: Default::default(),
                    seed: None,
                }),
            )
            .unwrap();
            g.try_insert_node(
                ModNodeId::new(1),
                ModNodeConfig::Target(ModTarget {
                    target: AutomationTarget::Track {
                        track: None,
                        param: crate::automation::TrackParam::Volume,
                    },
                    amount: 0.5,
                    combine: crate::mod_grid::CombineMode::Add,
                }),
            )
            .unwrap();
        }

        // Round-trip through JSON: the generation counter is #[serde(skip)] and
        // resets to 0, but the pool data survives intact.
        let json = serde_json::to_string(&song).unwrap();
        let mut restored: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mod_grid_generation(), 0);
        assert_eq!(restored.mod_graph_pool().len(), 1);
        assert_eq!(restored.mod_graph(gid).unwrap().nodes.len(), 2);
        // next_mod_graph_id survives, so a fresh create doesn't collide with gid.
        let fresh = restored.create_mod_graph("second");
        assert_ne!(fresh, gid);

        assert!(song.remove_mod_graph(gid).is_some());
        assert!(song.mod_graph(gid).is_none());
        assert_eq!(song.mod_graph_usage(gid), 0);
    }

    #[test]
    fn rebuild_mod_graphs_sanitizes_corrupt_cables() {
        use crate::ids::ModNodeId;
        use crate::mod_grid::{ModConnection, ModNodeConfig, ModuleNode};
        let mut song = Song::new("t");
        let gid = song.create_mod_graph("g");
        {
            let g = song.mod_graph_mut(gid).unwrap();
            for i in 0..2 {
                g.try_insert_node(
                    ModNodeId::new(i),
                    ModNodeConfig::Module(ModuleNode {
                        module_type: synth_core::ModuleType::Lfo,
                        params: Default::default(),
                        seed: None,
                    }),
                )
                .unwrap();
            }
            // Inject a cyclic cable set directly (bypassing try_connect), as a
            // corrupt save would carry. validate() would reject it.
            g.connections = vec![
                ModConnection::new(ModNodeId::new(0), "out", ModNodeId::new(1), "rate_cv"),
                ModConnection::new(ModNodeId::new(1), "out", ModNodeId::new(0), "rate_cv"),
            ];
            assert!(g.validate().is_err());
        }
        song.rebuild_mod_graphs();
        // One edge of the cycle is dropped so the graph validates again.
        let g = song.mod_graph(gid).unwrap();
        assert!(g.validate().is_ok());
        assert_eq!(g.connections.len(), 1);
    }
}
