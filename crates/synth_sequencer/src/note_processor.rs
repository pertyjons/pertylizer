//! Note processors — the generative articulation layer (Model B).
//!
//! A pattern owns an ordered *processor rack* (`Vec<NoteProcessor>`) next to its
//! notes and automation. At playback time the rack **expands the stored source
//! notes into the actual played notes** ([`Pattern::expand_at_tick`]); the
//! source pattern is never mutated. A one-shot
//! [`Pattern::freeze_processors`] bake (Model A on demand) replaces the source
//! notes with the expansion for hand-editing.
//!
//! ## Chaining order (LOCKED)
//!
//! `scale-quantize → chord → arp → ornaments/strum → humanize`
//! (quantize first so everything downstream stays in key; humanize last so it
//! perturbs the final result). [`Pattern::add_processor`] inserts at the
//! canonical stage position ([`NoteProcessor::chain_stage`]), stable among
//! same-stage processors. The engine executes the rack in `Vec` order.
//!
//! ## RT-safety contract
//!
//! [`Pattern::expand_at_tick`] runs on the audio thread (inside
//! `SequencerEngine::collect_events_at_tick`): no heap allocation, no locks, no
//! unbounded loops. Output goes through the pre-allocated, hard-capped
//! [`ExpansionBuffer`]; on overflow the newest events are **dropped and
//! counted** ([`ExpansionBuffer::dropped`]) — never reallocated. Reporting the
//! drop count is the (non-RT) caller's job; the audio thread must not log.

use serde::{Deserialize, Serialize};

use super::ids::NoteId;
use super::note::{Glide, Note, NoteExpression};
use super::pattern::Pattern;
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};

/// Hard cap on expanded note events per tick (per pattern/placement).
///
/// The overflow policy: drop the newest events, count them, never grow the
/// buffer. A pathological config (e.g. a 1 ms roll) hits this instead of
/// allocating on the audio thread.
pub const MAX_EXPANSION_EVENTS_PER_TICK: usize = 128;

// ============================================================================
// Scale primitives (used by the ScaleQuantize processor)
// ============================================================================

/// A pitch class (0–11, C = 0). Constructor wraps modulo 12.
#[must_use]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct PitchClass(u8);

impl PitchClass {
    /// Create a pitch class; values ≥ 12 wrap modulo 12.
    pub fn new(class: u8) -> Self {
        Self(class % 12)
    }

    /// The class as 0–11. Serde can construct the inner value directly from
    /// hand-crafted JSON, so this is the single defensive normalization point —
    /// callers never re-apply `% 12`.
    #[must_use]
    pub fn as_u8(self) -> u8 {
        self.0 % 12
    }
}

/// A 12-bit pitch-class set relative to a root (bit 0 = the root itself).
///
/// The persisted, RT-cheap scale representation: membership is one shift and
/// mask, no name lookup. Name → mask mapping for GUI/MCP lives at those
/// boundaries (the app crate's harmony tables cannot be a dependency here).
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ScaleMask(u16);

impl ScaleMask {
    /// All 12 pitch classes (quantize becomes a no-op).
    pub const CHROMATIC: Self = Self::from_intervals(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    /// Major (ionian).
    pub const MAJOR: Self = Self::from_intervals(&[0, 2, 4, 5, 7, 9, 11]);
    /// Natural minor (aeolian).
    pub const NATURAL_MINOR: Self = Self::from_intervals(&[0, 2, 3, 5, 7, 8, 10]);
    /// Harmonic minor.
    pub const HARMONIC_MINOR: Self = Self::from_intervals(&[0, 2, 3, 5, 7, 8, 11]);
    /// Major pentatonic.
    pub const PENTATONIC_MAJOR: Self = Self::from_intervals(&[0, 2, 4, 7, 9]);
    /// Minor pentatonic.
    pub const PENTATONIC_MINOR: Self = Self::from_intervals(&[0, 3, 5, 7, 10]);

    /// Build a mask from semitone intervals above the root (values wrap mod 12).
    pub const fn from_intervals(intervals: &[u8]) -> Self {
        let mut mask = 0u16;
        let mut i = 0;
        while i < intervals.len() {
            mask |= 1 << (intervals[i] % 12);
            i += 1;
        }
        Self(mask)
    }

    /// Whether the interval-class above the root (0–11) is in the set.
    #[must_use]
    pub fn contains_interval(self, interval: u8) -> bool {
        self.0 & (1 << (interval % 12)) != 0
    }

    /// True when no pitch class is set (the degenerate mask; quantize treats
    /// it as a no-op rather than searching forever).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 & 0x0FFF == 0
    }
}

