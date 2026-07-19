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
use synth_core::hash::{splitmix64, splitmix64_bipolar};
use synth_core::{Bpm, NormalizedValue, Semitones};

use super::ids::NoteId;
use super::note::{
    Glide, Note, NoteExpression, Ornament, OrnamentDynamics, OrnamentPlacement, OrnamentSpacing,
};
use super::note_graph::{HostKey, NoteGraph, NoteScopeCtx};
use super::pattern::Pattern;
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick};

/// Hard cap on expanded note events per tick (per pattern/placement).
///
/// The overflow policy: drop the newest events, count them, never grow the
/// buffer. A pathological config (e.g. a 1 ms roll) hits this instead of
/// allocating on the audio thread.
pub const MAX_EXPANSION_EVENTS_PER_TICK: usize = 128;

/// Hard cap on the hits a single per-note ornament may emit (a roll's `count`).
///
/// Bounds the per-tick window scan and, with [`MAX_EXPANSION_EVENTS_PER_TICK`],
/// keeps a pathological roll from unbounded work on the audio thread. A
/// `count` above this clamps down; it never grows a buffer.
pub const MAX_ORNAMENT_HITS: u8 = 64;

/// Longest look-back window (ticks) a Note Grid timing module (delay / echo /
/// ratchet) may declare — 2 bars at 4/4, 960 PPQ. Bounds each per-tick scan and
/// the freeze walk-range tail extension (plan `note-grid.md` §11).
pub const MAX_NOTE_DELAY_TICKS: u32 = 7680;

/// Hard recursion cap for a timing node reading *downstream of a transform*: the
/// length of the look-back scratch pool, i.e. how many timing nodes may nest in
/// series before the innermost degrades to pass-through (delay-behind-a-delay
/// termination, plan `note-grid.md` §11).
///
/// Deliberately small: per-tick look-back cost is `O(repeats^depth × spine)`, so
/// this exponent is the dominant term. `2` allows one level of nesting (a delay
/// echoing another delay's output) while keeping the worst case
/// (`MAX_DELAY_REPEATS^2`) an RT-safe constant.
pub const MAX_LOOKBACK_DEPTH: usize = 2;

/// Repeat clamp for a single delay/echo node (further bounded by the
/// [`MAX_EXPANSION_EVENTS_PER_TICK`] buffer cap). Kept modest because it is the
/// base of the `repeats^depth` look-back cost (see [`MAX_LOOKBACK_DEPTH`]).
pub const MAX_DELAY_REPEATS: u8 = 16;

/// Subdivision clamp for a single ratchet node (further bounded by the
/// [`MAX_EXPANSION_EVENTS_PER_TICK`] buffer cap and the same `n^depth` cost as
/// [`MAX_DELAY_REPEATS`]).
pub const MAX_RATCHET_SUBDIVISIONS: u8 = 16;

/// Tighter look-back window for a `StreamOnset` [`crate::NoteEnvelope`]'s
/// backward scan (1 beat at 960 PPQ). The scan re-runs the whole spine per probe
/// and only early-exits at the first onset, so an idle/blocked stream would
/// otherwise scan the full [`MAX_NOTE_DELAY_TICKS`] every tick. `StreamOnset`
/// envelopes retrigger per transformed onset (typically far closer than a beat),
/// so this bound is musically ample and keeps the worst case RT-safe.
pub const MAX_ENV_STREAM_WINDOW: u32 = 960;

/// A fresh timing look-back scratch pool: [`MAX_LOOKBACK_DEPTH`] pre-allocated
/// buffers, the recursion budget [`crate::NoteGraph::expand_at_tick`] takes as
/// its `lookback` argument. Allocate once and reuse — an engine field on the
/// audio thread, a local on non-RT paths (freeze / tracker / ghosts).
#[must_use]
pub fn lookback_pool() -> [ExpansionBuffer; MAX_LOOKBACK_DEPTH] {
    std::array::from_fn(|_| ExpansionBuffer::new())
}

/// Velocity-curve floor: a crescendo/decrescendo ornament ramps between this
/// fraction of the source velocity and the full source velocity.
const ORNAMENT_VEL_FLOOR: f32 = 0.35;

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
#[serde(default, deny_unknown_fields)]
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

/// Most tones a chord generator emits per source note. A source note expands
/// into at most this many pitches; excess intervals are ignored. Bounds the
/// audio-thread fan-out and the held-view buffer.
pub const MAX_CHORD_INTERVALS: usize = 8;

/// Order in which a strummed chord's tones are struck.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum StrumDirection {
    /// Lowest pitch first (an upstroke).
    #[default]
    Up,
    /// Highest pitch first (a downstroke).
    Down,
}

/// Chord generator (chain stage 1) — expand one source note into a chord.
///
/// Each source-note onset is **replaced** by the pitches at the source pitch
/// plus each stored interval (semitones; `0` keeps the root, negatives voice
/// below it). The classic chain is `chord → arp`: the arpeggiator reads the
/// generated tones through the widened held-notes seam ([`NoteProcessor::
/// expand_pitch`]), so one source note + chord + arp arpeggiates the full
/// chord. Intervals are stored inline (no heap) and capped at
/// [`MAX_CHORD_INTERVALS`].
///
/// **Strum:** with `strum > 0` the tones are struck one at a time, `strum`
/// ticks apart, ordered by `direction` (a block chord at `strum = 0`). Strum
/// shapes *onset timing*, so it only affects a standalone chord — a downstream
/// arp re-times everything and ignores it. A strummed chord *replaces* the
/// source stream (it scans the source notes by onset, like the arp), so — like
/// the arp — it bypasses per-note trigger probability; the block path respects
/// it (it transforms the already-gated buffer).
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Chord {
    /// Semitone offsets from the source pitch; only the first `count` are live.
    intervals: [i8; MAX_CHORD_INTERVALS],
    /// Number of live intervals (clamped to [`MAX_CHORD_INTERVALS`]).
    count: u8,
    /// Onset spread between adjacent tones (`0` = a simultaneous block chord).
    #[serde(default)]
    strum: Duration,
    /// Tone order when strumming.
    #[serde(default)]
    direction: StrumDirection,
}

impl Chord {
    /// Build a chord from semitone intervals above (or below) the source pitch.
    /// Excess beyond [`MAX_CHORD_INTERVALS`] is dropped. An empty set silences
    /// the source note (no tones) — callers wanting the root must include `0`.
    pub fn new(intervals: &[i8]) -> Self {
        let mut stored = [0_i8; MAX_CHORD_INTERVALS];
        let count = intervals.len().min(MAX_CHORD_INTERVALS);
        stored[..count].copy_from_slice(&intervals[..count]);
        #[allow(clippy::cast_possible_truncation)]
        Self {
            intervals: stored,
            count: count as u8,
            strum: Duration(0),
            direction: StrumDirection::Up,
        }
    }

    /// Strum the chord: tones struck `spread` ticks apart in `direction`
    /// (builder). `spread = 0` restores a block chord.
    pub fn with_strum(mut self, spread: Duration, direction: StrumDirection) -> Self {
        self.strum = spread;
        self.direction = direction;
        self
    }

    /// Tick span from the first strummed tone to the last (`0` for a block
    /// chord). Used to bound the strum scan window and extend the freeze walk
    /// (also from the Note Grid via [`NoteGraph::max_strum_tail`]).
    #[must_use]
    pub(crate) fn strum_tail(&self) -> u32 {
        let live = self.intervals().len() as u32;
        // `strum` is unbounded (serde); saturate so a pathological spread can't
        // overflow the scan window / freeze walk (matches the module's i64/
        // saturating discipline elsewhere).
        live.saturating_sub(1).saturating_mul(self.strum.0)
    }

    /// Major triad (root, +4, +7).
    pub fn major() -> Self {
        Self::new(&[0, 4, 7])
    }

    /// Minor triad (root, +3, +7).
    pub fn minor() -> Self {
        Self::new(&[0, 3, 7])
    }

    /// Dominant 7th (root, +4, +7, +10).
    pub fn dominant7() -> Self {
        Self::new(&[0, 4, 7, 10])
    }

    /// The live intervals (the single normalization point — `count` is clamped
    /// here so a hand-crafted JSON `count` can't read past the array).
    #[must_use]
    pub fn intervals(&self) -> &[i8] {
        &self.intervals[..(self.count as usize).min(MAX_CHORD_INTERVALS)]
    }

    /// Onset spread between adjacent tones (`0` = a simultaneous block chord).
    pub fn strum(&self) -> Duration {
        self.strum
    }

    /// Tone order when strumming.
    pub fn direction(&self) -> StrumDirection {
        self.direction
    }
}

impl Default for Chord {
    fn default() -> Self {
        Self::major()
    }
}

/// Arpeggiator step order over the held chord.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum ArpMode {
    /// Ascending pitch order.
    #[default]
    Up,
    /// Descending pitch order.
    Down,
    /// Ascending then descending, endpoints not repeated.
    UpDown,
    /// Descending then ascending, endpoints not repeated.
    DownUp,
    /// In the order the source notes start (scan order).
    AsPlayed,
    /// Deterministic pseudo-random member per step (seeded by step index, so a
    /// given playback position always picks the same note — reproducible
    /// offline renders).
    Random,
    /// The whole chord re-triggered every step (octave cycles per step).
    Chord,
    /// A fixed offset loop ([`Arpeggiator::custom`]) applied to the *single*
    /// source note's own pitch (offset 0 = the note), tracking a moving melody.
    /// The chiptune / SID arp: one moving note, one held gate, pitch cycles.
    Custom,
}

/// Tempo-synced arpeggiator rate, as a division of the 960-PPQN grid.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum ArpRate {
    /// 1/4 note (960 ticks).
    Quarter,
    /// Dotted 1/4 (1440 ticks).
    QuarterDotted,
    /// 1/4 triplet (640 ticks).
    QuarterTriplet,
    /// 1/8 note (480 ticks).
    Eighth,
    /// Dotted 1/8 (720 ticks).
    EighthDotted,
    /// 1/8 triplet (320 ticks).
    EighthTriplet,
    /// 1/16 note (240 ticks).
    #[default]
    Sixteenth,
    /// Dotted 1/16 (360 ticks).
    SixteenthDotted,
    /// 1/16 triplet (160 ticks).
    SixteenthTriplet,
    /// 1/32 note (120 ticks).
    ThirtySecond,
    /// Dotted 1/32 (180 ticks).
    ThirtySecondDotted,
    /// 1/32 triplet (80 ticks).
    ThirtySecondTriplet,
    /// Exact step length in ticks (sub-grid; e.g. 40 = one PAL frame @125 BPM).
    /// Clears the divided-grid floor for chiptune / SID arps. Clamped to ≥1.
    Ticks(u32),
    /// Frame-/tempo-independent step rate in **millihertz** (integer, so
    /// `ArpRate` keeps `Eq`/`Hash`): 50_000 = 50 Hz PAL, 60_000 = 60 Hz NTSC.
    /// The tick length is tempo-dependent and resolved against the live BPM in
    /// [`Arpeggiator::step_onset`] with drift-free absolute-time rounding.
    MilliHz(u32),
}

impl ArpRate {
    /// Step length in ticks. All named divisions are exact on the 960-PPQN grid
    /// and [`Self::Ticks`] is the raw sub-grid count (clamped to ≥1). For the
    /// tempo-dependent [`Self::MilliHz`] this is only a nominal length at the
    /// default tempo — the real per-step length is resolved against the live
    /// BPM in [`Arpeggiator::step_onset`]; this fallback is for UI/length hints.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn ticks(self) -> Duration {
        match self {
            Self::Quarter => Duration::QUARTER,
            Self::QuarterDotted => Duration::QUARTER.dotted(),
            Self::QuarterTriplet => Duration::TRIPLET_QUARTER,
            Self::Eighth => Duration::EIGHTH,
            Self::EighthDotted => Duration::EIGHTH.dotted(),
            Self::EighthTriplet => Duration::TRIPLET_EIGHTH,
            Self::Sixteenth => Duration::SIXTEENTH,
            Self::SixteenthDotted => Duration::SIXTEENTH.dotted(),
            Self::SixteenthTriplet => Duration::TRIPLET_SIXTEENTH,
            Self::ThirtySecond => Duration::THIRTY_SECOND,
            Self::ThirtySecondDotted => Duration::THIRTY_SECOND.dotted(),
            Self::ThirtySecondTriplet => Duration::TRIPLET_THIRTY_SECOND,
            Self::Ticks(n) => Duration(n.max(1)),
            Self::MilliHz(m) => {
                let len = (16_000.0 * Bpm::DEFAULT.as_f32() / m.max(1) as f32).round();
                Duration((len as u32).max(1))
            }
        }
    }
}

/// Most semitone offsets an [`ArpMode::Custom`] cycle holds. Chiptune arps are
/// short (≤ ~8 steps); the inline cap keeps [`Arpeggiator`] `Copy` and the
/// audio-thread scratch fixed.
pub const MAX_ARP_OFFSETS: usize = 16;

/// Bounded, inline list of semitone offsets for [`ArpMode::Custom`] (relative
/// to the source note's pitch; offset 0 = the note). Inline + `Copy` so
/// [`Arpeggiator`] stays `Copy`. Serializes as a bare JSON array (`[0,4,7]`).
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArpOffsets {
    /// Offset storage; only the first `len` entries are live.
    pub values: [i8; MAX_ARP_OFFSETS],
    /// Number of live offsets (≤ [`MAX_ARP_OFFSETS`]).
    pub len: u8,
}

impl ArpOffsets {
    /// Build from a slice, truncating past [`MAX_ARP_OFFSETS`].
    pub fn new(slice: &[i8]) -> Self {
        let mut values = [0; MAX_ARP_OFFSETS];
        let len = slice.len().min(MAX_ARP_OFFSETS);
        values[..len].copy_from_slice(&slice[..len]);
        Self {
            values,
            len: len as u8,
        }
    }

    /// The live offsets.
    #[must_use]
    pub fn as_slice(&self) -> &[i8] {
        &self.values[..self.len as usize]
    }

