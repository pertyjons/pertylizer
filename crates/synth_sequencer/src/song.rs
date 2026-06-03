//! Song structure and arrangement.

use serde::{Deserialize, Serialize};

use super::ids::{PatternId, ReturnBusId, SeqInstrumentId, TrackId};
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
        }
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
        id
    }

    /// Get a pattern by ID.
    #[must_use]
    pub fn pattern(&self, id: PatternId) -> Option<&Pattern> {
        self.patterns.iter().find(|p| p.id == id)
    }

    /// Get a mutable pattern by ID.
    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut Pattern> {
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
        (SeqInstrumentId, synth_core::ModuleType, u16),
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
        instrument: SeqInstrumentId,
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

        // Copy automation lanes and the note-processor rack
        new_pattern.automation = source.automation.clone();
        new_pattern.processors = source.processors.clone();

        self.patterns.push(new_pattern);
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
        // Also remove placements on this track
        self.arrangement.retain(|p| p.track_id != id);
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
        for p in &mut self.arrangement {
            if p.pattern_id == pattern_id && p.track_id == track_id && p.start == start {
                p.length_override = length;
                return true;
            }
        }
        false
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

    /// Set tempo at a position.
    pub fn set_tempo_at(&mut self, tick: Tick, bpm: Bpm) {
        // Remove existing at same tick
        self.tempo_changes.retain(|t| t.tick != tick);

        let change = TempoChange { tick, bpm };
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        self.tempo_changes.insert(pos, change);
    }

    /// Get tempo at a position.
    #[must_use]
    pub fn tempo_at(&self, tick: Tick) -> Bpm {
        let pos = self.tempo_changes.partition_point(|t| t.tick <= tick);
        if pos > 0 {
            self.tempo_changes[pos - 1].bpm
        } else {
            self.default_tempo
        }
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

    /// Convert tick to seconds (handles tempo changes).
    #[must_use]
    pub fn tick_to_seconds(&self, target: Tick) -> f64 {
        let mut seconds = 0.0;
        let mut current_tick = Tick(0);
        let mut current_tempo = self.default_tempo;

        for change in &self.tempo_changes {
            if change.tick >= target {
                break;
            }

            // Time to this tempo change
            let ticks = change.tick.0 - current_tick.0;
            let beats = ticks as f64 / TICKS_PER_QUARTER as f64;
            seconds += beats * 60.0 / f64::from(current_tempo.as_f32());

            current_tick = change.tick;
            current_tempo = change.bpm;
        }

        // Remaining ticks
        let remaining_ticks = target.0 - current_tick.0;
        let remaining_beats = remaining_ticks as f64 / TICKS_PER_QUARTER as f64;
        seconds += remaining_beats * 60.0 / f64::from(current_tempo.as_f32());

        seconds
    }

    /// Convert seconds to tick (handles tempo changes).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn seconds_to_tick(&self, target_seconds: f64) -> Tick {
        let mut seconds = 0.0;
        let mut current_tick = Tick(0);
        let mut current_tempo = self.default_tempo;

        for change in &self.tempo_changes {
            let ticks = change.tick.0 - current_tick.0;
            let beats = ticks as f64 / TICKS_PER_QUARTER as f64;
            let tempo = f64::from(current_tempo.as_f32());
            let segment_seconds = beats * 60.0 / tempo;

            if seconds + segment_seconds >= target_seconds {
                // Target is in this segment
                let remaining_seconds = target_seconds - seconds;
                let remaining_beats = remaining_seconds * tempo / 60.0;
                let remaining_ticks = (remaining_beats * TICKS_PER_QUARTER as f64) as u64;
                return Tick(current_tick.0 + remaining_ticks);
            }

            seconds += segment_seconds;
            current_tick = change.tick;
            current_tempo = change.bpm;
        }

        // Target is after all tempo changes
        let tempo = f64::from(current_tempo.as_f32());
        let remaining_seconds = target_seconds - seconds;
        let remaining_beats = remaining_seconds * tempo / 60.0;
        let remaining_ticks = (remaining_beats * TICKS_PER_QUARTER as f64) as u64;
        Tick(current_tick.0 + remaining_ticks)
    }

    /// Remove patterns and tracks not referenced by any arrangement placement.
    /// Returns `(removed_pattern_names, removed_track_names, used_instrument_ids)`.
    /// The caller is responsible for removing unused instruments from the engine.
    pub fn remove_unused(
        &mut self,
    ) -> (
        Vec<String>,
        Vec<String>,
        std::collections::HashSet<SeqInstrumentId>,
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
    fn test_calculate_length() {
        let mut song = Song::new("Test");

        let pattern_id = song.create_pattern(Duration(3840));
        let track_id = song.create_track("Track");

        song.place_pattern(pattern_id, track_id, Tick(0));
        song.place_pattern(pattern_id, track_id, Tick(3840));

        assert_eq!(song.calculate_length().0, 7680);
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
            instrument: SeqInstrumentId::new(2),
            module_type: ModuleType::Filter,
            instance: 1,
            param_id: "cutoff".into(),
        };

        // An empty lane (picker artifact) is not counted as a reference.
        song.pattern_mut(pid)
            .unwrap()
            .get_or_create_automation(target.clone());
        assert!(!song.is_module_automated(SeqInstrumentId::new(2), ModuleType::Filter, 1));
        assert!(song.automated_module_params().is_empty());

        // A point makes it a real reference.
        song.pattern_mut(pid)
            .unwrap()
            .get_or_create_automation(target)
            .add_point(AutomationPoint::new(
                PatternTick(0),
                NormalizedValue::new(0.5),
            ));

        assert!(song.is_module_automated(SeqInstrumentId::new(2), ModuleType::Filter, 1));
        // Different instance / instrument are not automated.
        assert!(!song.is_module_automated(SeqInstrumentId::new(2), ModuleType::Filter, 2));
        assert!(!song.is_module_automated(SeqInstrumentId::new(9), ModuleType::Filter, 1));

        let index = song.automated_module_params();
        let params = index
            .get(&(SeqInstrumentId::new(2), ModuleType::Filter, 1))
            .expect("filter instance 1 must be indexed");
        assert!(params.contains("cutoff"));
    }
}