impl Default for ScaleMask {
    fn default() -> Self {
        Self::MAJOR
    }
}

// ============================================================================
// Processor configs
// ============================================================================

/// Scale-quantize: snap every pitch to the nearest member of `root` + `mask`.
///
/// First stage of the locked chain so randomized / interval-built notes from
/// downstream generators stay in key. Equidistant candidates prefer the upper
/// pitch (the melody side of dyads, matching the app-side scale snapper).
#[must_use]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub struct ScaleQuantize {
    /// Tonic pitch class.
    pub root: PitchClass,
    /// Scale membership relative to the root.
    pub mask: ScaleMask,
}

impl ScaleQuantize {
    /// Whether `pitch` is in the scale.
    #[must_use]
    pub fn contains(&self, pitch: Pitch) -> bool {
        let interval = (u16::from(pitch.as_midi()) + 12 - u16::from(self.root.as_u8())) % 12;
        #[allow(clippy::cast_possible_truncation)]
        self.mask.contains_interval(interval as u8)
    }

    /// Snap `pitch` to the nearest in-scale pitch, searching ±6 semitones
    /// (a non-empty scale guarantees a member within a tritone). Ties prefer
    /// the upper pitch. Empty masks and unreachable edges return `pitch`.
    pub fn snap(&self, pitch: Pitch) -> Pitch {
        if self.mask.is_empty() || self.contains(pitch) {
            return pitch;
        }
        let midi = i16::from(pitch.as_midi());
        for delta in 1..=6_i16 {
            for candidate in [midi + delta, midi - delta] {
                if let Ok(m) = u8::try_from(candidate)
                    && let Some(p) = Pitch::new(m)
                    && self.contains(p)
                {
                    return p;
                }
            }
        }
        pitch
    }
}

// ============================================================================
// The processor rack
// ============================================================================

/// One processor in a pattern's rack — pure *config* (Model B), never baked
/// output. Serialized with the pattern, so it round-trips save/load with the
/// same path as notes and automation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum NoteProcessor {
    /// Snap pitches to a scale (chain stage 0).
    ScaleQuantize(ScaleQuantize),
}

impl NoteProcessor {
    /// Canonical chain stage of this processor kind (LOCKED order):
    /// scale-quantize = 0, chord = 1, arp = 2, ornaments/strum = 3,
    /// humanize = 4. [`Pattern::add_processor`] inserts by this key.
    #[must_use]
    pub fn chain_stage(&self) -> u8 {
        match self {
            Self::ScaleQuantize(_) => 0,
        }
    }

    /// Per-tick expansion step: transform/extend the notes in `buf` (the
    /// upstream chain's output at `tick`). `pattern` provides read-only source
    /// context (e.g. held notes for the arpeggiator). RT-safe: no allocation,
    /// bounded by `buf`'s cap.
    ///
    /// **Contract for future processor kinds:** the engine invokes the rack on
    /// *every* tick of every active placement, so generators may emit on ticks
    /// where no source note starts (arp steps, ornament repeats). Processors
    /// hold no cross-tick mutable state — each must be a pure, deterministic
    /// function of *(config, pattern, tick)* (step phase derived from `tick`,
    /// held chord derived from `pattern`, randomness via a seeded hash of
    /// stable ids). That is what keeps the audio path lock-free and offline
    /// renders reproducible.
    fn process_at_tick(&self, _pattern: &Pattern, _tick: PatternTick, buf: &mut ExpansionBuffer) {
        match self {
            Self::ScaleQuantize(q) => {
                for note in buf.notes_mut() {
                    note.pitch = q.snap(note.pitch);
                }
            }
        }
    }
}

/// A note produced by per-tick expansion, in pattern space (no placement
/// transpose applied). `Copy`/alloc-free so it lives in the pre-allocated
/// [`ExpansionBuffer`]. The start tick is implicit — everything in the buffer
/// starts at the tick that was expanded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExpandedNote {
    /// Duration in ticks (`None` = plays until cut), pre gate-shaping.
    pub duration: Option<Duration>,
    /// Pitch (pattern space; placement transpose is the engine's job).
    pub pitch: Pitch,
    /// Velocity, pre accent/ghost shaping.
    pub velocity: Velocity,
    /// Tie/legato intent (taxonomy primitive 2).
    pub legato: bool,
    /// Per-note glide.
    pub glide: Option<Glide>,
    /// Per-note expression block.
    pub expression: Option<NoteExpression>,
}