    /// Number of live offsets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the cycle is empty (a `Custom` arp with no offsets emits nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl serde::Serialize for ArpOffsets {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Bare JSON array, e.g. [0,4,7].
        self.as_slice().serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for ArpOffsets {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // UI/serde thread only; a transient Vec is fine here.
        Ok(Self::new(&Vec::<i8>::deserialize(d)?))
    }
}

impl schemars::JsonSchema for ArpOffsets {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ArpOffsets".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Identical surface to a plain i8 array.
        <Vec<i8>>::json_schema(generator)
    }
}

/// How the arpeggiator assigns step velocities.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum ArpVelocity {
    /// Each step uses its source note's velocity.
    #[default]
    AsPlayed,
    /// Crescendo across each cycle (scaled from the source velocity).
    RampUp,
    /// Decrescendo across each cycle.
    RampDown,
}

/// Most held notes the arpeggiator reads per tick. Excess notes (scan order =
/// start order) are ignored — a 16-note chord × 4 octaves already saturates a
/// cycle musically, and the bound keeps the audio-thread scratch array fixed.
pub const MAX_ARP_HELD: usize = 16;

/// Velocity-ramp floor: a cycle ramps between this fraction of the source
/// velocity and the full source velocity.
const ARP_RAMP_FLOOR: f32 = 0.4;

/// Arpeggiator (chain stage 2) — the flagship stream generator.
///
/// Replaces the source stream: at each step-grid onset it emits one member of
/// the currently *held* chord (every source note sounding at that tick, viewed
/// through the upstream chain's pitch transforms), and suppresses the source
/// chord onsets themselves. Step phase normally counts from the held chord's
/// earliest start; `continuous_phase` instead derives it from absolute pattern
/// time. Stateless and deterministic per the rack contract.
///
/// Trill / mordent / turn are constrained presets of this generator (NP3/NP6
/// expose them via the per-note ornament menu); they are not separate code.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Arpeggiator {
    /// Step order over the held chord.
    pub mode: ArpMode,
    /// Tempo-synced step length.
    pub rate: ArpRate,
    /// Octave range (1–4; values outside are clamped at use).
    pub octaves: u8,
    /// Step gate as a fraction of the step length (≥1 tick). A 1.0 gate on
    /// repeated pitches ties via the engine's boundary coalesce.
    pub gate: NormalizedValue,
    /// Swing: odd steps are delayed by `swing × step/2`.
    pub swing: NormalizedValue,
    /// Step velocity behavior.
    pub velocity: ArpVelocity,
    /// Hold the last chord through gaps (keeps arpeggiating after the source
    /// notes end, using the chord that sounded just before the gap).
    pub latch: bool,
    /// Hold one envelope across the whole figure (chip-style): each step is
    /// stamped `legato`, so the engine glides between steps instead of
    /// re-gating (one attack, pitch cycles). Needs a `gate` near 1.0 so steps
    /// meet end-to-end — a short gate leaves a hole the legato join can't span.
    pub legato: bool,
    /// Keep the step cycle aligned to absolute pattern time instead of
    /// restarting it at every new chord block.
    pub continuous_phase: bool,
    /// Semitone offsets cycled when `mode == Custom`, relative to the source
    /// note's own pitch (offset 0 = the note). `octaves` multiplies the cycle:
    /// `[0,4,7]` with `octaves == 2` plays `0,4,7, 12,16,19`. Ignored by the
    /// other modes.
    pub custom: ArpOffsets,
}

impl Default for Arpeggiator {
    fn default() -> Self {
        Self {
            mode: ArpMode::Up,
            rate: ArpRate::Sixteenth,
            octaves: 1,
            gate: NormalizedValue::new(0.75),
            swing: NormalizedValue::new(0.0),
            velocity: ArpVelocity::AsPlayed,
            latch: false,
            legato: false,
            continuous_phase: false,
            custom: ArpOffsets::default(),
        }
    }
}

/// One held source note, as the arpeggiator sees it (post upstream pitch
/// transforms). `Copy` so it lives in a fixed scratch array.
#[derive(Debug, Clone, Copy)]
struct HeldArpNote {
    pitch: Pitch,
    velocity: Velocity,
}

impl Arpeggiator {
    /// Phase-1 query (cheap, runs every tick): the source tick whose chord the
    /// arp plays at `tick`, plus the step anchor (the chord block's earliest
    /// start). No pitch mapping happens here — the upstream chain is only
    /// consulted on actual step onsets (phase 2).
    ///
    /// When nothing is held and `latch` is on, falls back to the tick just
    /// before the most recent note end — "hold the last chord", derived
    /// statelessly from the pattern.
    ///
    /// NB: membership is `is_playing_at` on the raw source notes — the arp
    /// *replaces* the stream, so per-note trigger probability (an onset-domain
    /// feature) deliberately has no effect under an arp. The anchor likewise
    /// reads raw source starts, which makes the locked chain order (humanize
    /// strictly after the arp) load-bearing: an upstream start-shifting
    /// transform could not move this grid.
    fn chord_source(&self, source: &[Note], tick: PatternTick) -> Option<(PatternTick, u32)> {
        if let Some(anchor) = Self::anchor_at(source, tick) {
            return Some((tick, anchor));
        }
        if self.latch {
            // Latest note end at or before `tick`; the chord playing on the
            // tick just before that end is the latched chord.
            let last_end = source
                .iter()
                .filter_map(Note::end)
                .filter(|e| *e <= tick)
                .max();
            if let Some(end) = last_end
                && end.0 > 0
            {
                let src_tick = PatternTick(end.0 - 1);
                if let Some(anchor) = Self::anchor_at(source, src_tick) {
                    return Some((src_tick, anchor));
                }
            }
        }
        None
    }

    /// Earliest start among the notes sounding at `tick` (`None` when silent).
    fn anchor_at(source: &[Note], tick: PatternTick) -> Option<u32> {
        source
            .iter()
            .filter(|n| n.is_playing_at(tick))
            .map(|n| n.start.0)
            .min()
    }

    /// Phase-2 (step onsets only): the chord at `tick`, viewed through the
    /// `upstream` chain (the 1→N seam — a chord generator expands each held
    /// note into its tones), in scan (start) order. Excess beyond
    /// [`MAX_ARP_HELD`] is truncated in scan order — *not* pitch order — so a
    /// cluster past 16 notes keeps its earliest-starting 16 (chord tones
    /// included) and Up/Down may miss the extremes.
    fn held_at(
        source: &[Note],
        tick: PatternTick,
        upstream: &[NoteProcessor],
        held: &mut [HeldArpNote; MAX_ARP_HELD],
    ) -> usize {
        let mut n = 0;
        for note in source {
            if !note.is_playing_at(tick) {
                continue;
            }
            if n == MAX_ARP_HELD {
                break;
            }
            // 1→N seam: a source note becomes its chord tones (snapped) when a
            // chord/quantize sits upstream, so `chord → arp` arpeggiates the
            // whole chord rather than the single source pitch.
            let velocity = note.velocity;
            expand_held_pitch(upstream, note.pitch, &mut |pitch| {
                if n < MAX_ARP_HELD {
                    held[n] = HeldArpNote { pitch, velocity };
                    n += 1;
                }
            });
        }
        n
    }

    /// Resolve whether `tick` is a step onset for an arp anchored at `anchor`,
    /// and if so the step index and its gated note duration (ticks).
    ///
    /// The named divisions and [`ArpRate::Ticks`] live on an integer grid, so
    /// the onset is an exact `step·len (+ swing)` and the divisor is whole. The
    /// frame-locked [`ArpRate::MilliHz`] rate has a *fractional* step length
    /// (e.g. 38.4 ticks at 50 Hz / 120 BPM), so it must round **absolute time**
    /// `round(step·len)` rather than accumulate a per-step integer (which would
    /// drift). Swing shifts odd steps off the plain grid, so the fractional path
    /// scans a ±1 window around the expected step to find the matching onset.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn step_onset(&self, anchor: u32, tick: PatternTick, bpm: Bpm) -> Option<(u32, u32)> {
        let rel = tick.0.saturating_sub(anchor);
        let gate = self.gate.as_f32();
        if let ArpRate::MilliHz(m_hz) = self.rate {
            // ticks/s = 960 PPQN·BPM/60 = 16·BPM; ÷ Hz = ÷(mHz/1000) ⇒ ×1000
            // folded in. Clamp BPM/Hz away from zero so the step length is finite,
            // and floor it at one tick: the integer grid cannot place two steps in
            // the same tick, so a sub-tick rate (Hz > 16·BPM, far above any frame
            // rate) would otherwise round consecutive steps onto one tick and skip
            // every other step. One step per tick is the finest the grid allows.
            let step_len = ((16_000.0 * bpm.as_f32().max(1.0)) / m_hz.max(1) as f32).max(1.0);
            let swing = self.swing.as_f32();
            let approx_step = (rel as f32 / step_len) as u32;
            let swing_for = |s: u32| {
                if s % 2 == 1 {
                    (step_len / 2.0 * swing).round() as i32
                } else {
                    0
                }
            };
            let onset_for = |s: u32| (s as f32 * step_len).round() as i32 + swing_for(s);
            // Scan ±1 around the expected step: swing can push an onset into the
            // neighbouring integer-step bucket.
            let step = (approx_step.saturating_sub(1)..=approx_step + 1).find(|&s| {
                let o = onset_for(s);
                o >= 0 && o as u32 == rel
            })?;
            let gap = (onset_for(step + 1) - onset_for(step)).max(1) as u32;
            let duration = ((step_len * gate).round() as u32).clamp(1, gap);
            return Some((step, duration));
        }

        // Integer-grid rates (named divisions + `Ticks`) — exact, unchanged.
        let step_len = self.rate.ticks().0;
        let step = rel / step_len;
        let swing_ticks = (step_len as f32 / 2.0 * self.swing.as_f32()) as u32;
        let onset = step * step_len + if step % 2 == 1 { swing_ticks } else { 0 };
        if rel != onset {
            return None;
        }
        // Step gate, clamped to the gap to the next (swung) onset so a long gate
        // on a swung grid can never overlap the following step — an overlap of
        // the same pitch would otherwise confuse the engine's NoteOff
        // bookkeeping (two active voices, first NoteOff cuts both).
        let gap = if step.is_multiple_of(2) {
            step_len + swing_ticks
        } else {
            step_len - swing_ticks
        };
        let duration = ((step_len as f32 * gate) as u32).clamp(1, gap.max(1));
        Some((step, duration))
    }

    /// Per-tick arp evaluation. Suppresses the upstream note stream and, when
    /// `tick` lands on a step onset (including the swung onset of odd steps),
    /// emits that step's chord member(s).
    ///
    /// The arp **replaces** the source stream: chord onsets never pass
    /// through, and per-note expression/glide on the source notes is not
    /// carried onto the generated steps. A consequence: two stream generators
    /// in one rack behave as "last one wins" — the later generator clears the
    /// earlier one's output and re-reads the source pattern.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    fn process_at_tick(
        &self,
        source: &[Note],
        tick: PatternTick,
        upstream: &[NoteProcessor],
        bpm: Bpm,
        host_salt: u64,
        buf: &mut ExpansionBuffer,
    ) {
        // The arp replaces the source stream: chord onsets never pass through.
        buf.clear_notes();

        // Phase 1 (every tick, cheap): locate the step grid. Phase counts from
        // the chord block's earliest start, so the figure restarts on each new
        // chord. Odd steps are swing-delayed.
        let Some((source_tick, anchor)) = self.chord_source(source, tick) else {
            return;
        };
        // Phase 1 also resolves the step index and its gated duration — both fall
        // out of the rate's onset grid (integer divisions for the named/`Ticks`
        // rates, drift-free absolute-time rounding for the frame-locked `MilliHz`
        // rate). `None` ⇒ `tick` is not a step onset for this arp.
        let phase_anchor = if self.continuous_phase { 0 } else { anchor };
        let Some((step, duration)) = self.step_onset(phase_anchor, tick, bpm) else {
            return;
        };

        // Custom bypasses the chord-collection model entirely: it cycles a fixed
        // offset list over the *single* source note's pitch (a moving melody),
        // not over a held chord.
        if self.mode == ArpMode::Custom {
            self.emit_custom(source, source_tick, anchor, upstream, (step, duration), buf);
            return;
        }

        // Phase 2 (step onsets only): resolve the chord through the upstream
        // chain and emit. The expensive per-note pitch mapping never runs on
        // the ~⌀239 of 240 non-onset ticks.
        const DUMMY: HeldArpNote = HeldArpNote {
            pitch: Pitch::MIDDLE_C,
            velocity: Velocity::MF,
        };
        let mut held = [DUMMY; MAX_ARP_HELD];
        let n = Self::held_at(source, source_tick, upstream, &mut held);
        if n == 0 {
            return;
        }
        let held = &mut held[..n];

        // Sort per mode (in-place on the fixed scratch — alloc-free).
        match self.mode {
            ArpMode::Up | ArpMode::UpDown => {
                held.sort_unstable_by_key(|h| h.pitch.as_midi());
            }
            ArpMode::Down | ArpMode::DownUp => {
                held.sort_unstable_by_key(|h| core::cmp::Reverse(h.pitch.as_midi()));
            }
            // Scan order is start order — exactly "as played". (`Custom` returns
            // earlier and never reaches the held-chord sort.)
            ArpMode::AsPlayed | ArpMode::Random | ArpMode::Chord | ArpMode::Custom => {}
        }

        let octaves = u32::from(self.octaves.clamp(1, 4));
        let n32 = n as u32;
        let total = n32 * octaves;

        if self.mode == ArpMode::Chord {
            // Whole chord per step; the octave cycles step-by-step, and the
            // velocity ramp spans that octave cycle.
            let oct = step % octaves;
            for h in held.iter() {
                if let Some(pitch) = h.pitch.transpose(Semitones::new(12.0 * oct as f32)) {
                    let velocity = self.step_velocity(h.velocity, oct, octaves);
                    let _ = buf.push(self.step_note(pitch, velocity, duration));
                }
            }
            return;
        }

        // Cycle position → member index. Up/Down/AsPlayed walk 0..total;
        // UpDown/DownUp mirror without repeating endpoints; Random hashes the
        // step index (deterministic).
        let cycle_len = match self.mode {
            ArpMode::UpDown | ArpMode::DownUp if total > 1 => 2 * total - 2,
            _ => total,
        };
        let pos = step % cycle_len;
        let idx = match self.mode {
            ArpMode::UpDown | ArpMode::DownUp if pos >= total => 2 * total - 2 - pos,
            ArpMode::Random => {
                (splitmix64(u64::from(step) ^ (u64::from(anchor) << 32) ^ host_salt)
                    % u64::from(total)) as u32
            }
            _ => pos,
        };

        let member = &held[(idx % n32) as usize];
        let octave = idx / n32;
        let Some(pitch) = member.pitch.transpose(Semitones::new(12.0 * octave as f32)) else {
            return; // transposed off the MIDI range — skip this step
        };
        let velocity = self.step_velocity(member.velocity, pos, cycle_len);
        let _ = buf.push(self.step_note(pitch, velocity, duration));
    }

    /// The source note that anchors a [`ArpMode::Custom`] cycle: among the notes
    /// sounding at `source_tick` whose start is the block anchor, the lowest
    /// pitch (a deterministic pick for the rare held-chord case; the intended
    /// use is a single moving note). `None` when nothing is held there.
    fn anchor_note(source: &[Note], source_tick: PatternTick, anchor: u32) -> Option<HeldArpNote> {
        source
            .iter()
            .filter(|n| n.is_playing_at(source_tick) && n.start.0 == anchor)
            .min_by_key(|n| n.pitch.as_midi())
            .map(|n| HeldArpNote {
                pitch: n.pitch,
                velocity: n.velocity,
            })
    }

    /// Emit one [`ArpMode::Custom`] step: cycle the offset list over the source
    /// note's pitch, with `octaves` multiplying the cycle
    /// (`offset(pos) = custom[pos % L] + 12·(pos / L)` over `L · octaves` steps).
    /// The root is viewed through the `upstream` chain (the 1→N seam) — an
    /// upstream scale-quantize snaps it, a chord contributes its root tone — so
    /// Custom composes with the rest of the rack like the other modes, while
    /// still arpeggiating a single moving line.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn emit_custom(
        &self,
        source: &[Note],
        source_tick: PatternTick,
        anchor: u32,
        upstream: &[NoteProcessor],
        onset: (u32, u32),
        buf: &mut ExpansionBuffer,
    ) {
        let (step, duration) = onset;
        let l = self.custom.len() as u32;
        if l == 0 {
            return; // an empty cycle has nothing to play
        }
        let Some(anchor_note) = Self::anchor_note(source, source_tick, anchor) else {
            return;
        };
        let root = upstream_pitch(upstream, anchor_note.pitch);
        let octaves = u32::from(self.octaves.clamp(1, 4));
        let cycle_len = l * octaves;
        let pos = step % cycle_len;
        let base = self.custom.as_slice()[(pos % l) as usize];
        let octave = pos / l;
        let semis = i32::from(base) + 12 * octave as i32;
        let Some(pitch) = root.transpose(Semitones::new(semis as f32)) else {
            return; // transposed off the MIDI range — skip this step
        };
        let velocity = self.step_velocity(anchor_note.velocity, pos, cycle_len);
        let _ = buf.push(self.step_note(pitch, velocity, duration));
    }

    /// Apply the velocity mode to one step (ramps span a cycle). A one-step
    /// cycle has nothing to ramp over and passes the source velocity through
    /// — without this, a degenerate ramp would pin every step to its floor.
    #[allow(clippy::cast_precision_loss)]
    fn step_velocity(&self, base: Velocity, pos: u32, cycle_len: u32) -> Velocity {
        if cycle_len <= 1 {
            return base;
        }
        let span = (cycle_len - 1) as f32;
        let t = pos as f32 / span;
        let factor = match self.velocity {
            ArpVelocity::AsPlayed => return base,
            ArpVelocity::RampUp => velocity_ramp(ARP_RAMP_FLOOR, t, true),
            ArpVelocity::RampDown => velocity_ramp(ARP_RAMP_FLOOR, t, false),
        };
        Velocity::new(base.as_f32() * factor)
    }

    fn step_note(&self, pitch: Pitch, velocity: Velocity, duration_ticks: u32) -> ExpandedNote {
        ExpandedNote {
            duration: Some(Duration(duration_ticks)),
            pitch,
            velocity,
            // Stamp each step `legato` so the engine's cross-pitch coalesce
            // (Phase B3) holds one envelope across the whole figure.
            legato: self.legato,
            glide: None,
            expression: None,
        }
    }
}