impl ExpandedNote {
    fn from_note(note: &Note) -> Self {
        Self {
            duration: note.duration,
            pitch: note.pitch,
            velocity: note.velocity,
            legato: note.legato,
            glide: note.glide,
            expression: note.expression,
        }
    }
}

/// Pre-allocated, hard-capped output sink for per-tick expansion.
///
/// Allocates its full capacity ([`MAX_EXPANSION_EVENTS_PER_TICK`]) once at
/// construction and never grows. Overflow drops the newest event and counts it.
#[derive(Debug)]
pub struct ExpansionBuffer {
    notes: Vec<ExpandedNote>,
    dropped: u32,
}

impl ExpansionBuffer {
    /// Create a buffer with the standard per-tick capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            notes: Vec::with_capacity(MAX_EXPANSION_EVENTS_PER_TICK),
            dropped: 0,
        }
    }

    /// Clear contents and the drop counter (reuses the allocation).
    pub fn clear(&mut self) {
        self.notes.clear();
        self.dropped = 0;
    }

    /// Push a note; on overflow, drop it and count (never reallocate).
    /// Returns whether the note was accepted.
    pub fn push(&mut self, note: ExpandedNote) -> bool {
        if self.notes.len() < self.notes.capacity() {
            self.notes.push(note);
            true
        } else {
            self.dropped = self.dropped.saturating_add(1);
            false
        }
    }

    /// The expanded notes collected so far.
    #[must_use]
    pub fn notes(&self) -> &[ExpandedNote] {
        &self.notes
    }

    /// Mutable view for in-place transforms (scale-quantize, humanize).
    pub fn notes_mut(&mut self) -> &mut [ExpandedNote] {
        &mut self.notes
    }

    /// Events dropped by the overflow policy since the last [`Self::clear`].
    #[must_use]
    pub fn dropped(&self) -> u32 {
        self.dropped
    }
}

impl Default for ExpansionBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Pattern integration: rack accessors, the expansion point, freeze
// ============================================================================

impl Pattern {
    /// The processor rack, in execution order.
    #[must_use]
    pub fn processors(&self) -> &[NoteProcessor] {
        &self.processors
    }

    /// Add a processor at its canonical chain position (stable among
    /// same-stage processors). Returns the insertion index.
    pub fn add_processor(&mut self, processor: NoteProcessor) -> usize {
        let stage = processor.chain_stage();
        let pos = self
            .processors
            .partition_point(|p| p.chain_stage() <= stage);
        self.processors.insert(pos, processor);
        pos
    }

    /// Remove the processor at `index`.
    pub fn remove_processor(&mut self, index: usize) -> Option<NoteProcessor> {
        (index < self.processors.len()).then(|| self.processors.remove(index))
    }

    /// Model-B playback-time expansion at one tick: collect the source notes
    /// that start at `tick` and pass `gate` (the engine's per-note probability
    /// roll; preview passes `|_| true`), then run the rack in order. The result
    /// lands in `buf` (cleared first).
    ///
    /// **Audio-thread hot path** — no allocation, bounded by `buf`'s cap.
    pub fn expand_at_tick(
        &self,
        tick: PatternTick,
        gate: impl Fn(&Note) -> bool,
        buf: &mut ExpansionBuffer,
    ) {
        buf.clear();
        for note in self.notes() {
            if note.start == tick && gate(note) {
                let _ = buf.push(ExpandedNote::from_note(note));
            }
        }
        for processor in &self.processors {
            processor.process_at_tick(self, tick, buf);
        }
    }

    /// One-shot Model-A bake: run the rack over every tick of the pattern,
    /// replace the source notes with the concrete expansion, and clear the
    /// rack. Returns the number of notes after the bake.
    ///
    /// UI-thread only (allocates; walks the full pattern length — generators
    /// may emit at ticks where no source note starts, so every tick is
    /// visited).
    pub fn freeze_processors(&mut self) -> usize {
        if self.processors.is_empty() {
            return self.note_count();
        }
        let mut buf = ExpansionBuffer::new();
        let mut frozen: Vec<(PatternTick, ExpandedNote)> = Vec::new();
        // Walk past `length` if any note starts beyond it (add_note does not
        // clamp), so an out-of-range source note survives the bake instead of
        // being silently dropped by the clear-and-reinsert below.
        let walk_end = self.length.0.max(
            self.notes()
                .last()
                .map_or(0, |n| n.start.0.saturating_add(1)),
        );
        for tick in 0..walk_end {
            let t = PatternTick(tick);
            self.expand_at_tick(t, |_| true, &mut buf);
            for expanded in buf.notes() {
                frozen.push((t, *expanded));
            }
        }
        self.clear_notes();
        for (start, e) in frozen {
            let mut note = Note::new(NoteId(0), start, e.pitch, e.velocity);
            note.duration = e.duration;
            note.legato = e.legato;
            note.glide = e.glide;
            note.expression = e.expression;
            let _ = self.insert_note(note);
        }
        self.processors.clear();
        self.note_count()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ids::PatternId;

    fn c_major() -> ScaleQuantize {
        ScaleQuantize {
            root: PitchClass::new(0),
            mask: ScaleMask::MAJOR,
        }
    }

    fn pattern_with_notes(pitches: &[u8]) -> Pattern {
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        for &m in pitches {
            let _ = p.add_note(PatternTick(0), Pitch::new(m).unwrap(), Velocity::MF);
        }
        p
    }

    #[test]
    fn scale_mask_membership() {
        assert!(ScaleMask::MAJOR.contains_interval(0));
        assert!(!ScaleMask::MAJOR.contains_interval(1));
        assert!(ScaleMask::MAJOR.contains_interval(11));
        assert!(ScaleMask::CHROMATIC.contains_interval(6));
        assert!(ScaleMask::from_intervals(&[]).is_empty());
        assert!(!ScaleMask::MAJOR.is_empty());
        // Intervals wrap mod 12.
        assert!(ScaleMask::from_intervals(&[12]).contains_interval(0));
    }

    #[test]
    fn pitch_class_wraps() {
        assert_eq!(PitchClass::new(13).as_u8(), 1);
        assert_eq!(PitchClass::new(11).as_u8(), 11);
    }

    #[test]
    fn scale_quantize_snaps_to_nearest_preferring_up() {
        let q = c_major();
        // In-scale stays put.
        assert_eq!(q.snap(Pitch::new(60).unwrap()).as_midi(), 60);
        // C# is equidistant from C and D → prefer up → D.
        assert_eq!(q.snap(Pitch::new(61).unwrap()).as_midi(), 62);
        // F# is equidistant from F and G → prefer up → G.
        assert_eq!(q.snap(Pitch::new(66).unwrap()).as_midi(), 67);
    }

    #[test]
    fn scale_quantize_empty_mask_is_noop() {
        let q = ScaleQuantize {
            root: PitchClass::new(0),
            mask: ScaleMask::from_intervals(&[]),
        };
        assert_eq!(q.snap(Pitch::new(61).unwrap()).as_midi(), 61);
    }

    #[test]
    fn scale_quantize_nonzero_root() {
        // A natural minor: A B C D E F G — G# (68) snaps up to A (69).
        let q = ScaleQuantize {
            root: PitchClass::new(9),
            mask: ScaleMask::NATURAL_MINOR,
        };
        assert!(q.contains(Pitch::new(69).unwrap()));
        assert_eq!(q.snap(Pitch::new(68).unwrap()).as_midi(), 69);
    }

    #[test]
    fn expansion_buffer_caps_and_counts_drops() {
        let mut buf = ExpansionBuffer::new();
        let note = ExpandedNote {
            duration: None,
            pitch: Pitch::MIDDLE_C,
            velocity: Velocity::MF,
            legato: false,
            glide: None,
            expression: None,
        };
        for _ in 0..MAX_EXPANSION_EVENTS_PER_TICK {
            assert!(buf.push(note));
        }
        assert!(!buf.push(note));
        assert!(!buf.push(note));
        assert_eq!(buf.notes().len(), MAX_EXPANSION_EVENTS_PER_TICK);
        assert_eq!(buf.dropped(), 2);
        // Capacity must never have grown past the cap.
        assert_eq!(buf.notes.capacity(), MAX_EXPANSION_EVENTS_PER_TICK);
        buf.clear();
        assert_eq!(buf.dropped(), 0);
        assert!(buf.notes().is_empty());
    }

    #[test]
    fn add_processor_inserts_in_canonical_order_stably() {
        let mut p = pattern_with_notes(&[]);
        let a = NoteProcessor::ScaleQuantize(c_major());
        let b = NoteProcessor::ScaleQuantize(ScaleQuantize {
            root: PitchClass::new(2),
            mask: ScaleMask::NATURAL_MINOR,
        });
        // Same stage → insertion order preserved (stable).
        let _ = p.add_processor(a.clone());
        let idx = p.add_processor(b.clone());
        assert_eq!(idx, 1);
        assert_eq!(p.processors(), &[a, b]);
    }

    #[test]
    fn remove_processor_bounds() {
        let mut p = pattern_with_notes(&[]);
        assert!(p.remove_processor(0).is_none());
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        assert!(p.remove_processor(1).is_none());
        assert!(p.remove_processor(0).is_some());
        assert!(p.processors().is_empty());
    }

    #[test]
    fn expand_at_tick_collects_gates_and_quantizes() {
        let mut p = pattern_with_notes(&[60, 61, 66]);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let mut buf = ExpansionBuffer::new();

        p.expand_at_tick(PatternTick(0), |_| true, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 62, 67]);

        // The gate filters source notes before the rack runs.
        p.expand_at_tick(PatternTick(0), |n| n.pitch.as_midi() != 61, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 67]);