/// Humanize (chain stage 4, last) — bounded, **seeded** random perturbation of
/// the final note stream, so a sequenced part loses its machine-uniformity
/// without losing reproducibility.
///
/// A pure buf transform on whatever the rack produced (arp steps, chord tones,
/// plain notes). The per-note random is a deterministic hash of
/// *(seed, tick, pitch, buffer index)* — never `Math.random` on the audio
/// thread — so a given `seed` renders byte-identical every time (offline and
/// live), and changing `seed` rerolls the feel.
///
/// **Scope:** velocity and gate (note-length) only. *Timing* humanize is
/// deliberately not here — shifting a note's onset is a multi-tick / engine
/// micro-timing concern (a stage-4 buf transform can only see notes already
/// landed on the current tick, and the per-tick model cannot *rush* a note
/// earlier than where the chain placed it). *Micro-pitch* needs a sub-semitone
/// detune field on the emitted note. Both are tracked as follow-ups.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Humanize {
    /// Max velocity deviation as a ± fraction of the note's velocity (0 = off).
    pub velocity: NormalizedValue,
    /// Max note-length deviation as a ± fraction of each note's duration
    /// (0 = off; notes with no explicit duration are untouched).
    pub gate: NormalizedValue,
    /// PRNG seed for this processor. Same seed → identical render every time
    /// (offline and live). NB: the seed lives on the pattern, so every
    /// placement of the pattern humanizes identically — per-placement variation
    /// is the deferred per-`PatternPlacement` override (NP0), which would mix a
    /// placement nonce into the per-note key the way the engine's probability
    /// roll mixes `roll_nonce`.
    pub seed: u64,
}

impl Default for Humanize {
    fn default() -> Self {
        Self {
            velocity: NormalizedValue::new(0.15),
            gate: NormalizedValue::new(0.0),
            seed: 0,
        }
    }
}

impl Humanize {
    /// Perturb each note in `buf` in place. RT-safe: no allocation, the random
    /// draws are pure hashes.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    fn apply(&self, tick: PatternTick, buf: &mut ExpansionBuffer) {
        let vel_amt = self.velocity.as_f32();
        let gate_amt = self.gate.as_f32();
        if vel_amt == 0.0 && gate_amt == 0.0 {
            return;
        }
        for (i, note) in buf.notes_mut().iter_mut().enumerate() {
            // Stable per-note key: same playback position + pitch + slot →
            // same perturbation (reproducible), decorrelated across notes. The
            // fields occupy disjoint bit ranges (tick: 0–31, pitch: 32–39,
            // slot: 40–47) so they can't alias even in very long patterns; the
            // velocity and gate sub-streams come from adjacent seeds.
            let key = self.seed
                ^ u64::from(tick.0)
                ^ (u64::from(note.pitch.as_midi()) << 32)
                ^ ((i as u64) << 40);
            if vel_amt > 0.0 {
                let bipolar = splitmix64_bipolar(key);
                note.velocity = Velocity::new(note.velocity.as_f32() * (1.0 + bipolar * vel_amt));
            }
            if gate_amt > 0.0
                && let Some(duration) = note.duration
            {
                let bipolar = splitmix64_bipolar(key.wrapping_add(1));
                let scaled = (duration.0 as f32 * (1.0 + bipolar * gate_amt)).max(1.0);
                note.duration = Some(Duration(scaled.round() as u32));
            }
        }
    }
}

// ============================================================================
// The processor rack
// ============================================================================

/// One processor in a pattern's rack — pure *config* (Model B), never baked
/// output. Serialized with the pattern, so it round-trips save/load with the
/// same path as notes and automation.
///
/// `Copy` (every variant's config is `Copy`): the Note Grid collects a spine's
/// upstream processors into a fixed stack array to reuse the arp / strummed-chord
/// held-pitch view (`expand_held_pitch`) — no allocation on the audio thread.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum NoteProcessor {
    /// Snap pitches to a scale (chain stage 0).
    ScaleQuantize(ScaleQuantize),
    /// Expand each source note into a chord (chain stage 1).
    Chord(Chord),
    /// Arpeggiate the held chord (chain stage 2).
    Arpeggiator(Arpeggiator),
    /// Seeded random perturbation of the final stream (chain stage 4).
    Humanize(Humanize),
}

impl NoteProcessor {
    /// Canonical chain stage of this processor kind (LOCKED order):
    /// scale-quantize = 0, chord = 1, arp = 2, ornaments/strum = 3,
    /// humanize = 4. [`Pattern::add_processor`] inserts by this key.
    #[must_use]
    pub fn chain_stage(&self) -> u8 {
        match self {
            Self::ScaleQuantize(_) => 0,
            Self::Chord(_) => 1,
            Self::Arpeggiator(_) => 2,
            Self::Humanize(_) => 4,
        }
    }

    /// Stable kind tag (snake_case) for readers / MCP.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ScaleQuantize(_) => "scale_quantize",
            Self::Chord(_) => "chord",
            Self::Arpeggiator(_) => "arpeggiator",
            Self::Humanize(_) => "humanize",
        }
    }

    /// Expand one source pitch as this processor contributes it to a downstream
    /// generator's held-notes view — the **1→N seam**. A pitch transform yields
    /// one pitch (scale-quantize snaps); a chord generator yields its tones; a
    /// stream replacer (the arp) is identity here (it never sits upstream of a
    /// held-notes consumer except behind another stream generator, where
    /// "last one wins"). Chaining these across an upstream slice is
    /// [`expand_held_pitch`], which is how the arp sees a preceding chord.
    fn expand_pitch(&self, pitch: Pitch, sink: &mut impl FnMut(Pitch)) {
        match self {
            Self::ScaleQuantize(q) => sink(q.snap(pitch)),
            Self::Chord(c) => c.tones(pitch, sink),
            Self::Arpeggiator(_) | Self::Humanize(_) => sink(pitch),
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
    /// The source-context processors (`Arpeggiator`, strummed `Chord`) read the
    /// raw `source` notes and the `upstream` pitch-transform chain, so both the
    /// rack (`upstream = &processors[..i]`) and the Note Grid (upstream = the
    /// spine-prefix processors) drive this one entry point. `host_salt` mixes
    /// into any seeded draw (0 for the rack — behaviour unchanged; the host key
    /// for a shared graph). RT-safe: no allocation.
    pub(crate) fn process_at_tick(
        &self,
        source: &[Note],
        tick: PatternTick,
        upstream: &[NoteProcessor],
        bpm: Bpm,
        host_salt: u64,
        buf: &mut ExpansionBuffer,
    ) {
        // The pattern-*independent* processors live in `apply_stateless` (the
        // single source of truth, shared with the Note Grid).
        if self.apply_stateless(tick, host_salt, buf) {
            return;
        }
        match self {
            Self::Chord(c) => c.expand_strummed(source, tick, upstream, buf),
            Self::Arpeggiator(a) => a.process_at_tick(source, tick, upstream, bpm, host_salt, buf),
            // Already handled by `apply_stateless` above.
            Self::ScaleQuantize(_) | Self::Humanize(_) => {}
        }
    }

    /// Note Grid reuse of the pattern-*independent* processors: transform `buf`
    /// in place, mixing `host_salt` (the Note Grid host key, plan §1.2) into any
    /// seeded randomness so a shared graph decorrelates per host. Returns `true`
    /// if handled.
    ///
    /// Returns `false` for the processors that need source context
    /// (`Arpeggiator`, strummed `Chord`); the caller then routes them through
    /// [`Self::process_at_tick`], which reads the source notes + upstream chain.
    /// RT-safe: no allocation (the `Humanize` salt is a stack copy of three
    /// scalar fields).
    pub(crate) fn apply_stateless(
        &self,
        tick: PatternTick,
        host_salt: u64,
        buf: &mut ExpansionBuffer,
    ) -> bool {
        match self {
            Self::ScaleQuantize(q) => {
                for note in buf.notes_mut() {
                    note.pitch = q.snap(note.pitch);
                }
                true
            }
            Self::Chord(c) if c.strum.0 == 0 => {
                c.expand_into(buf);
                true
            }
            Self::Humanize(h) => {
                let mut salted = *h;
                salted.seed ^= host_salt;
                salted.apply(tick, buf);
                true
            }
            Self::Chord(_) | Self::Arpeggiator(_) => false,
        }
    }
}

impl Chord {
    /// Emit this chord's pitches for a given `root` into `sink` — the single
    /// source of truth for "a chord's tones", shared by the buf-side
    /// [`Self::expand_into`] and the held-view [`NoteProcessor::expand_pitch`].
    /// Tones transposed off the MIDI range are skipped.
    fn tones(&self, root: Pitch, sink: &mut impl FnMut(Pitch)) {
        for &iv in self.intervals() {
            if let Some(p) = root.transpose(Semitones::new(f32::from(iv))) {
                sink(p);
            }
        }
    }

    /// Buf-side expansion (standalone chord, i.e. not consumed by a downstream
    /// stream generator): **replace** each note currently in `buf` — the
    /// upstream onsets at this tick — with its chord tones. The chained
    /// held-notes view ([`expand_held_pitch`]) is the *other* half: it lets a
    /// downstream arp see these tones without this buf rewrite (the arp clears
    /// the buffer). RT-safe: tones are produced through a fixed snapshot, no
    /// allocation; the snapshot is skipped entirely on the common no-onset tick.
    fn expand_into(&self, buf: &mut ExpansionBuffer) {
        if buf.notes().is_empty() {
            return;
        }
        const DUMMY: ExpandedNote = ExpandedNote {
            duration: None,
            pitch: Pitch::MIDDLE_C,
            velocity: Velocity::MF,
            legato: false,
            glide: None,
            expression: None,
        };
        let mut roots = [DUMMY; MAX_EXPANSION_EVENTS_PER_TICK];
        let n = buf.notes().len().min(MAX_EXPANSION_EVENTS_PER_TICK);
        roots[..n].copy_from_slice(&buf.notes()[..n]);
        buf.clear_notes();
        for root in &roots[..n] {
            self.tones(root.pitch, &mut |pitch| {
                let _ = buf.push(ExpandedNote { pitch, ..*root });
            });
        }
    }

    /// Strummed expansion (`strum > 0`): like [`Self::expand_into`] but each
    /// tone's onset is staggered by `strum` ticks in `direction`, so the chord
    /// spans several ticks. Because a tone can land on a tick after its source
    /// note's onset, this **scans the source notes** per tick (like the arp /
    /// ornaments) rather than transforming the buffer — the buffer at this tick
    /// no longer holds the originating note. Replaces the source stream at this
    /// stage (clears the buffer first). RT-safe: bounded by
    /// [`MAX_CHORD_INTERVALS`]; tones stage through a fixed array.
    #[allow(clippy::cast_possible_truncation)]
    fn expand_strummed(
        &self,
        source: &[Note],
        tick: PatternTick,
        upstream: &[NoteProcessor],
        buf: &mut ExpansionBuffer,
    ) {
        buf.clear_notes();
        let live = self.intervals().len();
        if live == 0 {
            return; // empty chord — nothing to strum, skip the source scan
        }
        let t = i64::from(tick.0);
        let spread = i64::from(self.strum.0); // > 0 (block chords take expand_into)
        let tail = i64::from(self.strum_tail());

        for note in source {
            let start = i64::from(note.start.0);
            // Only one tone of a note can land on this tick, at strum index
            // `j = (t - start) / spread` when the gap is an exact multiple of
            // the spread. Reject every other tick before building/sorting the
            // chord (the sorted order and root are tick-invariant, so doing
            // them only on the ~1-in-`spread` onset ticks avoids the bulk of
            // the per-tick work).
            let delta = t - start;
            if delta < 0 || delta > tail || delta % spread != 0 {
                continue;
            }
            let j = (delta / spread) as usize;
            if j >= live {
                continue;
            }

            // Chord tones, viewed through the upstream chain (so a quantize
            // ahead of the chord snaps the root first), ordered by the strum.
            let mut tones = [Pitch::MIDDLE_C; MAX_CHORD_INTERVALS];
            let mut tn = 0;
            let root = upstream_pitch(upstream, note.pitch);
            self.tones(root, &mut |p| {
                if tn < MAX_CHORD_INTERVALS {
                    tones[tn] = p;
                    tn += 1;
                }
            });
            let tones = &mut tones[..tn];
            tones.sort_unstable_by_key(|p| p.as_midi());
            if self.direction == StrumDirection::Down {
                tones.reverse();
            }
            if let Some(&pitch) = tones.get(j) {
                let _ = buf.push(ExpandedNote {
                    pitch,
                    ..ExpandedNote::from_note(note)
                });
            }
        }
    }
}

/// Expand one source pitch through the `upstream` chain (1→N at each stage)
/// and invoke `sink` for every resulting held pitch. This is the widened seam
/// that lets a downstream generator (the arp) view source material *through*
/// an upstream chord/quantize: a single note becomes the snapped chord tones.
///
/// Ping-pongs two fixed [`MAX_ARP_HELD`] stacks — no allocation; each stage's
/// fan-out is capped so a chord-of-chords cannot overflow the held view (the
/// excess is silently dropped, matching the held-notes truncation policy).
/// The first pitch `pitch` maps to through the `upstream` chain — the root a
/// chord generator builds from when scanning source notes (its upstream is
/// only pitch transforms like scale-quantize, which are 1→1, so "first" is
/// the whole story; a hypothetical 1→N upstream would just pick its first
/// tone as the strum root).
fn upstream_pitch(upstream: &[NoteProcessor], pitch: Pitch) -> Pitch {
    let mut root = pitch;
    let mut set = false;
    expand_held_pitch(upstream, pitch, &mut |p| {
        if !set {
            root = p;
            set = true;
        }
    });
    root
}

fn expand_held_pitch(upstream: &[NoteProcessor], pitch: Pitch, sink: &mut impl FnMut(Pitch)) {
    let mut cur = [Pitch::MIDDLE_C; MAX_ARP_HELD];
    cur[0] = pitch;
    let mut cur_n = 1;
    let mut next = [Pitch::MIDDLE_C; MAX_ARP_HELD];
    for proc in upstream {
        let mut next_n = 0;
        for &p in &cur[..cur_n] {
            proc.expand_pitch(p, &mut |out| {
                if next_n < MAX_ARP_HELD {
                    next[next_n] = out;
                    next_n += 1;
                }
            });
        }
        cur[..next_n].copy_from_slice(&next[..next_n]);
        cur_n = next_n;
    }
    for &p in &cur[..cur_n] {
        sink(p);
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
    pub(crate) fn from_note(note: &Note) -> Self {
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

    /// Discard the collected notes but keep the drop counter — used by
    /// generators that replace the upstream stream mid-rack (the arp), where
    /// resetting `dropped` would lose this tick's overflow count.
    fn clear_notes(&mut self) {
        self.notes.clear();
    }

    /// Fold another buffer's overflow count into this one — used when a
    /// note-scope sub-expansion's drops must be reported through the outer
    /// buffer (the outer buffer is the one the engine reads for drop reporting).
    pub fn add_dropped(&mut self, n: u32) {
        self.dropped = self.dropped.saturating_add(n);
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

    /// Keep only the notes for which `keep` returns true, in place (no
    /// allocation — reuses the buffer). Used by stream filters such as the Note
    /// Grid's `ProbabilityGate`. RT-safe.
    pub(crate) fn retain(&mut self, keep: impl FnMut(&ExpandedNote) -> bool) {
        self.notes.retain(keep);
    }

    /// Transform each note in place, dropping those for which `keep` returns
    /// false (no allocation — reuses the buffer). Used by the Note Grid's
    /// `NoteScriptTransform`, which rewrites fields and can drop a note. RT-safe.
    pub(crate) fn retain_mut(&mut self, keep: impl FnMut(&mut ExpandedNote) -> bool) {
        self.notes.retain_mut(keep);
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
// Per-note ornaments (NP3): the timed-repeat generator
// ============================================================================

/// Effective hit count of an ornament (clamped to the RT cap). `< 2` means
/// "no figure" — the note plays unchanged.
fn ornament_hits(orn: &Ornament) -> u32 {
    u32::from(orn.count.min(MAX_ORNAMENT_HITS))
}

/// Curve-warped onset offset (ticks from the figure's first hit) of hit `k` of
/// `n`, before strict-monotonic dedup. Monotonic non-decreasing in `k`, with
/// `offset(0) = 0` and `offset(n-1) = total_span`. The warp can *round* two
/// adjacent hits onto the same tick (e.g. a fast curve with a tiny `spacing`);
/// [`ornament_onsets`] resolves that into strictly increasing onsets, so this
/// is never consumed raw. `i64` throughout — `total_span` can exceed `u32`.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn ornament_offset(curve: OrnamentSpacing, k: u32, n: u32, total_span: i64) -> i64 {
    if n <= 1 || total_span == 0 {
        return 0;
    }
    let u = f64::from(k) / f64::from(n - 1);
    let w = match curve {
        OrnamentSpacing::Even => u,
        // Gaps shrink toward the end (onsets bunch up late) → accelerando.
        OrnamentSpacing::Accelerate => 2.0 * u - u * u,
        // Gaps grow toward the end → ritardando.
        OrnamentSpacing::Decelerate => u * u,
    };
    (total_span as f64 * w).round() as i64
}

/// Build the figure's `n` hit onsets (offsets from the first hit) into `out`,
/// returning the populated prefix. The curve shapes the spacing, but the
/// onsets are forced **strictly increasing with the endpoints pinned**
/// (`out[0] = 0`, `out[n-1] = total_span`): each onset is clamped to at least
/// one tick past its predecessor and to leave one tick per remaining hit. This
/// guarantees no two hits of one ornament ever collide on a tick — a collision
/// would emit two same-pitch `NoteOn`s whose differing `NoteOff`s would cut
/// each other short in the (pitch-keyed) voice bookkeeping. `base >= 1` makes
/// `total_span >= n - 1`, so the clamp range is always non-empty.
fn ornament_onsets(orn: &Ornament, n: u32, total_span: i64, out: &mut [i64]) {
    let mut prev = -1_i64;
    for k in 0..n {
        let raw = ornament_offset(orn.spacing_curve, k, n, total_span);
        let lower = prev + 1;
        let upper = total_span - i64::from(n - 1 - k);
        let onset = raw.clamp(lower, upper);
        out[k as usize] = onset;
        prev = onset;
    }
}

/// Shared velocity ramp: linear from `floor` (at `t = 0`) to `1.0` (at
/// `t = 1`), or the reverse when `!rising`. Used by both the arpeggiator's
/// velocity ramp and ornament dynamics so the curve shape stays in one place.
fn velocity_ramp(floor: f32, t: f32, rising: bool) -> f32 {
    if rising {
        floor + (1.0 - floor) * t
    } else {
        1.0 - (1.0 - floor) * t
    }
}

/// Velocity multiplier for hit `k` of `n` under the dynamics curve.
#[allow(clippy::cast_precision_loss)]
fn ornament_velocity_factor(dynamics: OrnamentDynamics, k: u32, n: u32) -> f32 {
    if n <= 1 {
        return 1.0;
    }
    let t = k as f32 / (n - 1) as f32;
    match dynamics {
        OrnamentDynamics::Flat => 1.0,
        OrnamentDynamics::Crescendo => velocity_ramp(ORNAMENT_VEL_FLOOR, t, true),
        OrnamentDynamics::Decrescendo => velocity_ramp(ORNAMENT_VEL_FLOOR, t, false),
    }
}

/// Emit the timed-repeat ornament hits of `note` that land **exactly** on
/// `tick`, into `buf`. A pure, stateless function of *(note, ornament, tick)* —
/// the engine calls it on every tick, so a lead-in figure's grace hits surface
/// at the ticks before `note.start`. RT-safe: no allocation (onsets live in a
/// fixed stack array bounded by [`MAX_ORNAMENT_HITS`]).
///
/// Exactly one hit is the *main* hit (source pitch + duration + expression);
/// the rest are short grace hits at `pitch_offset`. A grace whose onset falls
/// before tick 0 (a lead-in figure too close to the pattern start) is dropped.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn expand_ornament_at(note: &Note, orn: &Ornament, tick: PatternTick, buf: &mut ExpansionBuffer) {
    let n = ornament_hits(orn);
    if n < 2 {
        return;
    }
    // i64 throughout: `count` (≤ 64) × `spacing` (u32) overflows u32, and a
    // lead-in `figure_start` can go negative.
    let base = i64::from(orn.spacing.0.max(1));
    let total_span = (i64::from(n) - 1) * base;
    let start = i64::from(note.start.0);
    let figure_start = match orn.placement {
        OrnamentPlacement::LeadIn => start - total_span,
        OrnamentPlacement::OnBeat => start,
    };

    // Cheap window reject before building the onsets.
    let rel = i64::from(tick.0) - figure_start;
    if rel < 0 || rel > total_span {
        return;
    }

    let mut onsets = [0_i64; MAX_ORNAMENT_HITS as usize];
    let onsets = &mut onsets[..n as usize];
    ornament_onsets(orn, n, total_span, onsets);

    // Onsets are strictly increasing, so at most one hit lands on this tick.
    let Some(k) = (0..n as usize).find(|&k| onsets[k] == rel) else {
        return;
    };
    let is_main = match orn.placement {
        OrnamentPlacement::LeadIn => k + 1 == n as usize,
        OrnamentPlacement::OnBeat => k == 0,
    };
    let velocity = match orn.dynamics {
        OrnamentDynamics::Flat => note.velocity,
        _ => {
            let factor = ornament_velocity_factor(orn.dynamics, k as u32, n);
            Velocity::new(note.velocity.as_f32() * factor)
        }
    };

    if is_main {
        // The main hit is the source note, only its velocity reshaped.
        let mut e = ExpandedNote::from_note(note);
        e.velocity = velocity;
        let _ = buf.push(e);
    } else {
        let Some(pitch) = note.pitch.transpose(orn.pitch_offset) else {
            return; // grace transposed off the MIDI range — skip
        };
        // Grace duration = its gap to the next hit × grace_gate (the last hit
        // has no successor, so fall back to the base spacing).
        let gap = if k + 1 < n as usize {
            onsets[k + 1] - onsets[k]
        } else {
            base
        };
        let duration = ((gap as f64 * f64::from(orn.grace_gate.as_f32())) as i64).max(1);
        let _ = buf.push(ExpandedNote {
            duration: Some(Duration(duration as u32)),
            pitch,
            velocity,
            legato: false,
            glide: None,
            expression: None,
        });
    }
}

/// Seed `buf` with the source notes that sound at `tick` and pass `gate` —
/// plain onsets plus per-note ornament hits (which, for a lead-in figure, land
/// on ticks *before* the note's own start). Shared by the pattern rack and the
/// Note Grid so both see the same ornament-expanded source (plan §4). `buf` is
/// cleared first. Audio-thread hot path: no allocation, bounded by `buf`'s cap.
///
/// When `note_scope` is `Some`, each note that binds a resolvable note-scope
/// graph ([`Note::note_graph`], plan §2.1) is expanded through *that* graph on
/// its own material — per-note articulation (flam / strum / arp of one note),
/// decorrelated by [`HostKey::from`]`(note.id)` so one shared graph varies per
/// note. The single-note result splices into the seeded stream, so the
/// pattern-scope chain downstream sees the articulated notes (the note → pattern
/// pipeline). The inner expansion runs with note scope disabled, so a note-scope
/// graph never re-resolves its own note (no recursion). A note with no binding, a
/// dangling id, or `note_scope == None` falls through to the plain / ornament
/// path (pass-through — never a panic or silence). Probability gates the whole
/// articulation at the note level.
pub(crate) fn seed_source_at_tick(
    notes: &[Note],
    tick: PatternTick,
    bpm: Bpm,
    // `&dyn` (not `impl Fn`) on purpose: the note-scope branch re-enters
    // `NoteGraph::expand_at_tick`, which loops back into `seed_source_at_tick`.
    // A generic gate would make that a monomorphization cycle (each level wraps
    // the gate in another `&`); a single non-generic instantiation closes it.
    // The per-note dyn call is RT-safe (indirect call, no alloc/lock).
    gate: &dyn Fn(&Note) -> bool,
    mut note_scope: Option<&mut NoteScopeCtx<'_>>,
    buf: &mut ExpansionBuffer,
) {
    buf.clear();
    for note in notes {
        if let Some(ctx) = note_scope.as_deref_mut()
            && let Some(gid) = note.note_graph
            && let Some(graph) = ctx.pool.iter().find(|g| g.id == gid)
        {
            if gate(note) {
                // Expand this one note through its note-scope graph into the
                // scratch buffer (host = NoteId; note scope disabled inside so it
                // cannot re-resolve itself), then splice the result into `buf`.
                graph.expand_at_tick(
                    std::slice::from_ref(note),
                    tick,
                    HostKey::from(note.id),
                    bpm,
                    |_| true,
                    None,
                    // Note-scope articulation of a single note does not carry a
                    // timing look-back pool (a delay inside a note-scope graph is
                    // an edge case), so its timing nodes emit dry-only.
                    None,
                    ctx.scratch,
                );
                for i in 0..ctx.scratch.notes().len() {
                    let expanded = ctx.scratch.notes()[i];
                    let _ = buf.push(expanded);
                }
                buf.add_dropped(ctx.scratch.dropped());
            }
            continue;
        }
        match &note.ornament {
            Some(orn) if ornament_hits(orn) >= 2 => {
                if gate(note) {
                    expand_ornament_at(note, orn, tick, buf);
                }
            }
            _ => {
                if note.start == tick && gate(note) {
                    let _ = buf.push(ExpandedNote::from_note(note));
                }
            }
        }
    }
}

/// The widest strum tail any note-scope graph (plan §2.1) staggers past a note's
/// onset, across the notes bound to a graph resolvable in `pool`. The freeze
/// walk extends by this so a note-scope strummed chord near the pattern end
/// bakes its full spread — mirroring the pattern-scope `NoteGraph::max_strum_tail`.
///
/// Deliberately does NOT fold `max_delay_tail`: the note-scope inner expansion
/// runs with `lookback: None` (see `seed_source_at_tick`), so timing nodes in a
/// note-scope graph are dry-only and contribute no echo tail. If note-scope ever
/// gains a look-back pool, this must switch to `max_walk_tail` or note-scope
/// freeze will truncate echoes.
pub(crate) fn note_scope_strum_tail(notes: &[Note], pool: &[NoteGraph]) -> u32 {
    notes
        .iter()
        .filter_map(|n| n.note_graph)
        .filter_map(|gid| pool.iter().find(|g| g.id == gid))
        .map(NoteGraph::max_strum_tail)
        .max()
        .unwrap_or(0)
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
    ///
    /// The rack has no product authoring surface anymore — the Note Grid
    /// replaced it (a graph is bound per pattern, and legacy racks migrate to
    /// graphs on load, [`Song::migrate_processor_racks_to_graphs`]). This
    /// remains to construct legacy racks for the migration / persistence tests.
    pub fn add_processor(&mut self, processor: NoteProcessor) -> usize {
        let stage = processor.chain_stage();
        let pos = self
            .processors
            .partition_point(|p| p.chain_stage() <= stage);
        self.processors.insert(pos, processor);
        pos
    }

    /// Drop the whole rack. Used by freeze (a bound graph suppresses the rack at
    /// playback, so baking must retire the rack) and by the load-time rack →
    /// graph migration.
    pub fn clear_processors(&mut self) {
        self.processors.clear();
    }

    /// Model-B playback-time expansion at one tick: collect the source notes
    /// that sound at `tick` and pass `gate` (the engine's per-note probability
    /// roll; preview passes `|_| true`), then run the rack in order. The result
    /// lands in `buf` (cleared first).
    ///
    /// A plain note contributes one event at its own start tick. A note with a
    /// per-note [`Ornament`] instead contributes the ornament's timed-repeat
    /// hits that land on `tick` (which, for a lead-in figure, includes ticks
    /// *before* the note's start). Because an ornament spans many ticks, `gate`
    /// must roll consistently for a given note across all of them — the engine
    /// seeds it by the note's start, not the expansion tick.
    ///
    /// Per-note ornaments expand during collection, so an upstream
    /// scale-quantize snaps their grace pitches into key, and a stream
    /// generator that replaces the stream (the arpeggiator clears the buffer)
    /// consumes them — "last stream generator wins", as for the source chord.
    ///
    /// **Audio-thread hot path** — no allocation, bounded by `buf`'s cap.
    pub fn expand_at_tick(
        &self,
        tick: PatternTick,
        gate: impl Fn(&Note) -> bool,
        bpm: Bpm,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
        buf: &mut ExpansionBuffer,
    ) {
        self.expand_at_tick_through(tick, gate, bpm, self.processors.len(), note_scope, buf);
    }

    /// As [`Self::expand_at_tick`] but runs only the first `through` processors of
    /// the rack (clamped to the rack length). `through == processors().len()` is the
    /// full expansion; smaller values give the cumulative output *after* a given
    /// stage, which the tracker uses to show each processor's contribution as its
    /// own column. Same RT-safety contract as `expand_at_tick`.
    pub fn expand_at_tick_through(
        &self,
        tick: PatternTick,
        gate: impl Fn(&Note) -> bool,
        bpm: Bpm,
        through: usize,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
        buf: &mut ExpansionBuffer,
    ) {
        seed_source_at_tick(self.notes(), tick, bpm, &gate, note_scope, buf);
        for i in 0..through.min(self.processors.len()) {
            // Each processor sees its upstream slice so generators can view
            // source material through the preceding transforms (`map_pitch`).
            let (upstream, rest) = self.processors.split_at(i);
            // Rack host salt is 0 (a pattern's own rack is not a shared asset).
            rest[0].process_at_tick(self.notes(), tick, upstream, bpm, 0, buf);
        }
    }

    /// The last tick the pattern's own source material can reach — the max of
    /// its length and every note's span (note end plus any on-beat ornament
    /// tail). The bake/freeze walk is `0..end`; a rack stream generator's
    /// forward steps (strum, arp) extend this further at the call site.
    #[must_use]
    pub(crate) fn source_walk_end(&self) -> u32 {
        let note_reach = |n: &Note| {
            // No duration ⇒ plays until cut, so `start+1` (the first tick it can
            // emit); otherwise the note's end.
            let span_end = n.end().map_or(n.start.0.saturating_add(1), |e| e.0);
            // An on-beat multi-hit ornament staggers hits forward from the onset.
            let ornament_end = match &n.ornament {
                Some(o) if ornament_hits(o) >= 2 && o.placement == OrnamentPlacement::OnBeat => {
                    let span = (ornament_hits(o) - 1) * o.spacing.0.max(1);
                    n.start.0.saturating_add(1).saturating_add(span)
                }
                _ => 0,
            };
            span_end.max(ornament_end)
        };
        self.length
            .0
            .max(self.notes().iter().map(note_reach).max().unwrap_or(0))
    }

    /// Replace all notes with a baked `(tick, ExpandedNote)` set (from a rack or
    /// Note Grid bake), assigning fresh ids. The baked notes are plain (ornament
    /// config is not carried through). Non-RT (allocates).
    pub(crate) fn replace_with_baked(&mut self, baked: Vec<(PatternTick, ExpandedNote)>) {
        self.clear_notes();
        for (start, e) in baked {
            let mut note = Note::new(NoteId(0), start, e.pitch, e.velocity);
            note.duration = e.duration;
            note.legato = e.legato;
            note.glide = e.glide;
            note.expression = e.expression;
            let _ = self.insert_note(note);
        }
    }

    /// One-shot Model-A bake: run the rack (and any per-note ornaments) over
    /// every tick of the pattern, replace the source notes with the concrete
    /// expansion, and clear the rack. Returns the number of notes after the
    /// bake. Per-note ornament fields are dropped — the baked notes are plain.
    ///
    /// UI-thread only (allocates; walks the full pattern length — generators
    /// may emit at ticks where no source note starts, so every tick is
    /// visited). `bpm` resolves frame-locked arp rates ([`ArpRate::Ticks`] is
    /// tempo-independent; absolute-rate variants need the tempo).
    pub fn freeze_processors(&mut self, bpm: Bpm, note_scope_pool: &[NoteGraph]) -> usize {
        let has_ornament = self
            .notes()
            .iter()
            .any(|n| n.ornament.as_ref().is_some_and(|o| ornament_hits(o) >= 2));
        // A note bound to a note-scope graph must be baked even with an empty
        // rack and no ornaments — the binding alone drives expansion.
        let has_note_scope = self.notes().iter().any(|n| n.note_graph.is_some());
        if self.processors.is_empty() && !has_ornament && !has_note_scope {
            return self.note_count();
        }
        let mut buf = ExpansionBuffer::new();
        let mut ns_scratch = ExpansionBuffer::new();
        let mut frozen: Vec<(PatternTick, ExpandedNote)> = Vec::new();
        // Walk past `length` if any note (or its on-beat ornament tail) reaches
        // beyond it — add_note does not clamp — so an out-of-range source note
        // survives the bake instead of being silently dropped by the
        // clear-and-reinsert below. (A lead-in ornament reaches earlier ticks,
        // which the 0-based walk already covers.)
        // A strumming chord in the rack staggers tones up to its tail past a
        // note's onset, so the walk must reach that far past the source span.
        let strum_tail = self
            .processors
            .iter()
            .filter_map(|p| match p {
                NoteProcessor::Chord(c) => Some(c.strum_tail()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        // A note-scope graph can also stagger tones past a note's onset (a
        // strummed chord as articulation); extend the walk by the widest such
        // tail, mirroring the pattern-scope bake's `max_strum_tail`.
        let note_scope_tail = note_scope_strum_tail(self.notes(), note_scope_pool);
        let walk_end = self
            .source_walk_end()
            .saturating_add(strum_tail)
            .saturating_add(note_scope_tail);
        for tick in 0..walk_end {
            let t = PatternTick(tick);
            let mut ctx = NoteScopeCtx {
                pool: note_scope_pool,
                scratch: &mut ns_scratch,
            };
            self.expand_at_tick(t, |_| true, bpm, Some(&mut ctx), &mut buf);
            for expanded in buf.notes() {
                frozen.push((t, *expanded));
            }
        }
        self.replace_with_baked(frozen);
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
        let _ = p.add_processor(a);
        let idx = p.add_processor(b);
        assert_eq!(idx, 1);
        assert_eq!(p.processors(), &[a, b]);
    }

    #[test]
    fn expand_at_tick_collects_gates_and_quantizes() {
        let mut p = pattern_with_notes(&[60, 61, 66]);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let mut buf = ExpansionBuffer::new();

        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 62, 67]);

        // The gate filters source notes before the rack runs.
        p.expand_at_tick(
            PatternTick(0),
            |n| n.pitch.as_midi() != 61,
            Bpm::DEFAULT,
            None,
            &mut buf,
        );
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 67]);

        // A tick with no starting notes expands to nothing (no generators yet).
        p.expand_at_tick(PatternTick(240), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert!(buf.notes().is_empty());
    }

    #[test]
    fn expand_at_tick_without_rack_passes_through() {
        let p = pattern_with_notes(&[61]);
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes().len(), 1);
        assert_eq!(buf.notes()[0].pitch.as_midi(), 61);
    }

    #[test]
    fn freeze_processors_bakes_and_clears_rack() {
        let mut p = pattern_with_notes(&[60, 61, 66]);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));

        let count = p.freeze_processors(Bpm::DEFAULT, &[]);
        assert_eq!(count, 3);
        assert!(p.processors().is_empty());
        let pitches: Vec<u8> = p.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 62, 67]);
    }

    #[test]
    fn freeze_without_rack_is_noop() {
        let mut p = pattern_with_notes(&[61]);
        assert_eq!(p.freeze_processors(Bpm::DEFAULT, &[]), 1);
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

        assert_eq!(p.freeze_processors(Bpm::DEFAULT, &[]), 2);
        let pitches: Vec<(u32, u8)> = p
            .notes()
            .iter()
            .map(|n| (n.start.0, n.pitch.as_midi()))
            .collect();
        assert_eq!(pitches, vec![(0, 62), (5000, 67)]);
    }

    #[test]
    fn freeze_walk_covers_held_note_arp_steps_past_length() {
        // A note held past the pattern length, arpeggiated: the freeze walk must
        // reach the note's *end* so its late steps survive the bake. `note_reach`
        // used to stop at `start+1`, so every arp step at/after `length` was
        // dropped by the clear-and-reinsert (the walk never visited those ticks).
        let mut p = Pattern::new(PatternId(0), Duration(240));
        let id = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        // Held to 480 — well past the 240-tick pattern length.
        assert!(p.resize_note(id, Duration(480)));
        let mut a = arp(ArpMode::Up, 1);
        a.rate = ArpRate::Ticks(120);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));

        // Onsets at 0, 120, 240, 360 — all while the note is held [0,480).
        let count = p.freeze_processors(Bpm::DEFAULT, &[]);
        assert_eq!(
            count, 4,
            "all four arp steps (incl. those past `length`) must be baked"
        );
        assert!(
            p.notes().iter().any(|n| n.start.0 >= 240),
            "steps at/after the old `start+1` walk boundary must survive"
        );
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

    // --- Arpeggiator (NP2) --------------------------------------------------

    /// C major triad (C4 E4 G4) held for `hold` ticks from tick 0, in a
    /// 4-bar pattern.
    fn chord_pattern(hold: u32) -> Pattern {
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        for &m in &[60u8, 64, 67] {
            let id = p.add_note(PatternTick(0), Pitch::new(m).unwrap(), Velocity::MF);
            let _ = p.resize_note(id, Duration(hold));
        }
        p
    }

    /// A single note `midi` held for `hold` ticks from tick 0.
    fn chord_pattern_single(midi: u8, hold: u32) -> Pattern {
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        let id = p.add_note(PatternTick(0), Pitch::new(midi).unwrap(), Velocity::MF);
        let _ = p.resize_note(id, Duration(hold));
        p
    }

    fn arp(mode: ArpMode, octaves: u8) -> Arpeggiator {
        Arpeggiator {
            mode,
            octaves,
            ..Arpeggiator::default()
        }
    }

    /// Expand a pattern over `ticks` and collect (tick, midi) of emitted notes.
    fn expand_all(p: &Pattern, ticks: u32) -> Vec<(u32, u8)> {
        expand_all_bpm(p, ticks, Bpm::DEFAULT)
    }

    /// As [`expand_all`] but at a chosen tempo (for frame-locked rate tests).
    fn expand_all_bpm(p: &Pattern, ticks: u32, bpm: Bpm) -> Vec<(u32, u8)> {
        let mut buf = ExpansionBuffer::new();
        let mut out = Vec::new();
        for t in 0..ticks {
            p.expand_at_tick(PatternTick(t), |_| true, bpm, None, &mut buf);
            for n in buf.notes() {
                out.push((t, n.pitch.as_midi()));
            }
        }
        out
    }

    #[test]
    fn arp_up_emits_ascending_cycle_on_step_grid() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        // 16th steps = 240 ticks; cycle C E G repeating, chord onset suppressed.
        let got = expand_all(&p, 960);
        assert_eq!(
            got,
            vec![(0, 60), (240, 64), (480, 67), (720, 60)],
            "ascending 16th-note cycle expected"
        );
    }

    #[test]
    fn arp_continuous_phase_advances_across_short_notes() {
        let mut pattern = Pattern::new(PatternId(0), Duration(960));
        for start in [0, 240] {
            let id = pattern.add_note(PatternTick(start), Pitch::new(60).unwrap(), Velocity::MF);
            let _ = pattern.resize_note(id, Duration(120));
        }
        let mut arp = Arpeggiator {
            mode: ArpMode::Custom,
            continuous_phase: true,
            ..Arpeggiator::default()
        };
        arp.custom = ArpOffsets::new(&[0, 4, 7]);
        let _ = pattern.add_processor(NoteProcessor::Arpeggiator(arp));

        assert_eq!(expand_all(&pattern, 480), vec![(0, 60), (240, 64)]);
    }

    #[test]
    fn arp_rate_ticks_subdivides_below_floor() {
        // `Ticks(40)` is below the old 80-tick floor (ThirtySecondTriplet @125 BPM
        // = one PAL frame), and `ticks()` maps it straight through.
        assert_eq!(ArpRate::Ticks(40).ticks(), Duration(40));
        // A zero count clamps to a 1-tick floor — never a divide-by-zero onset grid.
        assert_eq!(ArpRate::Ticks(0).ticks(), Duration(1));

        // A single held note arpeggiated at 40-tick steps emits one onset per 40
        // ticks (well below the divided-grid floor).
        let mut p = chord_pattern_single(60, 480);
        let mut a = arp(ArpMode::Up, 1);
        a.rate = ArpRate::Ticks(40);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let onsets: Vec<u32> = expand_all(&p, 200).iter().map(|(t, _)| *t).collect();
        assert_eq!(onsets, vec![0, 40, 80, 120, 160], "40-tick step spacing");
    }

    #[test]
    #[allow(clippy::cast_precision_loss)]
    fn arp_rate_hz_is_tempo_independent() {
        // 50 Hz (PAL frame). The *mean* step is 20 ms regardless of project
        // tempo, and there is no cumulative drift over a long pattern.
        let mut p = chord_pattern_single(60, 4000);
        let mut a = arp(ArpMode::Up, 1);
        a.rate = ArpRate::MilliHz(50_000);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));

        for &(bpm_val, ticks) in &[(120.0_f32, 800u32), (240.0_f32, 1600u32)] {
            let bpm = Bpm::new(bpm_val);
            // ticks/s = 16·BPM; step = (16·BPM)/Hz = 16_000·BPM/mHz.
            let step_len = 16_000.0 * bpm_val / 50_000.0;
            let onsets: Vec<u32> = expand_all_bpm(&p, ticks, bpm)
                .iter()
                .map(|(t, _)| *t)
                .collect();
            assert!(onsets.len() >= 8, "expected several onsets, got {onsets:?}");

            // No cumulative drift: the k-th onset is `round(k·step_len)` exactly,
            // never the sum of a truncated per-step integer.
            for (k, &o) in onsets.iter().enumerate() {
                let expected = (k as f32 * step_len).round() as u32;
                assert_eq!(o, expected, "onset {k} drifted at {bpm_val} BPM");
            }

            // Mean step ≈ 20 ms (1/50 Hz) at every tempo.
            let span = (onsets[onsets.len() - 1] - onsets[0]) as f32;
            let mean_ticks = span / (onsets.len() - 1) as f32;
            let mean_seconds = mean_ticks / (16.0 * bpm_val);
            assert!(
                (mean_seconds - 0.020).abs() < 5e-4,
                "mean step {mean_seconds}s != 20 ms at {bpm_val} BPM"
            );
        }
    }

    #[test]
    fn arp_rate_hz_subtick_clamps_to_one_per_tick() {
        // A rate finer than the tick grid (600 Hz @20 BPM → 0.53 ticks/step)
        // floors at one step per tick — every tick fires once, no steps skipped.
        let mut p = chord_pattern_single(60, 200);
        let mut a = arp(ArpMode::Up, 1);
        a.rate = ArpRate::MilliHz(600_000);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let onsets: Vec<u32> = expand_all_bpm(&p, 20, Bpm::new(20.0))
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(onsets, (0..20).collect::<Vec<_>>(), "one onset per tick");
    }

    #[test]
    fn arp_custom_stays_copy() {
        // Compile-time guard: the bounded inline offset list keeps `Arpeggiator`
        // (and the rack note processors) `Copy`.
        fn assert_copy<T: Copy>() {}
        assert_copy::<ArpOffsets>();
        assert_copy::<Arpeggiator>();
    }

    #[test]
    fn arp_offsets_serde_roundtrip() {
        // Reads/writes as a bare JSON array, not a struct.
        let off = ArpOffsets::new(&[0, 4, 7]);
        let json = serde_json::to_string(&off).unwrap();
        assert_eq!(json, "[0,4,7]");
        let back: ArpOffsets = serde_json::from_str(&json).unwrap();
        assert_eq!(back, off);
        assert_eq!(back.as_slice(), &[0, 4, 7]);

        // Negative offsets and the empty cycle round-trip too.
        assert_eq!(
            serde_json::to_string(&ArpOffsets::new(&[-12, 3])).unwrap(),
            "[-12,3]"
        );
        let empty: ArpOffsets = serde_json::from_str("[]").unwrap();
        assert!(empty.is_empty());

        // Past the cap is truncated, never a panic.
        let long: Vec<i8> = (0..32).collect();
        let capped = ArpOffsets::new(&long);
        assert_eq!(capped.len(), MAX_ARP_OFFSETS);
    }

    #[test]
    fn arp_custom_cycles_offsets() {
        // [0,4,7] over one held C note → C E G C E G… at the step grid.
        let mut p = chord_pattern_single(60, 480);
        let mut a = arp(ArpMode::Custom, 1);
        a.rate = ArpRate::Ticks(40);
        a.custom = ArpOffsets::new(&[0, 4, 7]);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let pitches: Vec<u8> = expand_all(&p, 200).iter().map(|(_, m)| *m).collect();
        assert_eq!(pitches, vec![60, 64, 67, 60, 64], "C E G cycle");
    }

    #[test]
    fn arp_custom_octaves_multiply_the_cycle() {
        // `octaves = 2` plays the cycle then the cycle +12: 0,4,7, 12,16,19.
        let mut p = chord_pattern_single(60, 480);
        let mut a = arp(ArpMode::Custom, 2);
        a.rate = ArpRate::Ticks(40);
        a.custom = ArpOffsets::new(&[0, 4, 7]);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let pitches: Vec<u8> = expand_all(&p, 240).iter().map(|(_, m)| *m).collect();
        assert_eq!(pitches, vec![60, 64, 67, 72, 76, 79], "two-octave cycle");
    }

    #[test]
    fn arp_custom_tracks_moving_note() {
        // The offsets apply to each source note's own pitch: a C→D line yields
        // the figure transposed per note (60,64,67 then 62,66,69).
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        let id1 = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::MF);
        let _ = p.resize_note(id1, Duration(120));
        let id2 = p.add_note(PatternTick(120), Pitch::new(62).unwrap(), Velocity::MF);
        let _ = p.resize_note(id2, Duration(120));
        let mut a = arp(ArpMode::Custom, 1);
        a.rate = ArpRate::Ticks(40);
        a.custom = ArpOffsets::new(&[0, 4, 7]);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let pitches: Vec<u8> = expand_all(&p, 240).iter().map(|(_, m)| *m).collect();
        assert_eq!(
            pitches,
            vec![60, 64, 67, 62, 66, 69],
            "figure transposed per note"
        );
    }

    #[test]
    fn arp_custom_root_seen_through_upstream_quantize() {
        // An upstream scale-quantize snaps the Custom root before the offsets
        // apply, exactly as it does for the held-chord modes — C#4 (61), off the
        // C-major scale, snaps to D4 (62) (nearest, preferring up), then [0,7]
        // plays 62, 69. Without the upstream pass it would (wrongly) play 61, 68.
        let mut p = chord_pattern_single(61, 480);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let mut a = arp(ArpMode::Custom, 1);
        a.rate = ArpRate::Ticks(40);
        a.custom = ArpOffsets::new(&[0, 7]);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let pitches: Vec<u8> = expand_all(&p, 80).iter().map(|(_, m)| *m).collect();
        assert_eq!(
            pitches,
            vec![62, 69],
            "root snapped to D then offset by 0,7"
        );
    }

    #[test]
    fn arp_legato_stamps_steps() {
        // The `legato` flag rides each emitted step (so the engine coalesces the
        // figure into one held envelope); off by default it does not.
        let mut p = chord_pattern_single(60, 480);
        let mut a = arp(ArpMode::Custom, 1);
        a.rate = ArpRate::Ticks(40);
        a.custom = ArpOffsets::new(&[0, 4, 7]);
        a.legato = true;
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(40), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert!(!buf.notes().is_empty());
        assert!(
            buf.notes().iter().all(|n| n.legato),
            "each step stamped legato"
        );

        // Default (legato off) leaves steps un-tied.
        let mut p2 = chord_pattern_single(60, 480);
        let mut a2 = arp(ArpMode::Custom, 1);
        a2.rate = ArpRate::Ticks(40);
        a2.custom = ArpOffsets::new(&[0, 4, 7]);
        let _ = p2.add_processor(NoteProcessor::Arpeggiator(a2));
        let mut buf2 = ExpansionBuffer::new();
        p2.expand_at_tick(PatternTick(40), |_| true, Bpm::DEFAULT, None, &mut buf2);
        assert!(
            buf2.notes().iter().all(|n| !n.legato),
            "steps not legato by default"
        );
    }

    #[test]
    fn arp_custom_empty_cycle_emits_nothing() {
        // A `Custom` arp with no offsets is silent, not a divide-by-zero.
        let mut p = chord_pattern_single(60, 480);
        let mut a = arp(ArpMode::Custom, 1);
        a.rate = ArpRate::Ticks(40);
        // custom left at its empty default.
        let _ = p.add_processor(NoteProcessor::Arpeggiator(a));
        assert!(expand_all(&p, 200).is_empty());
    }

    #[test]
    fn arp_suppresses_source_chord() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(
            buf.notes().len(),
            1,
            "only the arp step, never the source chord: {:?}",
            buf.notes()
        );
    }

    #[test]
    fn arp_down_and_octave_range() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Down, 2)));
        // Descending over 2 octaves: G E C then the upper octave G+12 E+12 C+12?
        // No — Down sorts members descending (67 64 60), octave index rises
        // with the cycle: 67,64,60,79,76,72.
        let got: Vec<u8> = expand_all(&p, 1440).into_iter().map(|(_, m)| m).collect();
        assert_eq!(got, vec![67, 64, 60, 79, 76, 72]);
    }

    #[test]
    fn arp_updown_does_not_repeat_endpoints() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::UpDown, 1)));
        // Cycle: C E G E | C E G E ... (endpoints C and G not doubled).
        let got: Vec<u8> = expand_all(&p, 1920).into_iter().map(|(_, m)| m).collect();
        assert_eq!(got, vec![60, 64, 67, 64, 60, 64, 67, 64]);
    }

    #[test]
    fn arp_gate_sets_step_duration() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            gate: NormalizedValue::new(0.5),
            ..arp(ArpMode::Up, 1)
        }));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes()[0].duration, Some(Duration(120)));
    }

    #[test]
    fn arp_swing_delays_odd_steps() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            swing: NormalizedValue::new(0.5),
            ..arp(ArpMode::Up, 1)
        }));
        // Odd steps delayed by 0.5 * 240/2 = 60 ticks: onsets 0, 300, 480, 780.
        let ticks: Vec<u32> = expand_all(&p, 960).into_iter().map(|(t, _)| t).collect();
        assert_eq!(ticks, vec![0, 300, 480, 780]);
    }

    #[test]
    fn arp_latch_holds_last_chord_through_gap() {
        // Chord held only 480 ticks; latch keeps arpeggiating it after.
        let mut p = chord_pattern(480);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            latch: true,
            ..arp(ArpMode::Up, 1)
        }));
        let got = expand_all(&p, 1200);
        assert_eq!(
            got,
            vec![(0, 60), (240, 64), (480, 67), (720, 60), (960, 64)],
            "latch must continue the figure past the chord's end"
        );

        // Without latch the figure stops when the chord ends.
        let mut p2 = chord_pattern(480);
        let _ = p2.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        let got2 = expand_all(&p2, 1200);
        assert_eq!(got2, vec![(0, 60), (240, 64)]);
    }

    #[test]
    fn arp_random_is_deterministic_and_in_set() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Random, 1)));
        let a = expand_all(&p, 3840);
        let b = expand_all(&p, 3840);
        assert_eq!(a, b, "random mode must be deterministic per position");
        assert!(a.iter().all(|(_, m)| [60, 64, 67].contains(m)));
        assert_eq!(a.len(), 16, "one note per 16th step over 4 beats × 4");
    }

    #[test]
    fn arp_chord_mode_retriggers_whole_chord() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Chord, 1)));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(240), |_| true, Bpm::DEFAULT, None, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 64, 67]);
    }

    #[test]
    fn arp_velocity_ramp_up_is_monotonic_over_cycle() {
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            velocity: ArpVelocity::RampUp,
            ..arp(ArpMode::Up, 1)
        }));
        let mut buf = ExpansionBuffer::new();
        let mut vels = Vec::new();
        for t in [0u32, 240, 480] {
            p.expand_at_tick(PatternTick(t), |_| true, Bpm::DEFAULT, None, &mut buf);
            vels.push(buf.notes()[0].velocity.as_f32());
        }
        assert!(
            vels[0] < vels[1] && vels[1] < vels[2],
            "ramp-up must rise across the cycle: {vels:?}"
        );
    }

    #[test]
    fn arp_sees_held_notes_through_upstream_quantize() {
        // Held C#-F-G# through C-major quantize: the arp must arpeggiate the
        // SNAPPED pitches (D F A... C# -> D, F in scale, G# -> A).
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        for &m in &[61u8, 65, 68] {
            let id = p.add_note(PatternTick(0), Pitch::new(m).unwrap(), Velocity::MF);
            let _ = p.resize_note(id, Duration(3840));
        }
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        // Canonical insertion: quantize (stage 0) before arp (stage 2).
        assert_eq!(p.processors()[0].chain_stage(), 0);

        let got: Vec<u8> = expand_all(&p, 720).into_iter().map(|(_, m)| m).collect();
        assert_eq!(got, vec![62, 65, 69], "arp must see quantized pitches");
    }

    #[test]
    fn arp_step_phase_restarts_on_new_chord_block() {
        // First chord C-E-G for 480 ticks, then D-F-A from 480: the figure
        // restarts at the new block's first member.
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        for &m in &[60u8, 64, 67] {
            let id = p.add_note(PatternTick(0), Pitch::new(m).unwrap(), Velocity::MF);
            let _ = p.resize_note(id, Duration(480));
        }
        for &m in &[62u8, 65, 69] {
            let id = p.add_note(PatternTick(480), Pitch::new(m).unwrap(), Velocity::MF);
            let _ = p.resize_note(id, Duration(480));
        }
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        let got = expand_all(&p, 960);
        assert_eq!(
            got,
            vec![(0, 60), (240, 64), (480, 62), (720, 65)],
            "step phase must re-anchor at the new chord block"
        );
    }

    #[test]
    fn arp_chord_mode_velocity_ramp_passes_through_at_one_octave() {
        // Regression: a one-step ramp cycle (Chord mode, octaves=1) must NOT
        // pin every step at the ramp floor — it passes the source velocity.
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            velocity: ArpVelocity::RampUp,
            ..arp(ArpMode::Chord, 1)
        }));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert!(
            buf.notes()
                .iter()
                .all(|n| (n.velocity.as_f32() - Velocity::MF.as_f32()).abs() < 1e-6),
            "degenerate ramp must pass velocity through: {:?}",
            buf.notes()
        );
    }

    #[test]
    fn arp_swung_step_duration_never_crosses_next_onset() {
        // Regression: gate 1.0 + swing 1.0 — odd (swung) steps have only
        // step/2 ticks to the next onset; duration must clamp to that gap.
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            gate: NormalizedValue::new(1.0),
            swing: NormalizedValue::new(1.0),
            ..arp(ArpMode::Up, 1)
        }));
        let mut buf = ExpansionBuffer::new();
        // Step 1 (odd) is swung to tick 240 + 120 = 360; the next onset is
        // step 2 at 480, so the gap is 120 ticks.
        p.expand_at_tick(PatternTick(360), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes().len(), 1);
        assert_eq!(
            buf.notes()[0].duration,
            Some(Duration(120)),
            "swung step must clamp to the gap before the next onset"
        );
    }

    #[test]
    fn arp_freeze_bakes_concrete_steps() {
        let mut p = chord_pattern(960);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        let count = p.freeze_processors(Bpm::DEFAULT, &[]);
        assert_eq!(count, 4, "4 steps over 960 ticks at 16th rate");
        assert!(p.processors().is_empty());
        let starts: Vec<u32> = p.notes().iter().map(|n| n.start.0).collect();
        assert_eq!(starts, vec![0, 240, 480, 720]);
        assert!(p.notes().iter().all(|n| n.duration == Some(Duration(180))));
    }

    #[test]
    fn arp_serde_roundtrip() {
        let mut p = chord_pattern(960);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(Arpeggiator {
            mode: ArpMode::DownUp,
            rate: ArpRate::EighthTriplet,
            octaves: 3,
            gate: NormalizedValue::new(0.6),
            swing: NormalizedValue::new(0.25),
            velocity: ArpVelocity::RampDown,
            latch: true,
            legato: true,
            continuous_phase: true,
            custom: ArpOffsets::new(&[0, 4, 7]),
        }));
        let json = serde_json::to_string(&p).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.processors(), p.processors());
    }

    #[test]
    fn arp_back_compat() {
        // An arp serialized before the `custom`/`legato` fields (and the
        // data-carrying rate variants) existed still deserializes — the new
        // fields fall back to their defaults — and equals a default arp.
        let json = r#"{"mode":"Up","rate":"Sixteenth","octaves":1,"gate":0.75,
                       "swing":0.0,"velocity":"AsPlayed","latch":false}"#;
        let a: Arpeggiator = serde_json::from_str(json).unwrap();
        assert!(!a.legato, "missing legato defaults to false");
        assert!(a.custom.is_empty(), "missing custom defaults to empty");
        assert_eq!(a, Arpeggiator::default(), "old arp == default arp");
    }

    // --- Chord (NP4) --------------------------------------------------------

    #[test]
    fn chord_replaces_source_note_with_tones() {
        // One C note → C major triad (C E G) at the same tick.
        let mut p = pattern_with_notes(&[60]);
        let _ = p.add_processor(NoteProcessor::Chord(Chord::major()));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 64, 67]);
    }

    #[test]
    fn chord_tones_inherit_source_note_fields() {
        // Velocity (and other fields) carry from the source note to every tone.
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        let id = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::FF);
        let _ = p.resize_note(id, Duration(480));
        let _ = p.add_processor(NoteProcessor::Chord(Chord::minor()));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 63, 67], "minor triad");
        assert!(
            buf.notes().iter().all(
                |n| (n.velocity.as_f32() - Velocity::FF.as_f32()).abs() < 1e-6
                    && n.duration == Some(Duration(480))
            ),
            "every tone inherits velocity + duration"
        );
    }

    #[test]
    fn chord_empty_interval_set_silences_the_note() {
        let mut p = pattern_with_notes(&[60]);
        let _ = p.add_processor(NoteProcessor::Chord(Chord::new(&[])));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert!(buf.notes().is_empty(), "no intervals → no tones");
    }

    #[test]
    fn chord_new_clamps_excess_intervals() {
        let c = Chord::new(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(c.intervals().len(), MAX_CHORD_INTERVALS);
        assert_eq!(c.intervals(), &[0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn chord_then_arp_arpeggiates_the_whole_chord() {
        // THE SEAM: a single source note + chord + arp must arpeggiate all
        // three chord tones, not replay the one source pitch. This only works
        // if the arp's held-notes view expands 1→N through the upstream chord.
        let mut p = chord_pattern_single(60, 3840); // one held C, no authored chord
        let _ = p.add_processor(NoteProcessor::Chord(Chord::major()));
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        // Canonical insertion: chord (stage 1) before arp (stage 2).
        assert_eq!(p.processors()[0].chain_stage(), 1);
        assert_eq!(p.processors()[1].chain_stage(), 2);
        let got: Vec<u8> = expand_all(&p, 720).into_iter().map(|(_, m)| m).collect();
        assert_eq!(
            got,
            vec![60, 64, 67],
            "arp must see all chord tones via the widened held-notes seam"
        );
    }

    #[test]
    fn quantize_then_chord_builds_in_key_from_snapped_root() {
        // C# source → quantize snaps to D → D major triad (D F# A = 62 66 69).
        let mut p = pattern_with_notes(&[61]);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let _ = p.add_processor(NoteProcessor::Chord(Chord::major()));
        assert_eq!(p.processors()[0].chain_stage(), 0, "quantize before chord");
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        let pitches: Vec<u8> = buf.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![62, 66, 69]);
    }

    #[test]
    fn chord_freeze_bakes_tones() {
        let mut p = pattern_with_notes(&[60]);
        let _ = p.add_processor(NoteProcessor::Chord(Chord::dominant7()));
        let count = p.freeze_processors(Bpm::DEFAULT, &[]);
        assert_eq!(count, 4, "dominant 7th = four tones");
        assert!(p.processors().is_empty());
        let pitches: Vec<u8> = p.notes().iter().map(|n| n.pitch.as_midi()).collect();
        assert_eq!(pitches, vec![60, 64, 67, 70]);
    }

    #[test]
    fn chord_serde_roundtrip() {
        let mut p = pattern_with_notes(&[60]);
        let _ = p.add_processor(NoteProcessor::Chord(Chord::new(&[-12, 0, 7, 12])));
        let json = serde_json::to_string(&p).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.processors(), p.processors());
    }

    // --- Strum (NP4b) -------------------------------------------------------

    #[test]
    fn strum_up_staggers_tones_low_to_high() {
        // C major triad strummed up, 30 ticks apart: C@0, E@30, G@60.
        let mut p = chord_pattern_single(60, 960);
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(30), StrumDirection::Up),
        ));
        let got = expand_all(&p, 200);
        assert_eq!(got, vec![(0, 60), (30, 64), (60, 67)]);
    }

    #[test]
    fn strum_down_staggers_tones_high_to_low() {
        let mut p = chord_pattern_single(60, 960);
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(30), StrumDirection::Down),
        ));
        let got = expand_all(&p, 200);
        assert_eq!(got, vec![(0, 67), (30, 64), (60, 60)]);
    }

    #[test]
    fn strum_zero_spread_is_a_block_chord() {
        // strum 0 must behave exactly like an un-strummed chord (block, same tick).
        let mut p = chord_pattern_single(60, 960);
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(0), StrumDirection::Up),
        ));
        let got = expand_all(&p, 200);
        assert_eq!(got, vec![(0, 60), (0, 64), (0, 67)]);
    }

    #[test]
    fn strum_tones_inherit_velocity_and_duration() {
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        let id = p.add_note(PatternTick(0), Pitch::new(60).unwrap(), Velocity::FF);
        let _ = p.resize_note(id, Duration(480));
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::minor().with_strum(Duration(20), StrumDirection::Up),
        ));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(40), |_| true, Bpm::DEFAULT, None, &mut buf); // third tone (G)
        assert_eq!(buf.notes().len(), 1);
        assert_eq!(buf.notes()[0].pitch.as_midi(), 67);
        assert!((buf.notes()[0].velocity.as_f32() - Velocity::FF.as_f32()).abs() < 1e-6);
        assert_eq!(buf.notes()[0].duration, Some(Duration(480)));
    }

    #[test]
    fn strum_under_quantize_builds_from_snapped_root() {
        // C# source → quantize → D → D-major strummed up: D@0 F#@30 A@60.
        let mut p = chord_pattern_single(61, 960);
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(30), StrumDirection::Up),
        ));
        let got = expand_all(&p, 200);
        assert_eq!(got, vec![(0, 62), (30, 66), (60, 69)]);
    }

    #[test]
    fn strum_then_arp_ignores_strum_timing() {
        // A downstream arp re-times everything: chord(strum) → arp arps all
        // three tones on the arp grid, the strum offsets having no effect.
        let mut p = chord_pattern_single(60, 3840);
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(30), StrumDirection::Up),
        ));
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        let got: Vec<u8> = expand_all(&p, 720).into_iter().map(|(_, m)| m).collect();
        assert_eq!(got, vec![60, 64, 67], "arp grid, not the strum grid");
    }

    #[test]
    fn strum_huge_spread_does_not_overflow() {
        // Regression: strum_tail = (tones-1) * spread must saturate — an
        // unbounded spread used to risk a u32 overflow panic on the audio
        // thread. Expansion at the onset still emits the first tone, no panic.
        let mut p = chord_pattern_single(60, 960);
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(u32::MAX), StrumDirection::Up),
        ));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes().len(), 1, "first tone at the onset, no panic");
        assert_eq!(buf.notes()[0].pitch.as_midi(), 60);
    }

    #[test]
    fn strum_freeze_bakes_staggered_onsets() {
        let mut p = chord_pattern_single(60, 960);
        let _ = p.add_processor(NoteProcessor::Chord(
            Chord::major().with_strum(Duration(30), StrumDirection::Up),
        ));
        let count = p.freeze_processors(Bpm::DEFAULT, &[]);
        assert_eq!(count, 3);
        let baked: Vec<(u32, u8)> = p
            .notes()
            .iter()
            .map(|n| (n.start.0, n.pitch.as_midi()))
            .collect();
        assert_eq!(baked, vec![(0, 60), (30, 64), (60, 67)]);
    }

    // --- Humanize (NP5) -----------------------------------------------------

    fn humanize(velocity: f32, gate: f32, seed: u64) -> Humanize {
        Humanize {
            velocity: NormalizedValue::new(velocity),
            gate: NormalizedValue::new(gate),
            seed,
        }
    }

    /// A run of identical notes, one per quarter, so humanize has uniform
    /// material to perturb.
    fn uniform_run(n: u32) -> Pattern {
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        for i in 0..n {
            let id = p.add_note(PatternTick(i * 960), Pitch::new(60).unwrap(), Velocity::MF);
            let _ = p.resize_note(id, Duration(480));
        }
        p
    }

    #[test]
    fn humanize_is_deterministic_for_a_seed() {
        let mut p = uniform_run(8);
        let _ = p.add_processor(NoteProcessor::Humanize(humanize(0.3, 0.3, 42)));
        // Capture velocity+duration at each onset across two passes.
        let snapshot = |p: &Pattern| -> Vec<(u8, u32)> {
            let mut buf = ExpansionBuffer::new();
            let mut out = Vec::new();
            for i in 0..8 {
                p.expand_at_tick(PatternTick(i * 960), |_| true, Bpm::DEFAULT, None, &mut buf);
                for note in buf.notes() {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    out.push((
                        (note.velocity.as_f32() * 1000.0) as u8,
                        note.duration.unwrap().0,
                    ));
                }
            }
            out
        };
        assert_eq!(snapshot(&p), snapshot(&p), "same seed → identical render");
    }

    #[test]
    fn humanize_seed_changes_the_result() {
        let mut a = uniform_run(8);
        let _ = a.add_processor(NoteProcessor::Humanize(humanize(0.3, 0.0, 1)));
        let mut b = uniform_run(8);
        let _ = b.add_processor(NoteProcessor::Humanize(humanize(0.3, 0.0, 2)));
        let vels = |p: &Pattern| -> Vec<f32> {
            let mut buf = ExpansionBuffer::new();
            let mut out = Vec::new();
            for i in 0..8 {
                p.expand_at_tick(PatternTick(i * 960), |_| true, Bpm::DEFAULT, None, &mut buf);
                out.extend(buf.notes().iter().map(|n| n.velocity.as_f32()));
            }
            out
        };
        assert_ne!(vels(&a), vels(&b), "different seeds → different feel");
    }

    #[test]
    fn humanize_velocity_stays_within_bounds() {
        let mut p = uniform_run(16);
        let amt = 0.25;
        let _ = p.add_processor(NoteProcessor::Humanize(humanize(amt, 0.0, 7)));
        let base = Velocity::MF.as_f32();
        let mut buf = ExpansionBuffer::new();
        for i in 0..16 {
            p.expand_at_tick(PatternTick(i * 960), |_| true, Bpm::DEFAULT, None, &mut buf);
            for note in buf.notes() {
                let v = note.velocity.as_f32();
                // ±amt of the base, modulo the 0..1 clamp.
                assert!(
                    v <= base * (1.0 + amt) + 1e-6 && v >= (base * (1.0 - amt) - 1e-6).max(0.0),
                    "velocity {v} outside ±{amt} of {base}"
                );
            }
        }
    }

    #[test]
    fn humanize_zero_amounts_are_a_noop() {
        let mut p = uniform_run(4);
        let _ = p.add_processor(NoteProcessor::Humanize(humanize(0.0, 0.0, 99)));
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes()[0].velocity.as_f32(), Velocity::MF.as_f32());
        assert_eq!(buf.notes()[0].duration, Some(Duration(480)));
    }

    #[test]
    fn humanize_runs_last_perturbing_arp_steps() {
        // quantize → arp → humanize: humanize sits at stage 4 and reshapes the
        // arp's generated step velocities (not the source chord).
        let mut p = chord_pattern(3840);
        let _ = p.add_processor(NoteProcessor::Arpeggiator(arp(ArpMode::Up, 1)));
        let _ = p.add_processor(NoteProcessor::Humanize(humanize(0.4, 0.0, 5)));
        assert_eq!(p.processors()[1].chain_stage(), 4, "humanize is last");
        // Collect arp-step velocities; with a strong humanize they must vary
        // across steps (a plain arp would repeat the source velocity).
        let mut buf = ExpansionBuffer::new();
        let mut vels = Vec::new();
        for i in 0..8 {
            p.expand_at_tick(PatternTick(i * 240), |_| true, Bpm::DEFAULT, None, &mut buf);
            vels.extend(buf.notes().iter().map(|n| n.velocity.as_f32()));
        }
        assert!(
            vels.windows(2).any(|w| (w[0] - w[1]).abs() > 1e-4),
            "humanize must vary arp-step velocities: {vels:?}"
        );
    }

    #[test]
    fn humanize_gate_perturbs_duration() {
        let mut p = uniform_run(8);
        let _ = p.add_processor(NoteProcessor::Humanize(humanize(0.0, 0.5, 3)));
        let mut buf = ExpansionBuffer::new();
        let mut durations = Vec::new();
        for i in 0..8 {
            p.expand_at_tick(PatternTick(i * 960), |_| true, Bpm::DEFAULT, None, &mut buf);
            durations.extend(buf.notes().iter().map(|n| n.duration.unwrap().0));
        }
        assert!(
            durations.iter().any(|&d| d != 480),
            "gate humanize must vary note lengths: {durations:?}"
        );
        assert!(durations.iter().all(|&d| d >= 1), "duration floored at 1");
    }

    #[test]
    fn humanize_serde_roundtrip() {
        let mut p = pattern_with_notes(&[60]);
        let _ = p.add_processor(NoteProcessor::Humanize(humanize(0.2, 0.1, 0xDEAD_BEEF)));
        let json = serde_json::to_string(&p).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.processors(), p.processors());
    }

    // --- Ornaments (NP3) ----------------------------------------------------

    /// A single note at `start` lasting `dur`, with the given ornament.
    fn ornamented(start: u32, midi: u8, dur: u32, orn: Ornament) -> Pattern {
        let mut p = Pattern::new(PatternId(0), Duration(3840));
        let id = p.add_note(PatternTick(start), Pitch::new(midi).unwrap(), Velocity::MF);
        let _ = p.resize_note(id, Duration(dur));
        if let Some(n) = p.note_mut(id) {
            n.ornament = Some(orn);
        }
        p
    }

    fn flam() -> Ornament {
        Ornament {
            count: 2,
            spacing: Duration(120),
            spacing_curve: OrnamentSpacing::Even,
            dynamics: OrnamentDynamics::Flat,
            placement: OrnamentPlacement::LeadIn,
            pitch_offset: Semitones::new(0.0),
            grace_gate: NormalizedValue::new(0.5),
        }
    }

    #[test]
    fn ornament_count_below_two_is_a_noop() {
        let p = ornamented(480, 60, 240, Ornament { count: 1, ..flam() });
        // Plays as a single plain note at its own start, nowhere else.
        let got = expand_all(&p, 960);
        assert_eq!(got, vec![(480, 60)]);
    }

    #[test]
    fn ornament_lead_in_main_lands_on_start() {
        // count 2, spacing 120, lead-in: grace at 360, main at 480.
        let p = ornamented(480, 60, 240, flam());
        let got = expand_all(&p, 960);
        assert_eq!(
            got,
            vec![(360, 60), (480, 60)],
            "lead-in grace precedes the beat; main lands on the note start"
        );
    }

    #[test]
    fn ornament_on_beat_trails_the_start() {
        let p = ornamented(
            0,
            60,
            240,
            Ornament {
                count: 3,
                placement: OrnamentPlacement::OnBeat,
                ..flam()
            },
        );
        // Main on 0, graces trailing at 120 and 240.
        let got = expand_all(&p, 600);
        assert_eq!(got, vec![(0, 60), (120, 60), (240, 60)]);
    }

    #[test]
    fn ornament_grace_takes_pitch_offset_main_keeps_pitch() {
        // Acciaccatura: grace a tone above (62), main at 60.
        let p = ornamented(
            240,
            60,
            240,
            Ornament {
                pitch_offset: Semitones::new(2.0),
                ..flam()
            },
        );
        let got = expand_all(&p, 480);
        assert_eq!(got, vec![(120, 62), (240, 60)]);
    }

    #[test]
    fn ornament_lead_in_grace_before_tick_zero_is_dropped() {
        // Note at tick 60, lead-in grace would land at 60 - 120 = -60 → dropped;
        // only the main hit survives, on the beat.
        let p = ornamented(60, 60, 240, flam());
        let got = expand_all(&p, 480);
        assert_eq!(got, vec![(60, 60)]);
    }

    #[test]
    fn ornament_main_keeps_duration_grace_is_short() {
        let p = ornamented(480, 60, 960, flam());
        let mut buf = ExpansionBuffer::new();
        // Grace at 360: short (grace_gate 0.5 × spacing 120 = 60 ticks).
        p.expand_at_tick(PatternTick(360), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes()[0].duration, Some(Duration(60)));
        // Main at 480: keeps the source note's full duration.
        p.expand_at_tick(PatternTick(480), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes()[0].duration, Some(Duration(960)));
    }

    #[test]
    fn ornament_roll_is_bounded_by_hit_cap() {
        // A roll asking for more than the cap clamps; never unbounded.
        let p = ornamented(
            0,
            60,
            3840,
            Ornament {
                count: u8::MAX,
                spacing: Duration(10),
                placement: OrnamentPlacement::OnBeat,
                ..flam()
            },
        );
        let got = expand_all(&p, 3840);
        assert_eq!(
            got.len() as u8,
            MAX_ORNAMENT_HITS,
            "roll must clamp to the hit cap"
        );
    }

    #[test]
    fn ornament_crescendo_rises_across_the_figure() {
        let p = ornamented(
            0,
            60,
            240,
            Ornament {
                count: 4,
                placement: OrnamentPlacement::OnBeat,
                dynamics: OrnamentDynamics::Crescendo,
                ..flam()
            },
        );
        let mut buf = ExpansionBuffer::new();
        let mut vels = Vec::new();
        for t in [0u32, 120, 240, 360] {
            p.expand_at_tick(PatternTick(t), |_| true, Bpm::DEFAULT, None, &mut buf);
            vels.push(buf.notes()[0].velocity.as_f32());
        }
        assert!(
            vels.windows(2).all(|w| w[0] < w[1]),
            "crescendo must rise monotonically: {vels:?}"
        );
    }

    #[test]
    fn ornament_accelerate_shrinks_the_gaps() {
        // 4 hits, on-beat, accelerate: gaps must be non-increasing and the
        // figure still ends at start + total_span (3 × 120 = 360).
        let p = ornamented(
            0,
            60,
            240,
            Ornament {
                count: 4,
                placement: OrnamentPlacement::OnBeat,
                spacing_curve: OrnamentSpacing::Accelerate,
                ..flam()
            },
        );
        let ticks: Vec<u32> = expand_all(&p, 600).into_iter().map(|(t, _)| t).collect();
        assert_eq!(ticks.first(), Some(&0));
        assert_eq!(ticks.last(), Some(&360), "endpoint pinned to total span");
        let gaps: Vec<u32> = ticks.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(
            gaps.windows(2).all(|w| w[0] >= w[1]),
            "accelerating gaps must not grow: {gaps:?}"
        );
    }

    #[test]
    fn ornament_freeze_bakes_and_drops_the_field() {
        let mut p = ornamented(480, 60, 240, flam());
        let count = p.freeze_processors(Bpm::DEFAULT, &[]);
        assert_eq!(count, 2, "grace + main baked to concrete notes");
        let baked: Vec<(u32, u8)> = p
            .notes()
            .iter()
            .map(|n| (n.start.0, n.pitch.as_midi()))
            .collect();
        assert_eq!(baked, vec![(360, 60), (480, 60)]);
        assert!(
            p.notes().iter().all(|n| n.ornament.is_none()),
            "baked notes are plain — no residual ornament"
        );
    }

    #[test]
    fn ornament_under_quantize_snaps_grace_into_key() {
        // Grace +1 semitone (C# = 61) under C-major quantize snaps to D (62).
        let mut p = ornamented(
            240,
            60,
            240,
            Ornament {
                pitch_offset: Semitones::new(1.0),
                ..flam()
            },
        );
        let _ = p.add_processor(NoteProcessor::ScaleQuantize(c_major()));
        let got = expand_all(&p, 480);
        assert_eq!(got, vec![(120, 62), (240, 60)], "grace snapped into key");
    }

    #[test]
    fn ornament_curve_never_collides_two_hits_on_one_tick() {
        // Regression: a fast curve with tiny spacing used to round adjacent
        // onsets onto the same tick (Accelerate count=3 spacing=1 → [0,2,2]),
        // emitting the main hit + a grace at one tick. Same-pitch collisions
        // then make the grace's short NoteOff cut the main note short. Onsets
        // must be strictly increasing and the main hit must still land on start.
        for curve in [OrnamentSpacing::Accelerate, OrnamentSpacing::Decelerate] {
            let p = ornamented(
                480,
                60,
                240,
                Ornament {
                    count: 3,
                    spacing: Duration(1),
                    spacing_curve: curve,
                    ..flam()
                },
            );
            let got = expand_all(&p, 960);
            let ticks: Vec<u32> = got.iter().map(|(t, _)| *t).collect();
            let mut sorted = ticks.clone();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(ticks.len(), 3, "exactly three distinct hits: {got:?}");
            assert_eq!(sorted.len(), 3, "no two hits share a tick: {got:?}");
            assert_eq!(
                ticks.last(),
                Some(&480),
                "lead-in main still lands on the beat: {got:?}"
            );
        }
    }

    #[test]
    fn ornament_huge_spacing_does_not_overflow() {
        // Regression: total_span = (count-1) * spacing overflowed u32 and
        // panicked on the audio thread. i64 math must absorb it; expansion at
        // the anchor still emits the main hit, nowhere does it panic.
        let p = ornamented(
            0,
            60,
            240,
            Ornament {
                count: MAX_ORNAMENT_HITS,
                spacing: Duration(u32::MAX),
                placement: OrnamentPlacement::OnBeat,
                ..flam()
            },
        );
        let mut buf = ExpansionBuffer::new();
        p.expand_at_tick(PatternTick(0), |_| true, Bpm::DEFAULT, None, &mut buf);
        assert_eq!(buf.notes().len(), 1, "main hit at the anchor, no panic");
        assert_eq!(buf.notes()[0].pitch.as_midi(), 60);
    }

    #[test]
    fn ornament_serde_roundtrip() {
        let p = ornamented(
            480,
            60,
            240,
            Ornament {
                count: 5,
                spacing: Duration(80),
                spacing_curve: OrnamentSpacing::Decelerate,
                dynamics: OrnamentDynamics::Decrescendo,
                placement: OrnamentPlacement::OnBeat,
                pitch_offset: Semitones::new(-1.0),
                grace_gate: NormalizedValue::new(0.3),
            },
        );
        let json = serde_json::to_string(&p).unwrap();
        let back: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(back.notes(), p.notes());
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