        // A tick with no starting notes expands to nothing (no generators yet).
        p.expand_at_tick(PatternTick(240), |_| true, &mut buf);
        assert!(buf.notes().is_empty());
    }

    #[test]
    fn expand_at_tick_without_rack_passes_through() {
        let p = pattern_with_notes(&[61]);
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, &mut buf);
        assert_eq!(buf.notes().len(), 1);
        assert_eq!(buf.notes()[0].pitch.as_midi(), 61);
    }

    #[test]
    fn freeze_processors_bakes_and_clears_rack() {
        let mut p = pattern_with_notes(&[60, 61, 66]);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));

        let count = p.freeze_processors();
        assert_eq!(count, 3);
        assert!(p.processors().is_empty());
        let pitches: Vec<u8> = p.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 62, 67]);
    }

    #[test]
    fn freeze_without_rack_is_noop() {
        let mut p = pattern_with_notes(&[61]);
        assert_eq!(p.freeze_processors(), 1);
        assert_eq!(p.notes()[0].pitch.as_midi(), 61);
    }

    #[test]
    fn freeze_keeps_notes_starting_beyond_pattern_length() {
        // add_note does not clamp start to the pattern length; the bake must
        // not silently drop an out-of-range note.
        let mut p = pattern_with_notes(&[61]);
        let _ = p.add_note(PatternTick(5000), Pitch::new(66).unwrap(), Velocity::MF);
        assert!(p.length.0 < 5000);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));

        assert_eq!(p.freeze_processors(), 2);
        let pitches: Vec<(u32, u8)> = p
            .notes()
            .iter()
            .map(|n| (n.start.0, n.pitch.as_midi()))
            .collect();
        assert_eq!(pitches, vec![(0, 62), (5000, 67)]);
    }

    #[test]
    fn pattern_with_only_a_rack_is_not_empty() {
        let mut p = pattern_with_notes(&[]);
        assert!(p.is_empty());
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        assert!(
            !p.is_empty(),
            "a rack is content — cleanup must not prune it"
        );
    }

    #[test]
    fn duplicate_pattern_carries_the_rack() {
        use crate::song::Song;
        let mut song = Song::new("Dup");
        let pattern_id = song.create_pattern(Duration(3840));
        if let Some(p) = song.pattern_mut(pattern_id) {
            let _ = p.add_note(PatternTick(0), Pitch::new(61).unwrap(), Velocity::MF);
            let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        }
        let copy_id = song.duplicate_pattern(pattern_id).unwrap();
        let copy = song.pattern(copy_id).unwrap();
        assert_eq!(
            copy.processors(),
            song.pattern(pattern_id).unwrap().processors(),
            "duplicating a pattern must copy its processor rack"
        );
    }

    #[test]
    fn pitch_class_normalizes_raw_overflow_via_as_u8() {
        // Serde can construct the inner value directly; as_u8 is the single
        // defensive normalization point.
        let raw: PitchClass = serde_json::from_str("13").unwrap();
        assert_eq!(raw.as_u8(), 1);
    }

    #[test]
    fn rack_serde_roundtrip_and_legacy_default() {
        let mut p = pattern_with_notes(&[60]);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let json = serde_json::to_string(&p).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.processors(), p.processors());

        // Legacy pattern JSON without the rack field loads with an empty rack.
        let legacy = r#"{
            "id": 0,
            "name": "",
            "length": 3840,
            "notes": [],
            "automation": [],
            "next_note_id": 0
        }"#;
        let pat: Pattern = serde_json::from_str(legacy).unwrap();
        assert!(pat.processors().is_empty());
    }
}
